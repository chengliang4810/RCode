//! Correction hands off at a safe step boundary, with the tool result intact.
use futures::StreamExt;
use rcode_harness::{Harness, RunControls, SteerMessage, TurnControl, native::NativeHarness};
use rcode_proto::{AgentEvent, DoneStatus, HarnessId, RunRequest, SteeringMode};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Semaphore, mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

async fn read_request(socket: &mut tokio::net::TcpStream) -> Value {
    let mut request = vec![];
    let mut buffer = [0; 8192];
    loop {
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0);
        request.extend_from_slice(&buffer[..count]);
        if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&request[..end]);
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim().parse().unwrap())
                })
                .unwrap();
            if request.len() >= end + 4 + length {
                return serde_json::from_slice(&request[end + 4..end + 4 + length]).unwrap();
            }
        }
    }
}

async fn reply(socket: &mut tokio::net::TcpStream, delta: Value, finish: &str) {
    let first = json!({"choices":[{"delta":delta,"finish_reason":null}]});
    let last = json!({"choices":[{"delta":{},"finish_reason":finish}]});
    let body = format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n");
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
}

async fn correction_handoff(stop: bool) {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let output = project.path().join("step.txt");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let gate = Arc::new(Semaphore::new(0));
    let server_gate = gate.clone();
    let (ready_tx, ready) = oneshot::channel();
    let output_path = output.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        ready_tx.send(()).unwrap();
        server_gate.acquire().await.unwrap().forget();
        if !stop {
            reply(&mut socket, json!({"tool_calls":[{"index":0,"id":"write-step","type":"function","function":{
                "name":"Write", "arguments":json!({"file_path":output_path,"content":"step completed"}).to_string()
            }}]}), "tool_calls").await;
        }
        drop(socket);
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_request(&mut socket).await;
        let messages = request["messages"].as_array().unwrap();
        let last_user = messages
            .iter()
            .rev()
            .find(|message| message["role"] == "user")
            .unwrap();
        assert!(last_user["content"].to_string().contains(if stop {
            "fresh request"
        } else {
            "corrected direction"
        }));
        if stop {
            assert!(!messages.iter().any(|message| {
                message["content"]
                    .to_string()
                    .contains("corrected direction")
            }));
        } else {
            assert!(messages.iter().any(
                |message| message["role"] == "tool" && message["tool_call_id"] == "write-step"
            ));
            assert_eq!(
                std::fs::read_to_string(output_path).unwrap(),
                "step completed"
            );
        }
        reply(&mut socket, json!({"content":"Follow-up complete"}), "stop").await;
    });
    std::fs::write(root.path().join("providers.json"), json!({"providers":[{"id":"fixture","label":"Fixture","protocol":"open_ai_chat","baseUrl":format!("http://{address}/v1"),"apiKey":"fixture-only","models":[{"id":"fixture","model":"fixture","maxTokens":2048,"contextWindow":200000}]}],"defaultModel":"fixture"}).to_string()).unwrap();
    std::fs::write(
        root.path().join("agent-settings.json"),
        r#"{"mcp":false,"memory":false}"#,
    )
    .unwrap();
    let harness = NativeHarness::from_data_dir(root.path());
    assert_eq!(harness.steering_mode(), SteeringMode::StepBoundary);
    let (steer, steering) = mpsc::channel(1);
    let cancel = CancellationToken::new();
    let turn = TurnControl::default();
    let controls = RunControls {
        conversation_id: Some("correction-test".into()),
        goal: None,
        request_input: Box::new(|_| oneshot::channel().1),
        steering,
        interrupt: cancel.clone(),
        turn: turn.clone(),
        history: None,
    };
    let mut stream = harness
        .run(
            RunRequest {
                prompt: "Work on the original direction".into(),
                harness: Some(HarnessId::Native),
                model: Some("fixture".into()),
                reasoning: None,
                model_options: Default::default(),
                cwd: project.path().to_string_lossy().into(),
                resume: None,
                attachments: vec![],
                worktree: None,
            },
            controls,
        )
        .await
        .unwrap();
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let consumer = tokio::spawn(async move {
        while let Some(event) = stream.next().await {
            events_tx.send(event.unwrap()).unwrap();
        }
    });
    ready.await.unwrap();
    steer
        .send(SteerMessage::text("Use the corrected direction"))
        .await
        .unwrap();
    // Capacity returns only after the session has consumed the correction.
    drop(steer.reserve().await.unwrap());
    if stop {
        turn.stop_turn();
        loop {
            match events.recv().await.unwrap() {
                AgentEvent::Done { status, .. } => {
                    assert_eq!(status, DoneStatus::Interrupted);
                    break;
                }
                AgentEvent::Steered { .. } => panic!("Stop must discard an unconsumed correction"),
                _ => {}
            }
        }
        assert!(!output.exists());
        steer
            .send(SteerMessage::text("A fresh request after stopping"))
            .await
            .unwrap();
    }
    gate.add_permits(1);
    let mut steered = false;
    loop {
        match events.recv().await.unwrap() {
            AgentEvent::Steered { .. } => steered = true,
            AgentEvent::Done { status, .. } if steered => {
                assert_eq!(status, DoneStatus::Completed);
                break;
            }
            _ => {}
        }
    }
    server.await.unwrap();
    cancel.cancel();
    consumer.await.unwrap();
}

#[tokio::test]
async fn correction_reaches_next_model_request_with_completed_tool_results() {
    tokio::time::timeout(Duration::from_secs(30), correction_handoff(false))
        .await
        .unwrap();
}

#[tokio::test]
async fn stop_discards_pending_correction_and_allows_a_fresh_request() {
    tokio::time::timeout(Duration::from_secs(30), correction_handoff(true))
        .await
        .unwrap();
}
