//! Conversation-scoped background agents and their management tools.
use crate::subagents::SubagentDefinition;
use crate::tools::{AgentTool, NestedEventSender, SubagentRuntime, SubagentSpawn};
use crate::{Message, Tool, ToolContext, ToolError, ToolResult};
use futures::FutureExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub struct MessageInbox {
    queue: Mutex<VecDeque<String>>,
}
impl MessageInbox {
    pub fn push(&self, message: String) -> Result<(), String> {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if queue.len() >= 64 {
            return Err("The agent message queue is full".into());
        }
        queue.push_back(message);
        Ok(())
    }
    pub fn drain(&self) -> Vec<String> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
            .collect()
    }
    pub fn is_empty(&self) -> bool {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }
}

struct AgentTask {
    id: String,
    description: String,
    definition: Option<SubagentDefinition>,
    background: bool,
    tool_use_id: String,
    live_output: Arc<Mutex<String>>,
    output_file: Option<std::path::PathBuf>,
    cwd: std::path::PathBuf,
    status: &'static str,
    output: String,
    error: Option<String>,
    history: Vec<Message>,
    inbox: Arc<MessageInbox>,
    cancel: CancellationToken,
    started_at: std::time::Instant,
    generation: u64,
    notified: u64,
}

#[derive(Default)]
pub struct AgentTaskStore {
    tasks: Mutex<HashMap<String, AgentTask>>,
    changed: Notify,
    runtime_cancel: Mutex<CancellationToken>,
    report_background: Mutex<Option<Arc<dyn Fn(usize) + Send + Sync>>>,
}

impl AgentTaskStore {
    pub fn background(&self, tool_use_id: &str) -> bool {
        let count = {
            let mut tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks
                .values_mut()
                .find(|task| task.id == tool_use_id || task.tool_use_id == tool_use_id)
            else {
                return false;
            };
            if task.status != "running" || task.cancel.is_cancelled() {
                return false;
            }
            task.background = true;
            tasks
                .values()
                .filter(|task| task.background && matches!(task.status, "running" | "stopping"))
                .count()
        };
        if let Some(report) = self.report_background.lock().unwrap().as_ref() {
            report(count);
        }
        self.changed.notify_waiters();
        true
    }
}

pub struct AgentTasks {
    pub store: Arc<AgentTaskStore>,
    runtime: Arc<dyn SubagentRuntime>,
}

impl AgentTasks {
    pub fn new(store: Arc<AgentTaskStore>, runtime: Arc<dyn SubagentRuntime>) -> Self {
        Self { store, runtime }
    }

    pub fn register(self: &Arc<Self>, registry: &mut crate::ToolRegistry) {
        if registry.get("Bash").is_some() {
            registry.register(Arc::new(ManagedBash {
                tasks: self.clone(),
            }));
        }
        for name in [
            "Agent",
            "TaskList",
            "TaskOutput",
            "TaskStop",
            "TaskBackground",
            "SendMessage",
        ] {
            registry.register(Arc::new(AgentManagementTool {
                name,
                tasks: self.clone(),
            }));
        }
    }

    pub fn bind_runtime(
        &self,
        cancel: CancellationToken,
        report: Arc<dyn Fn(usize) + Send + Sync>,
    ) {
        *self.store.runtime_cancel.lock().unwrap() = cancel;
        *self.store.report_background.lock().unwrap() = Some(report);
        self.changed();
    }

    fn changed(&self) {
        let count = self
            .store
            .tasks
            .lock()
            .unwrap()
            .values()
            .filter(|task| task.background && matches!(task.status, "running" | "stopping"))
            .count();
        if let Some(report) = self.store.report_background.lock().unwrap().as_ref() {
            report(count);
        }
        self.store.changed.notify_waiters();
    }

    pub fn has_notifications(&self) -> bool {
        self.store.tasks.lock().unwrap().values().any(|task| {
            task.background
                && task.generation != task.notified
                && !matches!(task.status, "running" | "stopping")
        })
    }

    pub fn background(&self, tool_use_id: &str) -> bool {
        self.store.background(tool_use_id)
    }

