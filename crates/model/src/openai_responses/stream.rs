// OpenAI Responses API support is new for RCode.
//!
//! Decoding the Responses API's SSE stream.
//!
//! The event vocabulary is different from Chat Completions: every frame carries
//! a named `event:` type, and text, reasoning and tool calls arrive as separate
//! event families rather than deltas on one message object. Tool arguments
//! stream against an `item_id` that only later resolves to a `call_id`, so the
//! accumulator has to hold both.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use serde_json::Value;

use crate::{
    ContentBlock, JsonObject, ModelError, ModelMessage, ModelResponse, ModelResult,
    ModelStreamEvent, TokenUsage, ToolCall, runtime::stream::SseDecoderFactory,
    transport::SseEvent,
};

use super::response::{decode_usage, provider_protocol_error, stop_reason};

fn malformed_frame_error(summary: &str) -> ModelError {
    ModelError::protocol_with_summary(crate::ProtocolErrorKind::Provider, summary)
}

/// One function call being assembled.
#[derive(Default, Clone)]
struct FunctionCallAccumulator {
    /// The `call_id` the transcript will reference. Only present once the item
    /// is fully added.
    call_id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Default)]
struct StreamState {
    text: String,
    reasoning: String,
    /// Keyed by the item id the deltas stream against.
    function_calls: BTreeMap<String, FunctionCallAccumulator>,
    /// Preserves the order calls were added in — `BTreeMap` would sort by id.
    order: Vec<String>,
    usage: Option<TokenUsage>,
    request_id: Option<String>,
    /// The terminal status, once a `response.completed` frame arrives.
    status: Option<String>,
    incomplete_reason: Option<String>,
    failure: Option<String>,
}

pub(super) fn decoders() -> SseDecoderFactory {
    Arc::new(|| {
        let state = Arc::new(Mutex::new(StreamState::default()));
        let decoder = {
            let state = Arc::clone(&state);
            Arc::new(move |event, _header_request_id: Option<String>| decode_event(&state, event))
        };
        let completion_decoder = Arc::new(move || complete_stream(&state));
        (decoder, completion_decoder)
    })
}

fn decode_event(state: &Mutex<StreamState>, event: SseEvent) -> ModelResult<Vec<ModelStreamEvent>> {
    let body = event.data.clone();
    decode_event_inner(state, event).map_err(|error| error.with_body(body))
}

