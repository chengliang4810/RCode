// OpenAI Responses API support is new for RCode.
//! OpenAI **Responses** API provider (`POST /v1/responses`).
//!
//! This is a different protocol from Chat Completions, not a variation of it.
//! The differences that matter to the callers of this crate:
//!
//! - the conversation arrives as a flat `input` array rather than `messages`,
//!   with a system prompt carried separately as `instructions`;
//! - output items are tagged (`message`, `function_call`, `reasoning`) rather
//!   than a single assistant message with a `tool_calls` array;
//! - a tool call is a top-level item with a `call_id`, not a nested object.
//!
//! Everything else — the HTTP/SSE transport, retries, cancellation and the
//! `ModelStreamEvent` surface — is shared with the other providers.

mod request;
mod response;
mod stream;

#[cfg(test)]
mod tests;

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{
    ModelCapabilities, ModelError, ModelRequest, ModelResult, ModelRuntimeConfig, ModelStream,
    PreparedModelRequest,
    runtime::stream::runtime_http_sse_stream,
    transport::{HttpRequest, HttpTransport, ReqwestTransport},
};

use request::BuiltResponsesRequest;

const PROVIDER_NAME: &str = "openai-responses";
const DEFAULT_MAX_TOKENS: u32 = 32_000;

/// Strongly-typed configuration for the OpenAI Responses API.
///
/// Credentials live here and nowhere else; the `Debug` impl masks them.
pub struct OpenAiResponsesConfig {
    endpoint: Url,
    api_key: String,
    model: String,
    reasoning_effort: Option<String>,
    max_tokens: u32,
    context_window: Option<u32>,
    runtime: ModelRuntimeConfig,
}

impl OpenAiResponsesConfig {
    /// Build a config explicitly. Environment lookup belongs to the host
    /// application, not to the protocol crate.
    pub fn new(endpoint: Url, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            endpoint,
            api_key: api_key.into(),
            model: model.into(),
            reasoning_effort: None,
            max_tokens: DEFAULT_MAX_TOKENS,
            context_window: None,
            runtime: ModelRuntimeConfig::default(),
        }
    }

    pub fn with_reasoning_effort(mut self, effort: impl Into<String>) -> Self {
        self.reasoning_effort = Some(effort.into());
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// The context window resolved by the assembler. Left unset means unknown —
    /// this crate never guesses a window from a model name.
    pub fn with_context_window(mut self, context_window: Option<u32>) -> Self {
        self.context_window = context_window;
        self
    }

    pub fn with_runtime(mut self, runtime: ModelRuntimeConfig) -> Self {
        self.runtime = runtime;
        self
    }
}

// SECURITY: unlike the ported Chat Completions config, this `Debug` masks the
// API key. The port kept the original project's unmasked impl to avoid a silent behaviour
// change in code that already shipped; this module is new, so there is no such
// history and no reason to repeat the leak. Anything that logs a config now
// gets `"<redacted>"` and has to opt in explicitly to see the key.
impl fmt::Debug for OpenAiResponsesConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiResponsesConfig")
            .field("endpoint", &self.endpoint)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("max_tokens", &self.max_tokens)
            .field("context_window", &self.context_window)
            .field("runtime", &self.runtime)
            .finish()
    }
}

/// An OpenAI Responses API model.
pub struct OpenAiResponsesModel {
    config: OpenAiResponsesConfig,
    transport: Arc<dyn HttpTransport>,
    client: reqwest::Client,
}

impl OpenAiResponsesModel {
    pub fn new(config: OpenAiResponsesConfig) -> Self {
        let client = crate::transport::http::model_http_client();
        Self {
            config,
            transport: Arc::new(ReqwestTransport::new(client.clone())),
            client,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_transport(
        config: OpenAiResponsesConfig,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        Self {
            config,
            transport,
            client: reqwest::Client::new(),
        }
    }

    fn build_request(&self, request: &ModelRequest) -> ModelResult<BuiltResponsesRequest> {
        request::build_request(&self.config, request)
    }

    fn native_http_request(
        client: &reqwest::Client,
        api_key: &str,
        built: &BuiltResponsesRequest,
    ) -> ModelResult<HttpRequest> {
        let request = client
            .post(built.endpoint.clone())
            .bearer_auth(api_key)
            .json(&built.body)
            .build()
            .map_err(|error| {
                ModelError::protocol(crate::ProtocolErrorKind::Provider).with_error(&error)
            })?;
        Ok(HttpRequest::new(request))
    }
}

#[async_trait]
impl crate::Model for OpenAiResponsesModel {
    fn prepare_stream(&self, request: ModelRequest) -> ModelResult<crate::PreparedModelCall> {
        let built = Arc::new(self.build_request(&request)?);
        let checkpoint =
            crate::protocol::prepared::checkpoint(PROVIDER_NAME, &built.endpoint, &built.body)?;
        let client = self.client.clone();
        let api_key = self.config.api_key.clone();
        let request_factory =
            Arc::new(move || Self::native_http_request(&client, &api_key, &built));
        let runtime = self.config.runtime.clone();
        let transport = Arc::clone(&self.transport);
        Ok(crate::PreparedModelCall::new(
            checkpoint,
            move |cancellation| {
                Ok(runtime_http_sse_stream(
                    &runtime,
                    cancellation,
                    transport,
                    request_factory,
                    Arc::<str>::from(PROVIDER_NAME),
                    stream::decoders(),
                ))
            },
        ))
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            supports_tools: true,
            supports_reasoning: true,
            supports_vision: true,
            supports_streaming: true,
        }
    }

    fn output_token_limit(&self) -> Option<u32> {
        Some(self.config.max_tokens)
    }

    fn context_window(&self) -> Option<u32> {
        self.config.context_window
    }

    fn prepare_request(&self, request: &ModelRequest) -> ModelResult<PreparedModelRequest> {
        self.build_request(request)?.observe(&self.config.runtime)
    }

    async fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelResult<ModelStream> {
        if cancellation.is_cancelled() {
            return Err(ModelError::cancelled());
        }
        let built = Arc::new(self.build_request(&request)?);
        let client = self.client.clone();
        let api_key = self.config.api_key.clone();
        let request_factory = {
            let built = Arc::clone(&built);
            Arc::new(move || Self::native_http_request(&client, &api_key, &built))
        };

        Ok(runtime_http_sse_stream(
            &self.config.runtime,
            cancellation,
            Arc::clone(&self.transport),
            request_factory,
            Arc::<str>::from(PROVIDER_NAME),
            stream::decoders(),
        ))
    }
}
