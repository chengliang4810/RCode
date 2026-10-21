//! The `Agent` tool — delegating a task to a named subagent.
//!
//! A subagent is a child [`AgentRunner`] the host spawns with its own tool set
//! and its own model, running under a definition's system prompt. The parent
//! sees one tool call; the child's streamed events travel out of band (tagged
//! with the spawning call's id) so a host can render them as a nested
//! transcript rather than folding them into the parent's.
//!
//! What this file owns is the *contract*: input parsing, definition
//! resolution, and the shape of the handoff. What it deliberately does not own
//! is the child itself — spawning one needs a model client, which is the
//! host's ([`SubagentRuntime`]) to provide. This crate stays free of provider
//! code, and a test can substitute a runtime that does anything.

use async_trait::async_trait;
use std::sync::Arc;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::subagents::{ModelAlias, SubagentDefinition};
use crate::tool::{Tool, ToolContext, ToolError, ToolResult};

/// The tool's name. `Agent` is the spelling the transcript decoder recognises
/// as a subagent spawn, so the chip (and the tab it opens) is named after the
/// task, not the bare tool.
pub const AGENT: &str = "Agent";

/// The definition the model gets when it does not name one: the parent's own
/// tool set (minus this tool) under a minimal delegation prompt.
pub const GENERAL_PURPOSE: &str = "general-purpose";

/// What a nested run hands back.
#[derive(Debug, Clone)]
pub struct SubagentOutcome {
    /// The child's final message — the text a parent model reads as the tool
    /// result.
    pub output: String,
    pub history: Vec<crate::Message>,
}

/// One nested run, ready to spawn.
pub struct SubagentSpawn {
    pub history_sink: Option<Arc<std::sync::Mutex<Vec<crate::Message>>>>,
    pub history: Vec<crate::Message>,
    pub inbox: Option<Arc<crate::agent_tasks::MessageInbox>>,
    /// The resolved definition. Its [`SubagentDefinition::allows`] decides the
    /// child's tool set; its `system_prompt` is the child's standing
    /// instruction.
    pub definition: SubagentDefinition,
    /// The task itself, as the child's first user message.
    pub prompt: String,
    /// The working directory the child's tools resolve against — the parent
    /// run's, so a delegation never widens the sandbox.
    pub cwd: std::path::PathBuf,
    /// Where the child's events go. The parent loop tags them with the
    /// spawning tool call's id.
    pub events: NestedEventSender,
    /// Cancels this child. Managed agents inherit the session runtime token;
    /// foreground cancellation is coordinated until they are promoted.
    /// A task stop remains independent of the parent turn and sibling tasks.
    pub cancel: CancellationToken,
    /// The tool call this spawn belongs to — the id the child's transcript
    /// document is keyed by, and therefore what a stop names.
    pub tool_use_id: String,
}

/// The host side of a spawn: model resolution, the child loop, the tool
/// filtering.
///
/// A trait so the Agent tool can be built without a provider stack (and so a
/// test can drive delegation with a scripted runtime). The production
/// implementation lives in `rcode-harness`, next to the model client it needs.
#[async_trait]
pub trait SubagentRuntime: Send + Sync {
    /// The definitions the model may name at `cwd` — enabled ones only, with
    /// the project's copy shadowing the person's of the same name.
    fn definitions(&self, cwd: &std::path::Path) -> Vec<SubagentDefinition>;

    /// Run one nested agent to completion, streaming its events through
    /// `spawn.events`, and return its final message.
    ///
    /// An `Err` is a failure the parent model should see as the tool's error
    /// result (spawn refused, the child failed, the run was cancelled).
    async fn run(&self, spawn: SubagentSpawn) -> Result<SubagentOutcome, String>;
}

/// Where a nested run's events go.
///
/// The loop hands one to every tool call, bound to that call's id and the
/// run's event channel. A tool sends its child's events; the loop tags them
/// [`crate::AgentEvent::Nested`] on the way out. Cloning a sender keeps the
/// same destination, so a runtime can hand one to a spawned task.
#[derive(Clone)]
pub struct NestedEventSender {
    parent_tool_use_id: std::sync::Arc<String>,
    tx: tokio::sync::mpsc::Sender<Result<crate::AgentEvent, crate::AgentError>>,
}

impl std::fmt::Debug for NestedEventSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NestedEventSender")
            .field("parent_tool_use_id", &self.parent_tool_use_id)
            .finish_non_exhaustive()
    }
}

