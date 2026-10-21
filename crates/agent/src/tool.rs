//! The tool system: what a tool is, what it hands back, and the registry the
//! loop dispatches through.
//!
//! This is the framework only — no concrete tools live here. `Read`, `Write`,
//! `Bash` and the rest are separate implementations of [`Tool`]; see the tool
//! inventory in `docs/design/native-agent-rewrite.md` §5. Splitting the
//! framework from the tools is what lets a host register a different set per
//! conversation (and lets tests register a mock) without the loop knowing.
//!
//! ```text
//!   model ──tool call──▶ ToolRegistry::call ──▶ Arc<dyn Tool>::call
//!                              │                        │
//!                       ToolError::NotFound       ToolResult
//! ```
//!
//! Tools execute directly. The model adapter turns
//! [`ToolRegistry::definitions`] into the provider's wire format.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

/// A tool the model can call.
///
/// Model-side information ([`name`](Tool::name), [`description`](Tool::description),
/// [`input_schema`](Tool::input_schema)) and execution-side behaviour
/// ([`call`](Tool::call)) live on the same trait on purpose: a tool that
/// describes itself in a way its implementation does not honour is a bug that
/// only shows up as a confused model.
///
/// Implementations are shared across conversations and run concurrently, so
/// they must be `Send + Sync` and keep per-call state in the arguments or in
/// their own synchronisation, never in `&mut self`.
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    /// Tool name, PascalCase (`"Read"`, `"Bash"`, `"FolderOperations"`).
    ///
    /// This is the name the model emits and the key [`ToolRegistry`] looks up,
    /// so it must be unique within a registry and stable across releases.
    fn name(&self) -> &str;

    /// One line describing what the tool does, written for the model.
    ///
    /// Kept short: it goes into every request's tool list.
    fn description(&self) -> &str;

    /// The arguments as a JSON Schema, used by the provider to constrain the
    /// model's tool call.
    ///
    /// Return an object schema (`{"type": "object", "properties": {…}}`). An
    /// empty object (`{}`) means "no declared arguments".
    fn input_schema(&self) -> serde_json::Value;

    /// Run the tool. `input` is the JSON the model produced.
    ///
    /// Returning `Ok` with [`ToolResult::is_error`] set is not the same as
    /// returning `Err`: a failed `Bash` command is a normal result the model
    /// should see and react to, whereas `Err` is reserved for the call itself
    /// not happening (bad arguments or timeout).
    ///
    /// Long-running tools should honour `ctx.cancel`.
    async fn call(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError>;
}

/// What a tool hands back to the loop.
#[derive(Debug, Clone, Default)]
pub struct ToolResult {
    /// Text returned to the model. This is the model's only view of the call,
    /// so it should be self-contained.
    pub output: String,
    /// Whether the call failed in a way the model should treat as an error.
    pub is_error: bool,
    /// The file change an edit-shaped tool made, for the UI's diff pane.
    /// Never sent to the model.
    ///
    /// Structured rather than a unified-diff string because that is the shape
    /// the wire and the diff pane already speak: the before and after text,
    /// not a rendering of them. A tool that only has a unified diff would have
    /// to make the pane re-parse it.
    pub diff: Option<rcode_proto::ToolDiff>,
}

impl ToolResult {
    /// A successful result carrying `output`.
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: false,
            diff: None,
        }
    }

    /// A failed result carrying `output` — the model sees this and can retry.
    pub fn error(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: true,
            diff: None,
        }
    }

    /// Attach a diff for the UI. Does not change the output or the error flag.
    pub fn with_diff(mut self, diff: rcode_proto::ToolDiff) -> Self {
        self.diff = Some(diff);
        self
    }
}

/// Why a tool call did not run, or ran and broke.
///
/// Kept coarse and string-carrying, like [`crate::AgentError`]: the message is
/// what reaches the model, so it should read as something it can act on.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// The model's arguments did not fit the tool's schema or were missing.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// The tool ran and failed for a reason it could not express as a result.
    #[error("tool failed: {0}")]
    Execution(String),
    /// The tool did not finish in the time allowed.
    #[error("timed out: {0}")]
    Timeout(String),
    /// No tool with that name is registered.
    #[error("no such tool: {0}")]
    NotFound(String),
    /// The run was cancelled before the call started.
    #[error("cancelled")]
    Cancelled,
}

/// The environment a tool runs in, supplied by the host.
///
/// Deliberately minimal: only what the framework itself needs to pass through.
/// Host-injected capabilities (filesystem access, a PTY, a sandbox policy,
/// background-task handles) will be added here as the tools that need them are
/// ported — a tool should reach the outside world through this struct rather
/// than through globals, so the host can vary it per run.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Working directory: relative paths in tool arguments resolve against it.
    pub cwd: PathBuf,
    /// Cancellation for the whole run. Checked before dispatch and honoured by
    /// long-running tools.
    pub cancel: CancellationToken,
    /// Where a tool that runs its own nested agent — the `Agent` tool —
    /// streams that child's events. The loop binds the spawning tool call's
    /// id and the shared event channel; a tool sends the child's
    /// [`crate::AgentEvent`]s here and the host sees them tagged as nested
    /// under that call. `None` for a caller that does not consume nested
    /// events (tests, a bare registry); a tool then runs its child without
    /// anyone watching the transcript.
    pub nested: Option<crate::tools::NestedEventSender>,
}

