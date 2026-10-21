//! The hook executor: reading settings, matching, running, deciding.
use super::*;

fn write_settings(dir: &Path, body: &serde_json::Value) -> PathBuf {
    let settings = dir.join(".rcode");
    std::fs::create_dir_all(&settings).expect("mkdir");
    let path = settings.join("settings.json");
    std::fs::write(&path, body.to_string()).expect("write settings");
    path
}

fn context_for(tool: &str) -> HookContext {
    HookContext::for_tool(tool, &serde_json::json!({}), Path::new("/tmp"))
}

/// A settings file with one command hook on `PreToolUse`.
fn one_hook(dir: &Path, command: &str) {
    write_settings(
        dir,
        &serde_json::json!({
            "hooks": { "PreToolUse": [{ "hooks": [
                { "type": "command", "command": command }
            ]}]}
        }),
    );
}

#[test]
fn no_settings_file_means_no_hooks() {
    let dir = tempfile::tempdir().expect("temp dir");
    let executor = HookExecutor::load(None, dir.path());
    assert!(executor.is_empty());
}

#[test]
fn session_start_matches_source_and_nested_output_is_context() {
    let dir = tempfile::tempdir().unwrap();
    write_settings(dir.path(), &serde_json::json!({"hooks":{"SessionStart":[{
        "matcher":"startup|clear|compact", "hooks":[{"type":"command","command":"echo ready"}]
    }]}}));
    let executor = HookExecutor::load(None, dir.path());
    assert_eq!(executor.matching(HookEvent::SessionStart, &HookContext::default()).len(), 1);
    let context = HookContext { source: Some("compact".into()), ..Default::default() };
    assert_eq!(executor.matching(HookEvent::SessionStart, &context).len(), 1);
    let context = HookContext { source: Some("resume".into()), ..Default::default() };
    assert!(executor.matching(HookEvent::SessionStart, &context).is_empty());
    assert_eq!(parse_allow(r#"{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"bootstrap"}}"#), HookOutcome::Context("bootstrap".into()));
    assert_eq!(parse_allow(r#"{"hookSpecificOutput":{"permissionDecision":"deny","permissionDecisionReason":"blocked"}}"#), HookOutcome::Block("blocked".into()));
}

#[tokio::test]
#[ignore = "downloads and executes the public Superpowers SessionStart bootstrap"]
async fn superpowers_live_bootstrap_and_every_skill_are_usable() {
    use rcode_agent::tool::Tool;
    let config = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    rcode_plugins::marketplace::install(config.path(), "superpowers", "superpowers").await.unwrap();
    let executor = HookExecutor::load(Some(config.path()), cwd.path());
    for source in ["startup", "compact", "clear"] {
        let result = executor.fire(HookEvent::SessionStart, HookContext {
            source: Some(source.into()), cwd: cwd.path().to_owned(), ..Default::default()
        }).await;
        assert!(result.failures.is_empty(), "{source}: {:?}", result.failures);
        let context = result.outcome.context().expect("bootstrap must reach model context");
        assert!(context.contains("You have superpowers") && context.contains("using-superpowers"));
    }
    let roots = rcode_plugins::skill_roots(config.path());
    let skills = rcode_agent::skills::discover_with(cwd.path(), &roots);
    let plugin_skills: Vec<_> = skills.iter().filter(|skill| skill.source == rcode_agent::skills::SkillSource::Plugin).collect();
    let tool = rcode_agent::tools::SkillTool::with_plugin_roots(Default::default(), roots);
    for skill in &plugin_skills {
        assert!(skill.name.starts_with("superpowers:"));
        let result = tool.call(serde_json::json!({"skill":skill.name}), &rcode_agent::tool::ToolContext {
            cwd: cwd.path().to_owned(), cancel: tokio_util::sync::CancellationToken::new(), nested: None
        }).await.unwrap();
        assert!(!result.is_error && result.output.contains("Base directory:"));
        assert!(skill.manifest().is_file());
    }
    println!("Superpowers: {} skills loaded, startup/clear/compact bootstrap passed", plugin_skills.len());
    assert!(plugin_skills.iter().any(|skill| skill.name == "superpowers:using-superpowers"));

    // Exercise the bundled visual companion through the actual Bash tool.
    // No browser is opened; the server binds only to loopback and is stopped below.
    let scripts = config.path().join("plugins/superpowers/skills/brainstorming/scripts");
    let slash = |path: &Path| path.to_string_lossy().replace('\\', "/");
    let tool_context = rcode_agent::tool::ToolContext {
        cwd: cwd.path().to_owned(), cancel: tokio_util::sync::CancellationToken::new(), nested: None
    };
    let command = format!("bash \"{}/start-server.sh\" --project-dir \"{}\" --background --idle-timeout-minutes 1", slash(&scripts), slash(cwd.path()));
    let launched = rcode_agent::tools::BashTool.call(serde_json::json!({"command":command,"timeout":20}), &tool_context).await.unwrap();
    assert!(!launched.is_error, "visual companion launcher failed");
    let info: serde_json::Value = launched.output.lines().find_map(|line| serde_json::from_str::<serde_json::Value>(line).ok().filter(|value| value["type"] == "server-started")).expect("server must report readiness");
    let url = reqwest::Url::parse(info["url"].as_str().unwrap()).unwrap();
    assert!(matches!(url.host_str(), Some("localhost" | "127.0.0.1")));
    let response = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap().get(url).send().await;
    let state_dir = info["state_dir"].as_str().unwrap();
    let session_dir = Path::new(state_dir).parent().unwrap();
    let stop = format!("bash \"{}/stop-server.sh\" \"{}\"", slash(&scripts), slash(session_dir));
    let stopped = rcode_agent::tools::BashTool.call(serde_json::json!({"command":stop,"timeout":10}), &tool_context).await.unwrap();
    assert!(!stopped.is_error, "visual companion cleanup failed");
    for line in stopped.output.lines() { if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) { if let Some(status) = value.get("status").and_then(|v| v.as_str()) { println!("Visual companion stop status: {status}"); } } }
    assert!(response.ok().is_some_and(|response| response.status().as_u16() < 400), "visual companion did not survive the launcher returning");
    let port = info["port"].as_u64().unwrap() as u16;
    #[cfg(windows)]
    {
        // ConnectEx may retry a refused connection beyond a short timeout;
        // inspect the native listener table instead of guessing from a probe.
        let powershell = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:/Windows".into())).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let script = format!("@([System.Net.NetworkInformation.IPGlobalProperties]::GetIPGlobalProperties().GetActiveTcpListeners() | Where-Object {{ $_.Port -eq {port} }}).Count");
        let mut command = tokio::process::Command::new(powershell);
        command.args(["-NoProfile", "-NonInteractive", "-Command", &script]).creation_flags(0x0800_0000);
        let output = command.output().await.unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "0", "stop script left the visual companion listening");
    }
    #[cfg(not(windows))]
    assert!(tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(("127.0.0.1", port))).await.is_ok_and(|result| result.is_err()));
    println!("Superpowers visual companion: background startup, loopback HTTP, and stop script passed");
}

#[test]
fn a_matcher_star_matches_every_tool() {
    assert!(matches_tool("*", "Bash"));
    assert!(matches_tool("", "Write"));
}

#[test]
fn a_pipe_list_matches_any_listed_tool_exactly() {
    assert!(matches_tool("Write|Edit", "Write"));
    assert!(matches_tool("Write|Edit", "Edit"));
    assert!(!matches_tool("Write|Edit", "Bash"));
    // Exact, not a prefix: "Write" must not match "WriteFile".
    assert!(!matches_tool("Write|Edit", "WriteFile"));
}

#[test]
fn a_bare_name_is_tried_exactly_before_as_a_pattern() {
    assert!(matches_tool("Bash", "Bash"));
    assert!(!matches_tool("Bash", "BashTool"));
}

#[test]
fn anything_else_is_a_regular_expression() {
    assert!(matches_tool("^Bash.*", "BashTool"));
    assert!(matches_tool("Read.*", "ReadFile"));
    assert!(!matches_tool("^Write$", "WriteFile"));
}

#[test]
fn an_invalid_pattern_matches_nothing_rather_than_panicking() {
    // A malformed regex is a typo in a settings file; it must not take the run
    // down.
    assert!(!matches_tool("[unclosed", "Bash"));
}

#[test]
fn hooks_are_read_from_the_settings_shape_claude_code_writes() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_settings(
        dir.path(),
        &serde_json::json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [{ "type": "command", "command": "echo hi" }]
                }]
            }
        }),
    );
    let executor = HookExecutor::load(None, dir.path());
    assert_eq!(executor.hooks.len(), 1);
    assert_eq!(executor.hooks[0].event, HookEvent::PreToolUse);
    assert_eq!(executor.hooks[0].matcher.as_deref(), Some("Bash"));
    assert_eq!(executor.hooks[0].command, "echo hi");
}

