//! MCP configuration through the real RPC surface: what the settings page
//! writes is what the harness's loader reads, and a write touches only the
//! entry it named.

use std::sync::Arc;

use rcode_engine::EngineCore;
use rcode_proto::HarnessId;
use rcode_rpc::methods;

const CHAT: &str = "mcp-settings-chat";

#[tokio::test]
async fn creating_personal_servers_preserves_both_transports_and_other_entries() {
    let (_tmp, project_dir, core) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());
    for (name, server) in [
        (
            "files",
            serde_json::json!({
                "type": "stdio", "command": "node", "args": ["C:\\Program Files\\server.js", ""],
                "env": {"EXAMPLE_KEY": "example-value"},
            }),
        ),
        (
            "docs",
            serde_json::json!({
                "type": "http", "url": "https://example.invalid/mcp",
                "headers": {"X-Example": "example-value"},
            }),
        ),
    ] {
        let rows = client
            .call(
                methods::SET_MCP_SERVER,
                serde_json::json!({
                    "personal": true, "scope": "user", "createOnly": true,
                    "name": name, "server": server,
                }),
            )
            .await
            .unwrap();
        assert!(
            rows.as_array()
                .unwrap()
                .iter()
                .all(|row| row["source"] == "user")
        );
    }
    let written: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(core.config_root().join("mcp.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        written["mcpServers"]["files"]["args"],
        serde_json::json!(["C:\\Program Files\\server.js", ""])
    );
    assert_eq!(
        written["mcpServers"]["files"]["env"]["EXAMPLE_KEY"],
        "example-value"
    );
    assert_eq!(
        written["mcpServers"]["docs"]["headers"]["X-Example"],
        "example-value"
    );
    let loaded = rcode_mcp::config::load_personal(core.config_root()).unwrap();
    assert_eq!(loaded.len(), 2);
    assert!(!project_dir.path().join("project/.rcode/mcp.json").exists());
}

#[tokio::test]
async fn invalid_or_duplicate_creations_leave_the_config_unchanged() {
    let (_tmp, _project_dir, core) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());
    client
        .call(
            methods::SET_MCP_SERVER,
            serde_json::json!({
                "personal": true, "scope": "user", "createOnly": true,
                "name": "keep", "server": {"command": "original"},
            }),
        )
        .await
        .unwrap();
    let path = core.config_root().join("mcp.json");
    let original = std::fs::read(&path).unwrap();
    for (name, server) in [
        ("keep", serde_json::json!({"command":"replacement"})),
        (
            "invalid",
            serde_json::json!({"command":"node", "url":"https://example.invalid"}),
        ),
        ("invalid", serde_json::json!({"command":"node", "args":[1]})),
        (
            "invalid",
            serde_json::json!({"command":"node", "env":{"KEY":1}}),
        ),
        ("invalid", serde_json::json!({"url":"file:///tmp/server"})),
        (
            "invalid",
            serde_json::json!({"type":"sse", "url":"https://example.invalid"}),
        ),
        (
            "invalid",
            serde_json::json!({"url":"https://example.invalid", "headers":{"X-Test":"line\nbreak"}}),
        ),
        ("invalid", serde_json::json!([])),
        ("invalid", serde_json::Value::Null),
        ("", serde_json::json!({"command":"node"})),
    ] {
        assert!(
            client
                .call(
                    methods::SET_MCP_SERVER,
                    serde_json::json!({
                        "personal": true, "scope": "user", "createOnly": true,
                        "name": name, "server": server,
                    })
                )
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}

async fn core() -> (tempfile::TempDir, tempfile::TempDir, EngineCore) {
    let tmp = tempfile::tempdir().unwrap();
    // The project must live outside the process home: a user-scope entry is
    // written under the config root, and a project under `~` would otherwise
    // be the same directory.
    let project_dir = tempfile::tempdir().unwrap();
    let project = project_dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let data = tmp.path().join("data");
    let registry = rcode_engine::registry_with_native(&data);
    let core = EngineCore::assemble(&data, Arc::new(registry), HarnessId::Native, None)
        .expect("engine core assembles");
    let client = rcode_rpc::memory_client(core.rpc_service());
    client
        .call(
            methods::MUTATE,
            serde_json::json!({"op":"createChat", "chatId":CHAT, "deviceId":core.device_id}),
        )
        .await
        .unwrap();
    core.workspace
        .set_chat_cwd(CHAT, &project.to_string_lossy())
        .expect("the chat gets a cwd");
    (tmp, project_dir, core)
}

#[tokio::test]
async fn a_written_server_is_what_the_loader_reads() {
    let (_tmp, project_dir, core) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());
    let project = project_dir.path().join("project");

    let listed: Vec<serde_json::Value> = serde_json::from_value(
        client
            .call(
                methods::SET_MCP_SERVER,
                serde_json::json!({
                    "chatId": CHAT,
                    "name": "files",
                    "server": { "command": "npx", "args": ["-y", "server-filesystem"] },
                    "scope": "project",
                }),
            )
            .await
            .expect("the write replies with the listing"),
    )
    .unwrap();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["name"], "files");
    assert_eq!(listed[0]["source"], "project");
    assert_eq!(listed[0]["transport"], "npx -y server-filesystem");

    // The file is where the harness's loader looks.
    let written = project.join(".rcode").join("mcp.json");
    assert!(written.is_file(), "{}", written.display());
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&written).unwrap()).unwrap();
    assert_eq!(parsed["mcpServers"]["files"]["command"], "npx");

    // And the loader agrees.
    let servers = rcode_mcp::load(&project, core.config_root()).unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].name, "files");
}

