// OpenAI Responses API support is new for RCode.
//!
//! Request-body shape and stream decoding.
//!
//! These assert the *differences* from Chat Completions, because that is where
//! a copy-paste port would go wrong silently: the endpoint, `instructions`
//! instead of a system message, `input` instead of `messages`, flat tool
//! definitions, and `input_tokens`/`output_tokens` instead of
//! `prompt_tokens`/`completion_tokens`.

use serde_json::json;
use url::Url;

use super::OpenAiResponsesConfig;
use super::request::body_for_test;
use super::stream::decoders;
use crate::{
    ContentBlock, JsonObject, ModelMessage, ModelRequest, ModelStreamEvent, ToolCall,
    ToolDefinition,
};

fn config() -> OpenAiResponsesConfig {
    OpenAiResponsesConfig::new(
        Url::parse("https://api.openai.com/v1").expect("valid url"),
        "sk-test",
        "gpt-5",
    )
}

fn tool(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_owned(),
        description: Some(format!("{name} something")),
        input_schema: JsonObject::from_value(json!({ "type": "object" })).expect("valid schema"),
    }
}

#[test]
fn the_endpoint_is_responses_not_chat_completions() {
    let request = ModelRequest::new(vec![ModelMessage::user_text("hi")]);
    let body = body_for_test(&config(), &request);
    assert_eq!(body["model"], "gpt-5");
    // The body itself carries no endpoint, so assert via the builder's own
    // endpoint function through a second call.
    let endpoint = super::request::responses_endpoint(
        &Url::parse("https://api.openai.com/v1").expect("valid url"),
    )
    .expect("valid endpoint");
    assert_eq!(endpoint.as_str(), "https://api.openai.com/v1/responses");
}

#[test]
fn a_system_message_becomes_instructions() {
    let request = ModelRequest::new(vec![
        ModelMessage::system_text("be terse"),
        ModelMessage::user_text("hi"),
    ]);
    let body = body_for_test(&config(), &request);

    assert_eq!(body["instructions"], "be terse");
    // No system-role entry survives in `input`.
    let input = body["input"].as_array().expect("input is an array");
    assert!(
        !input
            .iter()
            .any(|item| item.get("role").and_then(|role| role.as_str()) == Some("system")),
        "the system prompt must not also appear as an input item"
    );
}

#[test]
fn a_user_message_uses_input_text_parts() {
    let request = ModelRequest::new(vec![ModelMessage::user_text("hello")]);
    let body = body_for_test(&config(), &request);

    let input = body["input"].as_array().expect("input is an array");
    assert_eq!(input.len(), 1);
    assert_eq!(input[0]["type"], "message");
    assert_eq!(input[0]["role"], "user");
    assert_eq!(input[0]["content"][0]["type"], "input_text");
    assert_eq!(input[0]["content"][0]["text"], "hello");
}

#[test]
fn tools_are_flat_not_nested_under_function() {
    let request =
        ModelRequest::new(vec![ModelMessage::user_text("hi")]).with_tools(vec![tool("Read")]);
    let body = body_for_test(&config(), &request);

    let tools = body["tools"].as_array().expect("tools is an array");
    assert_eq!(tools[0]["type"], "function");
    assert_eq!(tools[0]["name"], "Read");
    assert_eq!(tools[0]["description"], "Read something");
    assert!(tools[0].get("parameters").is_some());
    // Chat Completions nests these; Responses does not.
    assert!(tools[0].get("function").is_none());
}

#[test]
fn the_output_limit_uses_the_responses_field_name() {
    let request = ModelRequest::new(vec![ModelMessage::user_text("hi")]).with_max_tokens(512);
    let body = body_for_test(&config(), &request);

    assert_eq!(body["max_output_tokens"], 512);
    assert!(body.get("max_tokens").is_none());
}

#[test]
fn an_assistant_tool_call_becomes_a_top_level_function_call_item() {
    let call = ToolCall::new(
        "call-1",
        "Read",
        JsonObject::from_value(json!({ "file_path": "a.rs" })).expect("valid args"),
    );
    let request = ModelRequest::new(vec![
        ModelMessage::user_text("read a.rs"),
        ModelMessage::assistant(vec![ContentBlock::text("reading")], vec![call]),
        ModelMessage::tool_result(crate::ToolResult::success("call-1", "Read", "contents")),
    ]);
    let body = body_for_test(&config(), &request);

    let input = body["input"].as_array().expect("input is an array");
    let function_call = input
        .iter()
        .find(|item| item["type"] == "function_call")
        .expect("a function_call item");
    assert_eq!(function_call["call_id"], "call-1");
    assert_eq!(function_call["name"], "Read");

    let output = input
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .expect("a function_call_output item");
    assert_eq!(output["call_id"], "call-1");
    assert_eq!(output["output"], "contents");
}

#[test]
fn the_reasoning_effort_is_nested_under_reasoning() {
    let config = config().with_reasoning_effort("high");
    let request = ModelRequest::new(vec![ModelMessage::user_text("hi")]);
    let body = body_for_test(&config, &request);

    assert_eq!(body["reasoning"]["effort"], "high");
    // Chat Completions puts this at the top level; Responses does not.
    assert!(body.get("reasoning_effort").is_none());
}

#[test]
fn the_api_key_is_redacted_in_debug_output() {
    let rendered = format!("{:?}", config());
    assert!(rendered.contains("<redacted>"));
    assert!(
        !rendered.contains("sk-test"),
        "the key must not appear: {rendered}"
    );
}

