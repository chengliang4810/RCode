//! Subagent definitions through the real RPC surface: what the settings page
//! writes is what the listing reports, and the listing is what the `Task`
//! tool's runtime resolves — the two must agree or a user would save an agent
//! the model can never delegate to.

use std::sync::Arc;

use rcode_engine::EngineCore;
use rcode_proto::HarnessId;
use rcode_rpc::methods;

const CHAT: &str = "subagent-settings-chat";

/// `home_dir()` reads `USERPROFILE`/`HOME` from the process environment, so a
/// test that touches the user scope must point it at a temporary directory.
/// The variable is process-wide, which is why every user-scope test takes
/// [`HOME_LOCK`] and the tests that do not need it never set it.
static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// An engine with one projectless chat whose cwd is `project`. The catalog
/// root is resolved from a chat, not from a bare path — the same route the
/// settings page takes — so the test exercises the production resolution.
async fn core() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    EngineCore,
    String,
    String,
) {
    let tmp = tempfile::tempdir().unwrap();
    // The project must not live under the process home: a user-scope
    // definition is `~/.rcode/agents`, and a project beneath `~` would
    // otherwise resolve to the same directory and shadow itself.
    let project_dir = tempfile::tempdir().unwrap();
    let project = project_dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let data = tmp.path().join("data");
    // The real native registry: the definitions live on the harness this
    // engine serves, not on a mock slot.
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
    let cwd = project.to_string_lossy().into_owned();
    (
        tmp,
        project_dir,
        core,
        cwd,
        project.to_string_lossy().into_owned(),
    )
}

/// Points the process home at `home` until it is dropped.
///
/// `home_dir()` reads `USERPROFILE`/`HOME` from the environment, so a test
/// that touches the user scope must redirect it — otherwise it writes into
/// the developer's real `~/.rcode`. The variable is process-wide, which is
/// why every user-scope test takes [`HOME_LOCK`]; a guard (rather than a
/// callback) keeps that usable from an async test body.
struct HomeGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl HomeGuard {
    fn set(home: &std::path::Path) -> Self {
        let lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let previous = std::env::var_os(key);
        // SAFETY: `HOME_LOCK` serialises every test that reads the home.
        unsafe { std::env::set_var(key, home) };
        Self {
            key,
            previous,
            _lock: lock,
        }
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            // SAFETY: the lock is still held until this guard finishes.
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

#[tokio::test]
async fn removed_model_routing_methods_are_not_available() {
    let (_tmp, _project_dir, core, _cwd, _project) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());
    for method in ["GetModelRouting", "SaveModelRouting"] {
        assert!(client.call(method, serde_json::json!({})).await.is_err());
    }
}

#[tokio::test]
async fn a_created_definition_is_listed_and_written_where_the_runtime_reads() {
    let (_tmp, project_dir, core, _cwd, _project) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());
    let project = project_dir.path().join("project");

    let created = client
        .call(
            methods::CREATE_SUBAGENT,
            serde_json::json!({
                "chatId": CHAT,
                "name": "reviewer",
                "description": "Reviews a diff.",
                "systemPrompt": "You review.",
                "tools": ["Read", "Grep"],
                "disallowedTools": ["Bash"],
                "model": "model:[\"provider\",\"model\"]",
                "scope": "project",
            }),
        )
        .await
        .expect("create replies with the fresh list");
    let listed: Vec<serde_json::Value> = serde_json::from_value(created).unwrap();
    let reviewer = listed
        .iter()
        .find(|definition| definition["name"] == "reviewer")
        .expect("the new definition is in the reply");
    assert_eq!(reviewer["description"], "Reviews a diff.");
    assert_eq!(reviewer["enabled"], true);
    assert_eq!(reviewer["model"], "model:[\"provider\",\"model\"]");
    assert_eq!(reviewer["scope"], "project");
    assert_eq!(reviewer["tools"], serde_json::json!(["Read", "Grep"]));
    assert_eq!(reviewer["disallowedTools"], serde_json::json!(["Bash"]));

    // The file is where the runtime scan looks: `<cwd>/.rcode/agents`.
    let written = project.join(".rcode").join("agents").join("reviewer.md");
    assert!(written.is_file(), "{}", written.display());
    let text = std::fs::read_to_string(&written).unwrap();
    assert!(text.contains("name: reviewer"), "{text}");
    assert!(text.contains("You review."), "{text}");

