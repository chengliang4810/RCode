// Modified for RCode.
//! 与模型提供商无关的协议 DTO 和流式优先模型接口。

pub mod anthropic;
pub mod diagnostics;
mod endpoint;
pub mod openai_compatible;
pub mod openai_responses;
pub mod prompt_cache;
pub mod protocol;
pub mod runtime;
mod transport;

#[cfg(test)]
mod live_test;

pub use anthropic::{AnthropicConfig, AnthropicModel};
pub use openai_compatible::{OpenAiConfig, OpenAiModel};
pub use openai_responses::{OpenAiResponsesConfig, OpenAiResponsesModel};
pub use protocol::{
    ContentBlock, DocumentSource, ImageSource, JsonObject, MediaType, Model, ModelCapabilities,
    ModelMessage, ModelRequest, ModelResponse, ModelStream, ModelStreamEvent, PreparedModelCall,
    ProviderProtocol, StopReason, TokenUsage, ToolCall, ToolDefinition, ToolResult,
};
pub use runtime::{
    ModelError, ModelErrorCategory, ModelErrorDiagnostic, ModelErrorDiagnosticParts, ModelResult,
    ModelRuntimeConfig, ObservedProviderBody, PreparedModelRequest, ProtocolError,
    ProtocolErrorKind, RetryConfig, RetryErrorKind, RetryObservation, RetryObserver,
    RetryableErrorClasses, TransportErrorKind,
};
