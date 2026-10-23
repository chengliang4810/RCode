// Modified for RCode.
use std::{collections::VecDeque, sync::Arc, time::Duration};

const ERROR_BODY_READ_TIMEOUT: Duration = Duration::from_secs(2);

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::transport::{HttpRequest, HttpResponse, HttpTransport, SseEvent, SseParser};
use crate::{
    ModelError, ModelResult, ModelRuntimeConfig, ModelStream, ModelStreamEvent, RetryConfig,
    RetryObserver,
};

use super::retry::{StreamAttempt, retrying_stream};

/// Provider decoder 使用的 crate-private SSE 事件转换器。
///
/// The decoder returns the events a frame produced. `Completed` is terminal by
/// definition (see `ModelStreamEvent`), so a provider that ends on its own
/// frame — the Responses API's `response.completed` — emits it here and the
/// runtime stops reading, rather than waiting for the `[DONE]` sentinel that
/// belongs to Chat Completions.
pub(crate) type SseDecoder =
    Arc<dyn Fn(SseEvent, Option<String>) -> ModelResult<Vec<ModelStreamEvent>> + Send + Sync>;
/// Called when the stream ends *without* a terminal frame — EOF, or the
/// `[DONE]` sentinel. A provider whose terminal frame already produced
/// `Completed` never reaches this.
pub(crate) type SseCompletionDecoder =
    Arc<dyn Fn() -> ModelResult<Vec<ModelStreamEvent>> + Send + Sync>;
pub(crate) type SseDecoders = (SseDecoder, SseCompletionDecoder);
pub(crate) type SseDecoderFactory = Arc<dyn Fn() -> SseDecoders + Send + Sync>;

/// 将 crate-private HTTP seam、SSE parser、provider decoder 与通用 retry 串为一条可取消的流。
///
/// request factory 与 decoder 由后续 provider adapter 提供；本函数不认识 provider-native payload。
fn observed_http_sse_stream(
    config: RetryConfig,
    cancellation: CancellationToken,
    observer: Option<Arc<dyn RetryObserver>>,
    transport: Arc<dyn HttpTransport>,
    request: Arc<dyn Fn() -> ModelResult<HttpRequest> + Send + Sync>,
    provider: Arc<str>,
    decoders: SseDecoderFactory,
    diagnostics: Option<Arc<dyn crate::diagnostics::DiagnosticsObserver>>,
) -> ModelStream {
    let attempt: StreamAttempt = Arc::new(move |attempt_cancellation| {
        let transport = transport.clone();
        let request = request.clone();
        let provider = provider.clone();
        let diagnostics = diagnostics.clone();
        let (decoder, completion_decoder) = decoders();
        Box::pin(async move {
            let request = request()?;
            let trace = diagnostics.and_then(|observer| crate::diagnostics::AttemptTrace::start(
                observer, &request.request, &provider, attempt_cancellation.clone(),
            ));
            let response = match transport
                .send(request, attempt_cancellation.clone())
                .await {
                    Ok(response) => response,
                    Err(error) => {
                        if let Some(trace) = &trace { trace.failed(&error); }
                        return Err(error);
                    }
                };
            if let Some(trace) = &trace { trace.headers(response.status, response.request_id.as_deref()); }
            let decoder: SseDecoder = if let Some(trace) = &trace {
                let trace = trace.clone();
                Arc::new(move |event, request_id| {
                    trace.payload(&event.data);
                    decoder(event, request_id)
                })
            } else { decoder };
            let result = response_to_sse_stream(
                response,
                attempt_cancellation.clone(),
                provider,
                decoder,
                completion_decoder,
            )
            .await;
            match (result, trace) {
                (Ok(stream), Some(trace)) => Ok(trace.stream(stream, attempt_cancellation)),
                (Err(error), Some(trace)) => { trace.failed(&error); Err(error) }
                (result, None) => result,
            }
        })
    });
    retrying_stream(config, cancellation, observer, attempt)
}

/// 使用运行时配置构造 HTTP/SSE/retry 链路，确保上层注册的安全 observer 生效。
pub(crate) fn runtime_http_sse_stream(
    runtime: &ModelRuntimeConfig,
    cancellation: CancellationToken,
    transport: Arc<dyn HttpTransport>,
    request: Arc<dyn Fn() -> ModelResult<HttpRequest> + Send + Sync>,
    provider: Arc<str>,
    decoders: SseDecoderFactory,
) -> ModelStream {
    observed_http_sse_stream(
        runtime.retry().clone(),
        cancellation,
        runtime.retry_observer(),
        transport,
        request,
        provider,
        decoders,
        runtime.diagnostics(),
    )
}

