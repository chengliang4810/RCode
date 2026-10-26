use futures::StreamExt;
use rcode_engine::EngineCore;
use rcode_harness::{Harness, RunControls, SteerMessage, TurnControl, native::NativeHarness};
use rcode_proto::{AgentEvent, DoneStatus, HarnessId, RunRequest};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[path = "support/native_idle.rs"]
mod native_idle;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    let mut request = Vec::new();
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
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
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
    let last = json!({"choices":[{"delta":{},"finish_reason":finish}],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}});
    let body = format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n");
    socket
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
}

fn request(project: &std::path::Path, prompt: &str) -> RunRequest {
    RunRequest {
        prompt: prompt.into(),
        harness: Some(HarnessId::Native),
        model: Some("fixture".into()),
        reasoning: None,
        model_options: Default::default(),
        cwd: project.to_string_lossy().into(),
        resume: None,
        attachments: Vec::new(),
        worktree: None,
    }
}

#[tokio::test]
async fn excess_idle_native_runtime_is_released_and_restores_full_tool_history() {
    let _serial = TEST_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let home_key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let _home = HomeGuard(home_key, std::env::var_os(home_key));
    unsafe { std::env::set_var(home_key, home.path()) };
    let fixture_path = project.path().join("history.txt");
    std::fs::write(
        &fixture_path,
        format!(
            "{}FULL_TOOL_RESULT_TAIL",
            "full tool result line\n".repeat(250)
        ),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut phase = 0;
        while phase < 3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let messages = request["messages"].to_string();
            if messages.contains(native_idle::FILL_MARKER) {
                reply(&mut socket, json!({"content":"Warm slot ready"}), "stop").await;
                continue;
            }
            if phase == 0 {
                reply(
                    &mut socket,
                    json!({"tool_calls":[{"index":0,"id":"read-once","type":"function","function":{"name":"Read","arguments":json!({"file_path":fixture_path}).to_string()}}]}),
                    "tool_calls",
                )
                .await;
            } else {
                assert!(messages.contains("FULL_TOOL_RESULT_TAIL"));
                if phase == 2 {
                    assert!(messages.contains("First turn fixture"));
                    assert!(messages.contains("First answer marker"));
                    assert!(messages.contains("Continue after runtime release"));
                    let tool_calls = request["messages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|message| message["tool_calls"].as_array())
                        .flatten()
                        .filter(|call| call["id"] == "read-once")
                        .count();
                    assert_eq!(tool_calls, 1);
                }
                reply(
                    &mut socket,
                    json!({"content":if phase == 1 {"First answer marker"} else {"History restored"}}),
                    "stop",
                )
                .await;
            }
            phase += 1;
        }
    });
    std::fs::write(
        root.path().join("providers.json"),
        json!({"providers":[{"id":"fixture","protocol":"open_ai_chat","baseUrl":format!("http://{address}/v1"),"apiKey":"fixture-only","models":[{"id":"fixture","model":"fixture","maxTokens":2048,"contextWindow":200000}]}],"defaultModel":"fixture"}).to_string(),
    )
    .unwrap();
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
    native_idle::fill(&core, project.path()).await;
    let chat_id = "idle-recovery";
    core.workspace
        .create_chat(chat_id, None, Some(&core.device_id), None, None)
        .unwrap();
    core.workspace
        .rename_chat(chat_id, "Idle recovery")
        .unwrap();
    core.workspace
        .set_chat_cwd(chat_id, &project.path().to_string_lossy())
        .unwrap();
    let (_, mut events) = core.sessions.subscribe(chat_id, 0).unwrap();
    for prompt in ["First turn fixture", "Continue after runtime release"] {
        let retired_handle = {
            let handle = core.doc_host.open(chat_id).unwrap();
            Arc::downgrade(&handle)
        };
        core.sessions
            .dispatch(
                chat_id,
                HarnessId::Native,
                request(project.path(), prompt),
                None,
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if let AgentEvent::Done { status, .. } = events.recv().await.unwrap().event {
                    assert_eq!(status, DoneStatus::Completed, "prompt: {prompt}");
                    break;
                }
            }
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(15), async {
            while core.sessions.live_run_steerable(chat_id)
                || core.doc_host.sync_resources()["retainedBy"]["writers"] != 12
                || retired_handle.upgrade().is_some()
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let loads_before_reopen = core.doc_host.sync_resources()["documentLoads"]
            .as_u64()
            .unwrap();
        let handle = core.doc_host.open(chat_id).unwrap();
        assert_eq!(
            core.doc_host.sync_resources()["documentLoads"],
            loads_before_reopen + 1
        );
        assert_eq!(
            handle.doc().read_entries().unwrap().last().unwrap().status,
            Some(rcode_doc::MessageStatus::Complete)
        );
        while events.try_recv().is_ok() {}
    }
    tokio::time::timeout(Duration::from_secs(20), server)
        .await
        .unwrap()
        .unwrap();
    core.sessions.shutdown().await;
    assert_eq!(core.doc_host.sync_resources()["retainedBy"]["writers"], 0);
}

