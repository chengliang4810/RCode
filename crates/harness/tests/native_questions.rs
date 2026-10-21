//! Provider payload -> pending input -> typed answer -> resumed native run.
use futures::StreamExt;
use rcode_harness::{Harness, RunControls, TurnControl, native::NativeHarness};
use rcode_proto::{
    AgentEvent, DoneStatus, HarnessId, InteractionRequest, InteractionResponse, RunRequest,
    UserInputAnswer,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
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
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
}

#[tokio::test]
async fn native_object_questions_surface_once_wait_and_resume_with_selected_labels() {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let payloads = [
            json!({"questions":[{"options":[
                {"question":"Which scope?","prefill":"First","description":"Keep data"},
                {"question":"Which scope?","prefill":"Second","description":"Remove data"}]}]}),
            json!({"questions":[{"question":"Which scope?","options":[
                {"description":"First"},{"description":"Second"}]}]}),
            json!({"questions":[{"question":"Which scope?","multiSelect":true,"options":[
                {"label":"First","description":"Keep data"},{"label":"Second","description":"Remove data"}]}]}),
        ];
        for phase in 0..=payloads.len() {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            if phase == 0 {
                let tool = request["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|tool| tool["function"]["name"] == "AskUserQuestion")
                    .unwrap();
                assert!(tool["function"]["parameters"]["properties"]["questions"]["items"]["properties"]["options"]["items"]["anyOf"].is_array());
            } else {
                let answer = request["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .rev()
                    .find(|message| message["role"] == "tool")
                    .unwrap();
                assert!(answer["content"].as_str().unwrap().contains("Second"));
                assert!(
                    !answer["content"]
                        .as_str()
                        .unwrap()
                        .contains("invalid input")
                );
            }
            if phase < payloads.len() {
                reply(&mut socket, json!({"tool_calls":[{"index":0,"id":format!("ask-{phase}"),"type":"function","function":{
                    "name":"AskUserQuestion","arguments":payloads[phase].to_string()
                }}]}), "tool_calls").await;
            } else {
                reply(
                    &mut socket,
                    json!({"content":"All answers received."}),
                    "stop",
                )
                .await;
            }
        }
    });
    std::fs::write(root.path().join("providers.json"),json!({"providers":[{"id":"fixture","label":"Fixture","protocol":"open_ai_chat","baseUrl":format!("http://{address}/v1"),"apiKey":"fixture-only","models":[{"id":"fixture","model":"fixture","maxTokens":2048,"contextWindow":200000}]}],"defaultModel":"fixture"}).to_string()).unwrap();
    std::fs::write(
        root.path().join("agent-settings.json"),
        r#"{"mcp":false,"memory":false}"#,
    )
    .unwrap();
    let harness = NativeHarness::from_data_dir(root.path());
    let (requests_tx, mut requests) = mpsc::unbounded_channel();
    let (_steer, steering) = mpsc::channel(1);
    let controls = RunControls {
        conversation_id: None,
        goal: None,
        execution_lease: None,
        request_input: Box::new(move |request| {
            let (answer_tx, answer_rx) = oneshot::channel();
            requests_tx.send((request, answer_tx)).unwrap();
            answer_rx
        }),
        steering,
        interrupt: CancellationToken::new(),
        turn: TurnControl::default(),
        history: None,
    };
    let mut stream = harness
        .run(
            RunRequest {
                prompt: "Ask which scope to use.".into(),
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
    let run = tokio::spawn(async move {
        let mut events = vec![];
        while let Some(event) = stream.next().await {
            events.push(event.unwrap());
        }
        events
    });
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        for asked in 0..3 {
            let (InteractionRequest::Questions { questions }, answer_tx) =
                requests.recv().await.unwrap();
            assert_eq!(questions.len(), 1);
            assert_eq!(questions[0].question, "Which scope?");
            assert_eq!(questions[0].multi_select, asked == 2);
            assert_eq!(questions[0].options.len(), 2);
            assert!(questions[0].options[1].label.contains("Second"));
            let answer = InteractionResponse::Questions {
                answers: vec![UserInputAnswer {
                    question_id: questions[0].id.clone(),
                    selected_option_ids: vec![questions[0].options[1].id.clone()],
                    text: None,
                }],
            };
            assert!(!run.is_finished(), "the native run must wait for an answer");
            answer_tx.send(answer).unwrap();
        }
        let events = run.await.unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::Done {
                status: DoneStatus::Completed,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(event,
            AgentEvent::TextDelta { text } if text.contains("All answers received.")
        )));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentEvent::ToolResult { .. }))
                .count(),
            3
        );
        for event in events {
            match event {
                AgentEvent::ToolResult { is_error, .. } => assert!(!is_error),
                AgentEvent::Done { status, .. } => {
                    assert_eq!(status, DoneStatus::Completed);
                }
                AgentEvent::Error { message, .. } => panic!("{message}"),
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    server.await.unwrap();
}
