//! A plugin, from install to what the agent can see.
//!
//! The projection is the point: a plugin contributes *files*, and four
//! different scans have to find them — skills, subagent definitions, hooks and
//! MCP servers. A plugin that installs but contributes nothing is the failure
//! this file exists to catch.

use std::sync::Arc;

use rcode_engine::EngineCore;
use rcode_proto::HarnessId;
use rcode_rpc::methods;

const CHAT: &str = "plugins-chat";

/// A plugin directory with one contribution of each kind.
fn plugin_dir(root: &std::path::Path) {
    let manifest = root.join(".claude-plugin").join("plugin.json");
    std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    std::fs::write(
        &manifest,
        r#"{"name":"kit","version":"1.0.0","description":"A kit."}"#,
    )
    .unwrap();

    let skill = root.join("skills").join("release").join("SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(
        &skill,
        "---\nname: release\ndescription: Cut a release.\n---\n\nSteps.\n",
    )
    .unwrap();

    let agent = root.join("agents").join("reviewer.md");
    std::fs::create_dir_all(agent.parent().unwrap()).unwrap();
    std::fs::write(
        &agent,
        "---\nname: reviewer\ndescription: Reviews a diff.\n---\n\nReview.\n",
    )
    .unwrap();

    let hooks = root.join("hooks").join("hooks.json");
    std::fs::create_dir_all(hooks.parent().unwrap()).unwrap();
    std::fs::write(
        &hooks,
        r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"true"}]}]}}"#,
    )
    .unwrap();

    std::fs::write(
        root.join(".mcp.json"),
        r#"{"mcpServers":{"docs":{"url":"https://example.invalid/mcp"}}}"#,
    )
    .unwrap();
}

async fn core() -> (tempfile::TempDir, tempfile::TempDir, EngineCore) {
    let tmp = tempfile::tempdir().unwrap();
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
async fn installing_a_plugin_makes_every_contribution_visible() {
    let (_tmp, project_dir, core) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());
    plugin_dir(&core.config_root().join("plugins/kit"));

    let listed: Vec<serde_json::Value> = serde_json::from_value(
        client
            .call(
                methods::LIST_PLUGINS,
                serde_json::json!({}),
            )
            .await
            .expect("install replies with the listing"),
    )
    .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["name"], "kit");
    assert_eq!(listed[0]["enabled"], true);

    let config_root = core.config_root();

    // Each scan finds the plugin's file.
    assert_eq!(rcode_plugins::skill_roots(config_root).len(), 1);
    assert_eq!(rcode_plugins::agent_roots(config_root).len(), 1);
    assert_eq!(rcode_plugins::hook_files(config_root).len(), 1);
    assert_eq!(rcode_plugins::mcp_files(config_root).len(), 1);

    // And the MCP loader reports the server, namespaced by plugin so an
    // unqualified name still means the user's own entry.
    let project = project_dir.path().join("project");
    let servers = rcode_mcp::load(&project, config_root).unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].name, "kit:docs");
    assert_eq!(servers[0].source, rcode_mcp::McpSource::Plugin);
}

#[tokio::test]
async fn switching_a_plugin_off_removes_its_contributions() {
    let (_tmp, project_dir, core) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());
    plugin_dir(&core.config_root().join("plugins/kit"));
    let config_root = core.config_root();

    client
        .call(
            methods::SET_PLUGIN_ENABLED,
            serde_json::json!({ "name": "kit", "enabled": false }),
        )
        .await
        .unwrap();

    assert!(rcode_plugins::skill_roots(config_root).is_empty());
    assert!(rcode_plugins::hook_files(config_root).is_empty());
    let project = project_dir.path().join("project");
    assert!(
        rcode_mcp::load(&project, config_root).unwrap().is_empty(),
        "a switched-off plugin contributes no servers"
    );

    // Still installed and still listed, with its switch off.
    let listed: Vec<serde_json::Value> = serde_json::from_value(
        client
            .call(methods::LIST_PLUGINS, serde_json::json!({}))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["enabled"], false);
}

#[tokio::test]
async fn removing_a_plugin_takes_its_files_with_it() {
    let (_tmp, _project_dir, core) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());
    plugin_dir(&core.config_root().join("plugins/kit"));

    let listed: Vec<serde_json::Value> = serde_json::from_value(
        client
            .call(
                methods::UNINSTALL_PLUGIN,
                serde_json::json!({ "name": "kit" }),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(listed.is_empty());

    let config_root = core.config_root();
    assert!(!config_root.join("plugins").join("kit").exists());
    assert!(rcode_plugins::skill_roots(config_root).is_empty());
}

#[tokio::test]
async fn local_directory_install_rpc_is_removed() {
    let (_tmp, _project, core) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());
    assert!(client.call("InstallPlugin", serde_json::json!({"path": "."})).await.is_err());
    assert!(client.call(methods::INSTALL_MARKETPLACE_PLUGIN, serde_json::json!({"source": "official", "name": "../outside"})).await.is_err());
    assert!(client.call(methods::INSTALL_MARKETPLACE_PLUGIN, serde_json::json!({"source": "other", "name": "kit", "path": "."})).await.is_err());
}

#[tokio::test]
async fn marketplace_explicit_files_reach_skills_agents_and_slash_commands() {
    let (_tmp, project, core) = core().await;
    let root = core.config_root().join("plugins/kit");
    plugin_dir(&root);
    std::fs::write(root.join(".claude-plugin/plugin.json"), r#"{"name":"kit","skills":"./skills/release/SKILL.md","agents":["./agents/reviewer.md"],"commands":["./commands/review.md"]}"#).unwrap();
    std::fs::create_dir_all(root.join("commands")).unwrap();
    std::fs::write(root.join("commands/review.md"), "---\ndescription: Review\n---\nReview $ARGUMENTS\n").unwrap();
    let config = core.config_root();
    let skills = rcode_agent::skills::discover_personal(None, &rcode_plugins::skill_roots(config));
    assert!(skills.iter().any(|skill| skill.name == "kit:release"));
    let agents = rcode_agent::subagents::discover_personal(None, &rcode_plugins::agent_roots(config));
    assert!(agents.iter().any(|agent| agent.name == "kit:reviewer"));
    let commands = rcode_agent::custom_commands::list(None, None, &[("kit".into(), root.join("commands/review.md"))]);
    assert_eq!(commands.len(), 1);
    assert!(rcode_agent::custom_commands::expand(&commands, "/kit:review changes").unwrap().unwrap().contains("Review changes"));
    use rcode_agent::tool::Tool;
    let tool = rcode_agent::tools::SkillTool::with_plugin_roots(Default::default(), rcode_plugins::skill_roots(config));
    let result = tool.call(serde_json::json!({"skill":"kit:release"}), &rcode_agent::tool::ToolContext {
        cwd: project.path().to_owned(), cancel: tokio_util::sync::CancellationToken::new(), nested: None
    }).await.unwrap();
    assert!(!result.is_error && result.output.contains("Steps."));
}