#[test]
fn an_event_this_build_does_not_fire_is_skipped() {
    // A file written for a newer Claude Code lists events RCode has no point
    // for; skipping them is not an error.
    let dir = tempfile::tempdir().expect("temp dir");
    write_settings(
        dir.path(),
        &serde_json::json!({
            "hooks": {
                "TeammateIdle": [{ "hooks": [{ "type": "command", "command": "echo x" }] }],
                "PreToolUse": [{ "hooks": [{ "type": "command", "command": "echo y" }] }]
            }
        }),
    );
    let executor = HookExecutor::load(None, dir.path());
    assert_eq!(executor.hooks.len(), 1, "only the known event survives");
}

#[test]
fn a_malformed_settings_file_is_skipped_not_fatal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let settings = dir.path().join(".rcode");
    std::fs::create_dir_all(&settings).expect("mkdir");
    std::fs::write(settings.join("settings.json"), "{ not json").expect("write");
    // The user's agent must still run.
    let executor = HookExecutor::load(None, dir.path());
    assert!(executor.is_empty());
}

#[test]
fn a_hook_type_that_is_not_implemented_is_skipped() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_settings(
        dir.path(),
        &serde_json::json!({
            "hooks": {
                "PreToolUse": [{ "hooks": [
                    { "type": "prompt", "prompt": "think about it" },
                    { "type": "command", "command": "echo ok" }
                ]}]
            }
        }),
    );
    let executor = HookExecutor::load(None, dir.path());
    assert_eq!(executor.hooks.len(), 1, "only the command hook is kept");
    assert_eq!(executor.hooks[0].command, "echo ok");
}

