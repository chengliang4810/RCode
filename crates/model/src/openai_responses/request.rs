// OpenAI Responses API support is new for RCode.
//!
//! `ModelRequest` → the Responses API request body.
//!
//! The shape differs from Chat Completions in three ways that matter:
//! the conversation is a flat `input` list, the system prompt is hoisted to
//! `instructions`, and tool calls and their results are top-level items rather
//! than message fields.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use url::Url;

use crate::{
    ContentBlock, DocumentSource, ImageSource, ModelMessage, ModelRequest, ModelResult,
    PreparedModelRequest, ProviderProtocol, ToolDefinition,
    prompt_cache::strip_system_prompt_dynamic_boundaries,
};

use super::OpenAiResponsesConfig;

#[derive(Clone)]
pub(super) struct BuiltResponsesRequest {
    pub(super) endpoint: Url,
    pub(super) body: Value,
    model_id: String,
}

impl BuiltResponsesRequest {
    pub(super) fn observe(
        &self,
        runtime: &crate::ModelRuntimeConfig,
    ) -> ModelResult<PreparedModelRequest> {
        PreparedModelRequest::observe_with_runtime(
            ProviderProtocol::OpenAiResponses,
            self.model_id.clone(),
            self.endpoint.clone(),
            self.body.clone(),
            BTreeMap::new(),
            runtime,
        )
    }
}

pub(super) fn build_request(
    config: &OpenAiResponsesConfig,
    request: &ModelRequest,
) -> ModelResult<BuiltResponsesRequest> {
    let mut body = json!({
        "model": config.model,
        "input": input_items(&request.messages),
        "stream": true,
        "max_output_tokens": request.max_tokens.unwrap_or(config.max_tokens),
    });

    // The Responses API takes the system prompt as a separate field, not as a
    // message with `role: "system"`.
    if let Some(instructions) =
        request.system_prompt.as_ref().map(|prompt| prompt.text())
            .or_else(|| extract_system_message(&request.messages)).filter(|content| !content.trim().is_empty())
    {
        body["instructions"] = json!(instructions);
    }

    if !request.tools.is_empty() {
        body["tools"] = Value::Array(request.tools.iter().map(tool_to_responses).collect());
        body["tool_choice"] = json!("auto");
    }
    if let Some(effort) = &config.reasoning_effort {
        body["reasoning"] = json!({ "effort": effort });
    } else if let Some(temperature) = request.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(session_id) = &request.session_id {
        body["metadata"] = json!({ "session_id": session_id });
    }

    Ok(BuiltResponsesRequest {
        endpoint: responses_endpoint(&config.endpoint)?,
        body,
        model_id: config.model.clone(),
    })
}

/// `POST /v1/responses` — the endpoint differs from Chat Completions only in
/// its last path segment.
pub(super) fn responses_endpoint(endpoint: &Url) -> ModelResult<Url> {
    crate::endpoint::resolve(endpoint, &["responses"])
}

fn extract_system_message(messages: &[ModelMessage]) -> Option<String> {
    let parts = messages
        .iter()
        .filter_map(|message| match message {
            ModelMessage::System { content } => Some(content_text(content)),
            _ => None,
        })
        .filter(|content| !content.trim().is_empty())
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| strip_system_prompt_dynamic_boundaries(&parts.join("\n\n")))
}

/// Flatten the conversation into the `input` array.
///
/// A tool call and its result become two separate items, not fields on a
/// message — the API models them as first-class entries in the transcript.
fn input_items(messages: &[ModelMessage]) -> Vec<Value> {
    let mut items = Vec::new();
    for message in messages {
        match message {
            ModelMessage::System { .. } => {}
            ModelMessage::User { content } => {
                items.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": content_to_parts(content, "input"),
                }));
            }
            ModelMessage::Assistant {
                content,
                tool_calls,
            } => {
                let parts = content_to_parts(content, "output");
                if !parts.is_empty() {
                    items.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": parts,
                    }));
                }
                for tool_call in tool_calls {
                    items.push(json!({
                        "type": "function_call",
                        "call_id": tool_call.id(),
                        "name": tool_call.name(),
                        "arguments": serde_json::to_string(tool_call.arguments().as_map())
                            .expect("JsonObject always serializes"),
                    }));
                }
            }
            ModelMessage::ToolResult { result } => {
                items.push(json!({
                    "type": "function_call_output",
                    "call_id": result.tool_call_id,
                    "output": content_text(&result.content),
                }));
            }
        }
    }
    items
}

/// A tool definition in Responses form: flat, not nested under `function`.
fn tool_to_responses(tool: &ToolDefinition) -> Value {
    json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.input_schema.as_map(),
        "strict": false,
    })
}

fn content_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(ContentBlock::text_content)
        .collect()
}

/// `direction` picks the part-type prefix the API expects: `input_text` for
/// content being sent, `output_text` for content being replayed back.
fn content_to_parts(content: &[ContentBlock], direction: &str) -> Vec<Value> {
    content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => {
                Some(json!({ "type": format!("{direction}_text"), "text": text }))
            }
            ContentBlock::Image { source } => {
                let image_url = match source {
                    ImageSource::Url { url } => url.clone(),
                    ImageSource::Base64 { media_type, data } => {
                        format!("data:{};base64,{data}", media_type.as_str())
                    }
                };
                Some(json!({
                    "type": format!("{direction}_image"),
                    "image_url": image_url,
                }))
            }
            ContentBlock::Document { source, title } => {
                let file = match source {
                    DocumentSource::Url { url } => json!({ "file_url": url }),
                    DocumentSource::Base64 { media_type, data } => {
                        json!({ "file_data": format!("data:{};base64,{data}", media_type.as_str()) })
                    }
                    // The Responses API has no inline-text document part; the
                    // text is inlined as ordinary content instead.
                    DocumentSource::Text { text } => {
                        return Some(json!({
                            "type": format!("{direction}_text"),
                            "text": match title {
                                Some(title) => format!("{title}\n\n{text}"),
                                None => text.clone(),
                            },
                        }));
                    }
                };
                Some(json!({
                    "type": "input_file",
                    "filename": title.clone().unwrap_or_else(|| "document".to_owned()),
                    "file": file,
                }))
            }
            ContentBlock::Reasoning { .. }
            | ContentBlock::ToolUse { .. }
            | ContentBlock::ToolResult { .. }
            | ContentBlock::RedactedReasoning { .. } => None,
        })
        .collect()
}

#[cfg(test)]
pub(super) fn body_for_test(config: &OpenAiResponsesConfig, request: &ModelRequest) -> Value {
    build_request(config, request)
        .expect("test config is valid")
        .body
}
