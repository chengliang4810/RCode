//! The native subagent runtime — the host side of the `Agent` tool.
//!
//! [`rcode_agent::tools::AgentTool`] owns the contract (input, definition
//! resolution, the nested-event sink); this module owns the production
//! [`rcode_agent::tools::SubagentRuntime`]: it resolves the child's model
//! against the provider file, filters the child's tool registry from the
//! parent's, drives a child [`rcode_agent::AgentRunner`] under the
//! definition's system prompt, and forwards the child's events to the parent
//! stream tagged with the spawning tool call.
//!
//! Deliberate limits, both structural:
//!
//! - **Depth 1.** The child's registry is derived from the parent's *minus*
//!   the `Agent` tool, and the filter refuses the name even if a definition's
//!   allowlist asks for it. A subagent cannot spawn a subagent, so delegation
//!   trees cannot grow.
//! - **Concurrency 8** ([`MAX_ACTIVE_SUBAGENTS`]). One semaphore shared by
//!   every run of this harness bounds how many children exist at once; a
//!   spawn beyond the cap waits for a permit, honouring cancellation while it
//!   waits.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;

use rcode_agent::subagents::{self, ModelAlias, SubagentDefinition};
use rcode_agent::tools::{SubagentOutcome, SubagentRuntime, SubagentSpawn};
#[cfg(test)]
use rcode_agent::tools::AGENT;
use rcode_agent::{
    AgentConfig, AgentEvent as AgentLoopEvent, AgentRunner, DoneReason, ToolRegistry,
};
use rcode_proto::invocation::{SkillScope, SubagentDefinition as WireSubagentDefinition};

use crate::providers::ProvidersConfig;
use crate::settings::AgentSettings;

/// How many subagents may run at once, across every conversation of this
/// harness. A child waits for a permit (cancellably) rather than failing:
/// the cap exists to bound fan-out, not to refuse work.
pub const BUILTIN_PREFIX: &str = "builtin:";
pub(crate) const MAX_ACTIVE_SUBAGENTS: usize = 8;

/// The parent-side inputs one run's `Agent` tool is built from.
pub(crate) struct SubagentDeps {
    pub managed: Option<Arc<rcode_agent::agent_tasks::AgentTaskStore>>,
    pub goal: Option<Arc<dyn rcode_agent::goal::GoalStore>>,
    /// The providers snapshot this run resolved its own model from.
    pub providers: ProvidersConfig,
    /// Where `agent-settings.json` lives, if this run has one.
    pub data_dir: Option<PathBuf>,
    /// The settings snapshot this run's capability gates used — the same
    /// read the harness listing and the RPC toggles write back to, so the
    /// tool's view of "which definitions are on" cannot drift from the page's.
    pub settings: AgentSettings,
    /// The run's interaction bridge. Child questions reach the same user.
    pub request_input: Option<
        Arc<
            dyn Fn(
                    rcode_proto::InteractionRequest,
                )
                    -> tokio::sync::oneshot::Receiver<rcode_proto::InteractionResponse>
                + Send
                + Sync,
        >,
    >,
    /// The model the parent conversation is talking to — what `inherit`
    /// resolves to.
    pub parent_model: String,
    pub parent_reasoning: Option<rcode_proto::ReasoningLevel>,
    pub user_instructions: Arc<std::sync::OnceLock<String>>,
    /// The parent registry **without** `Agent`: the widest set a child can
    /// receive. Cloned per spawn, then filtered by the definition.
    pub base_tools: Arc<ToolRegistry>,
    /// Shared across every run of this harness.
    pub limits: Arc<tokio::sync::Semaphore>,
    /// The runs currently alive, keyed by the tool call that spawned them.
    ///
    /// Shared by every run and every conversation of the harness: a stop names
    /// a tool call id, and the id is unique within its conversation. The token
    /// is the spawn's own, so cancelling one entry stops exactly one child.
    pub active: Arc<ActiveSubagents>,
}

/// Every running subagent, and its cancellation token.
#[derive(Default)]
pub struct ActiveSubagents {
    runs: std::sync::Mutex<std::collections::HashMap<String, tokio_util::sync::CancellationToken>>,
}