#[tokio::test]
async fn a_write_keeps_the_other_servers_in_the_file() {
    let (_tmp, project_dir, core) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());
    let project = project_dir.path().join("project");

    for (name, command) in [("one", "cmd-one"), ("two", "cmd-two")] {
        client
            .call(
                methods::SET_MCP_SERVER,
                serde_json::json!({
                    "chatId": CHAT,
                    "name": name,
                    "server": { "command": command },
                    "scope": "project",
                }),
            )
            .await
            .unwrap();
    }

    let servers = rcode_mcp::load(&project, core.config_root()).unwrap();
    let names: Vec<&str> = servers.iter().map(|server| server.name.as_str()).collect();
    assert_eq!(names, ["one", "two"], "the first write survived the second");
}

#[tokio::test]
async fn removing_a_server_deletes_only_that_entry() {
    let (_tmp, _project_dir, core) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());

    for name in ["keep", "drop"] {
        client
            .call(
                methods::SET_MCP_SERVER,
                serde_json::json!({
                    "chatId": CHAT,
                    "name": name,
                    "server": { "command": "x" },
                    "scope": "project",
                }),
            )
            .await
            .unwrap();
    }
    let after: Vec<serde_json::Value> = serde_json::from_value(
        client
            .call(
                methods::SET_MCP_SERVER,
                serde_json::json!({
                    "chatId": CHAT,
                    "name": "drop",
                    "server": null,
                    "scope": "project",
                }),
            )
            .await
            .unwrap(),
    )
    .unwrap();

    let names: Vec<&str> = after.iter().map(|v| v["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["keep"]);
}

#[tokio::test]
async fn the_project_entry_shadows_the_users() {
    let (_tmp, _project_dir, core) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());

    for (scope, command) in [("user", "user-cmd"), ("project", "project-cmd")] {
        client
            .call(
                methods::SET_MCP_SERVER,
                serde_json::json!({
                    "chatId": CHAT,
                    "name": "shared",
                    "server": { "command": command },
                    "scope": scope,
                }),
            )
            .await
            .unwrap();
    }

    let listed: Vec<serde_json::Value> = serde_json::from_value(
        client
            .call(
                methods::LIST_MCP_SERVERS,
                serde_json::json!({ "chatId": CHAT }),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let shared: Vec<&serde_json::Value> = listed
        .iter()
        .filter(|server| server["name"] == "shared")
        .collect();
    assert_eq!(shared.len(), 1, "{listed:#?}");
    assert_eq!(shared[0]["source"], "project");
    assert_eq!(shared[0]["transport"], "project-cmd");
}