    pub fn cancel_foreground(&self) {
        for task in self.store.tasks.lock().unwrap().values() {
            if !task.background && matches!(task.status, "running" | "stopping") {
                task.cancel.cancel();
            }
        }
    }

    pub fn has_running(&self) -> bool {
        self.store
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .any(|task| matches!(task.status, "running" | "stopping"))
    }

    pub async fn wait_for_idle(&self) {
        loop {
            let changed = self.store.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !self.has_running() {
                return;
            }
            changed.await;
        }
    }

    pub fn cancel_running(&self) {
        for task in self
            .store
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
        {
            if matches!(task.status, "running" | "stopping") {
                task.cancel.cancel();
            }
        }
    }

    pub fn notifications(&self) -> Vec<String> {
        let mut tasks = self
            .store
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tasks
            .values_mut()
            .filter_map(|task| {
                if !task.background
                    || matches!(task.status, "running" | "stopping")
                    || task.generation == task.notified
                {
                    return None;
                }
                task.notified = task.generation;
                Some(format!(
                    "<task-notification>\n{}\n</task-notification>",
                    task_json(task)
                ))
            })
            .collect()
    }

    pub async fn wait_for_completion(&self, cancel: &CancellationToken) -> Result<(), ToolError> {
        loop {
            let notified = self.store.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let ready = self
                .store
                .tasks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .values()
                .any(|task| {
                    task.background
                        && task.generation != task.notified
                        && !matches!(task.status, "running" | "stopping")
                });
            if ready || !self.has_running() {
                return Ok(());
            }
            tokio::select! { biased; _ = cancel.cancelled() => return Err(ToolError::Cancelled), _ = notified => {} }
        }
    }