impl ActiveSubagents {
    /// Note a spawn as running. A reused tool-call id (a resumed run replaying
    /// its history) replaces the old entry: the newer run is the live one.
    fn register(&self, key: String, cancel: tokio_util::sync::CancellationToken) {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, cancel);
    }

    fn forget(&self, key: &str) {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(key);
    }

    /// Stop one running subagent. `true` when there was one to stop.
    pub fn cancel(&self, key: &str) -> bool {
        let token = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .cloned();
        match token {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// Whether a subagent is running under this id.
    pub fn is_running(&self, key: &str) -> bool {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(key)
    }
}

/// Forget a spawn however the run ends — success, failure, cancellation, or a
/// panic. A `Drop` guard rather than a call at the end of `run`: the error
/// paths are the ones that would otherwise leave a stale entry and make a dead
/// subagent look stoppable.
struct ActiveGuard {
    active: Arc<ActiveSubagents>,
    key: String,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.active.forget(&self.key);
    }
}

/// The production runtime behind one run's `Agent` tool.
pub(crate) struct NativeSubagents {
    deps: SubagentDeps,
}

/// The `Agent` tool wired to this run's runtime.
pub(crate) fn agent_tool(deps: SubagentDeps) -> rcode_agent::tools::AgentTool {
    rcode_agent::tools::AgentTool::new(Arc::new(NativeSubagents { deps }))
}

pub(crate) fn managed_tools(deps: SubagentDeps, store: Arc<rcode_agent::agent_tasks::AgentTaskStore>) -> Arc<rcode_agent::agent_tasks::AgentTasks> {
    Arc::new(rcode_agent::agent_tasks::AgentTasks::new(store, Arc::new(NativeSubagents { deps })))
}

/// The agents root one scope's writes target.
///
/// A scope names one of the two managed roots, never a path — mirroring
/// `skills_root_for` in `native.rs`, so an RPC request cannot aim a write
/// outside the directories the scan reads.
pub(crate) fn agents_root_for(cwd: &Path, scope: SkillScope) -> Result<PathBuf, String> {
    let home = rcode_agent::skills::home_dir();
    subagents::scope_root(cwd, scope.into(), home.as_deref()).ok_or_else(|| {
        "no home directory is known, so there is no user agents directory".to_owned()
    })
}

/// The settings key for a definition: its path, normalized.
///
/// Lexical, not canonical: the path is rebuilt from its components, which
/// folds the one spelling difference this module can produce on its own —
/// the scanner joins the `agents` root through a single `.rcode/agents`
/// component while a write target reaches the same directory as two joins,
/// and on Windows those render with different separators. Both normalize to
/// one string, so a toggle keyed by a listed path matches what the runtime
/// computes from a rescan, with no filesystem round trip and no
/// `\\?\`-style extended path leaking into the settings file.
pub(crate) fn definition_key(path: &Path) -> String {
    path.components()
        .collect::<std::path::PathBuf>()
        .to_string_lossy()
        .into_owned()
}

/// The settings' model override for a definition, folded over the file's own
/// alias: an override wins until it says `inherit`, which is how an override
/// is removed in place.
fn effective_model(definition: &SubagentDefinition, settings: &AgentSettings) -> Option<String> {
    let key = definition_key(&definition.path);
    let model = settings
        .subagent_models
        .get(&key)
        .and_then(|value| ModelAlias::parse(value))
        .unwrap_or_else(|| definition.model.clone());
    match model {
        ModelAlias::Inherit => None,
        model => Some(model.as_str().to_owned()),
    }
}

/// One definition as the wire sees it, with the settings' enable flag and
/// model override applied.
pub(crate) fn to_wire(
    definition: SubagentDefinition,
    settings: &AgentSettings,
) -> WireSubagentDefinition {
    let path = definition_key(&definition.path);
    let enabled = path.starts_with(BUILTIN_PREFIX) || !settings.disabled_subagents.contains(&path);
    let model = effective_model(&definition, settings);
    WireSubagentDefinition {
        name: definition.name,
        path: path.clone(),
        description: match path.as_str() {
            "builtin:general-purpose" => rcode_agent::builtin_subagent_prompts::GENERAL_PURPOSE_SUMMARY.into(),
            "builtin:Explore" => rcode_agent::builtin_subagent_prompts::EXPLORE_SUMMARY.into(),
            _ => definition.description,
        },
        enabled,
        tools: definition.tools,
        disallowed_tools: definition.disallowed_tools,
        model,
        scope: if path.starts_with(BUILTIN_PREFIX) {
            None
        } else {
            Some(definition.source.into())
        },
        system_prompt: Some(definition.system_prompt),
    }
}

