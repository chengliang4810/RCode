// Translation between the agent loop's message model and `rcode-model`'s, for
// RCode.
//!
//! Two types say almost the same thing in two vocabularies, and this module is
//! the only place that knows both. Keeping the translation here (rather than
//! making either crate learn the other's shapes) is what lets the loop stay
//! free of provider types and the model layer stay free of agent concepts.
//!
//! The load-bearing direction is the reply: the loop reads tool calls **only**
//! off the finished message, never off a delta, so the accumulation of
//! `ToolCallDelta` into complete `ToolCall`s has to be right or tool calls are
//! silently lost.

#[cfg(test)]
use futures::{StreamExt, stream::BoxStream};
use std::sync::Arc;

use rcode_agent::{ContentBlock, Message, MessageContent, MessageId, MessageRole, Tool};

use rcode_model::{
    ContentBlock as ModelContentBlock, JsonObject, ModelMessage, ModelRequest, ModelResponse,
    ModelStreamEvent, ToolCall as ModelToolCall, ToolDefinition, ToolResult as ModelToolResult,
};

/// Preserve terminal provider failures while translating streamed deltas. A
/// partial response must never become a completed message or execute tools.
#[cfg(test)]
pub(crate) fn reasoning_chunks(
    stream: rcode_model::ModelStream,
    model: String,
    provider: String,
) -> BoxStream<'static, Result<rcode_agent::ReasoningChunk, rcode_agent::AgentError>> {
    let call_id = uuid::Uuid::new_v4().to_string();
    let started = std::time::Instant::now();
    tracing::info!(%call_id, %model, %provider, "model stream started");
    Box::pin(futures::stream::unfold(
        (stream, ResponseBuilder::default(), false, 0_u64),
        move |(mut stream, mut builder, terminal, mut events)| {
            let (call_id, model, provider) = (call_id.clone(), model.clone(), provider.clone());
            async move {
                if terminal {
                    return None;
                }
                loop {
                    let Some(event) = stream.next().await else {
                        tracing::warn!(%call_id, %model, %provider, events,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "model stream ended before completion");
                        return Some((
                            Err(rcode_agent::AgentError::Model(
                                "model stream ended before completion".into(),
                            )),
                            (stream, builder, true, events),
                        ));
                    };
                    events += 1;
                    let elapsed_ms = started.elapsed().as_millis() as u64;
                    let failure = match event {
                        Ok(ModelStreamEvent::Interrupted {
                            error,
                            attempts,
                            max_attempts,
                        }) => {
                            tracing::warn!(%call_id, %model, %provider, events, elapsed_ms, attempts, max_attempts,
                                request_id = error.request_id(), error = %error, "model stream interrupted");
                            Some(error)
                        }
                        Err(error) => {
                            tracing::warn!(%call_id, %model, %provider, events, elapsed_ms,
                                request_id = error.request_id(), error = %error, "model stream failed");
                            Some(error)
                        }
                        Ok(event) => {
                            if let ModelStreamEvent::Completed(response) = &event {
                                tracing::info!(%call_id, %model, %provider, events, elapsed_ms,
                                    request_id = response.request_id(), "model stream completed");
                            }
                            if let Some(chunk) = builder.push(event) {
                                let done =
                                    matches!(chunk, rcode_agent::ReasoningChunk::Done { .. });
                                return Some((Ok(chunk), (stream, builder, done, events)));
                            }
                            None
                        }
                    };
                    if let Some(error) = failure {
                        return Some((
                            Err(rcode_agent::AgentError::Model(error.to_string())),
                            (stream, builder, true, events),
                        ));
                    }
                }
            }
        },
    ))
}

