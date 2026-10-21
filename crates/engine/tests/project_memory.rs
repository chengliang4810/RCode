use std::sync::Arc;

use rcode_engine::{EngineCore, HarnessRegistry};
use rcode_proto::{HarnessId, ProjectMemoryListing, WorkspaceFileText};
use rcode_rpc::methods;
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_memory_can_be_listed_edited_and_disabled_through_the_real_rpcs() {
    let fixture = tempfile::tempdir().unwrap();
    let project = fixture.path().join("project");
    let config = fixture.path().join("config");
    std::fs::create_dir_all(&project).unwrap();
    let core = EngineCore::assemble(
        &config,
        Arc::new(HarnessRegistry::new()),
        HarnessId::Mock,
        None,
    )
    .unwrap();
    core.workspace
        .create_chat(
            "memory-chat",
            None,
            Some(&core.device_id),
            None,
            Some(project.to_string_lossy().into_owned()),
        )
        .unwrap();
    let client = rcode_rpc::memory_client(core.rpc_service());
    let directory = rcode_agent::memory::directory(&project, &config);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("MEMORY.md"),
        "- [Shell](shell.md) — terminal preference",
    )
    .unwrap();
    std::fs::write(directory.join("shell.md"), "# Shell\n\nPrefer PowerShell\n").unwrap();
    let listing: ProjectMemoryListing = serde_json::from_value(
        client
            .call(
                methods::LIST_PROJECT_MEMORY,
                json!({"chatId":"memory-chat"}),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(listing.enabled);
    assert_eq!(listing.files.len(), 2);
    assert_eq!(listing.files[0].path, "MEMORY.md");
    let file: WorkspaceFileText = serde_json::from_value(
        client
            .call(
                methods::READ_WORKSPACE_FILE,
                json!({"chatId":"memory-chat","memory":true,"path":"shell.md"}),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(file.read_only_reason.is_none());
    assert!(file.text.as_deref().unwrap().contains("Prefer PowerShell"));
    let save = json!({"chatId":"memory-chat","memory":true,"path":"shell.md","text":"# Shell\n\nUse PowerShell 7\n","expectedCheckoutId":file.checkout_id,"expectedContentHash":file.content_hash,"encoding":"utf8","lineEnding":"lf"});
    let saved = client
        .call(methods::WRITE_WORKSPACE_FILE, save.clone())
        .await
        .unwrap();
    assert_eq!(saved["status"], "written");
    assert!(
        std::fs::read_to_string(directory.join("shell.md"))
            .unwrap()
            .contains("Use PowerShell 7")
    );
    let stale = client
        .call(methods::WRITE_WORKSPACE_FILE, save)
        .await
        .unwrap();
    assert_eq!(stale["status"], "conflict");
    assert!(
        client
            .call(
                methods::READ_WORKSPACE_FILE,
                json!({"chatId":"memory-chat","path":"shell.md"})
            )
            .await
            .is_err()
    );
    let disabled = client
        .call(methods::SET_AGENT_SETTINGS, json!({"memory":false}))
        .await
        .unwrap();
    assert_eq!(disabled["memory"], false);
    let listing: ProjectMemoryListing = serde_json::from_value(
        client
            .call(
                methods::LIST_PROJECT_MEMORY,
                json!({"chatId":"memory-chat"}),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(!listing.enabled);
    assert!(listing.directory.is_none());
    assert!(listing.files.is_empty());
    let opened_file = client
        .call(
            methods::READ_WORKSPACE_FILE,
            json!({"chatId":"memory-chat","memory":true,"path":"shell.md"}),
        )
        .await
        .unwrap();
    assert!(
        opened_file["text"]
            .as_str()
            .unwrap()
            .contains("Use PowerShell 7")
    );
    assert!(directory.join("shell.md").exists());
    core.shutdown().await;
}
