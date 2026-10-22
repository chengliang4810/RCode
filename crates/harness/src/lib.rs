//! rcode-harness — one interface over coding agents (plus a mock for tests).
//!
//! There are two drivers. [`NativeHarness`] runs the ReAct loop in this process
//! ([`rcode_agent`]) over the provider protocols in [`rcode_model`]; [`MockHarness`]
//! replays canned events for tests. The external-CLI drivers this crate used to
//! carry — the ACP adapters and the per-CLI wires that drove an installed agent
//! binary — are gone with the subprocess plumbing they needed, so a harness no
//! longer probes for or spawns a program at all (decision record:
//! `docs/research/acp.md`).

use async_trait::async_trait;
use futures::stream::BoxStream;
use tokio::sync::{mpsc, oneshot};
pub use tokio_util::sync::CancellationToken;

use rcode_proto::{
    AgentEvent, HarnessId, Model, ReasoningLevel, RunRequest, SlashCommand, SteeringMode,
};

#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error("harness binary not found: {0}")]
    NotInstalled(String),
    #[error("harness protocol error: {0}")]
    Protocol(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// A steer prompt pushed into a live run; delivered at the harness's steering boundary.
pub struct SteerMessage {
    pub prompt: String,
    /// User message delivery id; never reuse this as an assistant message id.
    pub message_id: Option<String>,
    /// Staged image paths sent with this prompt. Drivers that inline images
    /// deliver them with the text; the rest rely on the path refs the prompt
    /// text already carries.
    pub attachments: Vec<String>,
    /// The run configuration this prompt was sent with, when it differs
    /// from the live runtime's and the driver adopts it in place
    /// ([`Harness::reconfigures_in_place`]): apply it before the prompt.
    pub config: Option<Box<RunRequest>>,
}

impl SteerMessage {
    pub fn text(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            message_id: None,
            attachments: Vec::new(),
            config: None,
        }
    }
}