async fn response_to_sse_stream(
    response: HttpResponse,
    cancellation: CancellationToken,
    provider: Arc<str>,
    decoder: SseDecoder,
    completion_decoder: SseCompletionDecoder,
) -> ModelResult<ModelStream> {
    if !(200..=299).contains(&response.status) {
        return Err(
            read_http_error(response, cancellation, provider, ERROR_BODY_READ_TIMEOUT).await,
        );
    }

    let stream_provider = provider.clone();
    let events = futures::stream::unfold(
        SseReadState {
            body: response.body,
            parser: SseParser::new(),
            pending: VecDeque::new(),
            cancellation: cancellation.clone(),
            decoder,
            completion_decoder,
            request_id: response.request_id,
            done: false,
        },
        move |mut state| {
            let provider = stream_provider.clone();
            async move {
                loop {
                    if let Some(event) = state.pending.pop_front() {
                        return Some((event, state));
                    }
                    if state.done {
                        return None;
                    }
                    let chunk = tokio::select! {
                        biased;
                        _ = state.cancellation.cancelled() => return Some((Err(ModelError::cancelled()), state)),
                        chunk = state.body.next() => chunk,
                    };
                    match chunk {
                        Some(Ok(bytes)) => {
                            let events = state.parser.push(&bytes);
                            state.decode(events);
                        }
                        Some(Err(error)) => return Some((Err(error), state)),
                        None => {
                            let events = state.parser.finish();
                            state.decode(events);
                            if state.done {
                                continue;
                            }
                            tracing::warn!(provider = %provider,
                                request_id = state.request_id.as_deref(),
                                "HTTP response ended before SSE completion");
                            state.done = true;
                            state.pending.push_back(Err(ModelError::transport(
                                crate::TransportErrorKind::Other,
                                Some(provider.as_ref()),
                            )
                            .with_message("HTTP response ended before SSE completion")));
                        }
                    }
                }
            }
        },
    );
    Ok(ModelStream::with_cancellation(
        events,
        cancellation.child_token(),
    ))
}

async fn read_http_error(
    mut response: HttpResponse,
    cancellation: CancellationToken,
    provider: Arc<str>,
    budget: Duration,
) -> ModelError {
    let mut error = ModelError::http_status(
        response.status,
        provider.as_ref(),
        response.request_id.as_deref(),
    );
    // Ported adjustment: The original project's `time` boundary crate is not ported; use tokio's
    // monotonic clock directly. `sleep_until` below is likewise tokio's timer.
    let deadline = tokio::time::Instant::now() + budget;
    let mut bytes = Vec::new();
    let limit = super::error::MAX_DIAGNOSTIC_BYTES + 1;
    while bytes.len() < limit {
        let chunk = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return ModelError::cancelled(),
            _ = tokio::time::sleep_until(deadline) => {
                error = error.with_causes(vec![format!(
                    "HTTP error body read timed out after {} ms", budget.as_millis(),
                )]);
                break;
            }
            chunk = response.body.next() => chunk,
        };
        match chunk {
            Some(Ok(chunk)) => {
                let remaining = limit - bytes.len();
                bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                tokio::task::yield_now().await;
            }
            Some(Err(cause)) if cause.is_cancelled() => return cause,
            Some(Err(cause)) => {
                let diagnostic = cause.diagnostic();
                error = error.with_causes(
                    std::iter::once(cause.to_string()).chain(diagnostic.causes().iter().cloned()),
                );
                break;
            }
            None => break,
        }
    }
    if cancellation.is_cancelled() {
        return ModelError::cancelled();
    }
    error.with_body(String::from_utf8_lossy(&bytes))
}

struct SseReadState {
    body: crate::transport::HttpBody,
    parser: SseParser,
    pending: VecDeque<ModelResult<ModelStreamEvent>>,
    cancellation: CancellationToken,
    decoder: SseDecoder,
    completion_decoder: SseCompletionDecoder,
    request_id: Option<String>,
    done: bool,
}

impl SseReadState {
    fn decode(&mut self, events: Vec<ModelResult<SseEvent>>) {
        for event in events {
            match event.and_then(|event| (self.decoder)(event, self.request_id.clone())) {
                Ok(events) => {
                    for event in events {
                        self.done = matches!(event, ModelStreamEvent::Completed(_));
                        self.pending.push_back(Ok(event));
                        if self.done {
                            break;
                        }
                    }
                }
                Err(error) => {
                    // Deliver preceding deltas before the error, including at EOF.
                    self.pending.push_back(Err(error));
                    self.done = true;
                }
            }
            if self.done {
                break;
            }
        }
        if !self.done && self.parser.is_done() {
            match (self.completion_decoder)() {
                Ok(events) => self.pending.extend(events.into_iter().map(Ok)),
                Err(error) => self.pending.push_back(Err(error)),
            }
            self.done = true;
        }
    }
}