/// The tools available to one agent run.
///
/// Registration order is preserved, so [`definitions`](ToolRegistry::definitions)
/// and [`all`](ToolRegistry::all) hand the model a stable tool list — which
/// keeps the request prefix identical across turns and lets providers cache it.
/// Lookup goes through a name index, so `get`/`call` stay O(1) as the set grows.
///
/// `Clone` exists for a host that derives a *child* registry from a built one —
/// the subagent runtime clones the parent's set and registers only the tools a
/// definition allows. Tools are behind `Arc`, so a clone shares them rather
/// than rebuilding any.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    pub(crate) agent_tasks: Option<Arc<crate::agent_tasks::AgentTasks>>,
    tools: Vec<Arc<dyn Tool>>,
    /// Tool name → position in `tools`.
    index: HashMap<String, usize>,
}

impl ToolRegistry {
    pub fn managed_agents(&self) -> Option<Arc<crate::agent_tasks::AgentTasks>> { self.agent_tasks.clone() }
    pub fn set_managed_agents(&mut self, tasks: Arc<crate::agent_tasks::AgentTasks>) {
        tasks.register(self);
        self.agent_tasks = Some(tasks);
    }
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a tool, keyed by its own [`Tool::name`].
    ///
    /// Registering a name that is already present replaces the earlier tool in
    /// place, keeping its position in the list. That is what makes a host able
    /// to shadow a built-in (or an MCP tool) without reordering the rest.
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_owned();
        match self.index.get(&name) {
            Some(&at) => {
                tracing::debug!(tool = %name, "tool registry: replaced");
                self.tools[at] = tool;
            }
            None => {
                tracing::debug!(tool = %name, "tool registry: registered");
                self.index.insert(name, self.tools.len());
                self.tools.push(tool);
            }
        }
    }

    /// The tool with this name, if one is registered.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let canonical = match name { "Skill" => "SkillTool", _ => name };
        self.index.get(name).or_else(|| self.index.get(canonical)).map(|&at| Arc::clone(&self.tools[at]))
    }

    /// Every registered tool, in registration order.
    ///
    /// This is what the host's `reason` implementation receives; it normally
    /// wants [`definitions`](ToolRegistry::definitions) instead, which is the
    /// same list in model-facing form.
    pub fn all(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.clone()
    }

    /// The model-visible tool list, in registration order.
    ///
    /// Each entry is `{"name": …, "description": …, "input_schema": …}` — the
    /// Anthropic Messages tool shape, and the common core every provider needs.
    /// `rcode-model` adapts it per protocol (OpenAI Chat and Responses wrap it
    /// as `{"type": "function", "function": {"parameters": …}}`); keeping one
    /// neutral shape here means a tool never learns which provider is running.
    pub fn definitions(&self) -> Vec<serde_json::Value> {
        self.tools
            .iter()
            .map(|tool| definition(tool.as_ref()))
            .collect()
    }

    /// Run the named tool.
    ///
    /// Fails with [`ToolError::NotFound`] for an unknown name — the loop turns
    /// that into an error result for the model rather than aborting the run,
    /// because a model can invent a tool name and should get to correct itself.
    ///
    /// A cancelled context short-circuits with [`ToolError::Cancelled`] so no
    /// tool starts after the run is over.
    pub async fn call(
        &self,
        name: &str,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let tool = self
            .get(name)
            .ok_or_else(|| ToolError::NotFound(name.to_owned()))?;
        if ctx.cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        tracing::debug!(tool = %name, "tool registry: dispatching");
        tool.call(input, ctx).await
    }

    /// How many tools are registered.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether no tools are registered.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

