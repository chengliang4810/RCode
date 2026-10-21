//! Native parent completion, detached child persistence and automatic completion wake.
use rcode_engine::EngineCore;
use rcode_proto::{AgentEvent, DoneStatus, HarnessId, RunRequest};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

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
async fn parent_can_finish_then_child_persists_and_wakes_a_new_parent_turn_once() {
    scenario(false, false).await;
    scenario(true, false).await;
    scenario(false, true).await;
}

async fn scenario(stop_parent: bool, manual: bool) {
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
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let child_started = Arc::new(tokio::sync::Notify::new());
    let parent_blocked = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let notifications = Arc::new(AtomicUsize::new(0));
    let shutdown = CancellationToken::new();
    let server = {
        let gate = gate.clone();
        let calls = calls.clone();
        let notifications = notifications.clone();
        let shutdown = shutdown.clone();
        let child_started = child_started.clone();
        let parent_blocked = parent_blocked.clone();
        tokio::spawn(async move {
            let mut jobs = tokio::task::JoinSet::new();
            loop {
                let socket = tokio::select! {_ = shutdown.cancelled()=>break, accepted=listener.accept()=>accepted.unwrap().0};
                let gate = gate.clone();
                let calls = calls.clone();
                let notifications = notifications.clone();
                let child_started = child_started.clone();
                let parent_blocked = parent_blocked.clone();
                jobs.spawn(async move {
                let mut socket=socket; let request=read_request(&mut socket).await;
                let main=request["tools"].as_array().unwrap().iter().any(|tool|tool["function"]["name"]=="Agent");
                if main {
                    let step=calls.fetch_add(1,Ordering::SeqCst);
                    if step==0 {
                        reply(&mut socket,json!({"tool_calls":[{"index":0,"id":"detached-review","type":"function","function":{"name":"Agent","arguments":json!({"description":"Review","prompt":"Perform the review","run_in_background":!manual}).to_string()}}]}),"tool_calls").await;
                    } else {
                        if stop_parent && step==1 { parent_blocked.notify_one(); futures::future::pending::<()>().await; }
                        let notified=request["messages"].to_string().contains("<task-notification>");
                        if notified { notifications.fetch_add(1,Ordering::SeqCst); }
                        reply(&mut socket,json!({"content":if notified {"Review result received."} else {"Review running in background."}}),"stop").await;
                    }
                } else {
                    child_started.notify_one();
                    gate.acquire().await.unwrap().forget();
                    reply(&mut socket,json!({"content":"Detached child review finished."}),"stop").await;
                }
            });
            }
            jobs.abort_all();
            while jobs.join_next().await.is_some() {}
        })
    };
    std::fs::write(root.path().join("providers.json"),json!({"providers":[{"id":"fixture","label":"Fixture","protocol":"open_ai_chat","baseUrl":format!("http://{address}/v1"),"apiKey":"fixture-only","models":[{"id":"fixture","model":"fixture","maxTokens":2048,"contextWindow":200000},{"id":"fixture-two","model":"fixture","maxTokens":2048,"contextWindow":200000}]}],"defaultModel":"fixture"}).to_string()).unwrap();
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
        .create_chat("background-chat", None, Some(&core.device_id), None, None)
        .unwrap();
    core.workspace
        .rename_chat("background-chat", "Background fixture")
        .unwrap();
    core.workspace
        .set_chat_cwd("background-chat", &project.path().to_string_lossy())
        .unwrap();
    let (_, mut events) = core.sessions.subscribe("background-chat", 0).unwrap();
    let request = |prompt: &str| RunRequest {
        prompt: prompt.into(),
        harness: Some(HarnessId::Native),
        model: Some("fixture".into()),
        reasoning: None,
        model_options: Default::default(),
        cwd: project.path().to_string_lossy().into(),
        resume: None,
        attachments: vec![],
        worktree: None,
    };
    core.sessions
        .dispatch(
            "background-chat",
            HarnessId::Native,
            request("Start a background review"),
            None,
        )
        .await
        .unwrap();
    if manual {
        tokio::time::timeout(std::time::Duration::from_secs(15), child_started.notified()).await.unwrap();
        let client = rcode_rpc::memory_client(core.rpc_service());
        let reply = client.call(rcode_rpc::methods::BACKGROUND_SUBAGENT, json!({"chatId":"background-chat","parentToolUseId":"detached-review"})).await.unwrap();
        assert_eq!(reply["backgrounded"],true);
    }
    if stop_parent {
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            parent_blocked.notified(),
        )
        .await
        .unwrap();
        core.doc_host.queue_command("background-chat", rcode_doc::SessionCommandPayload::Interrupt {}).unwrap();
    }
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if let AgentEvent::Done { status, .. } = events.recv().await.unwrap().event {
                assert_eq!(
                    status,
                    if stop_parent {
                        DoneStatus::Interrupted
                    } else {
                        DoneStatus::Completed
                    }
                );
                break;
            }
        }
        if !manual { child_started.notified().await; }
    })
    .await
    .unwrap();
    assert!(core.sessions.holds_background_work("background-chat"));
    let doc = core.doc_host.open("background-chat").unwrap();
    let child_id = doc
        .doc()
        .read_entries()
        .unwrap()
        .iter()
        .flat_map(|entry| &entry.parts)
        .find_map(|part| match part {
            rcode_doc::MessagePart::Tool {
                id, subagent_ref, ..
            } if id == "detached-review" => subagent_ref.clone(),
            _ => None,
        })
        .expect("child doc linked before parent completion");
    if stop_parent {
        gate.add_permits(1);
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                if let AgentEvent::Subagent { event, .. } = events.recv().await.unwrap().event {
                    if matches!(*event, AgentEvent::Done { .. }) {
                        break;
                    }
                }
            }
            while core.sessions.holds_background_work("background-chat") {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "stopping a turn suppresses autonomous continuation"
        );
    }
    // A user turn can execute while the original child is still blocked.
    let mut progress_request = request("Report progress");
    progress_request.model = Some("fixture-two".into());
    core.sessions
        .dispatch("background-chat", HarnessId::Native, progress_request, None)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            match events.recv().await.unwrap().event {
                AgentEvent::Steered { next_assistant_message_id, .. } => {
                    assert!(next_assistant_message_id.is_none(), "native steering must not borrow the user delivery id");
                }
                AgentEvent::Done { status, .. } => {
                    assert_eq!(status, DoneStatus::Completed);
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    if !stop_parent {
        assert!(core.sessions.holds_background_work("background-chat"));
        gate.add_permits(1);
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
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
    assert_eq!(
        calls.load(Ordering::SeqCst),
        if stop_parent { 3 } else { 4 },
        "spawn, initial final, user progress and one wake"
    );
    let entries = doc.doc().read_entries().unwrap();
    let unique_ids: std::collections::HashSet<_> = entries.iter().map(|entry| &entry.id).collect();
    assert_eq!(unique_ids.len(), entries.len(), "warm user prompts and assistant turns need independent identities");
    assert_eq!(notifications.load(Ordering::SeqCst), 1);
    assert!(!core.sessions.holds_background_work("background-chat"));
    let child = core.doc_host.open(&child_id).unwrap();
    let child_entries = child.doc().read_entries().unwrap();
    assert!(
        serde_json::to_string(&child_entries)
            .unwrap()
            .contains("Detached child review finished.")
    );
    assert_eq!(
        child_entries.last().unwrap().status,
        Some(rcode_doc::MessageStatus::Complete)
    );
    assert!(
        serde_json::to_string(&doc.doc().read_entries().unwrap())
            .unwrap()
            .contains("Review result received.")
    );
    core.sessions.shutdown().await;
    shutdown.cancel();
    server.await.unwrap();
}
