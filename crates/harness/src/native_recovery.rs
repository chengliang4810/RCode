//! Continue interrupted model output with a single retry budget for all requests.
//! Only completed responses can supply executable tool calls.

use std::sync::Arc;

use futures::{StreamExt, future::BoxFuture, stream::BoxStream};
use rcode_agent::{AgentError, ContentBlock, MessageContent, ReasoningChunk};
use rcode_model::{
    ModelError, ModelMessage, ModelRequest, ModelResult, ModelStream, ModelStreamEvent, RetryConfig,
};
use tokio_util::sync::CancellationToken;

use crate::native_bridge::ResponseBuilder;

pub(crate) type StreamFactory = Arc<
    dyn Fn(ModelRequest, CancellationToken) -> BoxFuture<'static, ModelResult<ModelStream>>
        + Send
        + Sync,
>;

const CONTINUE_PROMPT: &str = "The connection was interrupted while you were generating the previous response. Continue the same task from exactly where the visible assistant text stopped, without repeating that text. If there is no visible assistant text, generate the unfinished response afresh. Earlier tool results remain valid: do not repeat completed side effects. Incomplete tool calls from the interrupted response were discarded and were never executed; issue any still-needed tool call again with complete arguments.";

struct RecoveryState {
    original: ModelRequest,
    request: ModelRequest,
    open: StreamFactory,
    cancellation: CancellationToken,
    retry: RetryConfig,
    model: String,
    provider: String,
    stream: Option<ModelStream>,
    builder: ResponseBuilder,
    prefix: String,
    attempt_text: String,
    has_output: bool,
    attempts: u32,
    terminal: bool,
    call_id: String,
    started: std::time::Instant,
}