#[tokio::test]
async fn immediate_followup_messages_survive_idle_capacity_handoffs() {
    let _serial = TEST_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let home_key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let _home = HomeGuard(home_key, std::env::var_os(home_key));
    unsafe { std::env::set_var(home_key, home.path()) };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut phase = 0;
        while phase < 20 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let messages = request["messages"].to_string();
            if messages.contains(native_idle::FILL_MARKER) {
                reply(&mut socket, json!({"content":"Warm slot ready"}), "stop").await;
                continue;
            }
            assert!(messages.contains(&format!("native-idle-handoff-{phase}")));
            reply(
                &mut socket,
                json!({"content":format!("Handoff answer {phase}")}),
                "stop",
            )
            .await;
            phase += 1;
        }
    });
    std::fs::write(
        root.path().join("providers.json"),
        json!({"providers":[{"id":"fixture","protocol":"open_ai_chat","baseUrl":format!("http://{address}/v1"),"apiKey":"fixture-only","models":[{"id":"fixture","model":"fixture","maxTokens":2048,"contextWindow":200000}]}],"defaultModel":"fixture"}).to_string(),
    )
    .unwrap();
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
    native_idle::fill(&core, project.path()).await;
    let chat_id = "idle-handoff";
    core.workspace
        .create_chat(chat_id, None, Some(&core.device_id), None, None)
        .unwrap();
    core.workspace.rename_chat(chat_id, "Idle handoff").unwrap();
    core.workspace
        .set_chat_cwd(chat_id, &project.path().to_string_lossy())
        .unwrap();
    let (_, mut events) = core.sessions.subscribe(chat_id, 0).unwrap();
    for phase in 0..20 {
        core.sessions
            .dispatch(
                chat_id,
                HarnessId::Native,
                request(project.path(), &format!("native-idle-handoff-{phase}")),
                None,
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if let AgentEvent::Done { status, .. } = events.recv().await.unwrap().event {
                    assert_eq!(status, DoneStatus::Completed);
                    break;
                }
            }
        })
        .await
        .unwrap();
    }
    let handle = core.doc_host.open(chat_id).unwrap();
    let entries = handle.doc().read_entries().unwrap();
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.role == rcode_doc::MessageRole::User)
            .count(),
        20
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.role == rcode_doc::MessageRole::Assistant)
            .count(),
        20
    );
    tokio::time::timeout(Duration::from_secs(20), server)
        .await
        .unwrap()
        .unwrap();
    core.sessions.shutdown().await;
}

