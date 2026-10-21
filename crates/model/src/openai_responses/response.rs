// OpenAI Responses API support is new for RCode.
//!
//! Usage and stop-reason decoding for the Responses API.
//!
//! Both differ from Chat Completions in ways that would silently produce wrong
//! numbers if copied: usage is `input_tokens`/`output_tokens` rather than
//! `prompt_tokens`/`completion_tokens`, and the stop reason comes from a
//! `status`/`incomplete_details.reason` pair rather than a `finish_reason`.

use serde_json::Value;

use crate::{ModelError, StopReason, TokenUsage};

pub(super) fn decode_usage(value: &Value) -> Option<TokenUsage> {
    let input_tokens = value.get("input_tokens")?.as_u64()?.try_into().ok()?;
    let output_tokens = value.get("output_tokens")?.as_u64()?.try_into().ok()?;
    let cache_read_input_tokens = value
        .get("input_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64)
        .and_then(|tokens| tokens.try_into().ok());
    Some(TokenUsage {
        input_tokens,
        output_tokens,
        cache_creation_input_tokens: None,
        cache_read_input_tokens,
    })
}

/// Map the Responses API's terminal state onto a `StopReason`.
///
/// `status` is the response's own field (`completed` / `incomplete`); when it
/// is `incomplete`, `incomplete_reason` says why (`max_output_tokens`,
/// `content_filter`).
pub(super) fn stop_reason(status: &str, incomplete_reason: Option<&str>) -> StopReason {
    match status {
        "completed" => StopReason::EndTurn,
        "incomplete" => match incomplete_reason {
            Some("max_output_tokens") => StopReason::MaxTokens,
            Some(reason) => StopReason::Other {
                value: reason.to_owned(),
            },
            None => StopReason::Other {
                value: "incomplete".to_owned(),
            },
        },
        other => StopReason::Other {
            value: other.to_owned(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn usage_reads_the_responses_field_names() {
        let usage = decode_usage(&json!({
            "input_tokens": 120,
            "output_tokens": 34,
            "input_tokens_details": { "cached_tokens": 100 }
        }))
        .expect("decodes");

        assert_eq!(usage.input_tokens, 120);
        assert_eq!(usage.output_tokens, 34);
        assert_eq!(usage.cache_read_input_tokens, Some(100));
    }

    #[test]
    fn chat_completions_field_names_are_not_accepted() {
        // `prompt_tokens` is Chat Completions vocabulary; reading it here would
        // mean the wrong protocol was wired up.
        assert!(
            decode_usage(&json!({
                "prompt_tokens": 1,
                "completion_tokens": 2
            }))
            .is_none()
        );
    }

    #[test]
    fn a_completed_response_ends_the_turn() {
        assert_eq!(stop_reason("completed", None), StopReason::EndTurn);
    }

    #[test]
    fn a_token_capped_response_reports_max_tokens() {
        assert_eq!(
            stop_reason("incomplete", Some("max_output_tokens")),
            StopReason::MaxTokens
        );
    }

    #[test]
    fn a_filtered_response_keeps_its_reason() {
        assert_eq!(
            stop_reason("incomplete", Some("content_filter")),
            StopReason::Other {
                value: "content_filter".to_owned()
            }
        );
    }

    #[test]
    fn an_incomplete_response_without_a_reason_is_still_other() {
        assert_eq!(
            stop_reason("incomplete", None),
            StopReason::Other {
                value: "incomplete".to_owned()
            }
        );
    }
}