fn decode_event_inner(
    state: &Mutex<StreamState>,
    event: SseEvent,
) -> ModelResult<Vec<ModelStreamEvent>> {
    if event.data == "[DONE]" {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_str(&event.data)
        .map_err(|error| provider_protocol_error().with_error(&error))?;

    // The frame's own type is authoritative; the JSON body repeats it as
    // `type`, and the SSE `event:` field is what arrives first.
    let kind = event
        .event
        .as_deref()
        .filter(|kind| !kind.is_empty())
        .map(str::to_owned)
        .or_else(|| value.get("type").and_then(Value::as_str).map(str::to_owned))
        .ok_or_else(|| malformed_frame_error("openai_responses_frame_without_type"))?;

    let mut state = state
        .lock()
        .map_err(|error| provider_protocol_error().with_message(error.to_string()))?;
    if state.request_id.is_none() {
        state.request_id = value
            .get("response")
            .and_then(|response| response.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| value.get("id").and_then(Value::as_str).map(str::to_owned));
    }

    let mut events = Vec::new();
    match kind.as_str() {
        // ── text ────────────────────────────────────────────────────────────
        "response.output_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                if !delta.is_empty() {
                    state.text.push_str(delta);
                    events.push(ModelStreamEvent::TextDelta {
                        text: delta.to_owned(),
                    });
                }
            }
        }
        // ── reasoning ───────────────────────────────────────────────────────
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                if !delta.is_empty() {
                    state.reasoning.push_str(delta);
                    events.push(ModelStreamEvent::ReasoningDelta {
                        text: delta.to_owned(),
                    });
                }
            }
        }
        // ── tool calls ──────────────────────────────────────────────────────
        "response.output_item.added" => {
            let Some(item) = value.get("item") else {
                return Ok(events);
            };
            if item.get("type").and_then(Value::as_str) != Some("function_call") {
                return Ok(events);
            }
            let Some(item_id) = item.get("id").and_then(Value::as_str) else {
                return Err(malformed_frame_error("openai_responses_item_without_id"));
            };
            let mut accumulator = FunctionCallAccumulator {
                call_id: item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                name: item.get("name").and_then(Value::as_str).map(str::to_owned),
                arguments: item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            };
            // Some servers send the arguments up front rather than streaming
            // them; in that case there is nothing left to accumulate.
            let arguments_delta = std::mem::take(&mut accumulator.arguments);
            let index = state.order.len();
            if !state.function_calls.contains_key(item_id) {
                state.order.push(item_id.to_owned());
            }
            state.function_calls.insert(item_id.to_owned(), accumulator);
            events.push(ModelStreamEvent::ToolCallDelta {
                index,
                id: state
                    .function_calls
                    .get(item_id)
                    .and_then(|call| call.call_id.clone()),
                name: state
                    .function_calls
                    .get(item_id)
                    .and_then(|call| call.name.clone()),
                arguments_delta,
            });
        }
        "response.function_call_arguments.delta" => {
            let Some(item_id) = value.get("item_id").and_then(Value::as_str) else {
                return Err(malformed_frame_error(
                    "openai_responses_arguments_without_item_id",
                ));
            };
            let Some(delta) = value.get("delta").and_then(Value::as_str) else {
                return Ok(events);
            };
            let index = state
                .order
                .iter()
                .position(|id| id == item_id)
                .unwrap_or_else(|| {
                    state.order.push(item_id.to_owned());
                    state.order.len() - 1
                });
            let accumulator = state.function_calls.entry(item_id.to_owned()).or_default();
            accumulator.arguments.push_str(delta);
            events.push(ModelStreamEvent::ToolCallDelta {
                index,
                id: accumulator.call_id.clone(),
                name: accumulator.name.clone(),
                arguments_delta: delta.to_owned(),
            });
        }
        // ── usage and terminal frames ───────────────────────────────────────
        //
        // These frames end the response. The decoder completes here rather than
        // waiting for `[DONE]`: that sentinel belongs to Chat Completions, and
        // the Responses API ends on `response.completed` (the official
        // streaming guide and the create-endpoint SSE example both do). Waiting
        // for `[DONE]` meant a perfectly good response hit EOF with no
        // terminal event, which the runtime reported as a transport
        // interruption — the whole reply discarded.
        "response.completed" => {
            if let Some(response) = value.get("response") {
                state.status = response
                    .get("status")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or(Some("completed".to_owned()));
                if let Some(usage) = response.get("usage").and_then(decode_usage) {
                    state.usage = Some(usage.clone());
                    events.push(ModelStreamEvent::Usage(usage));
                }
            } else {
                state.status = Some("completed".to_owned());
            }
            events.push(ModelStreamEvent::Completed(complete_now(&state)?));
        }
        "response.incomplete" => {
            state.status = Some("incomplete".to_owned());
            if let Some(response) = value.get("response") {
                state.incomplete_reason = response
                    .get("incomplete_details")
                    .and_then(|details| details.get("reason"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if let Some(usage) = response.get("usage").and_then(decode_usage) {
                    state.usage = Some(usage.clone());
                    events.push(ModelStreamEvent::Usage(usage));
                }
            }
            // Incomplete is still a finished response: the model stopped early
            // (a token limit, a content filter), and the partial answer plus
            // the reason is what the caller needs. Reporting it as an error
            // would throw away text the user already paid for.
            events.push(ModelStreamEvent::Completed(complete_now(&state)?));
        }
        "response.failed" => {
            state.status = Some("failed".to_owned());
            state.failure = value
                .get("response")
                .and_then(|response| response.get("error"))
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| Some("the provider reported a failure".to_owned()));
            // The failure is reported by the terminal event's error rather than
            // by silence at EOF, where it would look like a dropped connection.
            let message = state
                .failure
                .clone()
                .unwrap_or_else(|| "the provider reported a failure".to_owned());
            return Err(provider_protocol_error().with_message(message));
        }
        // Frames that carry no incremental state we need: lifecycle markers,
        // content-part boundaries, and the final full text (already streamed).
        "response.created"
        | "response.in_progress"
        | "response.output_item.done"
        | "response.output_text.done"
        | "response.reasoning_summary_text.done"
        | "response.reasoning_text.done"
        | "response.content_part.added"
        | "response.content_part.done"
        | "response.function_call_arguments.done" => {}
        other => {
            // An unknown event type is not fatal — providers add them over
            // time — but it must not be mistaken for a completion.
            tracing::debug!(event = other, "ignoring an unrecognised Responses event");
        }
    }
    Ok(events)
}