#[tokio::test]
async fn failed_history_persistence_keeps_unsaved_tool_results_across_repaired_turns() {
    let _serial = TEST_LOCK.lock().await;
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let home_key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let _home = HomeGuard(home_key, std::env::var_os(home_key));
    unsafe { std::env::set_var(home_key, home.path()) };
    let history_path = root.path().join("model-history");
    let history_backup = root.path().join("history-before-fault");
    let history_literal = history_path.to_string_lossy().replace('\'', "''");
    let backup_literal = history_backup.to_string_lossy().replace('\'', "''");
    let command = if cfg!(windows) {
        format!(
            "Move-Item -LiteralPath '{history_literal}' -Destination '{backup_literal}'; [IO.File]::WriteAllText('{history_literal}', 'fixture storage fault'); Write-Output ('UNSAVED_' + 'TOOL_OUTPUT')"
        )
    } else {
        format!(
            "mv '{history_literal}' '{backup_literal}' && printf 'fixture storage fault' > '{history_literal}' && printf 'UNSAVED_%s' 'TOOL_OUTPUT'"
        )
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut phase = 0;
        while phase < 3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            if request["messages"]
                .to_string()
                .contains(native_idle::FILL_MARKER)
            {
                reply(&mut socket, json!({"content":"Warm slot ready"}), "stop").await;
                continue;
            }
            if phase == 0 {
                reply(
                    &mut socket,
                    json!({"tool_calls":[{"index":0,"id":"fault-once","type":"function","function":{"name":"Bash","arguments":json!({"command":command}).to_string()}}]}),
                    "tool_calls",
                )
                .await;
            } else {
                let results = request["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|message| message["role"] == "tool")
                    .map(|message| message["content"].to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(results.contains("UNSAVED_TOOL_OUTPUT"), "{results}");
                if phase == 2 {
                    assert!(
                        request["messages"]
                            .to_string()
                            .contains("After repair answer")
                    );
                }
                reply(
                    &mut socket,
                    json!({"content":if phase == 1 {"After repair answer"} else {"Still retained"}}),
                    "stop",
                )
                .await;
            }
            phase += 1;
        }
    });
    std::fs::write(
        root.path().join("providers.json"),
        json!({"providers":[{"id":"fixture","protocol":"open_ai_chat","baseUrl":format!("http://{address}/v1"),"apiKey":"fixture-only","models":[{"id":"fixture","model":"fixture","maxTokens":2048,"contextWindow":200000}]}],"defaultModel":"fixture"}).to_string(),
    )
    .unwrap();
    std::fs::write(
        root.path().join("agent-settings.json"),
        r#"{"mcp":false,"memory":false}"#,
    )
    .unwrap();
    let harness = Arc::new(NativeHarness::from_data_dir(root.path()));
    let registry = rcode_engine::HarnessRegistry::new();
    registry.register(harness.clone());
    let core =
        EngineCore::assemble(root.path(), Arc::new(registry), HarnessId::Native, None).unwrap();
    native_idle::fill(&core, project.path()).await;
    let turn = TurnControl::default();
    let (steering, receiver) = tokio::sync::mpsc::channel(8);
    let mut events = harness
        .run(
            request(project.path(), "Begin fault"),
            RunControls {
                conversation_id: Some("idle-storage-fault".into()),
                goal: None,
                request_input: Box::new(|_| tokio::sync::oneshot::channel().1),
                steering: receiver,
                interrupt: tokio_util::sync::CancellationToken::new(),
                turn: turn.clone(),
                history: None,
            },
        )
        .await
        .unwrap();
    for (index, prompt) in ["Begin fault", "Continue after repair", "Continue again"]
        .into_iter()
        .enumerate()
    {
        if index > 0 {
            steering.send(SteerMessage::text(prompt)).await.unwrap();
        }
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if let AgentEvent::Done { status, error, .. } =
                    events.next().await.unwrap().unwrap()
                {
                    if index == 0 {
                        assert_eq!(status, DoneStatus::Errored);
                        assert!(error.unwrap().contains("cannot persist model history"));
                    } else {
                        assert_eq!(status, DoneStatus::Completed);
                    }
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), turn.wait_for_runtime_exit())
                .await
                .is_err()
        );
        if index == 0 {
            assert!(history_path.is_file());
            std::fs::remove_file(&history_path).unwrap();
            std::fs::rename(&history_backup, &history_path).unwrap();
        }
    }
    tokio::time::timeout(Duration::from_secs(20), server)
        .await
        .unwrap()
        .unwrap();
    drop(events);
    tokio::time::timeout(Duration::from_secs(10), turn.wait_for_runtime_exit())
        .await
        .unwrap();
    core.sessions.shutdown().await;
}