/// One tool in the model-visible shape documented on [`ToolRegistry::definitions`].
fn definition(tool: &dyn Tool) -> serde_json::Value {
    serde_json::json!({
        "name": tool.name(),
        "description": tool.description(),
        "input_schema": tool.input_schema(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A tool that echoes a `text` argument and counts its calls.
    struct EchoTool {
        name: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "echoes its input back"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
            })
        }

        async fn call(
            &self,
            input: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolResult, ToolError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let text = input
                .get("text")
                .and_then(|value| value.as_str())
                .ok_or_else(|| ToolError::InvalidInput("missing `text`".to_owned()))?;
            Ok(ToolResult::ok(text.to_owned()))
        }
    }

    /// A tool that always fails, to pin
    /// the trait's defaults.
    struct FailingTool;

    #[async_trait::async_trait]
    impl Tool for FailingTool {
        fn name(&self) -> &str {
            "Failing"
        }

        fn description(&self) -> &str {
            "always fails"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }

        async fn call(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolResult, ToolError> {
            Err(ToolError::Execution("boom".to_owned()))
        }
    }

    fn echo(name: &'static str, calls: &Arc<AtomicUsize>) -> Arc<dyn Tool> {
        Arc::new(EchoTool {
            name,
            calls: Arc::clone(calls),
        })
    }

    fn context() -> ToolContext {
        ToolContext {
            cwd: std::env::current_dir().unwrap_or_default(),
            cancel: CancellationToken::new(),
            nested: None,
        }
    }

    #[test]
    fn registering_a_tool_makes_it_findable() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        assert!(registry.is_empty());

        registry.register(echo("Echo", &calls));

        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());
        assert_eq!(registry.get("Echo").expect("registered").name(), "Echo");
        assert!(registry.get("Missing").is_none());
    }

    #[tokio::test]
    async fn registering_the_same_name_replaces_in_place() {
        let first = Arc::new(AtomicUsize::new(0));
        let second = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(echo("Echo", &first));
        registry.register(echo("Other", &first));
        registry.register(echo("Echo", &second));

        assert_eq!(registry.len(), 2, "the replacement did not add an entry");
        assert_eq!(
            registry
                .all()
                .iter()
                .map(|tool| tool.name().to_owned())
                .collect::<Vec<_>>(),
            vec!["Echo".to_owned(), "Other".to_owned()],
            "the replaced tool kept its position"
        );

        let result = registry
            .call("Echo", serde_json::json!({ "text": "hi" }), &context())
            .await
            .expect("call succeeds");
        assert_eq!(result.output, "hi");
        assert_eq!(
            first.load(Ordering::SeqCst),
            0,
            "the replaced tool did not run"
        );
        assert_eq!(second.load(Ordering::SeqCst), 1, "the replacement ran");
    }

    #[test]
    fn definitions_expose_name_description_and_schema_in_order() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(echo("Echo", &calls));
        registry.register(Arc::new(FailingTool));

        let definitions = registry.definitions();
        assert_eq!(definitions.len(), 2);
        assert_eq!(
            definitions
                .iter()
                .map(|definition| definition["name"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>(),
            vec!["Echo".to_owned(), "Failing".to_owned()],
            "registration order is preserved"
        );

        let echo = &definitions[0];
        assert_eq!(echo["description"], "echoes its input back");
        assert_eq!(echo["input_schema"]["type"], "object");
        assert_eq!(echo["input_schema"]["properties"]["text"]["type"], "string");
        assert_eq!(echo["input_schema"]["required"][0], "text");

        assert_eq!(
            definitions[1]["input_schema"],
            serde_json::json!({ "type": "object" })
        );
    }

    #[tokio::test]
    async fn call_dispatches_to_the_named_tool() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(echo("Echo", &calls));

        let result = registry
            .call("Echo", serde_json::json!({ "text": "hello" }), &context())
            .await
            .expect("call succeeds");

        assert_eq!(result.output, "hello");
        assert!(!result.is_error);
        assert!(result.diff.is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_unknown_tool_is_not_found() {
        let registry = ToolRegistry::new();

        let error = registry
            .call("Nope", serde_json::json!({}), &context())
            .await
            .expect_err("an unknown tool must not run");

        assert_eq!(error.to_string(), "no such tool: Nope");
        assert!(matches!(error, ToolError::NotFound(name) if name == "Nope"));
    }

    #[tokio::test]
    async fn a_tool_error_reaches_the_caller() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(FailingTool));

        let error = registry
            .call("Failing", serde_json::json!({}), &context())
            .await
            .expect_err("the tool failed");

        assert!(matches!(error, ToolError::Execution(message) if message == "boom"));
    }

    #[tokio::test]
    async fn bad_arguments_surface_as_invalid_input() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(echo("Echo", &calls));

        let error = registry
            .call("Echo", serde_json::json!({}), &context())
            .await
            .expect_err("`text` is required");

        assert!(matches!(error, ToolError::InvalidInput(message) if message.contains("text")));
    }

    #[tokio::test]
    async fn a_cancelled_context_does_not_start_a_tool() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(echo("Echo", &calls));

        let ctx = ToolContext {
            cwd: std::env::current_dir().unwrap_or_default(),
            cancel: CancellationToken::new(),
            nested: None,
        };
        ctx.cancel.cancel();

        let error = registry
            .call("Echo", serde_json::json!({ "text": "hi" }), &ctx)
            .await
            .expect_err("a cancelled run runs nothing");

        assert!(matches!(error, ToolError::Cancelled));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn results_are_constructible_without_naming_every_field() {
        assert!(!ToolResult::ok("done").is_error);
        assert!(ToolResult::error("nope").is_error);

        let result = ToolResult::ok("edited").with_diff(rcode_proto::ToolDiff {
            path: "a.rs".into(),
            old_text: Some("old".into()),
            new_text: "new".into(),
        });
        assert_eq!(result.diff.map(|diff| diff.path).as_deref(), Some("a.rs"));
        assert!(!result.is_error);
    }
}
