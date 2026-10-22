//! Hooks: the user's own code, run at points in the loop.
//!
//! A hook is a shell command (or, later, a prompt or an HTTP call) the user
//! registered against a lifecycle event. It exists because the loop cannot
//! anticipate every project's rules: one team wants a linter to run after every
//! edit, another wants a command refused outright, a third wants to be told
//! when a turn ends. Rather than growing an option for each, the loop exposes
//! the points and lets the project answer.
//!
//! # Why the contract lives here and not with the executor
//!
//! Some events are *decisions*, not notifications. `PreToolUse` runs before a
//! tool call and can refuse it — so the loop has to consult it synchronously,
//! in the middle of its own flow, and honour the answer. That makes the hook
//! point a property of the loop rather than of
//! whatever happens to implement it.
//!
//! So this module is the contract and nothing else: the events, what a hook
//! may answer, and the trait the loop calls. Running a shell command, reading
//! `hooks.json`, and matching tool names all live with the host
//! (`rcode-harness`), which already knows how to start a process and where the
//! user's files are. The loop stays ignorant of both.
//!
//! # Failure is not a veto
//!
//! A hook that cannot run — a missing binary, a timeout, unreadable config —
//! does not block the tool call it was attached to. Failing open is the
//! deliberate choice: the alternative lets a typo in a config file make the
//! agent unusable, which is a worse outcome than a lint that did not run. A
//! hook that *means* to refuse does so by exiting with the documented status,
//! which is a decision the user made rather than an accident.

use std::collections::BTreeMap;

/// A point in the loop a hook can attach to.
///
/// The set is deliberately the events this loop can actually fire. The original project
/// carries twenty-eight, most of which belong to subsystems RCode does not
/// have (teams, worktrees, MCP elicitation); accepting configuration for an
/// event that never fires would be a setting that silently does nothing.
///
/// Serialized as the PascalCase name Claude Code uses, so a `hooks.json` the
/// user already has keeps working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HookEvent {
    /// A run is starting, before the first model call.
    SessionStart,
    /// The user's message was accepted, before it reaches the model.
    UserPromptSubmit,
    /// A tool call is about to run. May refuse it.
    PreToolUse,
    /// A tool call finished, successfully or not.
    PostToolUse,
    /// A tool call failed. Fired after `PostToolUse`, never instead of it.
    PostToolUseFailure,
    /// The run ended.
    Stop,
    /// The model request failed; notification only, never a decision.
    StopFailure,
}

impl HookEvent {
    /// Every event, for iterating a configuration.
    pub const ALL: [HookEvent; 7] = [
        Self::SessionStart,
        Self::UserPromptSubmit,
        Self::PreToolUse,
        Self::PostToolUse,
        Self::PostToolUseFailure,
        Self::Stop,
        Self::StopFailure,
    ];

    /// The name on the wire and in `hooks.json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::PostToolUseFailure => "PostToolUseFailure",
            Self::Stop => "Stop",
            Self::StopFailure => "StopFailure",
        }
    }

    /// Parse the wire name. Unknown names are `None` rather than an error:
    /// a file naming an event this build does not fire should be skipped with
    /// a log line, not rejected whole.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|event| event.as_str() == name)
    }
}

/// What the loop was doing when a hook fired.
///
/// One struct for every event rather than one per event: the events overlap
/// heavily (most want to know which tool, which path), and a per-event type
/// would either duplicate those fields or hide them behind a match the hook
/// author has to write anyway.
#[derive(Debug, Clone, Default)]
pub struct HookContext {
    /// SessionStart source: startup, resume, clear, or compact.
    pub source: Option<String>,
    /// The tool call this event is about, for the tool-scoped events.
    pub tool_name: Option<String>,
    /// The tool's arguments, as the model produced them.
    pub tool_input: Option<serde_json::Value>,
    /// The tool's result, for the events that fire after it ran.
    pub tool_output: Option<String>,
    /// Whether the tool call failed. Set on `PostToolUse` and
    /// `PostToolUseFailure` alike, so a hook attached to either sees the truth.
    pub tool_failed: bool,
    /// The user's message, for `UserPromptSubmit` and `SessionStart`.
    pub prompt: Option<String>,
    /// The working directory the run is using.
    pub cwd: std::path::PathBuf,
    /// Why the run stopped, for `Stop`.
    pub stop_reason: Option<String>,
}

impl HookContext {
    pub fn for_tool(name: &str, input: &serde_json::Value, cwd: &std::path::Path) -> Self {
        Self {
            tool_name: Some(name.to_owned()),
            tool_input: Some(input.clone()),
            cwd: cwd.to_path_buf(),
            ..Self::default()
        }
    }

    /// The same context, describing what the tool returned.
    pub fn with_result(mut self, output: &str, failed: bool) -> Self {
        self.tool_output = Some(output.to_owned());
        self.tool_failed = failed;
        self
    }
}