/// Every definition visible at `cwd`, as the wire sees it — the same scan the
/// Agent tool's `definitions()` reads, with `settings`' flags applied, so the
/// settings page and the model cannot disagree about what is on.
pub(crate) async fn list(cwd: &Path, settings: &AgentSettings) -> Vec<WireSubagentDefinition> {
    subagents::discover(cwd)
        .into_iter()
        .map(|definition| to_wire(definition, settings))
        .collect()
}

/// Create a definition in `root` (already resolved from `scope`), refusing a
/// taken name.
pub(crate) fn create(
    root: &Path,
    name: &str,
    description: &str,
    system_prompt: &str,
    tools: &[String],
    disallowed_tools: &[String],
    model: ModelAlias,
) -> Result<(), String> {
    subagents::create(
        root,
        name,
        description,
        system_prompt,
        tools,
        disallowed_tools,
        model,
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

/// Delete the definition of `name` that lives in `root`'s scope.
pub(crate) fn delete(root: &Path, scope: SkillScope, name: &str) -> Result<(), String> {
    let found = subagents::discover_in(root, scope.into());
    let Some(definition) = found.iter().find(|definition| definition.name == name) else {
        return Err(format!(
            "no {} subagent named `{name}` was found to delete",
            scope.label()
        ));
    };
    subagents::delete(&definition.path, root).map_err(|error| error.to_string())
}

/// Rewrite the definition of `name` in `root`'s scope. `None` keeps the body.
pub(crate) fn update(
    root: &Path,
    scope: SkillScope,
    name: &str,
    new_name: &str,
    description: &str,
    system_prompt: Option<&str>,
    tools: &[String],
    disallowed_tools: &[String],
    model: ModelAlias,
) -> Result<(), String> {
    let found = subagents::discover_in(root, scope.into());
    let Some(definition) = found.iter().find(|definition| definition.name == name) else {
        return Err(format!(
            "no {} subagent named `{name}` was found to change",
            scope.label()
        ));
    };
    // The file keeps its name on disk; the frontmatter's `name` is the
    // identity that changes, exactly as a skill rename keeps its directory.
    subagents::update(
        &definition.path,
        new_name,
        description,
        tools,
        disallowed_tools,
        model,
        system_prompt,
    )
    .map_err(|error| error.to_string())
}

/// Read a `model:` alias from a request, refusing the unknown.
///
/// `inherit` is the spelling for "no override" — it parses, so a caller can
/// clear an override with the same field that sets one. Public because the
/// engine's RPC handlers validate before persisting: an alias a typo broke
/// must be refused at the toggle, not discovered as a silent fallback at run
/// time.
pub fn parse_model_alias(raw: &str) -> Result<ModelAlias, String> {
    ModelAlias::parse(raw).ok_or_else(|| {
        format!("`{raw}` is not a subagent model (want inherit or a provider/model selection)")
    })
}

impl NativeSubagents {
    /// The definitions this run may delegate to: enabled ones, with the
    /// settings' model override applied.
    fn live_definitions(&self, cwd: &Path) -> Vec<SubagentDefinition> {
        let plugin_roots = self
            .deps
            .data_dir
            .as_deref()
            .map(rcode_plugins::agent_roots)
            .unwrap_or_default();
        crate::builtin_subagents::with_builtins(subagents::discover_with_plugins(
            cwd,
            &plugin_roots,
        ))
        .into_iter()
        .filter(|definition| {
            let enabled = definition
                .path
                .to_string_lossy()
                .starts_with(BUILTIN_PREFIX)
                || !self
                    .deps
                    .settings
                    .disabled_subagents
                    .contains(&definition_key(&definition.path));
            if !enabled {
                tracing::debug!(agent = %definition.name, "subagent is switched off");
            }
            enabled
        })
        .map(|mut definition| {
            if let Some(model) = self
                .deps
                .settings
                .subagent_models
                .get(&definition_key(&definition.path))
                && let Some(choice) = ModelAlias::parse(model)
            {
                definition.model = choice;
            }
            definition
        })
        .collect()
    }

    /// The child's registry: the parent's, filtered through the definition,
    /// with delegation itself always removed.
    fn child_tools(&self, definition: &SubagentDefinition) -> ToolRegistry {
        let mut child = ToolRegistry::new();
        for tool in self.deps.base_tools.all() {
            if matches!(tool.name(), "Agent" | "TaskList" | "TaskOutput" | "TaskStop" | "TaskBackground" | "SendMessage") {
                // The base registry never carries it; refusing the name here
                // is what keeps `tools: [Agent]` in a definition from becoming
                // recursion if that ever changes.
                continue;
            }
            if definition.allows(tool.name()) {
                child.register(tool);
            }
        }
        child
    }

    fn selected_model<'a>(
        &self,
        definition: &SubagentDefinition,
        providers: &'a ProvidersConfig,
    ) -> Result<
        (
            &'a crate::providers::ProviderConfig,
            &'a crate::providers::ModelConfig,
        ),
        String,
    > {
        if let Some(selection) = crate::providers::ModelSelection::parse(definition.model.as_str()) {
            let (provider, model) = selection.find(providers).ok_or_else(|| format!("subagent model `{}` from provider `{}` is no longer available", selection.model_id, selection.provider_id))?;
            let environment = std::env::vars().collect();
            if provider.resolve_api_key(&environment).is_none() {
                return Err(format!("provider `{}` has no usable API key", provider.id));
            }
            return Ok((provider, model));
        }
        providers
            .diagnose(&self.deps.parent_model, &std::env::vars().collect())
            .map_err(|error| error.to_string())
    }

    fn selected_reasoning(&self, definition: &SubagentDefinition, model: &crate::providers::ModelConfig) -> Option<rcode_proto::ReasoningLevel> {
        let level = match crate::providers::ModelSelection::parse(definition.model.as_str()) {
            Some(selection) => selection.reasoning,
            None => self.deps.parent_reasoning,
        };
        level.filter(|level| model.reasoning_levels.contains(level))
    }

    fn child_prompt(&self, definition: &SubagentDefinition, cwd: &Path, tools: &ToolRegistry) -> String {
        let mut parts = vec![definition.system_prompt.clone()];
        // ZCode 的 Explore 不注入 AGENTS.md；同名自定义角色仍遵循自定义规则。
        if definition.path != Path::new("builtin:Explore") && let Some(instructions) = self.deps.user_instructions.get().filter(|text| !text.is_empty()) {
            parts.push(instructions.clone());
        }
        parts.push(format!("Working directory: {}\nPlatform: {}\nShell: {}", cwd.display(), std::env::consts::OS,
            rcode_agent::tools::command_shell_program()));
        if let Some(instructions) = crate::prompts::platform_shell_instructions(tools) {
            parts.push(instructions);
        }
        parts.join("\n\n")
    }
}

#[async_trait::async_trait]
impl SubagentRuntime for NativeSubagents {
    fn definitions(&self, cwd: &Path) -> Vec<SubagentDefinition> {
        self.live_definitions(cwd)
    }

    async fn run(&self, spawn: SubagentSpawn) -> Result<SubagentOutcome, String> {
        let _active = (!spawn.tool_use_id.is_empty()).then(|| {
            let key = spawn.tool_use_id.clone();
            self.deps.active.register(key.clone(), spawn.cancel.clone());
            ActiveGuard {
                active: Arc::clone(&self.deps.active),
                key,
            }
        });
        if spawn.cancel.is_cancelled() {
            return Err("the run was cancelled before the subagent started".to_owned());
        }
        // The concurrency cap. A spawn beyond it waits; waiting itself
        // honours cancellation, so stopping the parent cannot deadlock
        // behind a full cap.
        let _permit = tokio::select! {
            biased;
            _ = spawn.cancel.cancelled() => {
                return Err("the run was cancelled while the subagent waited for a slot".to_owned());
            }
            permit = self.deps.limits.clone().acquire_owned() => permit
                .map_err(|_| "the subagent runtime is shutting down".to_owned())?,
        };

        let definition = spawn.definition;
        let current_providers = self
            .deps
            .data_dir
            .as_deref()
            .and_then(|root| ProvidersConfig::load(root).ok())
            .unwrap_or_else(|| self.deps.providers.clone());
        let (provider, model_config) = self.selected_model(&definition, &current_providers)?;
        let model = model_config.id.clone();
        let provider = provider.clone();
        let model_config = model_config.clone();

        // One child token under the parent's: an interrupt of the parent run
        // (or a drop of its stream, through the drop guard's cancellation)
        // reaches the child's HTTP request and open tools.
        let run_cancel = spawn.cancel.child_token();
        let request_input = self.deps.request_input.clone();
        let host = super::native::NativeHost::for_subagent(
            model.clone(),
            provider.clone(),
            model_config.clone(),
            self.selected_reasoning(&definition, &model_config),
            run_cancel.clone(),
            Box::new(move |request| match &request_input {
                Some(bridge) => bridge(request),
                // A run built without a bridge (a test, a bare harness) has
                // nobody to ask; refusing every sensitive call is the safe
                // reading, and a question goes unanswered the way a stopped
                // run's does.
                None => {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    let _ = tx.send(rcode_proto::InteractionResponse::Dismissed);
                    rx
                }
            }),
        );
        let host = Arc::new(host);

        // The child's registry is built before the config consumes the
        // definition's prompt — both read the definition, and a partial
        // move would make the second read a compile error.
        let tools = Arc::new(self.child_tools(&definition));
        let config = AgentConfig {
            model,
            cwd: spawn.cwd.clone(),
            max_steps: None,
            system_prompt: Some(self.child_prompt(&definition, &spawn.cwd, &tools)),
            history: spawn.history.iter().filter(|message| message.role != rcode_agent::MessageRole::System).cloned().collect(),
            images: Vec::new(),
            compact: model_config
                .context_window
                .map(rcode_agent::CompactConfig::for_window),
        };
        let history_sink = spawn.history_sink.clone().unwrap_or_default();
        let runner = Arc::new(AgentRunner::new(host, tools, config).with_inbox(spawn.inbox.clone()).with_history_sink(history_sink.clone()).with_hooks(
            crate::hooks::executor_for(self.deps.data_dir.as_deref(), &spawn.cwd),
        ));

        // Drive the child, streaming its events to the parent and collecting
        // its final message. `Started` and `InputRequested` are the two loop
        // events a nested transcript does not carry: the run already announced
        // itself, and the ask bridge raises the user-facing event itself.
        let goal_id = self.deps.goal.as_ref().map(|store| store.read()).transpose()?
            .flatten().filter(|goal| goal.status == rcode_proto::goal::GoalStatus::Active).map(|goal| goal.id);
        let mut stream = runner.run(spawn.prompt, run_cancel.clone());
        let mut final_text = String::new();
        let mut reason: Option<DoneReason> = None;
        while let Some(item) = stream.next().await {
            match item {
                Ok(event) => match event {
                    AgentLoopEvent::Started { .. } | AgentLoopEvent::InputRequested { .. } => {}
                    AgentLoopEvent::Done { reason: done } => reason = Some(done),
                    AgentLoopEvent::TextDelta { text } => {
                        final_text.push_str(&text);
                        spawn.events.send(AgentLoopEvent::TextDelta { text }).await;
                    }
                    other => {
                        if let AgentLoopEvent::ModelCall { statistics, .. } = &other {
                            if let Some((store, id)) = self.deps.goal.as_ref().zip(goal_id.as_ref()) {
                                store.change(id, rcode_agent::goal::GoalChange::Usage {
                                    tokens: statistics.input_tokens.unwrap_or(0).saturating_add(statistics.output_tokens.unwrap_or(0)), elapsed_ms: 0,
                                })?;
                            }
                        }
                        spawn.events.send(other).await;
                    }
                },
                // The loop converts every ordinary failure into `Done`; an
                // item-level error is the host failing out of band, which ends
                // the child as a failure the parent model can read.
                Err(error) => {
                    return Err(format!("the subagent failed: {error}"));
                }
            }
        }

        match reason {
            Some(DoneReason::Complete) => Ok(SubagentOutcome {
                history: history_sink.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone(),
                output: if final_text.trim().is_empty() {
                    "The subagent finished without a final message.".to_owned()
                } else {
                    final_text
                },
            }),
            Some(DoneReason::Interrupted) => {
                Err("the subagent was interrupted before it finished".to_owned())
            }
            Some(DoneReason::Failed(message)) => Err(format!("the subagent failed: {message}")),
            None => Err("the subagent stream ended without a result".to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcode_agent::skills::SkillSource as DefinitionSource;
    include!("native_subagents_zcode_tests.rs");

    fn providers() -> ProvidersConfig {
        ProvidersConfig {
            providers: vec![crate::providers::ProviderConfig {
                id: "anthropic".into(),
                label: None,
                protocol: crate::providers::Protocol::AnthropicMessages,
                base_url: None,
                api_key: None,
                models: vec![
                    crate::providers::ModelConfig {
                        id: "claude-opus-4".into(),
                        model: "claude-opus-4".into(),
                        label: None,
                        context_window: None,
                        max_tokens: None,
                        input_format: Default::default(),
                        reasoning_levels: Vec::new(),
                    },
                    crate::providers::ModelConfig {
                        id: "claude-sonnet-4-5".into(),
                        model: "claude-sonnet-4-5".into(),
                        label: None,
                        context_window: Some(200_000),
                        max_tokens: None,
                        input_format: Default::default(),
                        reasoning_levels: Vec::new(),
                    },
                    crate::providers::ModelConfig {
                        id: "mock-fable-5".into(),
                        model: "mock-fable-5".into(),
                        label: Some("Fable 5".into()),
                        context_window: None,
                        max_tokens: None,
                        input_format: Default::default(),
                        reasoning_levels: Vec::new(),
                    },
                ],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn explicit_models_keep_provider_identity_and_deleted_choices_fail() {
        let mut providers = providers();
        providers.providers[0].api_key = Some("test-key".into());
        let runtime = NativeSubagents {
            deps: SubagentDeps {
                managed: None,
                goal: None,
                providers: providers.clone(),
                data_dir: None,
                settings: AgentSettings::default(),
                request_input: None,
                parent_model: "claude-opus-4".into(),
                parent_reasoning: None,
                user_instructions: Default::default(),
                base_tools: Arc::new(ToolRegistry::new()),
                limits: Arc::new(tokio::sync::Semaphore::new(MAX_ACTIVE_SUBAGENTS)),
                active: Arc::new(ActiveSubagents::default()),
            },
        };
        let mut definition = SubagentDefinition {
            name: "reviewer".into(),
            description: String::new(),
            tools: vec![],
            disallowed_tools: vec![],
            model: ModelAlias::Inherit,
            system_prompt: String::new(),
            path: PathBuf::from("reviewer.md"),
            source: DefinitionSource::User,
        };
        let explicit = crate::providers::ModelSelection {
            provider_id: "anthropic".into(),
            model_id: "claude-sonnet-4-5".into(),
            reasoning: None,
        }
        .selection();
        definition.model = ModelAlias::parse(&explicit).unwrap();
        assert_eq!(
            runtime
                .selected_model(&definition, &providers)
                .unwrap()
                .1
                .id,
            "claude-sonnet-4-5"
        );
        for selection in [
            "haiku".to_owned(),
            "fable".to_owned(),
            crate::providers::ModelSelection {
                provider_id: "deleted-provider".into(),
                model_id: "claude-sonnet-4-5".into(),
            reasoning: None,
            }
            .selection(),
            crate::providers::ModelSelection {
                provider_id: "anthropic".into(),
                model_id: "deleted-model".into(),
            reasoning: None,
            }
            .selection(),
        ] {
            definition.model = ModelAlias::parse(&selection).unwrap();
            if crate::providers::ModelSelection::parse(&selection).is_some() {
                assert!(runtime.selected_model(&definition, &providers).err().unwrap().contains("no longer available"));
                continue;
            }
            assert_eq!(
                runtime
                    .selected_model(&definition, &providers)
                    .unwrap()
                    .1
                    .id,
                "claude-opus-4"
            );
        }
        let mut settings = AgentSettings::default();
        settings
            .subagent_models
            .insert(definition_key(&definition.path), explicit.clone());
        assert_eq!(
            to_wire(definition.clone(), &settings).model.as_deref(),
            Some(explicit.as_str())
        );
        settings
            .subagent_models
            .insert(definition_key(&definition.path), "sonnet".into());
        assert!(to_wire(definition, &settings).model.is_none());
    }

    #[test]
    fn the_wire_shape_carries_the_settings_flags() {
        let mut settings = AgentSettings::default();
        let definition = SubagentDefinition {
            name: "reviewer".into(),
            description: "Reviews.".into(),
            tools: vec!["Read".into()],
            disallowed_tools: vec!["Bash".into()],
            model: ModelAlias::Inherit,
            system_prompt: "You review.".into(),
            path: PathBuf::from("/cfg/agents/reviewer.md"),
            source: DefinitionSource::User,
        };
        let wire = to_wire(definition.clone(), &settings);
        assert!(wire.enabled);
        assert_eq!(wire.model.as_deref(), None);
        assert_eq!(wire.scope, Some(SkillScope::User));
        assert_eq!(wire.system_prompt.as_deref(), Some("You review."));

        // Switched off by path, and overridden to another alias. The keys go
        // through `definition_key` because that is what a toggle stores: the
        // listing's own spelling.
        let key = definition_key(&PathBuf::from("/cfg/agents/reviewer.md"));
        settings.disabled_subagents.insert(key.clone());
        settings.subagent_models.insert(key, "haiku".into());
        let wire = to_wire(definition, &settings);
        assert!(!wire.enabled);
        assert_eq!(wire.model.as_deref(), None);

        // An override of `inherit` resets to the file's own alias being
        // absent-inherited — here it means "no override", so the file's
        // sonnet shows through.
        let mut reset = AgentSettings::default();
        reset
            .subagent_models
            .insert("/cfg/agents/reviewer.md".into(), "inherit".into());
        let definition = SubagentDefinition {
            model: ModelAlias::Inherit,
            ..SubagentDefinition {
                name: "reviewer".into(),
                description: "Reviews.".into(),
                tools: vec![],
                disallowed_tools: vec![],
                model: ModelAlias::Inherit,
                system_prompt: "You review.".into(),
                path: PathBuf::from("/cfg/agents/reviewer.md"),
                source: DefinitionSource::User,
            }
        };
        assert_eq!(to_wire(definition, &reset).model.as_deref(), None);
    }

    #[tokio::test]
    async fn definitions_exclude_disabled_and_apply_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".rcode").join("agents");
        let on_path =
            subagents::create(&agents, "on", "On.", "Body.", &[], &[], ModelAlias::Inherit)
                .expect("create on");
        let off_path = subagents::create(
            &agents,
            "off",
            "Off.",
            "Body.",
            &[],
            &[],
            ModelAlias::Inherit,
        )
        .expect("create off");
        let mut settings = AgentSettings::default();
        // Keyed by the path the *creator* returned — the same string the
        // listing reports, which is what a toggle passes back.
        settings
            .disabled_subagents
            .insert(off_path.to_string_lossy().into_owned());
        settings
            .subagent_models
            .insert(on_path.to_string_lossy().into_owned(), "haiku".into());

        let runtime = NativeSubagents {
            deps: SubagentDeps {
                managed: None,
                goal: None,
                providers: providers(),
                data_dir: None,
                settings,
                request_input: None,
                parent_model: "claude-opus-4".into(),
                parent_reasoning: None,
                user_instructions: Default::default(),
                base_tools: Arc::new(ToolRegistry::new()),
                limits: Arc::new(tokio::sync::Semaphore::new(MAX_ACTIVE_SUBAGENTS)),
                active: Arc::new(ActiveSubagents::default()),
            },
        };

        let found = runtime.definitions(dir.path());
        let names: Vec<&str> = found
            .iter()
            .map(|definition| definition.name.as_str())
            .collect();
        assert!(names.contains(&"on"));
        assert!(!names.contains(&"off"));
        assert!(names.contains(&"Explore"));
        assert_eq!(found[0].model, ModelAlias::Inherit, "the override applied");
    }

    /// Stopping one subagent must not stop its siblings, and the parent run's
    /// token must still reach it — that is the whole point of giving each
    /// spawn its own child token.
    #[test]
    fn the_active_registry_cancels_one_run_at_a_time() {
        let active = ActiveSubagents::default();
        let parent = tokio_util::sync::CancellationToken::new();
        let first = parent.child_token();
        let second = parent.child_token();
        active.register("call-1".to_owned(), first.clone());
        active.register("call-2".to_owned(), second.clone());

        assert!(active.is_running("call-1"));
        assert!(active.cancel("call-1"));
        assert!(first.is_cancelled(), "the named run stops");
        assert!(!second.is_cancelled(), "its sibling keeps going");
        assert!(!parent.is_cancelled(), "and the conversation keeps going");

        // A second stop of the same id finds nothing left to stop.
        assert!(active.cancel("call-1"));
        assert!(!active.cancel("never-existed"));
    }

    /// The parent's own interrupt still reaches every child.
    #[test]
    fn a_parent_cancellation_reaches_its_children() {
        let active = ActiveSubagents::default();
        let parent = tokio_util::sync::CancellationToken::new();
        let child = parent.child_token();
        active.register("call-1".to_owned(), child.clone());

        parent.cancel();
        assert!(child.is_cancelled());
    }

    /// A finished subagent leaves no entry behind, however it ended.
    #[test]
    fn the_registry_forgets_a_finished_run() {
        let active = Arc::new(ActiveSubagents::default());
        let token = tokio_util::sync::CancellationToken::new();
        active.register("call-1".to_owned(), token);
        assert!(active.is_running("call-1"));

        {
            let _guard = ActiveGuard {
                active: Arc::clone(&active),
                key: "call-1".to_owned(),
            };
        }
        assert!(
            !active.is_running("call-1"),
            "a dead subagent is not stoppable"
        );
        assert!(!active.cancel("call-1"));
    }

    #[tokio::test]
    async fn child_tools_honour_the_filters_and_never_carry_agent() {
        let mut base = ToolRegistry::new();
        for name in ["Read", "Bash", "Write"] {
            base.register(Arc::new(CountingTool(name.into())));
        }
        let definition = SubagentDefinition {
            name: "reader".into(),
            description: String::new(),
            tools: vec!["Read".into()],
            disallowed_tools: vec![],
            model: ModelAlias::Inherit,
            system_prompt: String::new(),
            path: PathBuf::new(),
            source: DefinitionSource::User,
        };
        let runtime = NativeSubagents {
            deps: SubagentDeps {
                managed: None,
                goal: None,
                providers: providers(),
                data_dir: None,
                settings: AgentSettings::default(),
                request_input: None,
                parent_model: String::new(),
                parent_reasoning: None,
                user_instructions: Default::default(),
                base_tools: Arc::new(base),
                limits: Arc::new(tokio::sync::Semaphore::new(MAX_ACTIVE_SUBAGENTS)),
                active: Arc::new(ActiveSubagents::default()),
            },
        };
        let child = runtime.child_tools(&definition);
        let names: Vec<String> = child
            .all()
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect();
        assert_eq!(names, ["Read"], "{names:?}");

        // No allowlist: everything but Task.
        let open = SubagentDefinition {
            tools: Vec::new(),
            ..definition.clone()
        };
        let child = runtime.child_tools(&open);
        let names: Vec<String> = child
            .all()
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect();
        assert_eq!(
            names,
            ["Read", "Bash", "Write"],
            "Task stays out: {names:?}"
        );

        // An allowlist that names Task — the one way a definition could ask
        // for recursion — yields nothing: the filter refuses the name before
        // the allowlist is consulted, so a child can never carry it.
        let recursive = SubagentDefinition {
            tools: vec![AGENT.into(), "Read".into()],
            ..definition
        };
        let child = runtime.child_tools(&recursive);
        let names: Vec<String> = child
            .all()
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect();
        assert_eq!(
            names,
            ["Read"],
            "an allowlisted Task is still refused: {names:?}"
        );
    }

    /// A minimal tool for registry-shape tests.
    struct CountingTool(String);

    #[async_trait::async_trait]
    impl rcode_agent::Tool for CountingTool {
        fn name(&self) -> &str {
            &self.0
        }
        fn description(&self) -> &str {
            "test tool"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }
        async fn call(
            &self,
            _input: serde_json::Value,
            _ctx: &rcode_agent::ToolContext,
        ) -> Result<rcode_agent::ToolResult, rcode_agent::ToolError> {
            Ok(rcode_agent::ToolResult::ok("ran"))
        }
    }

    #[tokio::test]
    async fn scope_roots_stay_derived_from_a_scope_name() {
        let cwd = Path::new("/repo");
        let project = agents_root_for(cwd, SkillScope::Project).expect("project");
        assert!(project.ends_with(".rcode/agents"));
        // The user scope needs a home; on a machine with one it is derived,
        // never accepted from the caller.
        if let Ok(user) = agents_root_for(cwd, SkillScope::User) {
            assert!(user.ends_with(".rcode/agents"));
        }
    }
}
