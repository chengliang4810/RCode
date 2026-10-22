//! Provider protocol regressions through public model APIs and a loopback SSE server.
//! No real provider, credential, application configuration, or source mutation is used.

use std::time::Duration;

use futures::StreamExt;
use rcode_model::{
    AnthropicConfig, Model, ModelMessage, ModelRequest, ModelResponse, ModelResult,
    ModelRuntimeConfig, ModelStreamEvent, OpenAiConfig, OpenAiModel, OpenAiResponsesConfig,
    OpenAiResponsesModel, RetryConfig,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use url::Url;

const DUMMY_KEY: &str = "synthetic-review-key-never-a-real-credential";

async fn replay(body: String, responses: bool) -> Vec<ModelResult<ModelStreamEvent>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Url::parse(&format!("http://{}/v1", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut received = Vec::new();
        let header_end = loop {
            let mut chunk = [0u8; 4096];
            let count = socket.read(&mut chunk).await.unwrap();
            assert!(count > 0, "request closed before its headers");
            received.extend_from_slice(&chunk[..count]);
            if let Some(index) = received.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                break index + 4;
            }
            assert!(
                received.len() < 65_536,
                "unexpectedly large request headers"
            );
        };
        let headers = String::from_utf8_lossy(&received[..header_end]);
        let content_length: usize = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .expect("JSON POST has a content length");
        while received.len() < header_end + content_length {
            let mut chunk = [0u8; 4096];
            let count = socket.read(&mut chunk).await.unwrap();
            assert!(count > 0, "request closed before its JSON body");
            received.extend_from_slice(&chunk[..count]);
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
    });

    let runtime =
        ModelRuntimeConfig::default().with_retry(RetryConfig::default().with_max_attempts(1));
    let model: Box<dyn Model> = if responses {
        Box::new(OpenAiResponsesModel::new(
            OpenAiResponsesConfig::new(endpoint, DUMMY_KEY, "synthetic-model")
                .with_runtime(runtime),
        ))
    } else {
        Box::new(OpenAiModel::new(
            OpenAiConfig::new(endpoint, DUMMY_KEY, "synthetic-model").with_runtime(runtime),
        ))
    };
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        model
            .stream(
                ModelRequest::new(vec![ModelMessage::user_text("synthetic review")]),
                CancellationToken::new(),
            )
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
    })
    .await;
    if result.is_err() {
        server.abort();
    } else {
        server.await.unwrap();
    }
    result.expect("loopback stream must terminate within five seconds")
}