/// What a hook decided.
///
/// The three shapes are the three things a hook can usefully do: say nothing,
/// say something, or stop what is happening. Everything else (logging,
/// notifying, running a side effect) is a hook that returns
/// [`Allow`](Self::Allow) and does its work as a side effect.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum HookOutcome {
    /// Continue. The hook had nothing to add.
    #[default]
    Allow,
    /// Continue, but the text is added to the model's context.
    ///
    /// This is the "tell the model something it does not know" case: the
    /// project's conventions, a warning about the file it just touched.
    Context(String),
    /// Refuse the tool call, or stop the run.
    ///
    /// The message reaches the model as the tool's error, so it should say
    /// what was wrong and what to do instead — the model can act on it.
    Block(String),
}

impl HookOutcome {
    /// Merge another hook's outcome into this one.
    ///
    /// A block wins over context, and the first block's message is kept: once
    /// a call is refused, what a later hook would have added to the context is
    /// moot. Context accumulates, because two hooks each with something to say
    /// should both be heard.
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Block(message), _) => Self::Block(message),
            (_, Self::Block(message)) => Self::Block(message),
            (Self::Context(mut existing), Self::Context(more)) => {
                existing.push('\n');
                existing.push_str(&more);
                Self::Context(existing)
            }
            (Self::Context(existing), Self::Allow) => Self::Context(existing),
            (Self::Allow, Self::Context(more)) => Self::Context(more),
            (Self::Allow, Self::Allow) => Self::Allow,
        }
    }

    /// The text to add to the model's context, if any.
    pub fn context(&self) -> Option<&str> {
        match self {
            Self::Context(text) => Some(text),
            _ => None,
        }
    }

    /// The refusal message, if this outcome blocks.
    pub fn block(&self) -> Option<&str> {
        match self {
            Self::Block(message) => Some(message),
            _ => None,
        }
    }
}

/// Why a hook did not run.
///
/// Carried rather than swallowed so the loop can log it, and so a test can
/// assert that a broken hook was reported instead of silently ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookFailure {
    pub event: HookEvent,
    pub message: String,
}

/// The hooks registered for one run.
///
/// Implemented by the host, called by the loop. `fire` takes every hook bound
/// to `event`, runs them in order, and merges their answers — a hook runner
/// never sees the loop's state, only the context it was handed.
#[async_trait::async_trait]
pub trait HookRunner: Send + Sync {
    /// Tool hooks may have side effects even when the tool itself is read-only.
    fn allows_concurrent_tools(&self) -> bool { false }
    /// Run every hook for `event` and merge what they decide.
    ///
    /// Must not fail: a hook that breaks is reported through the returned
    /// failures and the outcome falls back to [`HookOutcome::Allow`]. The loop
    /// has no sensible way to handle a hook *system* that is itself broken, and
    /// failing the run over it would be the wrong call (see the module note on
    /// failing open).
    async fn fire(&self, event: HookEvent, context: HookContext) -> HookResult;
}

/// What firing a batch of hooks produced.
#[derive(Debug, Clone, Default)]
pub struct HookResult {
    pub outcome: HookOutcome,
    /// Hooks that did not run, in the order they were tried.
    pub failures: Vec<HookFailure>,
}

impl HookResult {
    pub fn allow() -> Self {
        Self::default()
    }

    pub fn blocked(message: impl Into<String>) -> Self {
        Self {
            outcome: HookOutcome::Block(message.into()),
            failures: Vec::new(),
        }
    }

    /// Whether anything blocked.
    pub fn is_blocked(&self) -> bool {
        matches!(self.outcome, HookOutcome::Block(_))
    }
}

/// A no-op runner, for a host with no hooks configured.
///
/// Exists so the loop can hold a `dyn HookRunner` unconditionally rather than
/// branching on an `Option` at every mount point — and so the "no hooks" path
/// is the same code as the "hooks that do nothing" path, which is what the
/// tests exercise.
pub struct NoHooks;

#[async_trait::async_trait]
impl HookRunner for NoHooks {
    fn allows_concurrent_tools(&self) -> bool { true }
    async fn fire(&self, _event: HookEvent, _context: HookContext) -> HookResult {
        HookResult::allow()
    }
}

