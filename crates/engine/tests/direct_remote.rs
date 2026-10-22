//! Desktop memory RPC routes to an independently authenticated engine.
use rcode_doc::{SessionCommandEntry, SessionCommandPayload, SessionCommandStatus};
use rcode_engine::{EngineCore, HarnessRegistry};
use rcode_harness::mock::MockHarness;
use rcode_proto::{AgentEvent, DoneStatus, HarnessId, RunRequest};
use rcode_rpc::{RpcClient, methods};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

fn core(root: &std::path::Path) -> EngineCore {
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(MockHarness {
        script: vec![
            AgentEvent::TextDelta {
                text: "remote execution completed".into(),
            },
            AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: None,
            },
        ],
    }));
    EngineCore::assemble(root, Arc::new(registry), HarnessId::Mock, None).unwrap()
}

async fn catalog(
    client: &RpcClient,
    method: &str,
    mut predicate: impl FnMut(&Value) -> bool,
) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut stream = client.subscribe_scoped(method, json!({})).await.unwrap();
        while let Some(value) = stream.recv().await {
            if predicate(&value) {
                return value;
            }
        }
        panic!("catalog ended");
    })
    .await
    .expect("catalog converges")
}

#[tokio::test]
async fn memory_desktop_routes_remote_creates_commands_and_catalogs_without_replaying() {
    let local_root = tempfile::tempdir().unwrap();
    let remote_root = tempfile::tempdir().unwrap();
    let local = core(local_root.path());
    let remote = core(remote_root.path());
    let key = "Fixture12345";
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    remote
        .remote_access
        .start(Arc::new(
            remote
                .rpc_service()
                .as_ref()
                .clone()
                .without_remote_access(),
        ))
        .await;
    remote
        .remote_access
        .configure_runtime(key.into(), port)
        .await
        .unwrap();
    let local_client = rcode_rpc::memory_client(local.rpc_service());
    let host = local_client
        .call(
            "SaveRemoteHost",
            json!({"name":"Remote fixture","url":format!("ws://127.0.0.1:{port}"),"apiKey":key}),
        )
        .await
        .unwrap();
    catalog(&local_client, methods::WATCH_DEVICES, |rows| {
        rows.as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == remote.device_id)
    })
    .await;
    local_client
        .call(
            methods::MUTATE,
            json!({
                "op":"createChat","chatId":"remote-chat","deviceId":remote.device_id,
                "cwd":remote_root.path().to_string_lossy(),"config":{"harness":"mock"}
            }),
        )
        .await
        .unwrap();
    let entry = SessionCommandEntry {
        id: "direct-command-1".into(),
        payload: SessionCommandPayload::Run {
            message_id: "direct-message-1".into(),
            request: RunRequest {
                prompt: "complete on the remote host".into(),
                harness: Some(HarnessId::Mock),
                model: None,
                reasoning: None,
                model_options: Default::default(),
                cwd: remote_root.path().to_string_lossy().into(),
                attachments: vec![],
                worktree: None,
                resume: None,
            },
        },
        issued_by: local.device_id.clone(),
        issued_at: chrono::Utc::now().timestamp_millis(),
        based_on: None,
        expires_at: None,
        status: SessionCommandStatus::Pending,
        resolution: None,
    };
    // No targetDeviceId and no wait for WatchChats: accepted creates already route.
    local_client
        .call(
            methods::RELAY_COMMAND,
            json!({"chatId":"remote-chat","entry":entry}),
        )
        .await
        .unwrap();
    let mut transcript = local_client
        .subscribe_scoped(methods::WATCH_DOC_MESSAGES, json!({"chatId":"remote-chat"}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(frame) = transcript.recv().await {
            if frame.to_string().contains("remote execution completed") {
                break;
            }
        }
    })
    .await
    .unwrap();
    drop(transcript);
    local_client
        .call(
            methods::RELAY_COMMAND,
            json!({"chatId":"remote-chat","entry":entry}),
        )
        .await
        .unwrap();
    let handle = remote.doc_host.open("remote-chat").unwrap();
    assert_eq!(
        handle
            .doc()
            .read_entries()
            .unwrap()
            .iter()
            .filter(|entry| entry.id == "direct-message-1")
            .count(),
        1
    );
    assert!(
        local.workspace.chat("remote-chat").unwrap().is_none(),
        "foreign docs are not local execution state"
    );
    catalog(&local_client, methods::WATCH_CHATS, |rows| {
        rows.as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == "remote-chat")
    })
    .await;
    assert_eq!(
        remote
            .workspace
            .chat("remote-chat")
            .unwrap()
            .unwrap()
            .device_id,
        remote.device_id
    );
    local_client
        .call("RemoveRemoteHost", json!({"id":host["id"]}))
        .await
        .unwrap();
    let rows = catalog(&local_client, methods::WATCH_DEVICES, |rows| {
        !rows
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == remote.device_id)
    })
    .await;
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == local.device_id)
    );
    local.shutdown().await;
    remote.shutdown().await;
}