fn sse(event: &str, data: Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

fn completed(events: &[ModelResult<ModelStreamEvent>]) -> &ModelResponse {
    events
        .iter()
        .find_map(|event| match event {
            Ok(ModelStreamEvent::Completed(response)) => Some(response),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected Completed, received {events:?}"))
}

fn first_arguments(response: &ModelResponse) -> &rcode_model::JsonObject {
    match response.message() {
        ModelMessage::Assistant { tool_calls, .. } => {
            assert_eq!(tool_calls.len(), 1);
            assert_eq!(tool_calls[0].id(), "call_1");
            assert_eq!(tool_calls[0].name(), "Read");
            tool_calls[0].arguments()
        }
        other => panic!("expected assistant tool call, received {other:?}"),
    }
}

fn response_tool_stream(arguments_on_added: &str, delta: Option<&str>) -> String {
    let mut body = sse(
        "response.output_item.added",
        json!({"type":"response.output_item.added", "item": {
            "type":"function_call", "id":"item_1", "call_id":"call_1",
            "name":"Read", "arguments": arguments_on_added
        }}),
    );
    if let Some(delta) = delta {
        body.push_str(&sse(
            "response.function_call_arguments.delta",
            json!({"type":"response.function_call_arguments.delta", "item_id":"item_1", "delta":delta}),
        ));
    }
    body.push_str(&sse(
        "response.completed",
        json!({"type":"response.completed", "response":{"id":"response_1", "status":"completed"}}),
    ));
    body
}

#[tokio::test]
async fn responses_preserves_complete_arguments_received_on_item_added() {
    let body = response_tool_stream(r#"{"file_path":"a.rs"}"#, None);
    let events = replay(body, true).await;
    let deltas: String = events
        .iter()
        .filter_map(|event| match event {
            Ok(ModelStreamEvent::ToolCallDelta {
                arguments_delta, ..
            }) => Some(arguments_delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, r#"{"file_path":"a.rs"}"#);
    assert_eq!(
        first_arguments(completed(&events))
            .as_map()
            .get("file_path"),
        Some(&json!("a.rs")),
        "the Completed response must retain the same arguments as its delta"
    );
}

#[tokio::test]
async fn responses_control_preserves_arguments_received_as_deltas() {
    let events = replay(
        response_tool_stream("", Some(r#"{"file_path":"a.rs"}"#)),
        true,
    )
    .await;
    assert_eq!(
        first_arguments(completed(&events))
            .as_map()
            .get("file_path"),
        Some(&json!("a.rs"))
    );
}

#[tokio::test]
async fn responses_appends_deltas_to_initial_partial_arguments() {
    let events = replay(
        response_tool_stream(r#"{"file_path":"#, Some(r#""a.rs"}"#)),
        true,
    )
    .await;
    let deltas: String = events
        .iter()
        .filter_map(|event| match event {
            Ok(ModelStreamEvent::ToolCallDelta {
                arguments_delta, ..
            }) => Some(arguments_delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, r#"{"file_path":"a.rs"}"#);
    assert_eq!(
        first_arguments(completed(&events))
            .as_map()
            .get("file_path"),
        Some(&json!("a.rs"))
    );
}

fn chat_tool_stream(arguments: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"id":"chat_1", "choices":[{"index":0, "delta":{"tool_calls":[{
            "index":0, "id":"call_1", "type":"function",
            "function":{"name":"Read", "arguments":arguments}
        }]}, "finish_reason":null}]}),
        json!({"id":"chat_1", "choices":[{"index":0, "delta":{}, "finish_reason":"tool_calls"}]})
    )
}

#[tokio::test]
async fn chat_accepts_empty_arguments_for_a_parameterless_tool() {
    for arguments in ["", " \t\r\n"] {
        let events = replay(chat_tool_stream(arguments), false).await;
        assert!(first_arguments(completed(&events)).as_map().is_empty());
    }
}

#[tokio::test]
async fn chat_control_accepts_an_explicit_empty_object() {
    let events = replay(chat_tool_stream("{}"), false).await;
    assert!(first_arguments(completed(&events)).as_map().is_empty());
}

#[tokio::test]
async fn chat_rejects_malformed_or_non_object_arguments_instead_of_erasing_them() {
    for arguments in ["{", "[]", "null", "42", r#"{"file_path":}"#] {
        let events = replay(chat_tool_stream(arguments), false).await;
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Err(_) | Ok(ModelStreamEvent::Interrupted { .. })))
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Ok(ModelStreamEvent::Completed(_))))
        );
    }
}

#[test]
fn openai_chat_config_debug_does_not_disclose_the_credential() {
    let rendered = format!(
        "{:?}",
        OpenAiConfig::new(
            Url::parse("https://example.invalid/v1").unwrap(),
            DUMMY_KEY,
            "model"
        )
    );
    assert!(
        !rendered.contains(DUMMY_KEY),
        "Debug must redact the credential"
    );
}

#[test]
fn anthropic_config_debug_does_not_disclose_the_credential() {
    let rendered = format!(
        "{:?}",
        AnthropicConfig::new(
            Url::parse("https://example.invalid/v1").unwrap(),
            DUMMY_KEY,
            "model"
        )
    );
    assert!(
        !rendered.contains(DUMMY_KEY),
        "Debug must redact the credential"
    );
}

#[test]
fn responses_config_debug_control_does_not_disclose_the_credential() {
    let rendered = format!(
        "{:?}",
        OpenAiResponsesConfig::new(
            Url::parse("https://example.invalid/v1").unwrap(),
            DUMMY_KEY,
            "model"
        )
    );
    assert!(
        !rendered.contains(DUMMY_KEY),
        "Debug must redact the credential"
    );
}

#[test]
fn provider_config_debug_omits_url_credentials_queries_and_fragments() {
    let endpoint = Url::parse(
        "https://synthetic-user:synthetic-password@example.invalid/v1?api_key=synthetic-query-secret#synthetic-fragment",
    )
    .unwrap();
    let outputs = [
        format!(
            "{:?}",
            OpenAiConfig::new(endpoint.clone(), DUMMY_KEY, "model")
        ),
        format!(
            "{:?}",
            AnthropicConfig::new(endpoint.clone(), DUMMY_KEY, "model")
        ),
        format!(
            "{:?}",
            OpenAiResponsesConfig::new(endpoint, DUMMY_KEY, "model")
        ),
    ];
    for rendered in outputs {
        for secret in [
            DUMMY_KEY,
            "synthetic-user",
            "synthetic-password",
            "synthetic-query-secret",
            "synthetic-fragment",
        ] {
            assert!(
                !rendered.contains(secret),
                "configuration diagnostics leaked a credential"
            );
        }
        assert!(rendered.contains("<redacted>"));
        assert!(rendered.contains("example.invalid"));
        assert!(rendered.contains("/v1"));
    }
}