impl NestedEventSender {
    pub fn with_parent_tool_use_id(&self, id: String) -> Self { Self { parent_tool_use_id: Arc::new(id), tx: self.tx.clone() } }

    /// A sender bound to the tool call `parent_tool_use_id`, forwarding into
    /// the run's event channel.
    pub fn new(
        parent_tool_use_id: String,
        tx: tokio::sync::mpsc::Sender<Result<crate::AgentEvent, crate::AgentError>>,
    ) -> Self {
        Self {
            parent_tool_use_id: std::sync::Arc::new(parent_tool_use_id),
            tx,
        }
    }

    /// A sender whose events go nowhere — for a caller that built a Agent tool
    /// outside a live run (a test, a listing) and still owes the runtime a
    /// sink.
    pub fn null() -> Self {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        Self::new(String::new(), tx)
    }

    /// The tool call the events are attributed to.
    pub fn parent_tool_use_id(&self) -> &str {
        &self.parent_tool_use_id
    }

    /// Forward one child event. A send that cannot land (the parent run ended
    /// and dropped the channel) is dropped with it: a transcript nobody
    /// reads is not a failure of the work.
    pub async fn send(&self, event: crate::AgentEvent) {
        let _ = self
            .tx
            .send(Ok(crate::AgentEvent::Nested {
                parent_tool_use_id: (*self.parent_tool_use_id).clone(),
                event: Box::new(event),
            }))
            .await;
    }
}

pub struct AgentTool {
    runtime: std::sync::Arc<dyn SubagentRuntime>,
}

impl AgentTool {
    pub fn new(runtime: std::sync::Arc<dyn SubagentRuntime>) -> Self {
        Self { runtime }
    }

    /// The definition this call names, or the default one.
    ///
    /// A missing name is not an error: "delegate this without a specialist"
    /// is a normal request, and it resolves to the general-purpose agent — a
    /// definition explicitly named `general-purpose` if the user wrote one,
/// else the parent's own tool set (minus Task) under the original role prompt.
    pub(crate) fn resolve(
        &self,
        cwd: &std::path::Path,
        name: Option<&str>,
    ) -> Result<SubagentDefinition, ToolError> {
        let definitions = self.runtime.definitions(cwd);
        let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
            return Ok(definitions
                .iter()
                .find(|definition| definition.name == GENERAL_PURPOSE)
                .cloned()
                .unwrap_or_else(general_purpose));
        };
        let exact = definitions.iter().find(|definition| definition.name == name);
        let mut matching = definitions.iter().filter(|definition| definition.name.rsplit_once(':').is_some_and(|(_, suffix)| suffix == name));
        let unique = matching.next().filter(|_| matching.next().is_none());
        exact.or(unique)
            .cloned()
            .ok_or_else(|| {
                // Name what *is* available, like the skill tool: a model that
                // guessed wrongly should not spend another call finding out.
                let available = if definitions.is_empty() {
                    "no subagents are defined (the general-purpose agent is always available)"
                        .to_owned()
                } else {
                    format!(
                        "available: {} (or omit the name for the general-purpose agent)",
                        definitions
                            .iter()
                            .map(|definition| definition.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                ToolError::NotFound(format!("no subagent named `{name}` ({available})"))
            })
    }
}

/// The fallback definition: everything the parent has except delegation
/// itself, on the parent's model.
///
/// Share the complete original role text with the built-in catalog so the
/// fallback cannot silently revert to a shortened version.
fn general_purpose() -> SubagentDefinition {
    SubagentDefinition {
        name: GENERAL_PURPOSE.to_owned(),
        description: crate::builtin_subagent_prompts::GENERAL_PURPOSE_DESCRIPTION.to_owned(),
        tools: Vec::new(),
        disallowed_tools: vec![AGENT.to_owned()],
        model: ModelAlias::Inherit,
        system_prompt: crate::builtin_subagent_prompts::GENERAL_PURPOSE_PROMPT.to_owned(),
        path: std::path::PathBuf::new(),
        source: crate::skills::SkillSource::Project,
    }
}

#[async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        AGENT
    }

