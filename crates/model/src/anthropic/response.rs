// Modified for RCode.
use crate::{ModelError, StopReason};

pub(super) fn stop_reason(value: Option<&str>) -> StopReason {
    match value.unwrap_or("end_turn") {
        "end_turn" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        value => StopReason::Other {
            value: value.into(),
        },
    }
}

#[track_caller]
pub(super) fn provider_protocol_error() -> ModelError {
    ModelError::protocol(crate::ProtocolErrorKind::Provider).with_message(format!(
        "invalid provider payload at {}",
        std::panic::Location::caller()
    ))
}