    fn launch(self: &Arc<Self>, id: String, prompt: String, events: NestedEventSender) {
        let this = self.clone();
        tokio::spawn(async move {
            let mut prompt = prompt;
            loop {
                events
                    .send(crate::AgentEvent::UserMessage {
                        text: prompt.clone(),
                    })
                    .await;
                let history_sink = Arc::new(Mutex::new(Vec::new()));
                let spawn = {
                    let mut tasks = this
                        .store
                        .tasks
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let Some(task) = tasks.get_mut(&id) else {
                        return;
                    };
                    *history_sink
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = task.history.clone();
                    SubagentSpawn {
                        history_sink: Some(history_sink.clone()),
                        definition: task.definition.clone().expect("agent task"),
                        cwd: task.cwd.clone(),
                        prompt: prompt.clone(),
                        history: task.history.clone(),
                        inbox: Some(task.inbox.clone()),
                        events: events.clone(),
                        cancel: task.cancel.clone(),
                        tool_use_id: events.parent_tool_use_id().into(),
                    }
                };
                let run_cancel = spawn.cancel.clone();
                let outcome = std::panic::AssertUnwindSafe(this.runtime.run(spawn))
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| Err("The child agent runtime panicked".into()));
                let terminal = match &outcome {
                    _ if run_cancel.is_cancelled() => crate::DoneReason::Interrupted,
                    Ok(_) => crate::DoneReason::Complete,
                    Err(error) => crate::DoneReason::Failed(error.clone()),
                };
                // Publish the terminal child event before waking its coordinator.
                // A queued follow-up emits UserMessage and reopens the same child.
                events
                    .send(crate::AgentEvent::Done { reason: terminal })
                    .await;
                let mut repeat = None;
                {
                    let mut tasks = this
                        .store
                        .tasks
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let Some(task) = tasks.get_mut(&id) else {
                        return;
                    };
                    match outcome {
                        Ok(outcome) if !task.cancel.is_cancelled() => {
                            task.output = outcome.output;
                            task.history = outcome.history;
                            let queued = task.inbox.drain();
                            if queued.is_empty() {
                                task.status = "completed";
                            } else {
                                repeat = Some(queued.join("\n\n"));
                            }
                        }
                        Ok(outcome) => {
                            task.history = outcome.history;
                            task.status = "stopped";
                        }
                        Err(error) => {
                            task.history = history_sink
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .clone();
                            task.status = if task.cancel.is_cancelled() {
                                "stopped"
                            } else {
                                "failed"
                            };
                            task.error = Some(error);
                        }
                    }
                    if repeat.is_none() {
                        if let Some(path) = &task.output_file {
                            if let Err(error) = std::fs::write(
                                path,
                                format!(
                                    "{}{}",
                                    task.output,
                                    task.error
                                        .as_ref()
                                        .map(|error| format!("\n{error}"))
                                        .unwrap_or_default()
                                ),
                            ) {
                                task.error =
                                    Some(format!("Could not save the task output: {error}"));
                            }
                        }
                    }
                }
                this.changed();
                if let Some(next) = repeat {
                    prompt = next;
                } else {
                    break;
                }
            }
        });
    }

    async fn wait_foreground(
        &self,
        id: &str,
        auto_ms: Option<u64>,
        promote: bool,
        ctx: &ToolContext,
    ) -> Result<Value, ToolError> {
        if let Some(ms) = auto_ms {
            if !(1..=600_000).contains(&ms) {
                return Err(ToolError::InvalidInput(
                    "auto_background_ms must be between 1 and 600000".into(),
                ));
            }
        }
        let deadline = tokio::time::sleep(Duration::from_millis(auto_ms.unwrap_or(u64::MAX / 4)));
        tokio::pin!(deadline);
        loop {
            let notified = self.store.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut tasks = self.store.tasks.lock().unwrap();
                let task = tasks.get_mut(id).expect("registered task");
                if task.background {
                    return Ok(
                        json!({"status":"async_launched","isAsync":true,"agentId":id,"backgroundTaskId":id,"output_file":task.output_file}),
                    );
                }
                if !matches!(task.status, "running" | "stopping") {
                    task.notified = task.generation;
                    return Ok(json!({"retrieval_status":"success","task":task_json(task)}));
                }
            }
            tokio::select! { biased;
                _ = ctx.cancel.cancelled() => {
                    let tasks = self.store.tasks.lock().unwrap();
                    if let Some(task) = tasks.get(id) { if task.background { continue; } task.cancel.cancel(); }
                    return Err(ToolError::Cancelled);
                }
                _ = &mut deadline, if auto_ms.is_some() => {
                    if promote { self.background(id); }
                    else { if let Some(task) = self.store.tasks.lock().unwrap().get(id) { if task.background { continue; } task.cancel.cancel(); }
                        let reap = CancellationToken::new();
                        let _ = self.output(id, true, 600_000, &reap).await;
                        return Err(ToolError::Timeout("The foreground command timed out and its process tree was stopped".into())); }
                }
                _ = notified => {}
            }
        }
    }

    async fn output(
        &self,
        id: &str,
        block: bool,
        timeout: u64,
        cancel: &CancellationToken,
    ) -> Result<Value, ToolError> {
        let wait = async {
            loop {
                let notified = self.store.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let value = {
                    let tasks = self
                        .store
                        .tasks
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let task = tasks.get(id).ok_or_else(|| {
                        ToolError::NotFound("No such task in this conversation".into())
                    })?;
                    task_json(task)
                };
                let running = matches!(value["status"].as_str(), Some("running" | "stopping"));
                if !running || !block {
                    return Ok(
                        json!({"retrieval_status":if running {"not_ready"} else {"success"},"task":value}),
                    );
                }
                tokio::select! { biased; _ = cancel.cancelled() => return Err(ToolError::Cancelled), _ = notified => {} }
            }
        };
        match tokio::time::timeout(Duration::from_millis(timeout), wait).await {
            Ok(result) => result,
            Err(_) => {
                let tasks = self
                    .store
                    .tasks
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let task = tasks.get(id).ok_or_else(|| {
                    ToolError::NotFound("No such task in this conversation".into())
                })?;
                Ok(json!({"retrieval_status":"timeout","task":task_json(task)}))
            }
        }
    }
}