    fn description(&self) -> &str {
        "Hand a self-contained task to a named subagent — a separate agent with its own \
         context window that works in this directory and reports back its result. Use it \
         for work that would flood this conversation with intermediate output \
         (broad searches, bulk reads, an independent review). Discover the available \
         subagents with their descriptions before naming one; without a name the \
         general-purpose agent runs. Built-in types: general-purpose, Explore."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "A one-line label for the task, shown while the subagent works."
                },
                "prompt": {
                    "type": "string",
                    "description": "The complete task. The subagent sees nothing else from this conversation, so include every file, constraint and expected shape the result must satisfy."
                },
                "subagent_type": {
                    "type": "string",
                    "description": "Which subagent to run. Omit for the general-purpose agent."
                }
            },
            "required": ["prompt"]
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let prompt = input
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|prompt| !prompt.is_empty())
            .ok_or_else(|| ToolError::InvalidInput("`prompt` is required".to_owned()))?;
        let name = input
            .get("subagent_type")
            .or_else(|| input.get("subagentType"))
            .or_else(|| input.get("name"))
            .and_then(Value::as_str);
        let definition = self.resolve(&ctx.cwd, name)?;
        let description = input
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();

        let events = ctx.nested.clone().unwrap_or_else(NestedEventSender::null);
        let agent_name = definition.name.clone();
        let spawn = SubagentSpawn {
            history_sink: None,
            history: Vec::new(),
            inbox: None,
            definition,
            prompt: prompt.to_owned(),
            cwd: ctx.cwd.clone(),
            events,
            // Under the run's token, so stopping the run stops this child too;
            // separate from it, so this child can be stopped on its own.
            cancel: ctx.cancel.child_token(),
            tool_use_id: ctx
                .nested
                .as_ref()
                .map(|nested| nested.parent_tool_use_id().to_owned())
                .unwrap_or_default(),
        };
        let outcome = match self.runtime.run(spawn).await {
            Ok(outcome) => outcome,
            Err(message) => return Ok(ToolResult::error(message)),
        };
        // The parent model reads only this. Naming the agent (and the task,
        // when the model gave one) keeps a transcript with several
        // delegations readable without opening each nested transcript.
        let label = (!description.is_empty())
            .then(|| format!(" ({description})"))
            .unwrap_or_default();
        Ok(ToolResult::ok(format!(
            "Subagent `{}`{label} finished:\n\n{}",
            agent_name, outcome.output
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A runtime that records what it was asked to spawn and answers with a
    /// scripted outcome.
    struct ScriptedRuntime {
        definitions: Vec<SubagentDefinition>,
        spawns: Mutex<Vec<SubagentSpawn>>,
        outcome: Result<String, String>,
    }

    impl ScriptedRuntime {
        fn definitions(definitions: Vec<SubagentDefinition>) -> std::sync::Arc<Self> {
            std::sync::Arc::new(Self {
                definitions,
                spawns: Mutex::new(Vec::new()),
                outcome: Ok("done".to_owned()),
            })
        }
    }

    #[async_trait]
    impl SubagentRuntime for ScriptedRuntime {
        fn definitions(&self, _cwd: &std::path::Path) -> Vec<SubagentDefinition> {
            self.definitions.clone()
        }

        async fn run(&self, spawn: SubagentSpawn) -> Result<SubagentOutcome, String> {
            self.spawns.lock().unwrap().push(spawn);
            match &self.outcome {
                Ok(text) => Ok(SubagentOutcome {
                    history: Vec::new(),
                    output: text.clone(),
                }),
                Err(message) => Err(message.clone()),
            }
        }
    }

    fn definition(name: &str) -> SubagentDefinition {
        SubagentDefinition {
            name: name.to_owned(),
            description: format!("{name} does things."),
            tools: Vec::new(),
            disallowed_tools: vec![AGENT.to_owned()],
            model: ModelAlias::Inherit,
            system_prompt: format!("You are {name}."),
            path: std::path::PathBuf::from(format!("/cfg/agents/{name}.md")),
            source: crate::skills::SkillSource::Project,
        }
    }

    fn cwd() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    fn context(cwd: std::path::PathBuf) -> ToolContext {
        ToolContext {
            cwd,
            cancel: CancellationToken::new(),
            nested: Some(NestedEventSender::null()),
        }
    }

    #[tokio::test]
    async fn a_named_subagent_resolves_and_spawns() {
        let runtime = ScriptedRuntime::definitions(vec![definition("reviewer")]);
        let tool = AgentTool::new(runtime.clone());

        let result = tool
            .call(
                json!({
                    "prompt": "review src/",
                    "subagent_type": "reviewer",
                    "description": "the review"
                }),
                &context(cwd()),
            )
            .await
            .expect("runs");

        assert!(!result.is_error);
        assert!(result.output.contains("done"), "{}", result.output);
        assert!(result.output.contains("reviewer"), "{}", result.output);
        let spawns = runtime.spawns.lock().unwrap();
        assert_eq!(spawns.len(), 1);
        assert_eq!(spawns[0].definition.name, "reviewer");
        assert_eq!(spawns[0].prompt, "review src/");
        assert_eq!(spawns[0].definition.system_prompt, "You are reviewer.");
    }

    #[tokio::test]
    async fn no_name_resolves_to_general_purpose() {
        let runtime = ScriptedRuntime::definitions(vec![]);
        let tool = AgentTool::new(runtime.clone());

        tool.call(json!({ "prompt": "go" }), &context(cwd()))
            .await
            .expect("runs");

        let spawns = runtime.spawns.lock().unwrap();
        assert_eq!(spawns[0].definition.name, GENERAL_PURPOSE);
        assert_eq!(spawns[0].definition.system_prompt, crate::builtin_subagent_prompts::GENERAL_PURPOSE_PROMPT);
        assert_eq!(spawns[0].definition.description, crate::builtin_subagent_prompts::GENERAL_PURPOSE_DESCRIPTION);
        // The default child must never carry the Agent tool, or a subagent
        // could spawn a subagent.
        assert!(!spawns[0].definition.allows(AGENT));
    }

    #[tokio::test]
    async fn an_explicit_general_purpose_definition_wins_over_the_default() {
        let mut user_written = definition(GENERAL_PURPOSE);
        user_written.system_prompt = "The user's own.".to_owned();
        let runtime = ScriptedRuntime::definitions(vec![user_written]);
        let tool = AgentTool::new(runtime.clone());

        tool.call(json!({ "prompt": "go" }), &context(cwd()))
            .await
            .expect("runs");

        let spawns = runtime.spawns.lock().unwrap();
        assert_eq!(spawns[0].definition.system_prompt, "The user's own.");
    }

    #[tokio::test]
    async fn an_unknown_name_lists_what_is_available() {
        let runtime = ScriptedRuntime::definitions(vec![definition("reviewer")]);
        let tool = AgentTool::new(runtime);

        let error = tool
            .call(
                json!({ "prompt": "go", "subagent_type": "nope" }),
                &context(cwd()),
            )
            .await
            .expect_err("unknown");

        assert!(matches!(error, ToolError::NotFound(_)));
        let message = error.to_string();
        assert!(message.contains("reviewer"), "{message}");
    }

    #[tokio::test]
    async fn a_missing_prompt_is_refused() {
        let runtime = ScriptedRuntime::definitions(vec![]);
        let tool = AgentTool::new(runtime);
        let error = tool
            .call(json!({}), &context(cwd()))
            .await
            .expect_err("no prompt");
        assert!(matches!(error, ToolError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn a_runtime_failure_becomes_an_error_result() {
        let runtime = std::sync::Arc::new(ScriptedRuntime {
            definitions: vec![],
            spawns: Mutex::new(Vec::new()),
            outcome: Err("the subagent failed".to_owned()),
        });
        let tool = AgentTool::new(runtime);

        let result = tool
            .call(json!({ "prompt": "go" }), &context(cwd()))
            .await
            .expect("a failed child is a result, not a tool error");
        assert!(result.is_error);
        assert!(result.output.contains("the subagent failed"));
    }

    #[tokio::test]
    async fn events_are_tagged_with_the_spawning_call() {
        // The tag is the whole routing story: a host reads it to decide which
        // nested transcript an event belongs to.
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let sender = NestedEventSender::new("toolu_1".to_owned(), tx);
        sender
            .send(crate::AgentEvent::TextDelta { text: "hi".into() })
            .await;

        match rx.recv().await {
            Some(Ok(crate::AgentEvent::Nested {
                parent_tool_use_id,
                event,
            })) => {
                assert_eq!(parent_tool_use_id, "toolu_1");
                assert!(matches!(*event, crate::AgentEvent::TextDelta { .. }));
            }
            other => panic!("expected a tagged nested event, got {other:?}"),
        }
    }
}
