// Replays a Responses SSE body through the real runtime.
//
// The decoder-level tests above call the decoder directly, which is exactly how
// a bug in the *runtime's* handling of the provider's terminal frame stayed
// hidden: the decoder was correct in isolation, but the runtime only completed
// a stream on the Chat Completions `[DONE]` sentinel. A real
// `response.completed` followed by EOF was reported as a transport
// interruption, and the whole reply was discarded (review F06).
//
// These drive the same path production does: a transport, the runtime, and the
// provider decoder wired together.
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::super::{OpenAiResponsesConfig, OpenAiResponsesModel};
use crate::transport::{HttpRequest, HttpResponse, HttpTransport};
use crate::{Model, ModelMessage, ModelRequest, ModelResult, ModelStreamEvent};

/// Replays a fixed byte sequence through the real runtime.
///
/// A fake transport rather than a socket: what broke was the runtime's reading
/// of the frames, not the network, and a fake delivers the exact sequence —
/// including the EOF-after-terminal-frame that production hits.
struct ScriptedTransport {
    body: Vec<u8>,
}

#[async_trait]
impl HttpTransport for ScriptedTransport {
    async fn send(
        &self,
        _request: HttpRequest,
        _cancellation: CancellationToken,
    ) -> ModelResult<HttpResponse> {
        let chunks: Vec<ModelResult<Vec<u8>>> = vec![Ok(self.body.clone())];
        let stream: Pin<Box<dyn futures::Stream<Item = ModelResult<Vec<u8>>> + Send>> =
            Box::pin(futures::stream::iter(chunks));
        Ok(HttpResponse::new(
            200,
            None,
            stream,
            CancellationToken::new(),
        ))
    }
}

fn config(endpoint: &str) -> OpenAiResponsesConfig {
    OpenAiResponsesConfig::new(Url::parse(endpoint).expect("valid url"), "sk-test", "gpt-5")
}

/// Run one request through the model and collect every event.
async fn collect(body: &str) -> Vec<ModelResult<ModelStreamEvent>> {
    let transport = Arc::new(ScriptedTransport {
        body: body.as_bytes().to_vec(),
    });
    let model = OpenAiResponsesModel::with_transport(config("http://localhost:1/v1"), transport);
    let request = ModelRequest::new(vec![ModelMessage::user_text("hi")]);
    let stream = model
        .stream(request, CancellationToken::new())
        .await
        .expect("the stream starts");
    stream.collect().await
}

fn sse(event: &str, data: &str) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

fn completed(events: &[ModelResult<ModelStreamEvent>]) -> bool {
    events
        .iter()
        .any(|event| matches!(event, Ok(ModelStreamEvent::Completed(_))))
}

#[tokio::test]
async fn a_response_ending_on_completed_is_a_success_not_an_interruption() {
    // The exact shape production sends: deltas, then `response.completed`, then
    // the connection closes. There is no `[DONE]`.
    let body = format!(
        "{}{}{}",
        sse(
            "response.output_text.delta",
            r#"{"type":"response.output_text.delta","delta":"Hello"}"#
        ),
        sse(
            "response.output_text.delta",
            r#"{"type":"response.output_text.delta","delta":" world"}"#
        ),
        sse(
            "response.completed",
            r#"{"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":3,"output_tokens":2}}}"#
        ),
    );

    let events = collect(&body).await;

    let failure = events.iter().find_map(|event| event.as_ref().err());
    assert!(
        failure.is_none(),
        "a completed response must not produce an error: {failure:?}"
    );
    let last = events.last().expect("at least one event");
    assert!(
        matches!(last, Ok(ModelStreamEvent::Completed(_))),
        "the stream must end Completed, got {last:?}"
    );

    // And the text before it survives.
    let text: String = events
        .iter()
        .filter_map(|event| match event {
            Ok(ModelStreamEvent::TextDelta { text }) => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Hello world");
}

#[tokio::test]
async fn the_completed_frame_does_not_need_a_done_sentinel() {
    // A body that does end with `[DONE]` must still work — some deployments add
    // it — but it is not required.
    let body = format!(
        "{}{}",
        sse(
            "response.completed",
            r#"{"type":"response.completed","response":{"status":"completed"}}"#
        ),
        "data: [DONE]\n\n",
    );
    let events = collect(&body).await;
    assert!(
        completed(&events),
        "a trailing [DONE] must not break completion: {events:?}"
    );
}

#[tokio::test]
async fn a_failed_response_surfaces_the_provider_message() {
    let body = sse(
        "response.failed",
        r#"{"type":"response.failed","response":{"status":"failed","error":{"message":"the model refused"}}}"#,
    );
    let events = collect(&body).await;
    let error = events
        .iter()
        .find_map(|event| event.as_ref().err())
        .expect("a failure must surface as an error");
    assert!(
        format!("{error}").contains("the model refused"),
        "got: {error}"
    );
}

#[tokio::test]
async fn an_incomplete_response_completes_with_what_was_streamed() {
    // The model stopped early (token cap, content filter). The partial answer
    // is what the caller needs; erroring would discard paid-for output.
    let body = format!(
        "{}{}",
        sse(
            "response.output_text.delta",
            r#"{"type":"response.output_text.delta","delta":"partial"}"#
        ),
        sse(
            "response.incomplete",
            r#"{"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}}"#
        ),
    );
    let events = collect(&body).await;
    assert!(
        completed(&events),
        "an incomplete response must still complete: {events:?}"
    );
}

#[tokio::test]
async fn a_stream_that_ends_without_a_terminal_frame_is_still_an_error() {
    // The guard the fix must not weaken: no terminal frame means the response
    // was truncated, and calling that a completed turn would silently cut the
    // conversation short.
    //
    // The runtime surfaces this as `Interrupted`, not `Completed` — a
    // truncated stream is the retryable case, and the retry layer turns it into
    // that event rather than a plain error. What matters here is that it is
    // *not* reported as a finished turn.
    let body = sse(
        "response.output_text.delta",
        r#"{"type":"response.output_text.delta","delta":"cut off"}"#,
    );
    let events = collect(&body).await;

    assert!(
        !completed(&events),
        "a truncated stream must never complete: {events:?}"
    );
    let interrupted = events.iter().any(|event| {
        matches!(event, Ok(ModelStreamEvent::Interrupted { .. })) || event.as_ref().err().is_some()
    });
    assert!(
        interrupted,
        "a truncated stream must be reported as interrupted: {events:?}"
    );
}