/// Build the completed response, given a locked state.
///
/// Used both by the terminal-frame path (`response.completed` and friends,
/// which complete while decoding) and by the completion decoder (EOF or
/// `[DONE]`), so the two cannot disagree about what a finished response is.
fn complete_now(state: &StreamState) -> ModelResult<ModelResponse> {
    if let Some(failure) = &state.failure {
        return Err(provider_protocol_error().with_message(failure.clone()));
    }
    // Without a terminal frame the response is partial; reporting it as a
    // completed turn would silently truncate the conversation.
    let status = state
        .status
        .as_deref()
        .filter(|status| !status.trim().is_empty())
        .ok_or_else(|| malformed_frame_error("openai_responses_stream_without_terminal_frame"))?;
    completed_response(state, status, state.incomplete_reason.as_deref())
}

fn complete_stream(state: &Mutex<StreamState>) -> ModelResult<Vec<ModelStreamEvent>> {
    let state = state
        .lock()
        .map_err(|error| provider_protocol_error().with_message(error.to_string()))?;
    Ok(vec![ModelStreamEvent::Completed(complete_now(&state)?)])
}

fn completed_response(
    state: &StreamState,
    status: &str,
    incomplete_reason: Option<&str>,
) -> ModelResult<ModelResponse> {
    let mut tool_calls = Vec::with_capacity(state.order.len());
    for item_id in &state.order {
        let Some(call) = state.function_calls.get(item_id) else {
            continue;
        };
        let id = call
            .call_id
            .as_deref()
            .or(Some(item_id.as_str()))
            .ok_or_else(provider_protocol_error)?;
        let name = call.name.as_deref().ok_or_else(provider_protocol_error)?;
        // An empty argument string means "no arguments", which is legal.
        let arguments: Value = if call.arguments.trim().is_empty() {
            Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str(&call.arguments).map_err(|error| {
                provider_protocol_error()
                    .with_error(&error)
                    .with_body(&call.arguments)
            })?
        };
        let arguments = JsonObject::from_value(arguments)
            .map_err(|error| provider_protocol_error().with_error(&error))?;
        tool_calls.push(ToolCall::new(id, name, arguments));
    }

    let mut content = Vec::new();
    if !state.reasoning.is_empty() {
        content.push(ContentBlock::reasoning(&state.reasoning));
    }
    if !state.text.is_empty() {
        content.push(ContentBlock::text(&state.text));
    }

    ModelResponse::new(
        ModelMessage::assistant(content, tool_calls),
        stop_reason(status, incomplete_reason),
        state.usage.clone(),
        state.request_id.clone(),
    )
}

#[cfg(test)]
#[path = "stream_runtime_tests.rs"]
mod runtime_tests;