/// Host ↔ runtime signals that span a persistent runtime's turns.
///
/// Stopping a turn is not tearing down the runtime: a runtime holds work the
/// user never asked to stop (background subagents, background shells). A
/// driver that returns true from [`Harness::stops_turn_in_place`] answers
/// [`Self::stop_turn`] by ending only the in-flight turn with
/// `Done { status: Interrupted }` and staying alive for the next prompt; the
/// `interrupt` token remains the runtime teardown.
#[derive(Clone, Default)]
pub struct TurnControl {
    stop: std::sync::Arc<tokio::sync::Notify>,
    background: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl TurnControl {
    /// Host: end the in-flight turn, keeping the runtime and its background work.
    pub fn stop_turn(&self) {
        self.stop.notify_one();
    }

    /// Driver: resolves once per [`Self::stop_turn`].
    pub async fn stop_requested(&self) {
        self.stop.notified().await;
    }

    /// Driver: how many background tasks the runtime currently holds.
    pub fn set_background(&self, tasks: usize) {
        self.background
            .store(tasks, std::sync::atomic::Ordering::Release);
    }

    /// Host: the runtime reports live background work, so retiring it now
    /// would kill work nobody asked to stop.
    pub fn background_live(&self) -> bool {
        self.background.load(std::sync::atomic::Ordering::Acquire) > 0
    }
}

/// Run-config values compared as their wire meaning, not their JSON type:
/// clients disagree on spelling (the phone sends `"true"` where the desktop
/// sends `true`), and neither spelling is a different configuration.
pub fn same_model_options(
    a: &serde_json::Map<String, serde_json::Value>,
    b: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    fn canonical(value: &serde_json::Value) -> Option<String> {
        match value {
            serde_json::Value::Null => None,
            serde_json::Value::String(s) if s.is_empty() => None,
            serde_json::Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        }
    }
    let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    keys.into_iter()
        .all(|key| a.get(key).and_then(canonical) == b.get(key).and_then(canonical))
}

/// Host-side controls handed to a run: input-request bridge + steering mailbox.
pub struct RunControls {
    pub conversation_id: Option<String>,
    pub goal: Option<std::sync::Arc<dyn rcode_agent::goal::GoalStore>>,
    /// The run asks the user and awaits the answer (blocks the agent).
    ///
    /// Questions and answers are typed so the host preserves option ids and
    /// free-text answers while a run waits for user input.
    pub request_input: Box<
        dyn Fn(
                rcode_proto::InteractionRequest,
            ) -> oneshot::Receiver<rcode_proto::InteractionResponse>
            + Send
            + Sync,
    >,
    /// Steer prompts consumed at step/turn boundaries.
    pub steering: mpsc::Receiver<SteerMessage>,
    /// Cancel to interrupt the live run: the harness sends its protocol-level
    /// interrupt, then escalates to SIGTERM/SIGKILL on the child after a grace
    /// period. The run's stream ends with `Done { status: Interrupted }`.
    pub interrupt: CancellationToken,
    /// Turn-level stop and background-work reporting (see [`TurnControl`]).
    pub turn: TurnControl,
    /// The conversation so far, for a harness that replays it.
    ///
    /// Deliberately on the controls rather than on [`RunRequest`]: the request
    /// is serialized into the session's durable command ledger, so carrying
    /// the transcript there would store every turn's history again on each
    /// turn — quadratic growth in a document that is already the largest thing
    /// the app writes. The controls are runtime-only, which is what a replay
    /// input should be.
    ///
    /// `None` means the harness should run the prompt on its own. That is the
    /// right default for a driver that keeps its own session (an external CLI
    /// resumed by id) — replaying a transcript at it would duplicate history
    /// it already holds.
    pub history: Option<Vec<rcode_proto::ConversationTurn>>,
}

#[async_trait]
pub trait Harness: Send + Sync {
    fn id(&self) -> HarnessId;
    fn display_name(&self) -> &str;
    fn supports_steering(&self) -> bool;
    fn steering_mode(&self) -> SteeringMode;
    fn reasoning_levels(&self) -> &[ReasoningLevel];
    /// Whether the agent is available on this device — the settings gate for
    /// enabling the harness. Both drivers run in this process, so there is no
    /// CLI to probe and the answer is always yes.
    fn installed(&self) -> bool {
        true
    }
    /// Whether every turn shape — user-prompted AND agent-initiated
    /// (background-subagent wakes) — ends with a deterministic `Done` from
    /// the driver itself rather than from a quiesce watchdog.
    fn deterministic_turn_end(&self) -> bool {
        false
    }
    /// Whether a user-prompted turn has an authoritative completion signal.
    /// Such turns must never be parked merely because their stream is quiet.
    /// Unlike deterministic_turn_end, this need not cover autonomous activity.
    fn authoritative_prompt_end(&self) -> bool {
        self.deterministic_turn_end()
    }
    /// Whether this driver honors [`TurnControl::stop_turn`]: it ends only
    /// the in-flight turn (`Done { Interrupted }`) and the runtime stays up
    /// with its background work. Otherwise stopping a turn tears the runtime
    /// down through the `interrupt` token.
    fn stops_turn_in_place(&self) -> bool {
        false
    }
    /// Whether a live runtime started for `live` can take `next` as its
    /// next prompt. Only a difference that changes how the runtime was
    /// launched needs a fresh one — replacing a runtime kills its background
    /// work. The default covers what every driver's launch reads (the
    /// sandbox level and auto-approve are read only by drivers that
    /// override this); drivers narrow it further to their own launch.
    /// Whether a live runtime started for `live` adopts `next`'s differing
    /// configuration in place, before `next`'s prompt, instead of being
    /// replaced (which kills its background work). The prompt then arrives
    /// with [`SteerMessage::config`] set. Consulted only when
    /// [`Self::same_runtime`] is false.
    fn reconfigures_in_place(&self, _live: &RunRequest, _next: &RunRequest) -> bool {
        false
    }
    fn same_runtime(&self, live: &RunRequest, next: &RunRequest) -> bool {
        live.model == next.model
            && live.reasoning == next.reasoning
            && same_model_options(&live.model_options, &next.model_options)
            && live.cwd == next.cwd
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError>;
    /// Slash commands this driver advertises; empty for drivers without them.
    async fn commands(&self) -> Result<Vec<SlashCommand>, HarnessError> {
        Ok(Vec::new())
    }
    async fn custom_commands(&self, _cwd: Option<&std::path::Path>) -> Result<Vec<rcode_proto::custom_commands::CustomCommand>, HarnessError> {
        Ok(Vec::new())
    }
    async fn mutate_custom_command(&self, _mutation: rcode_proto::custom_commands::CommandMutation) -> Result<(), HarnessError> {
        Err(HarnessError::Protocol("This agent does not manage command files".into()))
    }
    async fn expand_custom_command(&self, _cwd: &std::path::Path, _input: &str) -> Result<Option<String>, HarnessError> {
        Ok(None)
    }
    /// Discover commands in the same directory as the eventual session.
    async fn commands_for(
        &self,
        _cwd: &std::path::Path,
    ) -> Result<Vec<SlashCommand>, HarnessError> {
        self.commands().await
    }
    /// Write a new skill into the scope's managed directory, or refuse because
    /// the name is taken.
    ///
    /// `root` is the project directory, not the skills directory: the layout
    /// (`<project>/.agents/skills`) is the agent crate's to define, so the
    /// caller names the place and this resolves the path. A writer that took a
    /// skills directory would let a caller pass the project and create skills
    /// the scan never looks at.
    ///
    /// `scope` picks which managed root receives the write — the project's, or
    /// the user's under the home directory. The scope is a name, never a path:
    /// the root is derived here, so a caller cannot aim a skill anywhere the
    /// scan does not look.
    ///
    /// On the harness rather than the caller because the engine does not depend
    /// on the agent crate, and because "what a skill is" must be one answer:
    /// this writes through the same module the scan reads with.
    fn create_skill(
        &self,
        _root: &std::path::Path,
        _scope: rcode_proto::invocation::SkillScope,
        _name: &str,
        _description: &str,
        _body: &str,
    ) -> Result<(), String> {
        Err("this harness cannot write skills".to_owned())
    }

    /// Remove a skill this application manages, from `scope`'s directory.
    fn delete_skill(
        &self,
        _root: &std::path::Path,
        _scope: rcode_proto::invocation::SkillScope,
        _name: &str,
    ) -> Result<(), String> {
        Err("this harness cannot delete skills".to_owned())
    }

    /// Rewrite a skill's name, description and — when `body` is `Some` — its
    /// instruction, within `scope`'s directory. `None` keeps the instruction.
    fn update_skill(
        &self,
        _root: &std::path::Path,
        _scope: rcode_proto::invocation::SkillScope,
        _name: &str,
        _new_name: &str,
        _description: &str,
        _body: Option<&str>,
    ) -> Result<(), String> {
        Err("this harness cannot update skills".to_owned())
    }

    /// Stop one running subagent, by the tool call that spawned it.
    ///
    /// `true` when there was one to stop. The id is the spawn's tool-call id —
    /// the same id the child's transcript document is keyed by — so a caller
    /// holding a subagent's document can stop exactly that child without
    /// touching the conversation that spawned it or its siblings.
    fn background_subagent(&self, _chat_id: &str, _tool_use_id: &str) -> bool { false }

    fn cancel_subagent(&self, _tool_use_id: &str) -> bool {
        false
    }

    /// The provider configuration store this harness shares with its engine.
    ///
    /// On the trait so the engine can reach the *same* instance the harness
    /// reads, rather than opening a second view of the same file: two views
    /// would drift, and the revision a save checks against would be the wrong
    /// one.
    fn provider_store(&self) -> Option<std::sync::Arc<crate::provider_store::ProviderStore>> {
        None
    }

    /// The skills visible at `cwd`; `None` means this provider does not
    /// advertise skills.
    ///
    /// Each entry's [`rcode_proto::invocation::Skill::scope`] says which
    /// managed root it came from (project or user) when the driver tracks
    /// scopes; file-backed skills of either scope are listed together, project
    /// before user, so the model sees the same precedence its tools resolve
    /// names with.
    async fn skills(
        &self,
        _cwd: &std::path::Path,
    ) -> Result<Option<Vec<rcode_proto::invocation::Skill>>, HarnessError> {
        Ok(None)
    }
    /// Personal settings catalog, without any project roots.
    async fn personal_skills(
        &self,
    ) -> Result<Option<Vec<rcode_proto::invocation::Skill>>, HarnessError> {
        Ok(None)
    }

    /// The subagent definitions visible at `cwd`; `None` means this driver
    /// does not implement delegation.
    ///
    /// `Some(..)` from a driver that *does* means what it says: an empty list
    /// is "no definitions here", not "no subagent support". Each entry's
    /// [`rcode_proto::invocation::SubagentDefinition::scope`] names the
    /// managed root it came from, so a caller can address a write at that
    /// exact copy, and `path` is the stable identity the enable and
    /// model-override settings are keyed by.
    async fn subagents(
        &self,
        _cwd: &std::path::Path,
    ) -> Result<Option<Vec<rcode_proto::invocation::SubagentDefinition>>, HarnessError> {
        Ok(None)
    }

    /// Personal settings catalog, without any project roots.
    async fn personal_subagents(
        &self,
    ) -> Result<Option<Vec<rcode_proto::invocation::SubagentDefinition>>, HarnessError> {
        Ok(None)
    }

    /// Write a new subagent definition into `scope`'s managed directory.
    ///
    /// Like the skill writes, `root` is the project directory (the layout is
    /// the agent crate's to resolve) and `scope` is a *name* for one of the
    /// two managed roots, never a path.
    fn create_subagent(
        &self,
        _root: &std::path::Path,
        _scope: rcode_proto::invocation::SkillScope,
        _name: &str,
        _description: &str,
        _system_prompt: &str,
        _tools: &[String],
        _disallowed_tools: &[String],
        _model: &str,
    ) -> Result<(), String> {
        Err("this harness cannot write subagent definitions".to_owned())
    }

    /// Remove a subagent definition this application manages, from `scope`'s
    /// directory.
    fn delete_subagent(
        &self,
        _root: &std::path::Path,
        _scope: rcode_proto::invocation::SkillScope,
        _name: &str,
    ) -> Result<(), String> {
        Err("this harness cannot delete subagent definitions".to_owned())
    }

    /// Rewrite a subagent definition's frontmatter and — when
    /// `system_prompt` is `Some` — its body, within `scope`'s directory.
    fn update_subagent(
        &self,
        _root: &std::path::Path,
        _scope: rcode_proto::invocation::SkillScope,
        _name: &str,
        _new_name: &str,
        _description: &str,
        _system_prompt: Option<&str>,
        _tools: &[String],
        _disallowed_tools: &[String],
        _model: &str,
    ) -> Result<(), String> {
        Err("this harness cannot update subagent definitions".to_owned())
    }

    /// Run an isolated title request. Drivers must opt in with title-specific
    /// instructions and restrictions; never fall back to an ordinary coding run.
    async fn run_title(
        &self,
        _request: RunRequest,
        _controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        Err(HarnessError::Protocol(
            "title generation is not supported by this harness".into(),
        ))
    }

    /// The context window of `model`, in tokens, when the harness knows it.
    ///
    /// Lets the *engine* decide whether a conversation needs compacting before
    /// it dispatches a run. The loop also asks (through
    /// `AgentHost::context_window`) so it can trim mid-run; both need the same
    /// number, and only the harness can resolve it.
    ///
    /// `None` means unknown, which disables compaction rather than guessing.
    fn context_window(&self, _model: &str) -> Option<u32> {
        None
    }

    /// Whether a conversation of `transcript` would overflow `model`'s window.
    ///
    /// The engine asks this before dispatching, so it can compact while it can
    /// still write the result to the document. The measurement lives on the
    /// harness because the token estimator does: the engine does not depend on
    /// the agent crate, and duplicating the estimate there is how the two
    /// would drift apart.
    ///
    /// `model` is the turn's selection, or `None` for "whatever this harness
    /// defaults to" — the harness resolves it the same way the run would, so
    /// the check measures the model that will actually answer.
    ///
    /// `transcript` is the flattened conversation, in the same shape a replay
    /// would carry.
    fn needs_compaction(&self, _model: Option<&str>, _transcript: &str) -> bool {
        false
    }

    /// Pre-dispatch compaction using the same per-conversation options as the run.
    fn needs_compaction_with_options(
        &self,
        model: Option<&str>,
        _options: &serde_json::Map<String, serde_json::Value>,
        transcript: &str,
    ) -> bool {
        self.needs_compaction(model, transcript)
    }

    /// Summarize a conversation transcript for `/compact`, in one isolated call.
    ///
    /// Like [`Self::run_title`], this must not become an ordinary run: it takes
    /// no tools, writes nothing to the transcript, and the caller reads only
    /// its text. The difference from a title is the input — a title summarizes
    /// one message, this summarizes everything the model is about to stop
    /// seeing — so it is a separate entry point rather than a flag.
    ///
    /// `focus` is whatever the user passed after the command; empty means
    /// "keep whatever matters".
    async fn summarize(&self, _transcript: String, _focus: String) -> Result<String, HarnessError> {
        Err(HarnessError::Protocol(
            "summarization is not supported by this harness".into(),
        ))
    }

    /// Invalidate prompt/context snapshots after a conversation is cleared or compacted.
    async fn reset_prompt_cache(&self, _session_id: &str) {}

    /// Bootstrap without a synthetic user prompt; providers opt in explicitly.
    async fn start_idle(
        &self,
        _request: RunRequest,
        _controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        Err(HarnessError::Protocol("idle runtime unsupported".into()))
    }

    /// Run one (persistent) session; the stream ends with `AgentEvent::Done`.
    async fn run(
        &self,
        request: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError>;
}

pub mod context_window;
pub mod hooks;
pub mod mock;
pub mod native;
mod native_session;
mod native_bridge;
pub mod native_subagents;
mod builtin_subagents;
pub mod prompts;
pub mod provider_store;
pub mod model_catalog;
pub mod providers;
pub mod settings;
pub mod shell_env;
#[cfg(windows)]
pub mod windows_process;

/// Add the login shell's PATH to a child process while preserving the PATH of
/// the current process. This lets GUI/service launches find user-installed
/// CLIs such as Homebrew's `gh` without changing the daemon's own environment.
pub fn compose_login_shell_path(cmd: &mut tokio::process::Command) {
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path));
    }
    if let Some(shell_path) = shell_env::login_shell_path() {
        paths.extend(std::env::split_paths(shell_path));
    }
    let mut seen = std::collections::HashSet::new();
    paths.retain(|p| !p.as_os_str().is_empty() && seen.insert(p.clone()));
    if let Ok(joined) = std::env::join_paths(paths) {
        cmd.as_std_mut().env("PATH", joined);
    }
}

/// Drivers with a restricted title-generation path.
pub fn supports_titles(id: HarnessId) -> bool {
    matches!(
        id,
        HarnessId::Codex | HarnessId::ClaudeCode | HarnessId::Mock | HarnessId::Native
    )
}