#[test]
fn the_matcher_filters_which_hooks_fire() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_settings(
        dir.path(),
        &serde_json::json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [{ "type": "command", "command": "echo bash-only" }]
                }]
            }
        }),
    );
    let executor = HookExecutor::load(None, dir.path());
    assert_eq!(
        executor
            .matching(HookEvent::PreToolUse, &context_for("Bash"))
            .len(),
        1
    );
    assert_eq!(
        executor
            .matching(HookEvent::PreToolUse, &context_for("Write"))
            .len(),
        0
    );
}

#[test]
fn a_timeout_is_clamped_to_the_ceiling() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_settings(
        dir.path(),
        &serde_json::json!({
            "hooks": {
                "Stop": [{ "hooks": [
                    { "type": "command", "command": "echo x", "timeout": 99_999 }
                ]}]
            }
        }),
    );
    let executor = HookExecutor::load(None, dir.path());
    assert_eq!(
        executor.hooks[0].timeout,
        Duration::from_secs(MAX_HOOK_SECONDS)
    );
}

// === Running a real command ===

#[tokio::test]
async fn a_zero_exit_allows() {
    let dir = tempfile::tempdir().expect("temp dir");
    one_hook(dir.path(), "exit 0");
    let result = HookExecutor::load(None, dir.path())
        .fire(HookEvent::PreToolUse, context_for("Bash"))
        .await;
    assert_eq!(result.outcome, HookOutcome::Allow);
    assert!(result.failures.is_empty());
}

#[tokio::test]
async fn a_two_exit_blocks_with_the_reason_from_stderr() {
    let dir = tempfile::tempdir().expect("temp dir");
    one_hook(dir.path(), "echo 'no rm -rf here' >&2; exit 2");
    let result = HookExecutor::load(None, dir.path())
        .fire(HookEvent::PreToolUse, context_for("Bash"))
        .await;
    assert!(
        result.is_blocked(),
        "an exit 2 must block: {:?}",
        result.outcome
    );
    assert!(
        result
            .outcome
            .block()
            .unwrap_or_default()
            .contains("no rm -rf here"),
        "the reason must come from stderr: {:?}",
        result.outcome
    );
}

