use futures::StreamExt;
use rcode_engine::{EngineCore, HarnessRegistry};
use rcode_preview::discovery::Listener;
use rcode_proto::HarnessId;
use rcode_rpc::{RpcReply, RpcService, methods};
use std::{path::Path, sync::Arc, time::Duration};

fn listener(root: &Path, port: u16) -> Listener {
    Listener {
        pid: 123,
        parent: 1,
        cwd: root.to_path_buf(),
        args: vec!["node".into(), "vite".into()],
        started_at: 1,
        address: ([127, 0, 0, 1], port).into(),
        rcode_owned: true,
    }
}

#[tokio::test]
async fn preview_watch_follows_the_session_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    // Canonical paths so the chat cwd and the observed service cwd compare equal.
    let a = std::fs::canonicalize(&a).unwrap();
    let b = std::fs::canonicalize(&b).unwrap();
    let core = EngineCore::assemble(
        &temp.path().join("engine"),
        Arc::new(HarnessRegistry::new()),
        HarnessId::Mock,
        None,
    )
    .unwrap();
    core.workspace
        .create_chat(
            "chat",
            None,
            None,
            None,
            Some(a.to_string_lossy().into_owned()),
        )
        .unwrap();
    let catalog = core.previews.catalog();
    // Two dev servers on this device, in two checkouts of the same project.
    catalog
        .replace_local(vec![
            (a.clone(), listener(&a, 5173)),
            (b.clone(), listener(&b, 5174)),
        ])
        .unwrap();
    let id_a = catalog
        .snapshot()
        .services
        .iter()
        .find(|s| s.project_cwd == a.to_string_lossy().as_ref())
        .unwrap()
        .id
        .clone();
    let rpc = core.rpc_service();
    let RpcReply::Stream(mut stream) = rpc
        .handle(
            methods::WATCH_PREVIEWS,
            serde_json::json!({"chatId":"chat"}),
        )
        .await
        .unwrap()
    else {
        panic!("expected preview watch")
    };
    let first = stream.next().await.unwrap();
    assert_eq!(first["services"].as_array().unwrap().len(), 1);
    assert_eq!(first["services"][0]["id"], id_a);
    core.workspace
        .set_chat_cwd("chat", &b.to_string_lossy())
        .unwrap();
    let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next["services"].as_array().unwrap().len(), 1);
    assert_eq!(
        next["services"][0]["projectCwd"],
        b.to_string_lossy().as_ref()
    );
    drop(stream);
    core.shutdown().await;
}