/// The loop's messages, as the provider layer needs them.
pub(crate) fn to_model_request(
    model: &str,
    messages: &[Message],
    tools: &[Arc<dyn Tool>],
) -> ModelRequest {
    let request = ModelRequest::new(messages.iter().flat_map(to_model_messages).collect());
    let definitions = tools
        .iter()
        .map(|tool| ToolDefinition {
            name: tool.name().to_owned(),
            description: Some(tool.description().to_owned()),
            // `Tool::input_schema` is already neutral JSON Schema, which is
            // exactly what `ToolDefinition` wants — no translation needed.
            input_schema: match JsonObject::from_value(tool.input_schema()) {
                Ok(schema) => schema,
                // A tool with an unrepresentable schema is a tool-call the
                // provider could never make. An empty object schema keeps the
                // tool visible and letting the call fail is better than
                // dropping it silently.
                Err(_) => JsonObject::default(),
            },
        })
        .collect();
    let request = request.with_tools(definitions);
    // The model id rides the config of the client, not the request; assert the
    // intent here so a caller that forgets is visible in a debug log.
    tracing::trace!(model, messages = messages.len(), "built a model request");
    request
}

/// One loop message may contain several tool results. The model layer uses
/// one message per result, so this direction is intentionally one-to-many.
fn to_model_messages(message: &Message) -> Vec<ModelMessage> {
    if message.role == MessageRole::Tool {
        let MessageContent::Blocks(blocks) = &message.content else {
            tracing::error!("a tool message has no structured results");
            return Vec::new();
        };
        return blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => Some(ModelMessage::tool_result(ModelToolResult {
                    id: None,
                    tool_call_id: tool_use_id.clone(),
                    name: String::new(),
                    content: vec![ModelContentBlock::text(content.clone())],
                    is_error: *is_error,
                })),
                _ => None,
            })
            .collect();
    }
    vec![match message.role {
        MessageRole::System => ModelMessage::System {
            content: to_model_blocks(&message.content),
        },
        MessageRole::User => ModelMessage::User {
            content: to_model_blocks(&message.content),
        },
        MessageRole::Assistant => {
            let (content, tool_calls) = match &message.content {
                MessageContent::Text(text) => (vec![ModelContentBlock::text(text)], Vec::new()),
                MessageContent::Blocks(blocks) => {
                    let mut content = Vec::new();
                    let mut tool_calls = Vec::new();
                    for block in blocks {
                        match block {
                            ContentBlock::ToolUse { id, name, input } => {
                                // A tool call whose arguments are not an object
                                // cannot be represented; skip it rather than
                                // sending a malformed call the provider will
                                // reject with a less useful error.
                                if let Ok(arguments) = JsonObject::from_value(input.clone()) {
                                    tool_calls.push(ModelToolCall::new(
                                        id.clone(),
                                        name.clone(),
                                        arguments,
                                    ));
                                } else {
                                    tracing::warn!(
                                        tool = %name,
                                        "dropping a tool call whose arguments are not an object"
                                    );
                                }
                            }
                            other => {
                                if let Some(block) = to_model_block(other) {
                                    content.push(block);
                                }
                            }
                        }
                    }
                    (content, tool_calls)
                }
            };
            ModelMessage::assistant(content, tool_calls)
        }
        MessageRole::Tool => unreachable!("handled above as one result per message"),
    }]
}

fn to_model_blocks(content: &MessageContent) -> Vec<ModelContentBlock> {
    match content {
        MessageContent::Text(text) => vec![ModelContentBlock::text(text)],
        MessageContent::Blocks(blocks) => blocks.iter().filter_map(to_model_block).collect(),
    }
}

fn to_model_block(block: &ContentBlock) -> Option<ModelContentBlock> {
    match block {
        ContentBlock::Text { text } => Some(ModelContentBlock::text(text)),
        ContentBlock::Reasoning { text, signature } => Some(ModelContentBlock::Reasoning {
            text: text.clone(),
            signature: signature.clone(),
        }),
        // An image the user attached. Carried through as a provider content
        // block so a vision model actually sees it — the loop's `Image` and the
        // provider's `Image` differ only in their media-type shape, which is
        // translated here.
        ContentBlock::Image { source } => Some(ModelContentBlock::Image {
            source: model_image_source(source),
        }),
        // Tool use and results are carried at the message level, not as content
        // blocks, in the provider layer's shape.
        ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. } => None,
    }
}