/// Feed a sequence of `(event_name, data)` frames through a fresh decoder and
/// collect what it produced.
fn decode(frames: &[(&str, serde_json::Value)]) -> Vec<ModelStreamEvent> {
    let (decoder, completion) = decoders()();
    let mut events = Vec::new();
    for (name, data) in frames {
        let event = crate::transport::SseEvent {
            event: Some((*name).to_owned()),
            data: data.to_string(),
        };
        events.extend(decoder(event, None).expect("frame decodes"));
    }
    events.extend(completion().expect("stream completes"));
    events
}

/// The text an assistant message carries.
fn message_text(message: &ModelMessage) -> String {
    message.text_content().unwrap_or_default()
}

/// The tool calls an assistant message carries.
fn message_tool_calls(message: &ModelMessage) -> &[ToolCall] {
    match message {
        ModelMessage::Assistant { tool_calls, .. } => tool_calls,
        other => panic!("expected an assistant message, got {other:?}"),
    }
}

#[test]
fn text_deltas_accumulate_into_one_message() {
    let events = decode(&[
        (
            "response.output_text.delta",
            json!({ "type": "response.output_text.delta", "delta": "Hel" }),
        ),
        (
            "response.output_text.delta",
            json!({ "type": "response.output_text.delta", "delta": "lo" }),
        ),
        (
            "response.completed",
            json!({
                "type": "response.completed",
                "response": { "id": "resp_1", "status": "completed" }
            }),
        ),
    ]);

    let deltas: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            ModelStreamEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, vec!["Hel", "lo"]);

    let completed = events
        .iter()
        .find_map(|event| match event {
            ModelStreamEvent::Completed(response) => Some(response),
            _ => None,
        })
        .expect("a completed response");
    assert_eq!(message_text(completed.message()), "Hello");
}

#[test]
fn usage_uses_the_responses_field_names() {
    let events = decode(&[(
        "response.completed",
        json!({
            "type": "response.completed",
            "response": {
                "id": "resp_1",
                "status": "completed",
                "usage": { "input_tokens": 10, "output_tokens": 3 }
            }
        }),
    )]);

    let usage = events
        .iter()
        .find_map(|event| match event {
            ModelStreamEvent::Usage(usage) => Some(usage),
            _ => None,
        })
        .expect("usage was decoded");
    assert_eq!(usage.input_tokens, 10);
    assert_eq!(usage.output_tokens, 3);
}

#[test]
fn a_stream_without_a_terminal_frame_is_an_error() {
    let (decoder, completion) = decoders()();
    decoder(
        crate::transport::SseEvent {
            event: Some("response.output_text.delta".to_owned()),
            data: json!({ "delta": "partial" }).to_string(),
        },
        None,
    )
    .expect("the frame itself decodes");

    assert!(
        completion().is_err(),
        "a partial stream must not be reported as a completed turn"
    );
}

#[test]
fn a_failed_response_surfaces_the_provider_message() {
    let (decoder, _completion) = decoders()();
    // `response.failed` is a terminal frame, so the failure surfaces here
    // rather than being held until EOF. Failing at the frame is the better
    // behaviour: the caller learns the reason immediately, and a connection
    // that never closes cannot delay it.
    let error = decoder(
        crate::transport::SseEvent {
            event: Some("response.failed".to_owned()),
            data: json!({
                "type": "response.failed",
                "response": { "error": { "message": "rate limited" } }
            })
            .to_string(),
        },
        None,
    )
    .expect_err("the failure propagates from the frame");

    assert!(error.to_string().contains("rate limited"));
}

#[test]
fn function_call_arguments_accumulate_across_deltas() {
    let events = decode(&[
        (
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "item": {
                    "type": "function_call",
                    "id": "item_1",
                    "call_id": "call_1",
                    "name": "Read",
                    "arguments": ""
                }
            }),
        ),
        (
            "response.function_call_arguments.delta",
            json!({
                "type": "response.function_call_arguments.delta",
                "item_id": "item_1",
                "delta": "{\"file_path\":"
            }),
        ),
        (
            "response.function_call_arguments.delta",
            json!({
                "type": "response.function_call_arguments.delta",
                "item_id": "item_1",
                "delta": "\"a.rs\"}"
            }),
        ),
        (
            "response.completed",
            json!({
                "type": "response.completed",
                "response": { "id": "resp_1", "status": "completed" }
            }),
        ),
    ]);

    let completed = events
        .iter()
        .find_map(|event| match event {
            ModelStreamEvent::Completed(response) => Some(response),
            _ => None,
        })
        .expect("a completed response");
    let calls = message_tool_calls(completed.message());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id(), "call_1");
    assert_eq!(calls[0].name(), "Read");
    assert_eq!(
        calls[0].arguments().as_map().get("file_path"),
        Some(&json!("a.rs"))
    );
}

#[test]
fn reasoning_summary_deltas_are_reported_as_reasoning() {
    let events = decode(&[
        (
            "response.reasoning_summary_text.delta",
            json!({ "type": "response.reasoning_summary_text.delta", "delta": "thinking" }),
        ),
        (
            "response.completed",
            json!({ "type": "response.completed", "response": { "status": "completed" } }),
        ),
    ]);

    let reasoning: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            ModelStreamEvent::ReasoningDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(reasoning, vec!["thinking"]);
}

#[test]
fn unknown_event_types_are_ignored_not_fatal() {
    let (decoder, _completion) = decoders()();
    let result = decoder(
        crate::transport::SseEvent {
            event: Some("response.some_future_event".to_owned()),
            data: json!({ "type": "response.some_future_event" }).to_string(),
        },
        None,
    );
    assert!(result.is_ok(), "an unknown event must not break the stream");
}
