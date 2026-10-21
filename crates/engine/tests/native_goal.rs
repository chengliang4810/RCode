//! Real engine -> native runner -> loopback provider -> durable goal state.
use rcode_engine::EngineCore;
use rcode_proto::{AgentEvent, DoneStatus, HarnessId, RunRequest, goal::GoalStatus};
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
#[tokio::test]
async fn native_goal_advertises_management_tools_and_completes_only_after_tool_free_verification() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let _home = HomeGuard(key, std::env::var_os(key));
    unsafe {
        std::env::set_var(key, home.path());
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut seen = vec![];
        let mut delegated = false;
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![];
            let mut buffer = [0; 8192];
            loop {
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..end]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .map(|(_, value)| value.trim().parse().unwrap())
                    })
                    .unwrap();
                if request.len() < end + 4 + length {
                    continue;
                }
                seen.push(
                    serde_json::from_slice::<Value>(&request[end + 4..end + 4 + length]).unwrap(),
                );
                break;
            }
            assert!(seen.len() <= 8, "unexpected continuation loop: {seen:?}");
            let request = seen.last().unwrap();
            let names: Vec<_> = request["tools"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|tool| tool["function"]["name"].as_str())
                .collect();
            let verifying = names.is_empty();
            let main = names.contains(&"Agent");
            let (delta, finish) = if main && !delegated {
                delegated = true;
                (
                    json!({"tool_calls":[{"index":0,"id":"delegate","type":"function","function":{"name":"Agent","arguments":json!({"description":"Inspect change","prompt":"Inspect and validate the change, then report your result.","run_in_background":true}).to_string()}}]}),
                    "tool_calls",
                )
            } else {
                let text = if verifying {
                    r#"{"passed":true,"reason":"implementation and child validation are recorded"}"#
                } else if main {
                    "Implemented and validated."
                } else {
                    "Child validation completed."
                };
                (json!({"content":text}), "stop")
            };
            let first = json!({"choices":[{"delta":delta,"finish_reason":null}]});
            let last = json!({"choices":[{"delta":{},"finish_reason":finish}],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}});
            let body = format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n");
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            if verifying {
                break;
            }
        }
        seen
    });
    std::fs::write(root.path().join("providers.json"), json!({
        "providers":[{"id":"test","label":"Local fixture","protocol":"open_ai_chat","baseUrl":format!("http://{address}/v1"),"apiKey":"fixture-only",
            "models":[{"id":"goal-test","model":"goal-test","label":"Fixture","maxTokens":2048,"contextWindow":200000}]}],"defaultModel":"goal-test"
    }).to_string()).unwrap();
    std::fs::write(
        root.path().join("agent-settings.json"),
        r#"{"mcp":false,"memory":false}"#,
    )
    .unwrap();
    let registry = Arc::new(rcode_engine::registry_with_native(root.path()));
    let core = EngineCore::assemble(root.path(), registry, HarnessId::Native, None).unwrap();
    core.workspace
        .create_chat("goal-chat", None, Some(&core.device_id), None, None)
        .unwrap();
    core.workspace
        .rename_chat("goal-chat", "Goal fixture")
        .unwrap();
    core.workspace
        .set_chat_cwd("goal-chat", &project.path().to_string_lossy())
        .unwrap();
    let (_, mut events) = core.sessions.subscribe("goal-chat", 0).unwrap();
    core.sessions
        .dispatch(
            "goal-chat",
            HarnessId::Native,
            RunRequest {
                prompt: "/goal Implement and validate the change".into(),
                harness: Some(HarnessId::Native),
                model: Some("goal-test".into()),
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
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if let AgentEvent::Done { status, .. } = events.recv().await.unwrap().event {
                assert_eq!(status, DoneStatus::Completed);
                if core.doc_host.open("goal-chat").unwrap().doc().goal().is_some_and(|goal| goal.status == GoalStatus::Complete) { break; }
            }
        }
    })
    .await
    .unwrap();
    let doc = core.doc_host.open("goal-chat").unwrap();
    let goal = doc.doc().goal().unwrap();
    assert_eq!(goal.status, GoalStatus::Complete);
    let requests = server.await.unwrap();
    assert!(
        requests.len() >= 4,
        "background child must reach the provider"
    );
    assert_eq!(goal.tokens_used, requests.len() as u64 * 12);
    assert!(
        serde_json::to_value(&goal)
            .unwrap()
            .get("tokenBudget")
            .is_none()
    );
    let names: Vec<_> = requests[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect();
    for name in [
        "Agent",
        "TaskList",
        "TaskOutput",
        "TaskStop",
        "TaskBackground",
        "SendMessage",
        "GoalRead",
    ] {
        assert!(names.contains(&name), "missing {name}: {names:?}");
    }
    assert!(
        requests
            .last()
            .unwrap()
            .get("tools")
            .is_none_or(|tools| tools.as_array().is_none_or(Vec::is_empty))
    );
    assert!(
        requests.last().unwrap()["messages"]
            .to_string()
            .contains("Independently verify")
    );
    assert!(
        requests.last().unwrap()["messages"]
            .to_string()
            .contains("Child validation completed.")
    );
    let entries = doc.doc().read_entries().unwrap();
    let child_id = entries
        .iter()
        .flat_map(|entry| &entry.parts)
        .find_map(|part| match part {
            rcode_doc::MessagePart::Tool {
                id,
                subagent_ref,
                subagent_status,
                ..
            } if id == "delegate" => {
                assert_eq!(*subagent_status, Some(rcode_doc::SubagentStatus::Done));
                subagent_ref.clone()
            }
            _ => None,
        })
        .expect("background agent has a child transcript");
    let child = core.doc_host.open(&child_id).unwrap();
    assert!(
        serde_json::to_string(&child.doc().read_entries().unwrap())
            .unwrap()
            .contains("Child validation completed.")
    );
    let replica = rcode_doc::SessionDoc::init("replica").unwrap();
    replica
        .doc()
        .import(&doc.doc().export_snapshot().unwrap())
        .unwrap();
    assert_eq!(replica.goal(), Some(goal));
    core.sessions.shutdown().await;
}