fn make_room(tasks: &mut HashMap<String, AgentTask>) -> Result<(), ToolError> {
    if tasks.len() < 256 {
        return Ok(());
    }
    let oldest = tasks
        .values()
        .filter(|task| {
            !matches!(task.status, "running" | "stopping")
                && (task.generation == task.notified
                    || (!task.background && task.cancel.is_cancelled()))
        })
        .min_by_key(|task| task.started_at)
        .map(|task| task.id.clone());
    if let Some(id) = oldest {
        tasks.remove(&id);
        Ok(())
    } else {
        Err(ToolError::InvalidInput(
            "This conversation already has 256 active or unreported tasks".into(),
        ))
    }
}

fn task_json(task: &AgentTask) -> Value {
    json!({"task_id":task.id,"agentId":task.id,"task_type":if task.definition.is_some() {"local_agent"} else {"local_bash"},"agentType":task.definition.as_ref().map(|definition| &definition.name),"isBackgrounded":task.background,"output_file":task.output_file,
        "description":task.description,"status":task.status,"output":crate::tools::truncate(&if task.output.is_empty() { task.live_output.lock().unwrap().clone() } else { task.output.clone() }, 30_000).0,"outputTruncated":crate::tools::truncate(&if task.output.is_empty() { task.live_output.lock().unwrap().clone() } else { task.output.clone() }, 30_000).1,"error":task.error})
}