#[tokio::test]
async fn any_other_exit_allows_because_the_hook_itself_failed() {
    // A hook that crashes says nothing about whether the call should proceed.
    // Blocking would make a broken script unusable.
    let dir = tempfile::tempdir().expect("temp dir");
    one_hook(dir.path(), "exit 7");
    let result = HookExecutor::load(None, dir.path())
        .fire(HookEvent::PreToolUse, context_for("Bash"))
        .await;
    assert_eq!(result.outcome, HookOutcome::Allow);
}

#[tokio::test]
async fn a_hook_that_times_out_allows_rather_than_blocking() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_settings(
        dir.path(),
        &serde_json::json!({
            "hooks": { "PreToolUse": [{ "hooks": [
                { "type": "command", "command": "sleep 30", "timeout": 1 }
            ]}]}
        }),
    );
    let result = HookExecutor::load(None, dir.path())
        .fire(HookEvent::PreToolUse, context_for("Bash"))
        .await;
    assert_eq!(result.outcome, HookOutcome::Allow);
}

#[tokio::test]
async fn json_on_stdout_adds_context_for_the_model() {
    let dir = tempfile::tempdir().expect("temp dir");
    one_hook(
        dir.path(),
        "echo '{\"additionalContext\":\"mind the formatter\"}'",
    );
    let result = HookExecutor::load(None, dir.path())
        .fire(HookEvent::PreToolUse, context_for("Bash"))
        .await;
    assert_eq!(
        result.outcome.context(),
        Some("mind the formatter"),
        "got: {:?}",
        result.outcome
    );
}

#[tokio::test]
async fn plain_stdout_adds_nothing() {
    let dir = tempfile::tempdir().expect("temp dir");
    one_hook(dir.path(), "echo just some noise");
    let result = HookExecutor::load(None, dir.path())
        .fire(HookEvent::PreToolUse, context_for("Bash"))
        .await;
    assert_eq!(result.outcome, HookOutcome::Allow);
}

#[tokio::test]
async fn a_block_wins_over_context_from_another_hook() {
    // Both fire; once a call is refused, what the other hook would have added
    // to the context is moot.
    let dir = tempfile::tempdir().expect("temp dir");
    write_settings(
        dir.path(),
        &serde_json::json!({
            "hooks": { "PreToolUse": [
                { "hooks": [{ "type": "command", "command": "echo '{\"additionalContext\":\"note\"}'" }] },
                { "hooks": [{ "type": "command", "command": "echo refused >&2; exit 2" }] }
            ]}
        }),
    );
    let result = HookExecutor::load(None, dir.path())
        .fire(HookEvent::PreToolUse, context_for("Bash"))
        .await;
    assert!(result.is_blocked());
}

#[tokio::test]
async fn the_hook_receives_the_tool_name_in_its_environment() {
    // A hook decides based on what the agent is doing, so the context has to
    // reach it.
    let dir = tempfile::tempdir().expect("temp dir");
    one_hook(dir.path(), "test \"$HOOK_TOOL_NAME\" = \"Bash\" || exit 2");
    let result = HookExecutor::load(None, dir.path())
        .fire(HookEvent::PreToolUse, context_for("Bash"))
        .await;
    assert_eq!(
        result.outcome,
        HookOutcome::Allow,
        "the hook should see its tool name: {:?}",
        result.outcome
    );
}

#[tokio::test]
async fn fires_only_for_the_event_it_was_registered_against() {
    let dir = tempfile::tempdir().expect("temp dir");
    one_hook(dir.path(), "echo x >&2; exit 2");
    let executor = HookExecutor::load(None, dir.path());
    assert!(
        executor
            .fire(HookEvent::PreToolUse, context_for("Bash"))
            .await
            .is_blocked()
    );
    assert_eq!(
        executor
            .fire(HookEvent::Stop, HookContext::default())
            .await
            .outcome,
        HookOutcome::Allow,
        "the hook must not fire on another event"
    );
}
