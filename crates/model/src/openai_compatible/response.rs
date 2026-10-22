// Modified for RCode.
use serde_json::Value;

use crate::{
    ContentBlock, JsonObject, ModelError, ModelMessage, ModelResponse, ModelResult, StopReason,
    TokenUsage, ToolCall,
};

#[allow(dead_code)]
pub(super) fn decode_completed_response(value: &Value) -> ModelResult<ModelResponse> {
    decode_completed_response_inner(value).map_err(|error| error.with_body(value.to_string()))
}

fn decode_completed_response_inner(value: &Value) -> ModelResult<ModelResponse> {
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(provider_protocol_error)?;
    let message = choice.get("message").ok_or_else(provider_protocol_error)?;
    let (content, tool_calls) = decode_assistant_message(message)?;
    let stop_reason = stop_reason(choice.get("finish_reason").and_then(Value::as_str));
    let usage = value.get("usage").and_then(decode_usage);
    let request_id = value.get("id").and_then(Value::as_str).map(str::to_owned);
    ModelResponse::new(
        ModelMessage::assistant(content, tool_calls),
        stop_reason,
        usage,
        request_id,
    )
}

pub(super) fn decode_assistant_message(
    value: &Value,
) -> ModelResult<(Vec<ContentBlock>, Vec<ToolCall>)> {
    let mut content = Vec::new();
    let top_level_reasoning = value
        .get("reasoning_content")
        .and_then(Value::as_str)
        .filter(|reasoning| !reasoning.is_empty())
        .or_else(|| {
            value
                .get("reasoning")
                .and_then(Value::as_str)
                .filter(|reasoning| !reasoning.is_empty())
        });
    if let Some(reasoning) = top_level_reasoning {
        content.push(ContentBlock::reasoning(reasoning));
    }

    match value.get("content") {
        Some(Value::String(text)) if !text.is_empty() => content.push(ContentBlock::text(text)),
        Some(Value::Array(parts)) => {
            for part in parts {
                match part.get("type").and_then(Value::as_str) {
                    Some("thinking") if top_level_reasoning.is_none() => {
                        if let Some(thinking) = part.get("thinking").and_then(Value::as_str) {
                            if !thinking.is_empty() {
                                content.push(ContentBlock::reasoning(thinking));
                            }
                        }
                    }
                    Some("text") => {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                content.push(ContentBlock::text(text));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Some(Value::Null) | None => {}
        _ => return Err(provider_protocol_error()),
    }

    let tool_calls = value
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .map(|call| {
                    let id = call
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(provider_protocol_error)?;
                    let function = call.get("function").ok_or_else(provider_protocol_error)?;
                    let name = function
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or_else(provider_protocol_error)?;
                    let arguments = decode_tool_arguments(
                        function
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    )?;
                    Ok(ToolCall::new(id, name, arguments))
                })
                .collect::<ModelResult<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok((content, tool_calls))
}

/// Some compatible endpoints encode parameterless calls as an empty string.
/// Keep streaming and non-streaming responses consistent while still rejecting
/// malformed JSON and valid JSON values that are not argument objects.
pub(super) fn decode_tool_arguments(arguments: &str) -> ModelResult<JsonObject> {
    let arguments = arguments.trim();
    if arguments.is_empty() {
        return Ok(JsonObject::default());
    }
    let value = serde_json::from_str(arguments).map_err(|error| {
        provider_protocol_error()
            .with_error(&error)
            .with_body(arguments)
    })?;
    JsonObject::from_value(value).map_err(|error| {
        provider_protocol_error()
            .with_error(&error)
            .with_body(arguments)
    })
}

pub(super) fn decode_usage(value: &Value) -> Option<TokenUsage> {
    let input_tokens = value.get("prompt_tokens")?.as_u64()?.try_into().ok()?;
    let output_tokens = value.get("completion_tokens")?.as_u64()?.try_into().ok()?;
    let cache_read_input_tokens = value
        .get("prompt_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64)
        .or_else(|| value.get("prompt_cache_hit_tokens").and_then(Value::as_u64))
        .and_then(|tokens| tokens.try_into().ok());
    Some(TokenUsage {
        input_tokens,
        output_tokens,
        cache_creation_input_tokens: None,
        cache_read_input_tokens,
    })
}

#[cfg(test)]
mod statistics_tests {
    use super::*;
    #[test]
    fn statistics_deepseek_cache_hits_do_not_increase_prompt_total() {
        let usage = decode_usage(&serde_json::json!({ "prompt_tokens": 1000, "completion_tokens": 100,
            "prompt_cache_hit_tokens": 800, "prompt_cache_miss_tokens": 200 })).unwrap();
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.cache_read_input_tokens, Some(800));
    }
}

pub(super) fn stop_reason(value: Option<&str>) -> StopReason {
    match value.unwrap_or("stop") {
        "stop" | "end_turn" => StopReason::EndTurn,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "length" => StopReason::MaxTokens,
        value => StopReason::Other {
            value: value.to_owned(),
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