struct AgentManagementTool {
    name: &'static str,
    tasks: Arc<AgentTasks>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartInput {
    description: String,
    prompt: String,
    #[serde(default)]
    subagent_type: Option<String>,
    #[serde(default)]
    run_in_background: bool,
    #[serde(default)]
    auto_background_ms: Option<u64>,
}

#[async_trait::async_trait]
impl Tool for AgentManagementTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        match self.name {
            "Agent" => {
                "Delegate to a configured subagent. Set run_in_background=true for parallel work; you may finish your turn while it runs; completion automatically wakes the conversation. Uses the role's configured model. Include complete task context. Returns an agentId for TaskOutput, TaskStop and SendMessage."
            }
            "TaskBackground" => {
                "Move a running foreground Agent or Bash task to the background without restarting it."
            }
            "TaskList" => "List Agent and Bash tasks belonging to this conversation.",
            "TaskOutput" => {
                "Read an Agent or Bash task's result or wait for it. task_id must belong to this conversation. block defaults true; timeout is milliseconds, at most 600000."
            }
            "TaskStop" => {
                "Cancel one Agent or Bash task by task_id without stopping the parent or other children."
            }
            _ => {
                "Send a message to a child agent by its agentId. Running agents receive it at a step boundary; completed agents resume in the background with their original role and history."
            }
        }
    }
    fn input_schema(&self) -> Value {
        match self.name {
            "Agent" => {
                json!({"type":"object","properties":{"description":{"type":"string"},"prompt":{"type":"string"},"subagent_type":{"type":"string"},"run_in_background":{"type":"boolean"},"auto_background_ms":{"type":"integer","minimum":1,"maximum":600000}},"required":["description","prompt"],"additionalProperties":false})
            }
            "TaskList" => json!({"type":"object","properties":{},"additionalProperties":false}),
            "TaskOutput" => {
                json!({"type":"object","properties":{"task_id":{"type":"string"},"block":{"type":"boolean"},"timeout":{"type":"integer","minimum":0,"maximum":600000}},"required":["task_id"],"additionalProperties":false})
            }
            "TaskStop" | "TaskBackground" => {
                json!({"type":"object","properties":{"task_id":{"type":"string"}},"required":["task_id"],"additionalProperties":false})
            }
            _ => {
                json!({"type":"object","properties":{"to":{"type":"string"},"summary":{"type":"string"},"message":{"type":"string"}},"required":["to","summary","message"],"additionalProperties":false})
            }
        }
    }
    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        if ctx.cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let object = input
            .as_object()
            .ok_or_else(|| ToolError::InvalidInput("Arguments must be an object".into()))?;
        let allowed: &[&str] = match self.name {
            "Agent" => &[
                "description",
                "prompt",
                "subagent_type",
                "run_in_background",
                "auto_background_ms",
            ],
            "TaskList" => &[],
            "TaskOutput" => &["task_id", "block", "timeout"],
            "TaskStop" | "TaskBackground" => &["task_id"],
            _ => &["to", "summary", "message"],
        };
        if object.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(ToolError::InvalidInput(
                "Unknown agent management argument".into(),
            ));
        }
        let text = |key: &str, max: usize| -> Result<String, ToolError> {
            input[key]
                .as_str()
                .filter(|text| !text.trim().is_empty() && text.chars().count() <= max)
                .map(str::to_owned)
                .ok_or_else(|| {
                    ToolError::InvalidInput(format!(
                        "`{key}` must be non-empty text, at most {max} characters"
                    ))
                })
        };
        let result = match self.name {
            "Agent" => {
                let start: StartInput = serde_json::from_value(input.clone())
                    .map_err(|error| ToolError::InvalidInput(error.to_string()))?;
                if start
                    .auto_background_ms
                    .is_some_and(|ms| !(1..=600_000).contains(&ms))
                {
                    return Err(ToolError::InvalidInput(
                        "auto_background_ms must be between 1 and 600000".into(),
                    ));
                }
                text("description", 200)?;
                text("prompt", 100_000)?;
                let definition = AgentTool::new(self.tasks.runtime.clone())
                    .resolve(&ctx.cwd, start.subagent_type.as_deref())?;
                let id = format!("agent_{}", uuid::Uuid::new_v4());
                let directory = std::env::temp_dir().join("rcode-task-output");
                std::fs::create_dir_all(&directory)
                    .map_err(|error| ToolError::Execution(error.to_string()))?;
                let output_file = directory.join(format!("{id}.log"));
                std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&output_file)
                    .map_err(|error| ToolError::Execution(error.to_string()))?;
                {
                    let mut tasks = self
                        .tasks
                        .store
                        .tasks
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    make_room(&mut tasks)?;
                    tasks.insert(
                        id.clone(),
                        AgentTask {
                            id: id.clone(),
                            description: start.description,
                            definition: Some(definition),
                            background: start.run_in_background,
                            tool_use_id: ctx
                                .nested
                                .as_ref()
                                .map(|events| events.parent_tool_use_id().to_owned())
                                .unwrap_or_default(),
                            live_output: Arc::default(),
                            output_file: Some(output_file),
                            cwd: ctx.cwd.clone(),
                            status: "running",
                            output: String::new(),
                            error: None,
                            history: Vec::new(),
                            inbox: Arc::new(MessageInbox::default()),
                            cancel: self
                                .tasks
                                .store
                                .runtime_cancel
                                .lock()
                                .unwrap()
                                .child_token(),
                            started_at: std::time::Instant::now(),
                            generation: 1,
                            notified: 0,
                        },
                    );
                }
                self.tasks.changed();
                self.tasks.launch(
                    id.clone(),
                    start.prompt,
                    ctx.nested.clone().unwrap_or_else(NestedEventSender::null),
                );
                if start.run_in_background {
                    self.tasks.wait_foreground(&id, None, true, ctx).await?
                } else {
                    self.tasks
                        .wait_foreground(&id, start.auto_background_ms, true, ctx)
                        .await?
                }
            }
            "TaskList" => {
                let tasks = self
                    .tasks
                    .store
                    .tasks
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let mut rows: Vec<_> = tasks.values().map(|task| json!({"task_id":task.id,"agentId":task.id,"task_type":if task.definition.is_some() {"local_agent"} else {"local_bash"},"description":task.description,"status":task.status,"isBackgrounded":task.background,"agentType":task.definition.as_ref().map(|definition| &definition.name),"output_file":task.output_file})).collect();
                rows.sort_by(|a, b| a["task_id"].as_str().cmp(&b["task_id"].as_str()));
                json!({"tasks":rows})
            }
            "TaskOutput" => {
                let id = text("task_id", 200)?;
                let block = match input.get("block") {
                    None => true,
                    Some(Value::Bool(value)) => *value,
                    _ => return Err(ToolError::InvalidInput("block must be boolean".into())),
                };
                let timeout = match input.get("timeout") {
                    None => 30_000,
                    Some(value) => value
                        .as_u64()
                        .filter(|value| *value <= 600_000)
                        .ok_or_else(|| {
                            ToolError::InvalidInput(
                                "timeout must be an integer between 0 and 600000".into(),
                            )
                        })?,
                };
                self.tasks.output(&id, block, timeout, &ctx.cancel).await?
            }
            "TaskBackground" => {
                let id = text("task_id", 200)?;
                json!({"task_id":id,"backgrounded":self.tasks.background(&id)})
            }
            "TaskStop" => {
                let id = text("task_id", 200)?;
                let mut tasks = self
                    .tasks
                    .store
                    .tasks
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let task = tasks.get_mut(&id).ok_or_else(|| {
                    ToolError::NotFound("No such task in this conversation".into())
                })?;
                if matches!(task.status, "running" | "stopping") {
                    task.status = "stopping";
                    task.cancel.cancel();
                }
                json!({"task_id":id,"status":task.status})
            }
            _ => {
                let id = text("to", 200)?;
                text("summary", 200)?;
                let message = text("message", 20_000)?;
                let resumed = {
                    let mut tasks = self
                        .tasks
                        .store
                        .tasks
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let task = tasks.get_mut(&id).ok_or_else(|| {
                        ToolError::NotFound("No such task in this conversation".into())
                    })?;
                    if task.definition.is_none() {
                        return Err(ToolError::InvalidInput(
                            "SendMessage requires an Agent task".into(),
                        ));
                    }
                    if task.status == "stopping" {
                        return Err(ToolError::InvalidInput(
                            "The agent is stopping; wait for its result before resuming".into(),
                        ));
                    }
                    if task.status == "running" {
                        task.inbox
                            .push(message.clone())
                            .map_err(ToolError::InvalidInput)?;
                        false
                    } else {
                        task.status = "running";
                        task.output.clear();
                        task.error = None;
                        task.generation += 1;
                        task.background = true;
                        task.cancel = self
                            .tasks
                            .store
                            .runtime_cancel
                            .lock()
                            .unwrap()
                            .child_token();
                        task.inbox = Arc::new(MessageInbox::default());
                        true
                    }
                };
                self.tasks.changed();
                if resumed {
                    let original_id = self
                        .tasks
                        .store
                        .tasks
                        .lock()
                        .unwrap()
                        .get(&id)
                        .unwrap()
                        .tool_use_id
                        .clone();
                    let events = ctx
                        .nested
                        .as_ref()
                        .map(|events| {
                            if original_id.is_empty() {
                                events.clone()
                            } else {
                                events.with_parent_tool_use_id(original_id)
                            }
                        })
                        .unwrap_or_else(NestedEventSender::null);
                    self.tasks.launch(id.clone(), message, events);
                }
                json!({"status":"success","agentId":id,"messageId":uuid::Uuid::new_v4().to_string(),"delivery":if resumed {"resumed_background"} else {"queued"}})
            }
        };
        if matches!(self.name, "Agent")
            && matches!(
                result["task"]["status"].as_str(),
                Some("failed" | "stopped")
            )
        {
            return Ok(ToolResult::error(result.to_string()));
        }
        Ok(ToolResult::ok(result.to_string()))
    }
}