/// Translate the loop's image source into the provider layer's.
///
/// The two differ only in the media type: the loop keeps it as the raw string
/// off the file (`image/png`), while the provider layer wraps it in a
/// `MediaType`. Passed through rather than matched against a list here — the
/// provider's own encoder decides what its wire format accepts, and it can
/// report a problem with the actual media type in the message, which is more
/// useful than dropping the image silently, which is what happened before.
fn model_image_source(source: &rcode_agent::ImageSource) -> rcode_model::ImageSource {
    match source {
        rcode_agent::ImageSource::Base64 { media_type, data } => rcode_model::ImageSource::Base64 {
            media_type: rcode_model::MediaType::new(media_type.clone()),
            data: data.clone(),
        },
        rcode_agent::ImageSource::Url { url } => rcode_model::ImageSource::Url { url: url.clone() },
    }
}

/// Accumulates a streamed response into the loop's terms.
///
/// Text and reasoning pass through as deltas; tool calls are assembled from
/// their fragments, because the loop only ever reads complete calls off the
/// finished message.
#[derive(Default)]
pub(crate) struct ResponseBuilder {
    text: String,
    reasoning: String,
    tool_calls: std::collections::BTreeMap<usize, PartialToolCall>,
}

#[derive(Default)]
struct PartialToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl ResponseBuilder {
    /// Fold one provider event into the accumulated response.
    ///
    /// Returns the chunk to hand the loop, if the event was incremental.
    pub(crate) fn push(&mut self, event: ModelStreamEvent) -> Option<rcode_agent::ReasoningChunk> {
        use rcode_agent::ReasoningChunk;
        match event {
            ModelStreamEvent::TextDelta { text } => {
                self.text.push_str(&text);
                Some(ReasoningChunk::Text(text))
            }
            ModelStreamEvent::ReasoningDelta { text } => {
                self.reasoning.push_str(&text);
                Some(ReasoningChunk::Reasoning(text))
            }
            ModelStreamEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            } => {
                let entry = self.tool_calls.entry(index).or_default();
                if let Some(id) = id {
                    entry.id = Some(id);
                }
                if let Some(name) = name {
                    entry.name = Some(name);
                }
                entry.arguments.push_str(&arguments_delta);
                // Tool calls are not surfaced as deltas — see the module note.
                Some(ReasoningChunk::Activity)
            }
            ModelStreamEvent::Usage(_) => None,
            ModelStreamEvent::Completed(response) => {
                let usage = response.usage().map(|usage| rcode_agent::CallUsage {
                    // Model adapters normalize input_tokens to include cache
                    // reads/writes already; the cache fields are subsets.
                    input_tokens: u64::from(usage.input_tokens),
                    output_tokens: u64::from(usage.output_tokens),
                    cache_read_tokens: usage.cache_read_input_tokens.map(u64::from),
                    cache_write_tokens: usage.cache_creation_input_tokens.map(u64::from),
                });
                Some(ReasoningChunk::Done {
                    message: self.finish_from_response(&response),
                    usage,
                })
            }
            ModelStreamEvent::Interrupted { error, .. } => {
                tracing::warn!(%error, "the model stream was interrupted");
                None
            }
        }
    }

    /// The finished assistant message, once the provider says it is done.
    ///
    /// The provider's own `Completed` response is authoritative for tool calls
    /// when it carries them; the accumulated deltas are the fallback, since
    /// some endpoints send the calls only as deltas.
    pub(crate) fn finish_from_response(&self, response: &ModelResponse) -> Message {
        // Completed content is authoritative: delta text alone cannot retain
        // block boundaries or the signature an extended-thinking API requires
        // on replay. Use deltas only when the final content omitted that kind.
        let mut blocks = Vec::new();
        if let ModelMessage::Assistant { content, .. } = response.message() {
            for block in content {
                match block {
                    ModelContentBlock::Text { text } => {
                        blocks.push(ContentBlock::text(text.clone()))
                    }
                    ModelContentBlock::Reasoning { text, signature } => {
                        blocks.push(ContentBlock::Reasoning {
                            text: text.clone(),
                            signature: signature.clone(),
                        })
                    }
                    _ => {}
                }
            }
        }
        if !self.reasoning.is_empty()
            && !blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::Reasoning { .. }))
        {
            blocks.push(ContentBlock::Reasoning {
                text: self.reasoning.clone(),
                signature: None,
            });
        }
        if !self.text.is_empty()
            && !blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::Text { .. }))
        {
            blocks.push(ContentBlock::Text {
                text: self.text.clone(),
            });
        }

        let from_response = provider_tool_calls(response);
        let tool_calls = if from_response.is_empty() {
            self.assembled_tool_calls()
        } else {
            from_response
        };
        for (id, name, input) in tool_calls {
            blocks.push(ContentBlock::ToolUse { id, name, input });
        }

        Message {
            id: MessageId::new(),
            role: MessageRole::Assistant,
            content: MessageContent::Blocks(blocks),
        }
    }

    fn assembled_tool_calls(&self) -> Vec<(String, String, serde_json::Value)> {
        self.tool_calls
            .values()
            .filter_map(|call| {
                let id = call.id.clone()?;
                let name = call.name.clone()?;
                // A function that streams no arguments means "no arguments",
                // which is legal for a no-parameter tool.
                let input = if call.arguments.trim().is_empty() {
                    serde_json::json!({})
                } else {
                    match serde_json::from_str(&call.arguments) {
                        Ok(input) => input,
                        Err(error) => {
                            tracing::warn!(
                                %error,
                                arguments = %call.arguments,
                                "a tool call's arguments did not parse; sending an empty object"
                            );
                            serde_json::json!({})
                        }
                    }
                };
                Some((id, name, input))
            })
            .collect()
    }
}

