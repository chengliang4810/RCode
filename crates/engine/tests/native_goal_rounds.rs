//! Goal verification closes each durable turn before the host starts the next one.
use rcode_engine::EngineCore;
use rcode_proto::{AgentEvent, DoneStatus, HarnessId, RunRequest};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct HomeGuard(&'static str, Option<std::ffi::OsString>);
impl Drop for HomeGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(value) = self.1.take() {
                std::env::set_var(self.0, value);
            } else {
                std::env::remove_var(self.0);
            }
        }
    }
}
async fn read_request(socket: &mut tokio::net::TcpStream) -> Value {
    let mut request = vec![];
    let mut buffer = [0; 8192];
    loop {
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0);
        request.extend_from_slice(&buffer[..count]);
        if let Some(end) = request.windows(4).position(|b| b == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&request[..end]);
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                        .map(|(_, v)| v.trim().parse().unwrap())
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
    let last = json!({"choices":[{"delta":{},"finish_reason":finish}],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}});
    let body = format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n");
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
}

#[tokio::test]
async fn native_goal_iterations_are_separate_durable_assistant_turns_with_shared_context() {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let _home = HomeGuard(key, std::env::var_os(key));
    unsafe {
        std::env::set_var(key, home.path());
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut requests = vec![];
        for phase in 0..6 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let verify = phase % 2 == 1;
            let no_tools = request
                .get("tools")
                .is_none_or(|tools| tools.as_array().is_none_or(Vec::is_empty));
            assert_eq!(no_tools, verify);
            if phase == 2 || phase == 4 {
                assert!(
                    request["messages"]
                        .to_string()
                        .contains("Goal checkpoint 1"),
                    "new turns retain prior work"
                );
            }
            requests.push(request);
            let text = if verify {
                if phase == 5 {
                    r#"{"passed":true,"reason":"all checkpoints validated"}"#.to_owned()
                } else {
                    r#"{"passed":false,"reason":"next checkpoint remains","nextAction":"Continue the next checkpoint"}"#.to_owned()
                }
            } else {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                format!("Goal checkpoint {}", phase / 2 + 1)
            };
            reply(&mut socket, json!({"content":text}), "stop").await;
        }
        requests
    });
    std::fs::write(root.path().join("providers.json"),json!({"providers":[{"id":"fixture","label":"Fixture","protocol":"open_ai_chat","baseUrl":format!("http://{address}/v1"),"apiKey":"fixture-only","models":[{"id":"fixture","model":"fixture","maxTokens":2048,"contextWindow":200000}]}],"defaultModel":"fixture"}).to_string()).unwrap();
    std::fs::write(
        root.path().join("agent-settings.json"),
        r#"{"mcp":false,"memory":false}"#,
    )
    .unwrap();
    let core = EngineCore::assemble(
        root.path(),
        Arc::new(rcode_engine::registry_with_native(root.path())),
        HarnessId::Native,
        None,
    )
    .unwrap();
    core.workspace
        .create_chat("goal-rounds", None, Some(&core.device_id), None, None)
        .unwrap();
    core.workspace
        .rename_chat("goal-rounds", "Goal rounds")
        .unwrap();
    core.workspace
        .set_chat_cwd("goal-rounds", &project.path().to_string_lossy())
        .unwrap();
    let (_, mut events) = core.sessions.subscribe("goal-rounds", 0).unwrap();
    core.sessions
        .dispatch(
            "goal-rounds",
            HarnessId::Native,
            RunRequest {
                prompt: "/goal Complete three checkpoints and validate the result".into(),
                harness: Some(HarnessId::Native),
                model: Some("fixture".into()),
                reasoning: None,
                model_options: Default::default(),
                cwd: project.path().to_string_lossy().into(),
                resume: None,
                attachments: vec![],
                worktree: None,
            },
            None,
        )
        .await
        .unwrap();
    let mut finished = 0;
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while finished < 3 {
            if let AgentEvent::Done { status, .. } = events.recv().await.unwrap().event {
                assert_eq!(status, DoneStatus::Completed);
                finished += 1;
            }
        }
    })
    .await
    .unwrap();
    let doc = core.doc_host.open("goal-rounds").unwrap();
    let goal = doc.doc().goal().unwrap();
    assert_eq!(goal.status, rcode_proto::goal::GoalStatus::Complete);
    assert_eq!(goal.iteration, 3);
    let entries = doc.doc().read_entries().unwrap();
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.role == rcode_doc::MessageRole::User)
            .count(),
        1,
        "internal continuations must not create user bubbles"
    );
    let assistants: Vec<_> = entries
        .iter()
        .filter(|entry| entry.role == rcode_doc::MessageRole::Assistant)
        .collect();
    assert_eq!(assistants.len(), 3);
    for (i, entry) in assistants.iter().enumerate() {
        assert!(entry.continuation_of.is_none());
        assert_eq!(entry.status, Some(rcode_doc::MessageStatus::Complete));
        assert!(entry.duration_ms.is_some_and(|ms| ms > 0));
        assert!(
            serde_json::to_string(entry)
                .unwrap()
                .contains(&format!("Goal checkpoint {}", i + 1))
        );
    }
    let ids: std::collections::HashSet<_> = assistants.iter().map(|entry| &entry.id).collect();
    assert_eq!(ids.len(), 3);
    let replica = rcode_doc::SessionDoc::init("replica").unwrap();
    replica
        .doc()
        .import(&doc.doc().export_snapshot().unwrap())
        .unwrap();
    assert_eq!(replica.read_entries().unwrap(), entries);
    assert_eq!(server.await.unwrap().len(), 6);
    core.sessions.shutdown().await;
}
