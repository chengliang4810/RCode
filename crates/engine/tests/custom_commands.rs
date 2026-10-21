//! Command file ownership, RPC management and live native discovery.
use rcode_engine::EngineCore;
use rcode_proto::{
    HarnessId,
    custom_commands::{CommandMutation, CustomCommand},
};
use rcode_rpc::methods;
use serde_json::json;
use std::{path::Path, sync::Arc};

struct HomeGuard(&'static str, Option<std::ffi::OsString>);
impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.1.take() {
            Some(value) => unsafe { std::env::set_var(self.0, value) },
            None => unsafe { std::env::remove_var(self.0) },
        }
    }
}
fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}
#[tokio::test]
async fn native_custom_command_catalog_and_personal_management() {
    // A separate integration-test process contains only this test. Its spawned
    // discovery jobs share the same isolated home and cannot touch real files.
    let home = tempfile::tempdir().unwrap();
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let _guard = HomeGuard(key, std::env::var_os(key));
    unsafe { std::env::set_var(key, home.path()) };
    let config = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    put(
        &project.path().join(".agents/commands/shared.md"),
        "Project command: $ARGUMENTS",
    );
    put(
        &config.path().join("plugins/kit/.claude-plugin/plugin.json"),
        r#"{"name":"kit"}"#,
    );
    put(
        &config.path().join("plugins/kit/commands/check.md"),
        "Plugin check",
    );
    let registry = Arc::new(rcode_engine::registry_with_native(config.path()));
    let harness = registry.resolve(HarnessId::Native).unwrap();
    let core = EngineCore::assemble(config.path(), registry, HarnessId::Native, None).unwrap();
    core.workspace
        .create_chat("command-test", None, Some(&core.device_id), None, None)
        .unwrap();
    core.workspace
        .set_chat_cwd("command-test", &project.path().to_string_lossy())
        .unwrap();
    let client = rcode_rpc::memory_client(core.rpc_service());
    let created = client.call(methods::MUTATE_CUSTOM_COMMAND, serde_json::to_value(CommandMutation::Save {
        name: "shared".into(), content: "---\ndescription: Global review\nargument-hint: '[scope]'\n---\nGlobal command: $ARGUMENTS".into(), expected_revision: None,
    }).unwrap()).await.unwrap();
    let commands: Vec<CustomCommand> = serde_json::from_value(created).unwrap();
    let user = commands.iter().find(|c| c.name == "shared").unwrap();
    assert!(home.path().join(".rcode/commands/shared.md").exists());
    assert!(!config.path().join("commands/shared.md").exists());
    assert_eq!(commands.len(), 2);
    assert!(commands.iter().any(|c| c.name == "plugin:kit:check"));
    let catalogue = client
        .call(
            methods::LIST_COMMANDS,
            json!({"harness":"native", "chatId":"command-test"}),
        )
        .await
        .unwrap();
    let catalogue: Vec<rcode_proto::SlashCommand> = serde_json::from_value(catalogue).unwrap();
    assert_eq!(catalogue.iter().filter(|c| c.name == "shared").count(), 1);
    assert!(catalogue.iter().any(|c| c.name == "compact"));
    let selected = format!(
        "{} auth",
        rcode_proto::invocation::Invocation::Command {
            name: "shared".into()
        }
        .link()
    );
    assert!(
        harness
            .expand_custom_command(project.path(), &selected)
            .await
            .unwrap()
            .unwrap()
            .contains("Project command: auth")
    );
    let disabled = client
        .call(
            methods::MUTATE_CUSTOM_COMMAND,
            serde_json::to_value(CommandMutation::SetEnabled {
                name: user.name.clone(),
                enabled: false,
                expected_revision: user.revision.clone(),
            })
            .unwrap(),
        )
        .await
        .unwrap();
    let disabled: Vec<CustomCommand> = serde_json::from_value(disabled).unwrap();
    let disabled = disabled.into_iter().find(|c| c.name == "shared").unwrap();
    assert!(!disabled.enabled);
    assert!(
        harness
            .expand_custom_command(home.path(), "/shared")
            .await
            .is_err()
    );
    // Project selection remains available when the same user command is off.
    assert!(
        harness
            .expand_custom_command(project.path(), "/shared")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        client
            .call(
                methods::MUTATE_CUSTOM_COMMAND,
                serde_json::to_value(CommandMutation::Save {
                    name: user.name.clone(),
                    content: "stale overwrite".into(),
                    expected_revision: Some(user.revision.clone()),
                })
                .unwrap()
            )
            .await
            .is_err()
    );
    client
        .call(
            methods::MUTATE_CUSTOM_COMMAND,
            serde_json::to_value(CommandMutation::Delete {
                name: disabled.name,
                expected_revision: disabled.revision,
            })
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(!home.path().join(".rcode/commands/shared.md").exists());
    rcode_plugins::set_enabled(config.path(), "kit", false).unwrap();
    assert!(
        !harness
            .commands_for(project.path())
            .await
            .unwrap()
            .iter()
            .any(|c| c.name == "plugin:kit:check")
    );
    core.shutdown().await;
}
