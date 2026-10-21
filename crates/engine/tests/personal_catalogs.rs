//! Settings catalogs remain personal even with a project context and name collisions.

use std::{path::Path, sync::Arc};

use rcode_engine::EngineCore;
use rcode_proto::HarnessId;
use rcode_rpc::methods;
use serde_json::{Value, json};

struct HomeGuard(&'static str, Option<std::ffi::OsString>);

impl HomeGuard {
    fn set(home: &Path) -> Self {
        let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let previous = std::env::var_os(key);
        // This integration binary has a single test, on a current-thread runtime.
        unsafe { std::env::set_var(key, home) };
        Self(key, previous)
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.1.take() {
            Some(previous) => unsafe { std::env::set_var(self.0, previous) },
            None => unsafe { std::env::remove_var(self.0) },
        }
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn only_personal(rows: &Value) {
    assert!(
        rows.as_array().unwrap().iter().all(|row| {
            let scope = row.get("scope").or_else(|| row.get("source")).unwrap_or(&Value::Null);
            scope == "user"
                || scope == "plugin"
                || row["path"]
                    .as_str()
                    .is_some_and(|path| path.starts_with("builtin:"))
        }),
        "{rows:#}"
    );
}

#[tokio::test]
async fn settings_lists_and_writes_exclude_projects() {
    let home = tempfile::tempdir().unwrap();
    let _home = HomeGuard::set(home.path());
    let config = home.path().join(".rcode");
    let registry = rcode_engine::registry_with_native(&config);
    let core = EngineCore::assemble(&config, Arc::new(registry), HarnessId::Native, None).unwrap();
    let client = rcode_rpc::memory_client(core.rpc_service());
    let project = tempfile::tempdir().unwrap();
    client
        .call(
            methods::MUTATE,
            json!({
                "op": "createChat", "chatId": "project", "deviceId": core.device_id,
            }),
        )
        .await
        .unwrap();
    core.workspace
        .set_chat_cwd("project", &project.path().to_string_lossy())
        .unwrap();

    // Deliberate collisions prove that filtering a merged project listing is insufficient.
    write(
        &project.path().join(".agents/skills/shared/SKILL.md"),
        "---\nname: shared\ndescription: Project skill\n---\nProject instructions.",
    );
    write(
        &project.path().join(".rcode/agents/shared.md"),
        "---\nname: shared\ndescription: Project agent\n---\nProject prompt.",
    );
    write(
        &project.path().join(".rcode/mcp.json"),
        r#"{"mcpServers":{"shared":{"command":"project-server"}}}"#,
    );

    for (method, extra) in [
        (
            methods::CREATE_SKILL,
            json!({ "body": "Personal instructions." }),
        ),
        (
            methods::CREATE_SUBAGENT,
            json!({ "systemPrompt": "Personal prompt." }),
        ),
    ] {
        let mut params = json!({
            "personal": true, "chatId": "project", "scope": "user",
            "name": "shared", "description": "Personal definition",
        });
        params
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let rows = client.call(method, params).await.unwrap();
        only_personal(&rows);
        assert_eq!(rows[0]["description"], "Personal definition");
    }
    let rows = client
        .call(
            methods::SET_MCP_SERVER,
            json!({
                "personal": true, "chatId": "project", "scope": "user", "name": "shared",
                "server": { "command": "personal-server" },
            }),
        )
        .await
        .unwrap();
    only_personal(&rows);
    assert_eq!(rows[0]["transport"], "personal-server");
    assert!(config.join("skills/shared/SKILL.md").is_file());
    assert!(config.join("agents/shared.md").is_file());

    for (method, path_field, path) in [
        (
            methods::SET_SKILL_ENABLED,
            "skillPath",
            config.join("skills").join("shared"),
        ),
        (
            methods::SET_SUBAGENT_ENABLED,
            "definitionPath",
            config.join("agents").join("shared.md"),
        ),
    ] {
        let mut params = json!({
            "personal": true, "chatId": "project", "scope": "user", "enabled": false,
        });
        params[path_field] = path.to_string_lossy().into_owned().into();
        let rows = client.call(method, params).await.unwrap();
        only_personal(&rows);
        assert_eq!(rows[0]["enabled"], false);
    }

    // Plugin contributions remain part of the global settings catalogs.
    let plugin = core.config_root().join("plugins/kit");
    write(
        &plugin.join(".claude-plugin/plugin.json"),
        r#"{"name":"kit"}"#,
    );
    write(
        &plugin.join("skills/plugin-skill/SKILL.md"),
        "---\nname: plugin-skill\ndescription: Plugin skill\n---\nInstructions.",
    );
    write(
        &plugin.join("agents/plugin-agent.md"),
        "---\nname: plugin-agent\ndescription: Plugin agent\n---\nInstructions.",
    );
    write(
        &plugin.join(".mcp.json"),
        r#"{"mcpServers":{"docs":{"command":"plugin-server"}}}"#,
    );
    let plugins = client
        .call(methods::LIST_PLUGINS, json!({}))
        .await
        .unwrap();
    assert_eq!(plugins[0]["name"], "kit");

    for method in [
        methods::LIST_SKILLS,
        methods::LIST_SUBAGENTS,
        methods::LIST_MCP_SERVERS,
    ] {
        let personal = json!({ "personal": true, "harness": "native" });
        let rows = client.call(method, personal.clone()).await.unwrap();
        only_personal(&rows);
        assert_eq!(
            rows.as_array()
                .unwrap()
                .iter()
                .filter(|row| !row["path"]
                    .as_str()
                    .is_some_and(|path| path.starts_with("builtin:")))
                .count(),
            2,
            "{rows:#}"
        );
        // A stale or missing project selection has no bearing on personal catalogs.
        let mut with_project = personal.clone();
        with_project["chatId"] = "project".into();
        assert_eq!(client.call(method, with_project).await.unwrap(), rows);
        let mut missing_project = personal;
        missing_project["chatId"] = "missing".into();
        assert_eq!(client.call(method, missing_project).await.unwrap(), rows);
    }

    // Writes through the settings surface cannot target project files.
    for method in [
        methods::CREATE_SKILL,
        methods::CREATE_SUBAGENT,
        methods::SET_MCP_SERVER,
    ] {
        assert!(client.call(method, json!({
            "personal": true, "chatId": "project", "scope": "project", "name": "refused",
            "description": "Refused", "server": {"command": "refused"},
        })).await.is_err());
    }

    // Personal removal replies preserve plugin rows and leave project definitions intact.
    for method in [
        methods::DELETE_SKILL,
        methods::DELETE_SUBAGENT,
        methods::SET_MCP_SERVER,
    ] {
        let rows = client
            .call(
                method,
                json!({
                    "personal": true, "chatId": "project", "scope": "user", "name": "shared",
                    "server": null,
                }),
            )
            .await
            .unwrap();
        only_personal(&rows);
        assert_eq!(
            rows.as_array()
                .unwrap()
                .iter()
                .filter(|row| !row["path"]
                    .as_str()
                    .is_some_and(|path| path.starts_with("builtin:")))
                .count(),
            1
        );
    }
    assert!(
        project
            .path()
            .join(".agents/skills/shared/SKILL.md")
            .is_file()
    );
    assert!(project.path().join(".rcode/agents/shared.md").is_file());
    assert!(
        std::fs::read_to_string(project.path().join(".rcode/mcp.json"))
            .unwrap()
            .contains("project-server")
    );
    core.shutdown().await;
}
