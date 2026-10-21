//! Running the user's hooks.
//!
//! [`rcode_agent::hook`] defines *what* a hook is and *when* it fires; this is
//! the part that reads the files, matches a tool name, starts the process and
//! turns its exit status into a decision.
//!
//! It lives in the harness rather than with the contract because it needs a
//! shell and a filesystem, and the loop is deliberately ignorant of both — the
//! tests drive it with a fake.
//!
//! # Where hooks come from
//!
//! Three files, in this order, later ones able to add to earlier ones:
//!
//! ```text
//! {data_dir}/settings.json                  the person's, everywhere
//! {cwd}/.rcode/settings.json                the project's, committed
//! {cwd}/.rcode/settings.local.json          the project's, not committed
//! ```
//!
//! Each holds a `hooks` object keyed by event name, matching the shape Claude
//! Code writes — so a `hooks.json` the user already has keeps working:
//!
//! ```json
//! { "hooks": { "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "…" } ] } ] } }
//! ```
//!
//! # What a hook decides
//!
//! Exit status is the signal, as in Claude Code: `0` means the hook ran and
//! allowed (its stdout may carry JSON with more to say), `2` means refuse and
//! the reason is on stderr, anything else is a warning that does not block.
//! Failing open is deliberate for the other codes: a hook that crashes, or a
//! binary that is missing, must not make the agent unusable.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use rcode_agent::hook::{HookContext, HookEvent, HookFailure, HookOutcome, HookResult, HookRunner};

/// A ceiling on one hook, whatever it declares.
///
/// A hook that hangs holds the loop open, and the loop is in the middle of a
/// tool call: an unattended wait would look like the agent freezing.
const MAX_HOOK_SECONDS: u64 = 60;
/// The default when a hook does not say.
const DEFAULT_HOOK_SECONDS: u64 = 30;
/// How much of a hook's output is kept.
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// One hook, as read from a settings file.
#[derive(Debug, Clone)]
struct Hook {
    event: HookEvent,
    /// The tool-name pattern, for the tool-scoped events. `None` matches all.
    matcher: Option<String>,
    command: String,
    timeout: Duration,
    plugin_root: Option<PathBuf>,
    shell: Option<String>,
    asynchronous: bool,
}

/// The hooks to run for one run, resolved at start.
pub struct HookExecutor {
    hooks: Vec<Hook>,
    cwd: PathBuf,
    session_id: String,
}