fn provider_tool_calls(response: &ModelResponse) -> Vec<(String, String, serde_json::Value)> {
    let message = response.message();
    match message {
        ModelMessage::Assistant { tool_calls, .. } => tool_calls
            .iter()
            .map(|call| {
                (
                    call.id().to_owned(),
                    call.name().to_owned(),
                    serde_json::Value::Object(
                        call.arguments().as_map().clone().into_iter().collect(),
                    ),
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcode_model::{
        ModelMessage as ProviderMessage, ModelResponse as ProviderResponse, StopReason, TokenUsage,
    };

    #[tokio::test]
    async fn interrupted_stream_preserves_cause_and_never_completes_partial_tools() {
        let stream = rcode_model::ModelStream::new(futures::stream::iter(vec![
            Ok(ModelStreamEvent::TextDelta {
                text: "partial".into(),
            }),
            Ok(ModelStreamEvent::ToolCallDelta {
                index: 0,
                id: Some("call-1".into()),
                name: Some("Bash".into()),
                arguments_delta: "{\"command\":".into(),
            }),
            Ok(ModelStreamEvent::Interrupted {
                error: rcode_model::ModelError::stream_interrupted(
                    Some("openai-compatible"),
                    Some("request-123"),
                )
                .with_message("HTTP response ended before SSE completion"),
                attempts: 1,
                max_attempts: 6,
            }),
        ]));
        let chunks = reasoning_chunks(stream, "model".into(), "provider".into())
            .collect::<Vec<_>>()
            .await;
        assert!(
            matches!(&chunks[0], Ok(rcode_agent::ReasoningChunk::Text(text)) if text == "partial")
        );
        assert!(matches!(
            &chunks[1],
            Ok(rcode_agent::ReasoningChunk::Activity)
        ));
        let error = chunks[2].as_ref().err().unwrap().to_string();
        assert!(
            error.contains("HTTP response ended before SSE completion"),
            "{error}"
        );
        assert!(error.contains("request-123"), "{error}");
        assert_eq!(chunks.len(), 3);
    }

    #[tokio::test]
    async fn completed_stream_does_not_report_false_eof() {
        let stream = rcode_model::ModelStream::new(futures::stream::iter(vec![Ok(
            ModelStreamEvent::Completed(completed("answer")),
        )]));
        let chunks = reasoning_chunks(stream, "model".into(), "provider".into())
            .collect::<Vec<_>>()
            .await;
        assert_eq!(chunks.len(), 1);
        assert!(matches!(
            &chunks[0],
            Ok(rcode_agent::ReasoningChunk::Done { .. })
        ));
    }

    fn user(text: &str) -> Message {
        Message {
            id: MessageId::new(),
            role: MessageRole::User,
            content: MessageContent::text(text),
        }
    }

    fn assistant_with_call(id: &str, name: &str) -> Message {
        Message {
            id: MessageId::new(),
            role: MessageRole::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: id.to_owned(),
                name: name.to_owned(),
                input: serde_json::json!({ "file_path": "a.rs" }),
            }]),
        }
    }

    fn tool_message(id: &str, output: &str) -> Message {
        Message {
            id: MessageId::new(),
            role: MessageRole::Tool,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: id.to_owned(),
                content: output.to_owned(),
                is_error: false,
            }]),
        }
    }

    #[test]
    fn a_user_message_becomes_a_provider_user_message() {
        let request = to_model_request("m", &[user("hello")], &[]);
        assert_eq!(request.messages.len(), 1);
        assert!(matches!(request.messages[0], ProviderMessage::User { .. }));
        assert_eq!(request.messages[0].text_content().as_deref(), Some("hello"));
    }

    #[test]
    fn a_tool_call_round_trips_into_the_provider_shape() {
        let messages = vec![
            user("read a.rs"),
            assistant_with_call("call-1", "Read"),
            tool_message("call-1", "contents"),
        ];
        let request = to_model_request("m", &messages, &[]);

        let ProviderMessage::Assistant { tool_calls, .. } = &request.messages[1] else {
            panic!(
                "expected an assistant message, got {:?}",
                request.messages[1]
            );
        };
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id(), "call-1");
        assert_eq!(tool_calls[0].name(), "Read");

        let ProviderMessage::ToolResult { result } = &request.messages[2] else {
            panic!("expected a tool result");
        };
        assert_eq!(result.tool_call_id, "call-1");
    }

    #[test]
    fn every_tool_result_preserves_its_body_id_and_error_state() {
        let body = "     1\t中文内容 ORANGE-42\nsecond line\n";
        let message = Message::tool_results(vec![
            ContentBlock::ToolResult {
                tool_use_id: "read-1".into(),
                content: body.into(),
                is_error: false,
            },
            ContentBlock::ToolResult {
                tool_use_id: "edit-2".into(),
                content: "permission denied".into(),
                is_error: true,
            },
        ]);
        let request = to_model_request("m", &[message], &[]);
        assert_eq!(request.messages.len(), 2);
        for (index, expected_id, expected_text, expected_error) in [
            (0, "read-1", body, false),
            (1, "edit-2", "permission denied", true),
        ] {
            let ModelMessage::ToolResult { result } = &request.messages[index] else {
                panic!("a result must stay a result");
            };
            assert_eq!(result.tool_call_id, expected_id);
            assert_eq!(result.is_error, expected_error);
            assert_eq!(
                request.messages[index].text_content().as_deref(),
                Some(expected_text)
            );
        }
    }

    #[test]
    fn completed_reasoning_keeps_its_signature_on_replay() {
        let response = ProviderResponse::new(
            ProviderMessage::assistant(
                vec![
                    ModelContentBlock::Reasoning {
                        text: "reasoning".into(),
                        signature: Some("provider-signature".into()),
                    },
                    ModelContentBlock::text("answer"),
                ],
                Vec::new(),
            ),
            StopReason::EndTurn,
            None,
            None,
        )
        .unwrap();
        let message = ResponseBuilder::default().finish_from_response(&response);
        let request = to_model_request("m", &[message], &[]);
        let ModelMessage::Assistant { content, .. } = &request.messages[0] else {
            panic!("assistant");
        };
        assert!(
            matches!(&content[0], ModelContentBlock::Reasoning { text, signature }
            if text == "reasoning" && signature.as_deref() == Some("provider-signature"))
        );
        assert_eq!(content[1].text_content(), Some("answer"));
    }

    #[test]
    fn tools_become_provider_definitions_with_their_schema() {
        struct ReadTool;
        #[async_trait::async_trait]
        impl Tool for ReadTool {
            fn name(&self) -> &str {
                "Read"
            }
            fn description(&self) -> &str {
                "reads a file"
            }
            fn input_schema(&self) -> serde_json::Value {
                serde_json::json!({
                    "type": "object",
                    "properties": { "file_path": { "type": "string" } }
                })
            }
            async fn call(
                &self,
                _input: serde_json::Value,
                _ctx: &rcode_agent::ToolContext,
            ) -> Result<rcode_agent::ToolResult, rcode_agent::ToolError> {
                unimplemented!()
            }
        }

        let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(ReadTool)];
        let request = to_model_request("m", &[user("hi")], &tools);

        assert_eq!(request.tools.len(), 1);
        assert_eq!(request.tools[0].name, "Read");
        assert_eq!(
            request.tools[0].description.as_deref(),
            Some("reads a file")
        );
        assert!(
            request.tools[0]
                .input_schema
                .as_map()
                .contains_key("properties")
        );
    }

    fn completed(text: &str) -> ProviderResponse {
        ProviderResponse::new(
            ProviderMessage::assistant_text(text),
            StopReason::EndTurn,
            Some(TokenUsage::new(1, 1)),
            None,
        )
        .expect("a well-formed response")
    }

    #[test]
    fn statistics_cache_counts_remain_subsets_of_input() {
        let mut usage = TokenUsage::new(1000, 200);
        usage.cache_read_input_tokens = Some(800);
        usage.cache_creation_input_tokens = Some(100);
        let response = ProviderResponse::new(
            ProviderMessage::assistant_text("ok"),
            StopReason::EndTurn,
            Some(usage),
            None,
        )
        .unwrap();
        let Some(rcode_agent::ReasoningChunk::Done {
            usage: Some(usage), ..
        }) = ResponseBuilder::default().push(ModelStreamEvent::Completed(response))
        else {
            panic!("completed usage missing")
        };
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.output_tokens, 200);
        assert_eq!(usage.cache_read_tokens, Some(800));
        assert_eq!(usage.cache_write_tokens, Some(100));
    }

    #[test]
    fn text_deltas_pass_through_and_accumulate_into_the_finished_message() {
        let mut builder = ResponseBuilder::default();
        assert!(matches!(
            builder.push(ModelStreamEvent::TextDelta { text: "Hel".into() }),
            Some(rcode_agent::ReasoningChunk::Text(text)) if text == "Hel"
        ));
        builder.push(ModelStreamEvent::TextDelta { text: "lo".into() });

        let message = builder.finish_from_response(&completed("Hello"));
        assert_eq!(message.content.text_content(), "Hello");
    }

    #[test]
    fn streamed_tool_call_fragments_are_assembled_into_one_call() {
        let mut builder = ResponseBuilder::default();
        builder.push(ModelStreamEvent::ToolCallDelta {
            index: 0,
            id: Some("call-1".into()),
            name: Some("Read".into()),
            arguments_delta: String::new(),
        });
        builder.push(ModelStreamEvent::ToolCallDelta {
            index: 0,
            id: None,
            name: None,
            arguments_delta: "{\"file_path\":".into(),
        });
        builder.push(ModelStreamEvent::ToolCallDelta {
            index: 0,
            id: None,
            name: None,
            arguments_delta: "\"a.rs\"}".into(),
        });

        // The provider's own completed response carries no calls, so the
        // accumulated deltas are what must produce them.
        let message = builder.finish_from_response(&completed(""));
        let calls = message.content.tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "call-1");
        assert_eq!(calls[0].1, "Read");
        assert_eq!(calls[0].2["file_path"], "a.rs");
    }

    #[test]
    fn a_tool_call_with_unparseable_arguments_still_produces_a_call() {
        let mut builder = ResponseBuilder::default();
        builder.push(ModelStreamEvent::ToolCallDelta {
            index: 0,
            id: Some("call-1".into()),
            name: Some("Bash".into()),
            arguments_delta: "{not json".into(),
        });

        let message = builder.finish_from_response(&completed(""));
        let calls = message.content.tool_calls();
        assert_eq!(calls.len(), 1, "the call must not be dropped");
        assert_eq!(calls[0].2, serde_json::json!({}));
    }

    #[test]
    fn a_tool_call_with_no_arguments_is_an_empty_object() {
        let mut builder = ResponseBuilder::default();
        builder.push(ModelStreamEvent::ToolCallDelta {
            index: 0,
            id: Some("call-1".into()),
            name: Some("CronList".into()),
            arguments_delta: String::new(),
        });

        let message = builder.finish_from_response(&completed(""));
        let calls = message.content.tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].2, serde_json::json!({}));
    }

    #[test]
    fn reasoning_deltas_become_a_reasoning_block() {
        let mut builder = ResponseBuilder::default();
        builder.push(ModelStreamEvent::ReasoningDelta {
            text: "thinking".into(),
        });

        let message = builder.finish_from_response(&completed(""));
        let MessageContent::Blocks(blocks) = &message.content else {
            panic!("expected blocks");
        };
        assert!(blocks.iter().any(|block| matches!(
            block,
            ContentBlock::Reasoning { text, .. } if text == "thinking"
        )));
    }

    #[test]
    fn usage_and_interrupted_events_do_not_produce_chunks() {
        let mut builder = ResponseBuilder::default();
        assert!(
            builder
                .push(ModelStreamEvent::Usage(TokenUsage::new(1, 2)))
                .is_none()
        );
        assert!(
            builder
                .push(ModelStreamEvent::Interrupted {
                    error: rcode_model::ModelError::cancelled(),
                    attempts: 1,
                    max_attempts: 3,
                })
                .is_none()
        );
    }
    #[test]
    fn an_image_block_survives_into_the_provider_request() {
        // The bug this guards (review F07): the bridge dropped every
        // `ContentBlock::Image`, so a screenshot the user attached reached the
        // model as nothing at all while the UI showed it as sent.
        let message = Message {
            id: MessageId::new(),
            role: MessageRole::User,
            content: MessageContent::Blocks(vec![
                ContentBlock::text("what is wrong here?"),
                ContentBlock::Image {
                    source: rcode_agent::ImageSource::Base64 {
                        media_type: "image/png".to_owned(),
                        data: "aGVsbG8=".to_owned(),
                    },
                },
            ]),
        };

        let request = to_model_request("m", &[message], &[]);

        let images: Vec<_> = request
            .messages
            .iter()
            .flat_map(|message| match message {
                ProviderMessage::User { content } => content.clone(),
                _ => Vec::new(),
            })
            .filter_map(|block| match block {
                rcode_model::ContentBlock::Image { source } => Some(source),
                _ => None,
            })
            .collect();

        assert_eq!(images.len(), 1, "the image did not reach the request");
        match &images[0] {
            rcode_model::ImageSource::Base64 { media_type, data } => {
                assert_eq!(media_type.as_str(), "image/png");
                assert_eq!(data, "aGVsbG8=", "the bytes must pass through unchanged");
            }
            other => panic!("expected an inline image, got {other:?}"),
        }
    }

    #[test]
    fn a_text_only_message_gains_no_image_block() {
        let request = to_model_request("m", &[user("just text")], &[]);
        let has_image = request.messages.iter().any(|message| match message {
            ProviderMessage::User { content } => content
                .iter()
                .any(|block| matches!(block, rcode_model::ContentBlock::Image { .. })),
            _ => false,
        });
        assert!(
            !has_image,
            "a text-only message must not grow an image block"
        );
    }
}