struct ManagedBash {
    tasks: Arc<AgentTasks>,
}
#[async_trait::async_trait]
impl Tool for ManagedBash {
    fn name(&self) -> &str {
        "Bash"
    }
    fn description(&self) -> &str {
        "Run a shell command. Supports run_in_background; foreground timeout (seconds, default 120, max 600) promotes the same process to background. Use TaskList/TaskOutput/TaskStop/TaskBackground to manage tasks. auto_background=false keeps timeout termination. Pure sleep commands do not auto-promote."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"command":{"type":"string"},"timeout":{"type":"integer","minimum":1,"maximum":600},"run_in_background":{"type":"boolean"},"auto_background":{"type":"boolean"}},"required":["command"],"additionalProperties":false})
    }
    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let command = input["command"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| ToolError::InvalidInput("command is required".into()))?;
        let background = match input.get("run_in_background") {
            None => false,
            Some(Value::Bool(v)) => *v,
            _ => {
                return Err(ToolError::InvalidInput(
                    "run_in_background must be boolean".into(),
                ));
            }
        };
        let seconds = match input.get("timeout") {
            None => 120,
            Some(v) => v
                .as_u64()
                .filter(|v| (1..=600).contains(v))
                .ok_or_else(|| {
                    ToolError::InvalidInput("timeout must be seconds between 1 and 600".into())
                })?,
        };
        let auto = match input.get("auto_background") {
            None => {
                !command.trim_start().starts_with("sleep ")
                    && !command
                        .trim_start()
                        .to_ascii_lowercase()
                        .starts_with("start-sleep ")
            }
            Some(Value::Bool(v)) => *v,
            _ => {
                return Err(ToolError::InvalidInput(
                    "auto_background must be boolean".into(),
                ));
            }
        };
        let id = format!("bash_{}", uuid::Uuid::new_v4());
        let directory = std::env::temp_dir().join("rcode-task-output");
        std::fs::create_dir_all(&directory).map_err(|e| ToolError::Execution(e.to_string()))?;
        let path = directory.join(format!("{id}.log"));
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|e| ToolError::Execution(e.to_string()))?;
        let file = Arc::new(Mutex::new(file));
        let live_output = Arc::new(Mutex::new(String::new()));
        let cancel = self
            .tasks
            .store
            .runtime_cancel
            .lock()
            .unwrap()
            .child_token();
        {
            let mut tasks = self.tasks.store.tasks.lock().unwrap();
            make_room(&mut tasks)?;
            tasks.insert(
                id.clone(),
                AgentTask {
                    id: id.clone(),
                    description: command.chars().take(200).collect(),
                    definition: None,
                    background,
                    tool_use_id: ctx
                        .nested
                        .as_ref()
                        .map(|events| events.parent_tool_use_id().to_owned())
                        .unwrap_or_default(),
                    live_output: live_output.clone(),
                    output_file: Some(path),
                    cwd: ctx.cwd.clone(),
                    status: "running",
                    output: String::new(),
                    error: None,
                    history: Vec::new(),
                    inbox: Arc::default(),
                    cancel: cancel.clone(),
                    started_at: std::time::Instant::now(),
                    generation: 1,
                    notified: 0,
                },
            );
        }
        self.tasks.changed();
        let tasks = self.tasks.clone();
        let task_id = id.clone();
        let ctx_owned = ToolContext {
            cwd: ctx.cwd.clone(),
            cancel,
            nested: None,
        };
        let progress = Arc::new(move |text: String| {
            use std::io::{Seek, Write};
            *live_output.lock().unwrap() = text.clone();
            let mut file = file.lock().unwrap();
            let _ = file.rewind();
            let _ = file.set_len(0);
            let _ = file.write_all(text.as_bytes());
        });
        tokio::spawn(async move {
            let result = std::panic::AssertUnwindSafe(crate::tools::bash::execute(
                input,
                &ctx_owned,
                false,
                Some(progress.clone()),
            ))
            .catch_unwind()
            .await
            .unwrap_or_else(|_| Err(ToolError::Execution("The shell task panicked".into())));
            {
                let mut all = tasks.store.tasks.lock().unwrap();
                let task = all.get_mut(&task_id).unwrap();
                match result {
                    Ok(result) => {
                        task.status = if result.is_error {
                            "failed"
                        } else {
                            "completed"
                        };
                        task.output = result.output;
                    }
                    Err(error) => {
                        task.status = if task.cancel.is_cancelled() {
                            "stopped"
                        } else {
                            "failed"
                        };
                        task.error = Some(error.to_string());
                        task.output = task.live_output.lock().unwrap().clone();
                    }
                }
                progress(task.output.clone());
            }
            tasks.changed();
        });
        let result = self
            .tasks
            .wait_foreground(
                &id,
                if !background {
                    Some(seconds * 1000)
                } else {
                    None
                },
                auto,
                ctx,
            )
            .await?;
        if result["isAsync"] == true {
            return Ok(ToolResult::ok(result.to_string()));
        }
        let task = &result["task"];
        Ok(ToolResult {
            output: task["output"].as_str().unwrap_or_default().to_owned()
                + &task["error"]
                    .as_str()
                    .map(|s| format!("\n{s}"))
                    .unwrap_or_default(),
            is_error: task["status"] != "completed",
            diff: None,
        })
    }
}