pub(crate) fn recovering_reasoning_chunks(
    request: ModelRequest,
    open: StreamFactory,
    cancellation: CancellationToken,
    retry: RetryConfig,
    model: String,
    provider: String,
) -> BoxStream<'static, Result<ReasoningChunk, AgentError>> {
    let state = RecoveryState {
        original: request.clone(),
        request,
        open,
        cancellation,
        retry,
        model,
        provider,
        stream: None,
        builder: ResponseBuilder::default(),
        prefix: String::new(),
        attempt_text: String::new(),
        has_output: false,
        attempts: 0,
        terminal: false,
        call_id: uuid::Uuid::new_v4().to_string(),
        started: std::time::Instant::now(),
    };
    Box::pin(futures::stream::unfold(state, |mut state| async move {
        if state.terminal {
            return None;
        }
        loop {
            let failure = if state.cancellation.is_cancelled() {
                ModelError::cancelled()
            } else if state.stream.is_none() {
                state.attempts += 1;
                tracing::info!(call_id = %state.call_id, model = %state.model,
                    provider = %state.provider, attempt = state.attempts, "model stream started");
                let opened = tokio::select! {
                    biased;
                    _ = state.cancellation.cancelled() => Err(ModelError::cancelled()),
                    result = (state.open)(state.request.clone(), state.cancellation.clone()) => result,
                };
                match opened {
                    Ok(stream) => {
                        state.stream = Some(stream);
                        continue;
                    }
                    Err(error) => error,
                }
            } else {
                let stream = state.stream.as_mut().expect("stream was opened");
                let event = tokio::select! {
                    biased;
                    _ = state.cancellation.cancelled() => Some(Err(ModelError::cancelled())),
                    event = stream.next() => event,
                };
                match event {
                    Some(Ok(ModelStreamEvent::Interrupted { error, .. })) => {
                        state.has_output = true;
                        error
                    }
                    Some(Err(error)) => error,
                    None => ModelError::protocol(
                        rcode_model::ProtocolErrorKind::StreamEndedWithoutCompleted,
                    ),
                    Some(Ok(event)) => {
                        state.has_output |= matches!(
                            event,
                            ModelStreamEvent::TextDelta { .. }
                                | ModelStreamEvent::ReasoningDelta { .. }
                                | ModelStreamEvent::ToolCallDelta { .. }
                        );
                        if let ModelStreamEvent::TextDelta { text } = &event {
                            state.attempt_text.push_str(text);
                        }
                        if let Some(mut chunk) = state.builder.push(event) {
                            if let ReasoningChunk::Done { message, .. } = &mut chunk {
                                // The final provider response owns signatures and tool calls.
                                // Prepend only the visible text from failed attempts, once.
                                if !state.prefix.is_empty() {
                                    if let MessageContent::Blocks(blocks) = &mut message.content {
                                        let index = blocks
                                            .iter()
                                            .position(|block| {
                                                matches!(block, ContentBlock::Text { .. })
                                            })
                                            .unwrap_or(blocks.len());
                                        blocks.insert(
                                            index,
                                            ContentBlock::text(std::mem::take(&mut state.prefix)),
                                        );
                                    }
                                }
                                state.terminal = true;
                                tracing::info!(call_id = %state.call_id, model = %state.model,
                                    provider = %state.provider, attempts = state.attempts,
                                    elapsed_ms = state.started.elapsed().as_millis() as u64,
                                    "model response completed");
                            }
                            return Some((Ok(chunk), state));
                        }
                        continue;
                    }
                }
            };

            // Drop the failed body before waiting. Its incomplete tool arguments
            // and unsigned thinking must never enter the next request or builder.
            state.stream = None;
            if failure.is_cancelled() || state.cancellation.is_cancelled() {
                state.terminal = true;
                return Some((Err(AgentError::Cancelled), state));
            }
            if !state.retry.is_retryable(&failure) || state.attempts >= state.retry.max_attempts() {
                state.terminal = true;
                tracing::warn!(call_id = %state.call_id, model = %state.model,
                    provider = %state.provider, attempts = state.attempts,
                    elapsed_ms = state.started.elapsed().as_millis() as u64,
                    request_id = failure.request_id(), error = %failure, "model recovery stopped");
                let message = format!(
                    "model request failed after {} attempt(s): {failure}",
                    state.attempts
                );
                return Some((Err(AgentError::Model(message)), state));
            }

            state
                .prefix
                .push_str(&std::mem::take(&mut state.attempt_text));
            state.builder = ResponseBuilder::default();
            state.request = state.original.clone();
            if !state.prefix.is_empty() {
                state
                    .request
                    .messages
                    .push(ModelMessage::assistant_text(state.prefix.clone()));
            }
            if state.has_output {
                state
                    .request
                    .messages
                    .push(ModelMessage::user_text(CONTINUE_PROMPT));
            }
            let delay = state.retry.delay_for_retry(state.attempts);
            tracing::warn!(call_id = %state.call_id, model = %state.model, provider = %state.provider,
                attempt = state.attempts, max_attempts = state.retry.max_attempts(),
                elapsed_ms = state.started.elapsed().as_millis() as u64,
                request_id = failure.request_id(),
                delay_ms = delay.as_millis() as u64, error = %failure,
                "retrying model request after interruption");
            tokio::select! {
                biased;
                _ = state.cancellation.cancelled() => {
                    state.terminal = true;
                    return Some((Err(AgentError::Cancelled), state));
                }
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcode_model::{JsonObject, ModelResponse, StopReason, ToolCall, TransportErrorKind};
    use std::{collections::VecDeque, sync::Mutex, time::Duration};

    enum Attempt {
        OpenError(ModelError),
        Events(Vec<ModelResult<ModelStreamEvent>>),
    }

    type Requests = Arc<Mutex<Vec<(ModelRequest, tokio::time::Instant)>>>;

    fn factory(attempts: Vec<Attempt>) -> (StreamFactory, Requests) {
        let attempts = Mutex::new(VecDeque::from(attempts));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let open: StreamFactory = Arc::new(move |request, _| {
            captured
                .lock()
                .unwrap()
                .push((request, tokio::time::Instant::now()));
            let attempt = attempts
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected extra request");
            Box::pin(async move {
                match attempt {
                    Attempt::OpenError(error) => Err(error),
                    Attempt::Events(events) => Ok(ModelStream::new(futures::stream::iter(events))),
                }
            })
        });
        (open, requests)
    }

    fn request() -> ModelRequest {
        ModelRequest::new(vec![ModelMessage::user_text("finish the task")])
    }

    fn retry() -> RetryConfig {
        RetryConfig::default()
            .with_base_delay(Duration::ZERO)
            .with_jitter(false)
    }

    fn stream(
        open: StreamFactory,
        cancellation: CancellationToken,
        retry: RetryConfig,
    ) -> BoxStream<'static, Result<ReasoningChunk, AgentError>> {
        recovering_reasoning_chunks(
            request(),
            open,
            cancellation,
            retry,
            "model".into(),
            "provider".into(),
        )
    }

    fn text(value: &str) -> ModelResult<ModelStreamEvent> {
        Ok(ModelStreamEvent::TextDelta { text: value.into() })
    }

    fn interrupted() -> ModelResult<ModelStreamEvent> {
        Ok(ModelStreamEvent::Interrupted {
            error: ModelError::stream_interrupted(Some("test"), None::<&str>)
                .with_message("peer closed connection without sending TLS close_notify"),
            attempts: 1,
            max_attempts: 1,
        })
    }

    fn done(value: &str) -> ModelResult<ModelStreamEvent> {
        Ok(ModelStreamEvent::Completed(
            ModelResponse::new(
                ModelMessage::assistant_text(value),
                StopReason::EndTurn,
                None,
                None,
            )
            .unwrap(),
        ))
    }

    #[tokio::test]
    async fn tls_interruption_continues_text_without_completing_partial_tools() {
        let partial_tool = Ok(ModelStreamEvent::ToolCallDelta {
            index: 0,
            id: Some("discard-me".into()),
            name: Some("Bash".into()),
            arguments_delta: "{\"command\":".into(),
        });
        let final_tool = ToolCall::new("final-call", "Read", JsonObject::default());
        let final_response = ModelResponse::new(
            ModelMessage::assistant(
                vec![rcode_model::ContentBlock::text("world")],
                vec![final_tool],
            ),
            StopReason::ToolUse,
            None,
            None,
        )
        .unwrap();
        let (open, requests) = factory(vec![
            Attempt::Events(vec![text("Hello "), partial_tool, interrupted()]),
            Attempt::Events(vec![
                text("world"),
                Ok(ModelStreamEvent::Completed(final_response)),
            ]),
        ]);
        let chunks: Vec<_> = stream(open, CancellationToken::new(), retry())
            .collect()
            .await;
        assert!(chunks.iter().all(Result::is_ok), "{chunks:?}");
        let visible: String = chunks
            .iter()
            .filter_map(|chunk| match chunk {
                Ok(ReasoningChunk::Text(text)) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(visible, "Hello world");
        let Ok(ReasoningChunk::Done { message, .. }) = chunks.last().unwrap() else {
            panic!("missing final message")
        };
        assert_eq!(message.content.text_content(), "Hello world");
        let calls = message.content.tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "final-call");
        let sent = requests.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1].0.messages.len(), 3);
        assert_eq!(
            sent[1].0.messages[1].text_content().as_deref(),
            Some("Hello ")
        );
        let serialized = serde_json::to_string(&sent[1].0).unwrap();
        assert!(!serialized.contains("discard-me"));
    }

    #[tokio::test]
    async fn successive_interruptions_keep_one_prefix_and_one_continuation_prompt() {
        let (open, requests) = factory(vec![
            Attempt::Events(vec![text("A"), interrupted()]),
            Attempt::OpenError(ModelError::transport(
                TransportErrorKind::Connection,
                None::<&str>,
            )),
            Attempt::Events(vec![text("B"), interrupted()]),
            Attempt::Events(vec![text("C"), done("C")]),
        ]);
        let chunks: Vec<_> = stream(open, CancellationToken::new(), retry())
            .collect()
            .await;
        let Ok(ReasoningChunk::Done { message, .. }) = chunks.last().unwrap() else {
            panic!("missing final message")
        };
        assert_eq!(message.content.text_content(), "ABC");
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[3].0.messages.len(), 3);
        assert_eq!(
            requests[3].0.messages[1].text_content().as_deref(),
            Some("AB")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_budget_is_ten_retries_total_with_the_expected_backoff() {
        let failures = (0..11)
            .map(|_| Attempt::Events(vec![text("partial"), interrupted()]))
            .collect();
        let (open, requests) = factory(failures);
        let chunks: Vec<_> = stream(
            open,
            CancellationToken::new(),
            RetryConfig::default().with_jitter(false),
        )
        .collect()
        .await;
        assert!(
            matches!(chunks.last(), Some(Err(AgentError::Model(error))) if error.contains("11 attempt(s)") && error.contains("close_notify"))
        );
        assert!(
            !chunks
                .iter()
                .any(|chunk| matches!(chunk, Ok(ReasoningChunk::Done { .. })))
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 11);
        let delays: Vec<_> = requests
            .windows(2)
            .map(|pair| pair[1].1.duration_since(pair[0].1).as_secs())
            .collect();
        assert_eq!(delays, [1, 3, 7, 15, 31, 60, 60, 60, 60, 60]);
    }

    #[tokio::test]
    async fn a_connection_failure_before_output_retries_the_original_request() {
        let (open, requests) = factory(vec![
            Attempt::OpenError(ModelError::transport(
                TransportErrorKind::Connection,
                None::<&str>,
            )),
            Attempt::Events(vec![done("answer")]),
        ]);
        let chunks: Vec<_> = stream(open, CancellationToken::new(), retry())
            .collect()
            .await;
        assert!(matches!(
            chunks.last(),
            Some(Ok(ReasoningChunk::Done { .. }))
        ));
        let requests = requests.lock().unwrap();
        assert_eq!(requests[0].0, requests[1].0);
    }

    #[tokio::test]
    async fn authentication_errors_are_not_retried() {
        let (open, requests) = factory(vec![Attempt::OpenError(ModelError::http_status(
            401,
            "test",
            None::<&str>,
        ))]);
        let chunks: Vec<_> = stream(open, CancellationToken::new(), retry())
            .collect()
            .await;
        assert!(matches!(chunks.as_slice(), [Err(AgentError::Model(_))]));
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_tool_only_interruption_discards_fragments_and_requests_a_fresh_complete_call() {
        let (open, requests) = factory(vec![
            Attempt::Events(vec![
                Ok(ModelStreamEvent::ToolCallDelta {
                    index: 0,
                    id: Some("discard-me".into()),
                    name: Some("Bash".into()),
                    arguments_delta: "{\"command\":".into(),
                }),
                interrupted(),
            ]),
            Attempt::Events(vec![done("answer")]),
        ]);
        let chunks: Vec<_> = stream(open, CancellationToken::new(), retry())
            .collect()
            .await;
        let Ok(ReasoningChunk::Done { message, .. }) = chunks.last().unwrap() else {
            panic!("missing completion")
        };
        assert!(message.content.tool_calls().is_empty());
        let requests = requests.lock().unwrap();
        assert_eq!(requests[1].0.messages.len(), 2);
        assert!(
            requests[1].0.messages[1]
                .text_content()
                .unwrap()
                .contains("generate the unfinished response afresh")
        );
        assert!(
            !serde_json::to_string(&requests[1].0)
                .unwrap()
                .contains("discard-me")
        );
    }

    #[tokio::test]
    async fn stopping_during_backoff_prevents_the_next_request() {
        let (open, requests) = factory(vec![Attempt::Events(vec![text("partial"), interrupted()])]);
        let cancellation = CancellationToken::new();
        let mut chunks = stream(open, cancellation.clone(), RetryConfig::default());
        assert!(matches!(
            chunks.next().await,
            Some(Ok(ReasoningChunk::Text(_)))
        ));
        let waiting = tokio::spawn(async move { chunks.next().await });
        tokio::task::yield_now().await;
        cancellation.cancel();
        assert!(matches!(
            waiting.await.unwrap(),
            Some(Err(AgentError::Cancelled))
        ));
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_clean_eof_without_completion_is_retried_not_accepted() {
        let (open, _) = factory(vec![
            Attempt::Events(vec![text("partial")]),
            Attempt::Events(vec![text(" rest"), done(" rest")]),
        ]);
        let chunks: Vec<_> = stream(open, CancellationToken::new(), retry())
            .collect()
            .await;
        let Ok(ReasoningChunk::Done { message, .. }) = chunks.last().unwrap() else {
            panic!("missing completion")
        };
        assert_eq!(message.content.text_content(), "partial rest");
    }

    #[tokio::test]
    async fn a_real_truncated_http_body_recovers_through_the_provider_runtime() {
        use rcode_model::{Model, ModelRuntimeConfig, OpenAiResponsesConfig, OpenAiResponsesModel};
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for attempt in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let body_offset = loop {
                    let mut chunk = [0; 4096];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(offset) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                        break offset + 4;
                    }
                };
                let header = String::from_utf8_lossy(&bytes[..body_offset]);
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                while bytes.len() < body_offset + length {
                    let mut chunk = [0; 4096];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                }
                requests.push(
                    serde_json::from_slice::<serde_json::Value>(
                        &bytes[body_offset..body_offset + length],
                    )
                    .unwrap(),
                );
                let text = if attempt == 0 { "Hello " } else { "world" };
                let mut body = format!(
                    "event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":\"{text}\"}}\n\n"
                );
                if attempt == 1 {
                    body.push_str("event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n");
                }
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
                socket
                    .write_all(format!("{:x}\r\n{body}\r\n", body.len()).as_bytes())
                    .await
                    .unwrap();
                if attempt == 1 {
                    socket.write_all(b"0\r\n\r\n").await.unwrap();
                }
                // On the first attempt close without the final HTTP chunk.
                // Reqwest must surface an actual body-read error after text.
                socket.shutdown().await.unwrap();
            }
            requests
        });
        let model = Arc::new(OpenAiResponsesModel::new(
            OpenAiResponsesConfig::new(url::Url::parse(&endpoint).unwrap(), "test", "test")
                .with_runtime(
                    ModelRuntimeConfig::default().with_retry(retry().with_max_attempts(1)),
                ),
        ));
        let open: StreamFactory = Arc::new(move |request, cancellation| {
            let model = model.clone();
            Box::pin(async move { model.stream(request, cancellation).await })
        });
        let chunks = tokio::time::timeout(
            Duration::from_secs(10),
            stream(open, CancellationToken::new(), retry()).collect::<Vec<_>>(),
        )
        .await
        .unwrap();
        assert!(chunks.iter().all(Result::is_ok), "{chunks:?}");
        let Ok(ReasoningChunk::Done { message, .. }) = chunks.last().unwrap() else {
            panic!("missing completion")
        };
        assert_eq!(message.content.text_content(), "Hello world");
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].to_string().contains("Hello "));
        assert!(requests[1].to_string().contains("Incomplete tool calls"));
    }
}