    // A second listing sees the same thing, so the page and the runtime read
    // one scan.
    let listed_again: Vec<serde_json::Value> = serde_json::from_value(
        client
            .call(
                methods::LIST_SUBAGENTS,
                serde_json::json!({ "chatId": CHAT }),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(listed_again.len(), listed.len());
}

#[tokio::test]
async fn a_definition_toggle_is_keyed_by_the_path_the_listing_reported() {
    let (_tmp, project_dir, core, _cwd, _project) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());
    let project = project_dir.path().join("project");

    client
        .call(
            methods::CREATE_SUBAGENT,
            serde_json::json!({
                "chatId": CHAT,
                "name": "reviewer",
                "description": "Reviews.",
                "systemPrompt": "You review.",
                "scope": "project",
            }),
        )
        .await
        .unwrap();
    let listed: Vec<serde_json::Value> = serde_json::from_value(
        client
            .call(
                methods::LIST_SUBAGENTS,
                serde_json::json!({ "chatId": CHAT }),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let path = listed[0]["path"].as_str().expect("a path").to_owned();

    // Switching off by the listing's own path must land — a toggle that used
    // another spelling would silently do nothing.
    let toggled: Vec<serde_json::Value> = serde_json::from_value(
        client
            .call(
                methods::SET_SUBAGENT_ENABLED,
                serde_json::json!({
                    "chatId": CHAT,
                    "definitionPath": path,
                    "enabled": false,
                    "scope": "project",
                }),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(toggled[0]["enabled"], false);

    // A path that is not a listed definition of that scope is refused, so a
    // caller cannot park an arbitrary file path in the settings file.
    let refused = client
        .call(
            methods::SET_SUBAGENT_ENABLED,
            serde_json::json!({
                "chatId": CHAT,
                "definitionPath": project.join("elsewhere.md").to_string_lossy(),
                "enabled": false,
                "scope": "project",
            }),
        )
        .await;
    assert!(
        refused.is_err(),
        "an unknown definition path must be refused"
    );
}

#[tokio::test]
async fn a_user_definition_and_a_project_definition_of_one_name_stay_separate() {
    // The user scope is `~/.rcode/agents`, so this test owns the process home
    // for its duration. Without the override it would write into the real one.
    let home = tempfile::tempdir().unwrap();
    let _home = HomeGuard::set(home.path());
    let (_tmp, project_dir, core, _cwd, _project) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());
    let project = project_dir.path().join("project");

    for scope in ["user", "project"] {
        client
            .call(
                methods::CREATE_SUBAGENT,
                serde_json::json!({
                    "chatId": CHAT,
                    "name": "reviewer",
                    "description": format!("The {scope} reviewer."),
                    "systemPrompt": "You review.",
                    "scope": scope,
                }),
            )
            .await
            .unwrap_or_else(|error| panic!("{scope}: {error}"));
    }

    let listed: Vec<serde_json::Value> = serde_json::from_value(
        client
            .call(
                methods::LIST_SUBAGENTS,
                serde_json::json!({ "chatId": CHAT }),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    // The project's copy shadows the person's by name — one row, and it is
    // the project's.
    let reviewers: Vec<&serde_json::Value> = listed
        .iter()
        .filter(|definition| definition["name"] == "reviewer")
        .collect();
    assert_eq!(reviewers.len(), 1, "{listed:#?}");
    assert_eq!(reviewers[0]["scope"], "project");
    assert_eq!(reviewers[0]["description"], "The project reviewer.");

    // Both files exist: the shadowed user copy is still on disk, and each
    // lives in its own managed root.
    assert!(
        project
            .join(".rcode")
            .join("agents")
            .join("reviewer.md")
            .is_file()
    );
    assert!(
        home.path()
            .join(".rcode")
            .join("agents")
            .join("reviewer.md")
            .is_file(),
        "the user copy is written under the (temporary) home"
    );
}

#[tokio::test]
async fn updating_a_definition_keeps_its_prompt_when_none_is_sent() {
    let (_tmp, project_dir, core, _cwd, _project) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());
    let project = project_dir.path().join("project");

    client
        .call(
            methods::CREATE_SUBAGENT,
            serde_json::json!({
                "chatId": CHAT,
                "name": "reviewer",
                "description": "Reviews.",
                "systemPrompt": "The original instruction.",
                "scope": "project",
            }),
        )
        .await
        .unwrap();

    let listed: Vec<serde_json::Value> = serde_json::from_value(
        client
            .call(
                methods::UPDATE_SUBAGENT,
                serde_json::json!({
                    "chatId": CHAT,
                    "name": "reviewer",
                    "description": "Reviews more.",
                    "scope": "project",
                }),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(listed[0]["description"], "Reviews more.");

    let text =
        std::fs::read_to_string(project.join(".rcode").join("agents").join("reviewer.md")).unwrap();
    assert!(text.contains("The original instruction."), "{text}");
}

#[tokio::test]
async fn an_invalid_model_alias_is_refused_at_the_write() {
    let (_tmp, _project_dir, core, _cwd, _project) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());

    let refused = client
        .call(
            methods::CREATE_SUBAGENT,
            serde_json::json!({
                "chatId": CHAT,
                "name": "reviewer",
                "description": "Reviews.",
                "systemPrompt": "You review.",
                "model": "gpt-7-turbo",
                "scope": "project",
            }),
        )
        .await;
    assert!(refused.is_err(), "a typo must not park a dead override");
}

#[tokio::test]
async fn cancelling_an_unknown_subagent_reports_that_nothing_stopped() {
    let (_tmp, _project_dir, core, _cwd, _project) = core().await;
    let client = rcode_rpc::memory_client(core.rpc_service());

    // No run is in flight, so there is nothing under that id. The reply says
    // so rather than failing: a stale tab's stop is a race, not an error.
    let reply = client
        .call(
            methods::CANCEL_SUBAGENT,
            serde_json::json!({ "chatId": CHAT, "parentToolUseId": "call-never-ran" }),
        )
        .await
        .expect("the call answers");
    assert_eq!(reply["stopped"], false);
}