/// The environment variables a hook command receives, as a map.
///
/// The loop supplies the values; the host turns them into a process
/// environment. Kept here so the names are one list rather than a string
/// literal at the spawn site and another in the documentation.
pub fn hook_environment(context: &HookContext) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    if let Some(tool) = &context.tool_name {
        environment.insert("HOOK_TOOL_NAME".to_owned(), tool.clone());
    }
    if let Some(input) = &context.tool_input {
        // A serialization failure means the arguments cannot be represented as
        // JSON, which cannot happen for a value that arrived as JSON — an
        // empty string is the honest fallback rather than dropping the key.
        environment.insert(
            "HOOK_TOOL_INPUT".to_owned(),
            serde_json::to_string(input).unwrap_or_default(),
        );
    }
    if let Some(output) = &context.tool_output {
        environment.insert("HOOK_TOOL_OUTPUT".to_owned(), output.clone());
    }
    if context.tool_failed {
        environment.insert("HOOK_TOOL_FAILED".to_owned(), "1".to_owned());
    }
    if let Some(prompt) = &context.prompt {
        environment.insert("HOOK_PROMPT".to_owned(), prompt.clone());
    }
    environment
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_round_trips_through_its_wire_name() {
        for event in HookEvent::ALL {
            assert_eq!(HookEvent::parse(event.as_str()), Some(event));
        }
    }

    #[test]
    fn the_wire_names_are_the_ones_claude_code_writes() {
        // A user's existing hooks.json must keep working; these are the exact
        // keys Claude Code documents.
        assert_eq!(HookEvent::PreToolUse.as_str(), "PreToolUse");
        assert_eq!(HookEvent::PostToolUseFailure.as_str(), "PostToolUseFailure");
        assert_eq!(HookEvent::UserPromptSubmit.as_str(), "UserPromptSubmit");
        assert_eq!(HookEvent::SessionStart.as_str(), "SessionStart");
    }

    #[test]
    fn an_unknown_event_name_is_none_rather_than_an_error() {
        assert_eq!(HookEvent::parse("TeammateIdle"), None);
        assert_eq!(HookEvent::parse(""), None);
    }

    #[test]
    fn a_block_wins_over_context_whichever_order_they_arrive_in() {
        let block = HookOutcome::Block("no".to_owned());
        let context = HookOutcome::Context("note".to_owned());

        assert_eq!(block.clone().merge(context.clone()), block);
        assert_eq!(context.merge(block.clone()), block);
    }

    #[test]
    fn the_first_block_is_the_one_reported() {
        let first = HookOutcome::Block("first".to_owned());
        let second = HookOutcome::Block("second".to_owned());
        assert_eq!(first.merge(second), HookOutcome::Block("first".to_owned()));
    }

    #[test]
    fn context_accumulates_from_every_hook_that_produced_some() {
        let merged =
            HookOutcome::Context("one".to_owned()).merge(HookOutcome::Context("two".to_owned()));
        assert_eq!(merged, HookOutcome::Context("one\ntwo".to_owned()));
    }

    #[test]
    fn allow_merges_as_identity() {
        assert_eq!(
            HookOutcome::Allow.merge(HookOutcome::Allow),
            HookOutcome::Allow
        );
        assert_eq!(
            HookOutcome::Allow.merge(HookOutcome::Context("x".to_owned())),
            HookOutcome::Context("x".to_owned())
        );
        assert_eq!(
            HookOutcome::Context("x".to_owned()).merge(HookOutcome::Allow),
            HookOutcome::Context("x".to_owned())
        );
    }

    #[test]
    fn a_context_outcome_exposes_its_text_and_blocks_nothing() {
        let outcome = HookOutcome::Context("note".to_owned());
        assert_eq!(outcome.context(), Some("note"));
        assert_eq!(outcome.block(), None);
    }

    #[test]
    fn a_tool_context_carries_the_call_and_then_its_result() {
        let context = HookContext::for_tool(
            "Read",
            &serde_json::json!({ "file_path": "a.rs" }),
            std::path::Path::new("/tmp"),
        );
        assert_eq!(context.tool_name.as_deref(), Some("Read"));
        assert!(!context.tool_failed);
        assert_eq!(context.tool_output, None);

        let after = context.with_result("contents", false);
        assert_eq!(after.tool_output.as_deref(), Some("contents"));
        assert!(!after.tool_failed);
    }

    #[test]
    fn the_environment_exposes_only_the_keys_the_context_has() {
        let context = HookContext::for_tool(
            "Read",
            &serde_json::json!({ "file_path": "a.rs" }),
            std::path::Path::new("/tmp"),
        );
        let environment = hook_environment(&context);

        assert_eq!(
            environment.get("HOOK_TOOL_NAME").map(String::as_str),
            Some("Read")
        );
        assert_eq!(
            environment.get("HOOK_TOOL_INPUT").map(String::as_str),
            Some(r#"{"file_path":"a.rs"}"#)
        );
        // Nothing produced these yet, so they must be absent rather than empty:
        // a hook testing `-n "$HOOK_TOOL_OUTPUT"` must see "unset".
        assert!(!environment.contains_key("HOOK_TOOL_OUTPUT"));
        assert!(!environment.contains_key("HOOK_TOOL_FAILED"));
        assert!(!environment.contains_key("HOOK_PROMPT"));
    }

    #[test]
    fn the_environment_marks_a_failed_tool_call() {
        let context =
            HookContext::for_tool("Bash", &serde_json::json!({}), std::path::Path::new("/tmp"))
                .with_result("boom", true);
        let environment = hook_environment(&context);
        assert_eq!(
            environment.get("HOOK_TOOL_FAILED").map(String::as_str),
            Some("1")
        );
    }

    #[tokio::test]
    async fn the_no_op_runner_allows_and_reports_nothing() {
        let result = NoHooks
            .fire(HookEvent::PreToolUse, HookContext::default())
            .await;
        assert_eq!(result.outcome, HookOutcome::Allow);
        assert!(result.failures.is_empty());
        assert!(!result.is_blocked());
    }

    #[test]
    fn a_blocked_result_says_so() {
        let result = HookResult::blocked("refused");
        assert!(result.is_blocked());
        assert_eq!(result.outcome.block(), Some("refused"));
        assert!(HookResult::allow().failures.is_empty());
    }
}