impl HookExecutor {
    /// Read every hook that applies to a run in `cwd`.
    ///
    /// A missing file is not an error — most projects have no hooks. A
    /// malformed one is logged and skipped rather than failing the run: the
    /// user's agent should still work when their settings file has a typo, and
    /// the log line names the file so it can be fixed.
    pub fn load(data_dir: Option<&Path>, cwd: &Path) -> Self {
        let mut files: Vec<PathBuf> = Vec::new();
        if let Some(data_dir) = data_dir {
            files.push(data_dir.join("settings.json"));
            // Installed plugins contribute hook files in the same shape; they
            // come after the user's own file so an explicit hook always wins a
            // collision of command strings.
            files.extend(rcode_plugins::hook_files(data_dir));
        }
        files.push(cwd.join(".rcode").join("settings.json"));
        files.push(cwd.join(".rcode").join("settings.local.json"));

        let mut hooks = Vec::new();
        for file in files {
            hooks.extend(read_hooks(&file));
        }
        Self {
            hooks,
            cwd: cwd.to_path_buf(),
            session_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    /// Whether any hook could fire — lets a caller skip installing the runner.
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// The hooks bound to `event` whose matcher accepts this context.
    fn matching(&self, event: HookEvent, context: &HookContext) -> Vec<&Hook> {
        self.hooks
            .iter()
            .filter(|hook| hook.event == event)
            .filter(|hook| match (&hook.matcher, &context.tool_name) {
                // A matcher on an event with no tool name (SessionStart, Stop)
                // has nothing to match against, so it does not apply.
                (Some(matcher), Some(tool)) => matches_tool(matcher, tool) || matches_tool(matcher, match tool.as_str() { "SkillTool" => "Skill", _ => tool.as_str() }),
                (Some(matcher), None) if event == HookEvent::SessionStart => matches_tool(matcher, context.source.as_deref().unwrap_or("startup")),
                (Some(_), None) => false,
                // No matcher: every firing of the event.
                (None, _) => true,
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl HookRunner for HookExecutor {
    async fn fire(&self, event: HookEvent, context: HookContext) -> HookResult {
        let mut result = HookResult::allow();
        for hook in self.matching(event, &context) {
            if hook.asynchronous {
                let (hook, context, cwd, session_id) = (hook.clone(), context.clone(), self.cwd.clone(), self.session_id.clone());
                tokio::spawn(async move {
                    if let Err(error) = run_hook(&hook, &context, &cwd, &session_id).await { tracing::warn!(%error, "asynchronous plugin hook failed"); }
                });
                continue;
            }
            match run_hook(hook, &context, &self.cwd, &self.session_id).await {
                Ok(outcome) => result.outcome = result.outcome.merge(outcome),
                Err(message) => result.failures.push(HookFailure { event, message }),
            }
        }
        result
    }
}

/// Run one hook and read its decision off the exit status.
async fn run_hook(hook: &Hook, context: &HookContext, cwd: &Path, session_id: &str) -> Result<HookOutcome, String> {
    let mut command = if let Some(program) = hook.shell.as_deref().filter(|program| matches!(*program, "powershell" | "pwsh")) {
        let mut command = tokio::process::Command::new(program);
        command.args(["-NoProfile", "-NonInteractive", "-Command", &hook.command]);
        command
    } else { shell(hook.command.as_str()) };
    command
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // A GUI app has no console; without this the hook flashes a window.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    for (key, value) in rcode_agent::hook::hook_environment(context) {
        command.env(key, value);
    }

    command.env("CLAUDE_PROJECT_DIR", cwd.to_string_lossy().replace('\\', "/"));
    command.env("CLAUDE_SESSION_ID", session_id);
    if let Some(root) = &hook.plugin_root {
        command.env("CLAUDE_PLUGIN_ROOT", root.to_string_lossy().replace('\\', "/"));
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not start: {error}"))?;

    // The hook reads the same context from stdin that it gets in the
    // environment. Both are offered because a shell script finds one easier
    // than the other, and the payload is small.
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt as _;
        let payload = serde_json::json!({
            "hookEventName": hook.event.as_str(),
            "hook_event_name": hook.event.as_str(),
            "event": hook.event.as_str(),
            "session_id": session_id,
            "source": context.source.as_deref().unwrap_or("startup"),
            "tool_name": context.tool_name,
            "tool_input": context.tool_input,
            "tool_response": context.tool_output,
            "stop_hook_active": false,
            "error": if hook.event == HookEvent::StopFailure { Some("unknown") } else { None },
            "error_details": context.stop_reason,
            "last_assistant_message": context.stop_reason,
            "toolName": context.tool_name,
            "toolInput": context.tool_input,
            "toolOutput": context.tool_output,
            "toolFailed": context.tool_failed,
            "prompt": context.prompt,
            "cwd": context.cwd.to_string_lossy(),
        })
        .to_string();
        // A hook that never reads stdin closes the pipe; that is not an error.
        let _ = stdin.write_all(payload.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }

    let output = match tokio::time::timeout(hook.timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(format!("could not run: {error}")),
        Err(_) => {
            // A timeout does not block: the hook had its say and ran out of
            // time, which is not a reason to refuse the tool call.
            return Ok(HookOutcome::Allow);
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = truncate(&stdout, MAX_OUTPUT_BYTES);
    let stderr = truncate(&stderr, MAX_OUTPUT_BYTES);

    match output.status.code() {
        // Allowed. Stdout may carry JSON with context to add, but a plain-text
        // stdout is the common case and means "nothing to add".
        Some(0) => {
            let outcome = parse_allow(&stdout);
            if outcome == HookOutcome::Allow && !stdout.trim().is_empty() && !stdout.trim_start().starts_with('{')
                && matches!(hook.event, HookEvent::SessionStart | HookEvent::UserPromptSubmit) {
                Ok(HookOutcome::Context(stdout.trim().to_owned()))
            } else { Ok(outcome) }
        },
        // The documented block code, matching Claude Code. The reason is on
        // stderr so a hook can `echo >&2` and refuse.
        Some(2) => Ok(HookOutcome::Block(if stderr.trim().is_empty() {
            "a hook refused this".to_owned()
        } else {
            stderr.trim().to_owned()
        })),
        // Any other status is a warning: the hook itself failed, which says
        // nothing about whether the tool call should proceed.
        Some(code) => {
            tracing::warn!(code, "a hook exited with an unexpected status; allowing");
            Ok(HookOutcome::Allow)
        }
        None => {
            // Killed by a signal — the timeout path, or the OS reclaiming it.
            tracing::warn!("a hook was killed before it exited; allowing");
            Ok(HookOutcome::Allow)
        }
    }
}

/// Read an allow-status hook's stdout for context to add.
///
/// A hook that says nothing (or prints prose) adds nothing. JSON is honoured
/// because it is how a hook says "here is something the model should know"
/// without the plain-text case becoming ambiguous.
fn parse_allow(stdout: &str) -> HookOutcome {
    let trimmed = stdout.trim();
    if !trimmed.starts_with('{') {
        return HookOutcome::Allow;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return HookOutcome::Allow;
    };
    if value.get("continue").and_then(|v| v.as_bool()) == Some(false)
        || value.get("decision").and_then(|v| v.as_str()) == Some("block")
        || value.pointer("/hookSpecificOutput/permissionDecision").and_then(|v| v.as_str()) == Some("deny") {
        let reason = value.get("stopReason").or_else(|| value.get("reason"))
            .or_else(|| value.pointer("/hookSpecificOutput/permissionDecisionReason"))
            .and_then(|v| v.as_str()).unwrap_or("a hook refused this");
        return HookOutcome::Block(reason.to_owned());
    }
    if let Some(text) = value.pointer("/hookSpecificOutput/additionalContext").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
        return HookOutcome::Context(text.to_owned());
    }
    // The keys Claude Code documents for "add this to the model's context".
    for key in ["additionalContext", "additional_context", "systemMessage"] {
        if let Some(text) = value.get(key).and_then(|value| value.as_str()) {
            if !text.trim().is_empty() {
                return HookOutcome::Context(text.to_owned());
            }
        }
    }
    HookOutcome::Allow
}

/// Whether a tool name satisfies a matcher.
///
/// Three forms, matching Claude Code's: `*` or empty matches everything, a
/// `|`-separated list matches any of its entries exactly, and anything else is
/// a regular expression.
///
/// A plain name (`Bash`, `mcp__thing`) is **exact only** and never falls
/// through to being a pattern. That distinction is the whole reason the check
/// is three-way rather than "try exact, then regex": as a regex, `Bash` matches
/// inside `BashTool` and `Read` matches inside `ReadFile`, so a hook written to
/// guard one tool would silently also fire on others. A matcher that is not a
/// bare name is treated as a pattern, which is how `^Bash.*` opts into
/// matching many.
pub fn matches_tool(matcher: &str, tool_name: &str) -> bool {
    if matcher == "*" || matcher.is_empty() {
        return true;
    }
    if matcher.contains('|') && matcher.split('|').all(is_bare_name) {
        return matcher.split('|').any(|entry| entry.trim() == tool_name);
    }
    if is_bare_name(matcher) {
        return matcher == tool_name;
    }
    regex::Regex::new(matcher)
        .map(|regex| regex.is_match(tool_name))
        .unwrap_or(false)
}

/// Whether a matcher is a plain identifier rather than a pattern.
///
/// Letters, digits and `_` — which is what tool names are made of, including
/// the `mcp__server__tool` spelling. A `-` or a `.` would make it something
/// else (a glob, a regex), so those are patterns.
fn is_bare_name(matcher: &str) -> bool {
    !matcher.is_empty()
        && matcher
            .chars()
            .all(|character| character.is_alphanumeric() || character == '_')
}

/// Read the hooks out of one settings file.
fn read_hooks(path: &Path) -> Vec<Hook> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        tracing::warn!(
            path = %path.display(),
            "settings file is not valid JSON; its hooks are ignored"
        );
        return Vec::new();
    };
    let Some(hooks) = value.get("hooks").and_then(|hooks| hooks.as_object()) else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for (event_name, rules) in hooks {
        // An event this build does not fire is skipped, not an error: a file
        // written for a newer Claude Code lists events RCode has no point for.
        let Some(event) = HookEvent::parse(event_name) else {
            tracing::debug!(
                event = event_name,
                "ignoring a hook for an event this build does not fire"
            );
            continue;
        };
        let Some(rules) = rules.as_array() else {
            continue;
        };
        for rule in rules {
            let matcher = rule
                .get("matcher")
                .and_then(|matcher| matcher.as_str())
                .map(str::to_owned);
            let Some(entries) = rule.get("hooks").and_then(|hooks| hooks.as_array()) else {
                continue;
            };
            for entry in entries {
                let Some(command) = entry.get("command").and_then(|command| command.as_str())
                else {
                    // `type: "prompt"` and `type: "http"` hooks are not
                    // implemented; skipping them with a log line is honest,
                    // whereas running an empty command would not be.
                    if let Some(kind) = entry.get("type").and_then(|kind| kind.as_str()) {
                        tracing::debug!(kind, "hook type is not supported yet; skipping it");
                    }
                    continue;
                };
                let timeout = entry
                    .get("timeout")
                    .and_then(|timeout| timeout.as_u64())
                    .unwrap_or(DEFAULT_HOOK_SECONDS)
                    .clamp(1, MAX_HOOK_SECONDS);
                found.push(Hook {
                    event,
                    matcher: matcher.clone(),
                    command: if let Some(args) = entry.get("args").and_then(|v| v.as_array()) {
                        let quoted = args.iter().filter_map(|v| v.as_str()).map(|arg| {
                            format!("\"{}\"", arg.replace('\\', "/").replace('\"', "\\\""))
                        }).collect::<Vec<_>>().join(" ");
                        format!("{command} {quoted}")
                    } else { command.to_owned() },
                    timeout: Duration::from_secs(timeout),
                    plugin_root: path.ancestors().find(|ancestor| ancestor.join(".claude-plugin/plugin.json").is_file()).map(Path::to_path_buf),
                    shell: entry.get("shell").and_then(|v| v.as_str()).map(str::to_owned),
                    asynchronous: entry.get("async").and_then(|v| v.as_bool()).unwrap_or(false),
                });
            }
        }
    }
    found
}

/// The shell a hook runs through.
///
/// Hooks are written as shell commands, so they need a shell. POSIX first on
/// Windows for the same reason the Bash tool prefers it: a hook ported from
/// elsewhere is a `sh` script, and `cmd` cannot run one.
fn shell(command: &str) -> tokio::process::Command {
    let mut process = match rcode_agent::tools::posix_shell() {
        Some(shell) => {
            let mut process = tokio::process::Command::new(shell);
            process.arg("-c").arg(command);
            process
        }
        None => {
            let mut process = tokio::process::Command::new("cmd");
            process.arg("/C").arg(command);
            process
        }
    };
    process.kill_on_drop(true);
    process
}

fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… (truncated)", &text[..end])
}

pub fn executor_for_session(data_dir: Option<&Path>, cwd: &Path, session_id: &str) -> std::sync::Arc<dyn HookRunner> {
    let mut executor = HookExecutor::load(data_dir, cwd);
    executor.session_id = session_id.to_owned();
    std::sync::Arc::new(executor)
}

/// The hooks a run should use, or nothing when none are configured.
///
/// Returns the no-op runner when there is nothing to do, so the loop's mount
/// points stay unconditional.
pub fn executor_for(data_dir: Option<&Path>, cwd: &Path) -> std::sync::Arc<dyn HookRunner> {
    let executor = HookExecutor::load(data_dir, cwd);
    if executor.is_empty() {
        return std::sync::Arc::new(rcode_agent::hook::NoHooks);
    }
    tracing::debug!(
        hooks = executor.hooks.len(),
        "hooks are configured for this run"
    );
    std::sync::Arc::new(executor)
}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
