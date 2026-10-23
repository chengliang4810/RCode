//! The ReAct loop.
//!
//! One turn is: append the user message, ask the model, stream its output as
//! events, run whatever tools it asked for, append the results, repeat until it
//! answers without tool calls. The loop owns no storage and no UI — it emits
//! [`AgentEvent`]s and asks the [`AgentHost`] when it needs a decision.
//!
//! Cancellation is cooperative: the token is checked between steps and passed
//! into every tool call, so a long `Bash` is interrupted where it can be.

use std::sync::Arc;

use futures::{FutureExt, stream::{BoxStream, Stream, StreamExt}};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::message::{ContentBlock, Message};
use crate::tool::{ToolContext, ToolRegistry};
use crate::{
    AgentError, AgentEvent, AgentHost, DoneReason, InputAnswer, InputRequest, ReasoningChunk,
};

/// How one run behaves. The provider and model are already resolved by the host
/// that built the runner.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Model name, echoed in [`AgentEvent::Started`] and used by the host's
    /// model client (which the loop never inspects directly).
    pub model: String,
    /// Working directory tool calls resolve relative paths against.
    pub cwd: std::path::PathBuf,
    /// Optional ceiling on model round-trips in one run.
    ///
    /// Unset by default: the loop runs until the model answers without asking
    /// for another tool, or the caller cancels. A ceiling is available for a
    /// host that wants a guard rail, but it is not imposed — a long refactor
    /// legitimately takes many steps, and stopping one halfway is worse than
    /// the risk of an unattended loop, which the caller's budget already
    /// bounds.
    pub max_steps: Option<usize>,
    /// Text placed before the conversation as a system turn.
    ///
    /// The host owns this: what the agent is told about its environment,
    /// its memory and its instructions is a property of the host, not of the
    /// loop, and keeping it here lets the loop stay ignorant of where any of
    /// it came from. Empty means no system turn is sent.
    pub system_prompt: Option<String>,
    /// The conversation before this turn, oldest first.
    ///
    /// The loop is stateless: it is handed the history and replays it every
    /// turn rather than holding a conversation between calls. That is what
    /// makes a run cancellable and a process restart survivable — there is no
    /// in-memory session to lose, only the host's own record, which it already
    /// persists for the UI.
    ///
    /// Empty for the first turn, and for a host that keeps no history.
    pub history: Vec<crate::message::Message>,
    /// Images to attach to this turn's user message.
    ///
    /// Separate from `history` because these belong to *this* prompt: the loop
    /// puts them on the user turn it is about to send, which is where the
    /// provider expects them. A host that reads them from disk owns the
    /// decoding — the loop takes blocks ready to send.
    pub images: Vec<crate::message::ContentBlock>,
    /// When the loop compacts on its own, or `None` to never.
    ///
    /// The loop owns this because it is the only place that sees the request
    /// as it is about to be sent. A caller cannot compact for it: a tool loop
    /// grows the conversation between the caller's turns, so the moment the
    /// context fills is by definition *inside* a run.
    ///
    /// `None` when the host cannot summarize or does not know the model's
    /// window — see [`AgentHost::summarize`] and [`AgentHost::context_window`].
    pub compact: Option<crate::compact::CompactConfig>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            cwd: std::env::current_dir().unwrap_or_default(),
            max_steps: None,
            system_prompt: None,
            history: Vec::new(),
            images: Vec::new(),
            compact: None,
        }
    }
}

/// Drives one conversation turn at a time.
///
/// Cheap to clone behind an `Arc`; a host usually keeps one per conversation,
/// which is what makes several conversations run concurrently without any
/// coordination between them.
pub struct AgentRunner {
    host: Arc<dyn AgentHost>,
    tools: Arc<ToolRegistry>,
    config: AgentConfig,
    /// The user's hooks, or [`crate::hook::NoHooks`].
    ///
    /// Never an `Option`: the loop fires at every mount point, and branching
    /// on "are there hooks" at each one would repeat the same check through
    /// the middle of its flow.
    hooks: Arc<dyn crate::hook::HookRunner>,
    goal: Option<Arc<dyn crate::goal::GoalStore>>,
    goal_id: Option<String>,
    goal_continuation: Option<Arc<std::sync::Mutex<Option<crate::goal::GoalContinuation>>>>,
    tasks: Option<Arc<crate::agent_tasks::AgentTasks>>,
    inbox: Option<Arc<crate::agent_tasks::MessageInbox>>,
    history_sink: Option<Arc<std::sync::Mutex<Vec<Message>>>>,
    history_recorder: Option<Arc<dyn crate::HistoryRecorder>>,
    context_source: Option<Arc<dyn crate::context::ContextSource>>,
    /// Hand control back to the session at a safe boundary, preserving tool results.
    turn_yield: Option<CancellationToken>,
}

impl AgentRunner {
    pub fn new(host: Arc<dyn AgentHost>, tools: Arc<ToolRegistry>, config: AgentConfig) -> Self {
        let tasks = tools.managed_agents();
        Self {
            host,
            tools,
            config,
            hooks: Arc::new(crate::hook::NoHooks),
            goal: None,
            goal_id: None,
            goal_continuation: None,
            tasks,
            inbox: None,
            history_sink: None,
            history_recorder: None,
            context_source: None,
            turn_yield: None,
        }
    }

    /// Attach the host's hook runner.
    ///
    /// A builder rather than a fourth `new` argument: most hosts have no
    /// hooks, and a constructor that forced every caller to name
    /// [`crate::hook::NoHooks`] would make the common case the verbose one.
    pub fn with_hooks(mut self, hooks: Arc<dyn crate::hook::HookRunner>) -> Self {
        self.hooks = hooks;
        self
    }

    pub fn config(&self) -> &AgentConfig {
        &self.config
    }

    pub fn with_goal(mut self, goal: Option<Arc<dyn crate::goal::GoalStore>>) -> Self {
        self.goal_id = goal.as_ref().and_then(|store| store.read().ok().flatten())
            .filter(|goal| goal.status == rcode_proto::goal::GoalStatus::Active).map(|goal| goal.id);
        self.goal = goal;
        self
    }

    pub fn with_expected_goal(mut self, id: String) -> Self { self.goal_id = Some(id); self }

    pub fn with_goal_continuation(mut self, sink: Arc<std::sync::Mutex<Option<crate::goal::GoalContinuation>>>) -> Self {
        self.goal_continuation = Some(sink);
        self
    }

    pub fn with_tasks(mut self, tasks: Arc<crate::agent_tasks::AgentTasks>) -> Self { self.tasks = Some(tasks); self }
    pub fn with_inbox(mut self, inbox: Option<Arc<crate::agent_tasks::MessageInbox>>) -> Self { self.inbox = inbox; self }
    pub fn with_history_sink(mut self, sink: Arc<std::sync::Mutex<Vec<Message>>>) -> Self { self.history_sink = Some(sink); self }

    pub fn with_history_recorder(mut self, recorder: Arc<dyn crate::HistoryRecorder>) -> Self {
        self.history_recorder = Some(recorder);
        self
    }

    pub fn with_context_source(mut self, source: Arc<dyn crate::context::ContextSource>) -> Self {
        self.context_source = Some(source);
        self
    }

    async fn refresh_context(&self, messages: &mut Vec<Message>, cancel: &CancellationToken) -> Result<(), AgentError> {
        let Some(source) = &self.context_source else { return Ok(()); };
        let snapshots = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            snapshots = source.snapshots() => snapshots,
        };
        for snapshot in snapshots {
            if cancel.is_cancelled() { return Err(AgentError::Cancelled); }
            if let Some(record) = crate::context::next_record(messages, snapshot) {
                let message = record.message();
                // Persistence must succeed before delivery; no separate cursor can get ahead.
                self.record_message(&message).await?;
                messages.push(message);
                self.remember_history(messages);
            }
        }
        Ok(())
    }

    pub fn with_turn_yield(mut self, signal: CancellationToken) -> Self {
        self.turn_yield = Some(signal);
        self
    }

    async fn yield_turn(&self, messages: &[Message], tx: &mpsc::Sender<Result<AgentEvent, AgentError>>) -> bool {
        if !self.turn_yield.as_ref().is_some_and(CancellationToken::is_cancelled) {
            return false;
        }
        self.remember_history(messages);
        // This is an internal handoff, not Stop: the session already owns the
        // next user prompt and the engine suppresses completion notifications.
        let _ = tx.send(Ok(AgentEvent::Done { reason: DoneReason::Complete })).await;
        true
    }

    async fn record_message(&self, message: &Message) -> Result<(), AgentError> {
        if let Some(recorder) = &self.history_recorder { recorder.record(message).await?; }
        Ok(())
    }

    fn remember_history(&self, messages: &[Message]) {
        if let Some(sink) = &self.history_sink {
            *sink.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = messages.to_vec();
        }
    }

    fn active_goal(&self) -> Result<Option<rcode_proto::goal::SessionGoal>, AgentError> {
        let Some((store, id)) = self.goal.as_ref().zip(self.goal_id.as_ref()) else { return Ok(None); };
        Ok(store.read().map_err(AgentError::Host)?.filter(|goal| goal.id == *id && goal.status == rcode_proto::goal::GoalStatus::Active))
    }

    fn account_goal(&self, statistics: &rcode_proto::ModelCallStatistics) -> Result<(), AgentError> {
        if let Some((store, id)) = self.goal.as_ref().zip(self.goal_id.as_ref()) {
            store.change(id, crate::goal::GoalChange::Usage {
                tokens: statistics.input_tokens.unwrap_or(0).saturating_add(statistics.output_tokens.unwrap_or(0)),
                elapsed_ms: 0,
            }).map_err(AgentError::Host)?;
        }
        Ok(())
    }

    /// Run one turn and stream what happens.
    ///
    /// Emits exactly one parent [`AgentEvent::Done`]. Detached children may
    /// continue sending nested events afterward. The host owns their session
    /// lifetime and starts a new parent turn for completion notifications.
    /// The cancellation token stops this foreground turn.
    pub fn run(
        self: Arc<Self>,
        prompt: String,
        cancel: CancellationToken,
    ) -> BoxStream<'static, Result<AgentEvent, AgentError>> {
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            // Losing the event consumer stops this foreground run as well.
            // Keep polling drive after cancellation so tools can reap processes
            // and persist their terminal state instead of dropping their futures.
            let cancel = cancel.child_token();
            let drive = self.drive(prompt, &cancel, &tx);
            tokio::pin!(drive);
            let outcome = tokio::select! {
                biased;
                _ = tx.closed() => {
                    cancel.cancel();
                    drive.await
                }
                outcome = &mut drive => outcome,
            };
            if let Err(error) = outcome {
                if let Some(tasks) = &self.tasks { tasks.cancel_foreground(); }
                if let Some((store, id)) = self.goal.as_ref().zip(self.goal_id.as_ref()) {
                    let reason = if cancel.is_cancelled() { "Stopped by the user".to_owned() } else { error.to_string() };
                    if let Err(error) = store.change(id, crate::goal::GoalChange::Pause { reason }) {
                        tracing::error!(%error, "failed to pause goal after runner exit");
                    }
                }
                // Every exit still ends with Done. A cancelled transport can
                // report EOF or an error, but neither turns a stop into failure.
                let reason = if cancel.is_cancelled() || matches!(error, AgentError::Cancelled) {
                    DoneReason::Interrupted
                } else {
                    DoneReason::Failed(error.to_string())
                };
                if !cancel.is_cancelled() && !matches!(error, AgentError::Cancelled) {
                    let result = self.hooks.fire(crate::hook::HookEvent::StopFailure, crate::hook::HookContext {
                        cwd: self.config.cwd.clone(), stop_reason: Some(error.to_string()), ..Default::default()
                    }).await;
                    self.report_hook_failures(&result);
                }
                let _ = tx.send(Ok(AgentEvent::Done { reason })).await;
            }
        });
        Box::pin(receiver_stream(rx))
    }

    async fn drive(
        &self,
        prompt: String,
        cancel: &CancellationToken,
        tx: &mpsc::Sender<Result<AgentEvent, AgentError>>,
    ) -> Result<(), AgentError> {
        if cancel.is_cancelled() { return Err(AgentError::Cancelled); }
        let mut messages = Vec::with_capacity(self.config.history.len() + 2);
        // The system turn goes first, and only when the host supplied one:
        // an empty system message is a request the provider has to serialize
        // for nothing.
        if let Some(system) = self.config.system_prompt.as_deref() {
            if !system.trim().is_empty() {
                messages.push(Message::system(system));
            }
        }
        // Replay the conversation before this turn. The loop is stateless, so
        // this is the whole of its memory — see `AgentConfig::history`.
        messages.extend(self.config.history.iter().cloned());
        // The turn's images ride the user message they belong to: a provider
        // reads them from there, and an image on its own turn would not be
        // attributed to anything.
        messages.push(if self.config.images.is_empty() {
            Message::user(prompt)
        } else {
            let mut blocks = vec![crate::message::ContentBlock::text(prompt)];
            blocks.extend(self.config.images.iter().cloned());
            Message::user(crate::message::MessageContent::Blocks(blocks))
        });
        let mut applied_goal_revision = None;
        if let Some(goal) = self.active_goal()? {
            applied_goal_revision = Some(goal.revision);
            messages.push(Message::user(crate::goal::reminder(&goal)));
        }
        let _ = tx
            .send(Ok(AgentEvent::Started {
                model: self.config.model.clone(),
            }))
            .await;

        // SessionStart and UserPromptSubmit fire before the first model call.
        // Context either contributes is appended as a second user-role turn
        // rather than folded into the system prompt: the system prompt is the
        // host's standing instruction, and a hook that fires per run is not
        // standing. A block on UserPromptSubmit refuses the run outright —
        // there is nothing to attach a refusal to when no tool call exists yet.
        if !self.fire_start_hooks(&mut messages, tx).await {
            if let Some(goal) = self.active_goal()? {
                self.goal.as_ref().expect("active goal has store").change(&goal.id, crate::goal::GoalChange::Pause { reason: "A startup hook blocked the run".into() }).map_err(AgentError::Host)?;
            }
            let _ = tx
                .send(Ok(AgentEvent::Done {
                    reason: DoneReason::Complete,
                }))
                .await;
            return Ok(());
        }

        // `usize::MAX` when unset: the loop then ends only on a final answer or
        // cancellation, which is the default.
        let ceiling = self.config.max_steps.unwrap_or(usize::MAX);
        for _step in 0..ceiling {
            if cancel.is_cancelled() { return Err(AgentError::Cancelled); }
            if self.yield_turn(&messages, tx).await { return Ok(()); }

            if self.goal_id.is_some() && self.active_goal()?.is_none() {
                if let Some(tasks) = &self.tasks { tasks.cancel_foreground(); }
                self.remember_history(&messages);
                let _ = tx.send(Ok(AgentEvent::Done { reason: DoneReason::Complete })).await;
                return Ok(());
            }
            if let Some(goal) = self.active_goal()? {
                if applied_goal_revision != Some(goal.revision) {
                    messages.push(Message::user(format!("The user updated the session goal. Use this objective from this step onward, replacing previous goal instructions.\n{}", crate::goal::reminder(&goal))));
                    applied_goal_revision = Some(goal.revision);
                    if _step > 0 {
                        let _ = tx.send(Ok(AgentEvent::TextDelta { text: "\n\n".into() })).await;
                    }
                }
            }
            if let Some(inbox) = &self.inbox {
                for message in inbox.drain() {
                    let _ = tx.send(Ok(AgentEvent::UserMessage { text: message.clone() })).await;
                    messages.push(Message::user(format!("Message from coordinator:\n{message}")));
                }
            }
            if let Some(tasks) = &self.tasks {
                for notification in tasks.notifications() { messages.push(Message::user(notification)); }
            }
            self.remember_history(&messages);

            // Compaction belongs here, at the top of each step, because a tool
            // loop grows the conversation between the caller's turns: the
            // moment the context fills is inside a run, and no caller can see
            // it coming.
            self.refresh_context(&mut messages, cancel).await?;
            let first = messages.iter().find(|message| message.role != crate::MessageRole::System).map(|message| message.id.clone());
            self.compact_if_needed(&mut messages, tx).await;
            // A compacted tail may retain a patch but lose its baseline.
            if first != messages.iter().find(|message| message.role != crate::MessageRole::System).map(|message| message.id.clone()) {
                self.refresh_context(&mut messages, cancel).await?;
            }

            let todos = crate::todo::State::from_history(&messages);
            if self.tools.get("TodoWrite").is_some() && todos.reminder_due() {
                let reminder = todos.record("reminder");
                self.record_message(&reminder).await?;
                messages.push(reminder);
                self.remember_history(&messages);
            }
            let reasoned = self.reason(&messages, cancel, tx).await;
            // A cancelled provider may finish with EOF or an error rather than
            // a final message. Cancellation owns the terminal outcome either
            // way, including when the host finishes as the token is cancelled.
            if cancel.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            let assistant = match reasoned? {
                Some(message) => message,
                None => {
                    // Stream ended without a message: report and stop rather
                    // than looping on a broken provider.
                    return Err(AgentError::Model("the model stream ended without a message".to_owned()));
                }
            };

            self.record_message(&assistant).await?;
            let _ = tx
                .send(Ok(AgentEvent::MessageComplete {
                    message_id: assistant.id.clone(),
                }))
                .await;

            let calls = assistant.content.tool_calls();
            messages.push(assistant);
            self.remember_history(&messages);

            if calls.is_empty() {
                if self.yield_turn(&messages, tx).await { return Ok(()); }
                if let Some(inbox) = &self.inbox && !inbox.is_empty() { continue; }
                if let Some(tasks) = &self.tasks {
                    let notifications = tasks.notifications();
                    if !notifications.is_empty() {
                        for notification in notifications { messages.push(Message::user(notification)); }
                        continue;
                    }
                }
                if self.tasks.as_ref().is_some_and(|tasks| tasks.has_running()) {
                    self.remember_history(&messages);
                    self.fire_stop_hook(None).await;
                    let _ = tx.send(Ok(AgentEvent::Done { reason: DoneReason::Complete })).await;
                    return Ok(());
                }
                let todos = crate::todo::State::from_history(&messages);
                if let Some(goal) = self.active_goal()? {
                    if self.goal.as_ref().expect("active goal has store").should_yield().map_err(AgentError::Host)? {
                        self.remember_history(&messages);
                        self.fire_stop_hook(None).await;
                        let _ = tx.send(Ok(AgentEvent::Done { reason: DoneReason::Complete })).await;
                        return Ok(());
                    }
                    // An edit during the model/tool step is first sent to the next
                    // reasoning call. Do not verify its objective using an old answer.
                    if applied_goal_revision != Some(goal.revision) { continue; }
                    // 明确记录阻塞并不意味着目标完成；暂停并保留原待办，避免无进展续跑。
                    if let Some(resolution) = todos.resolution() {
                        let changed = self.goal.as_ref().expect("active goal has store").change_if_current(
                            &goal.id, Some(goal.revision), crate::goal::GoalChange::Pause {
                                reason: format!("Unfinished todos ({:?}): {}", resolution.kind, resolution.reason),
                            },
                        ).map_err(AgentError::Host)?;
                        if changed.is_none() && self.active_goal()?.is_some() { continue; }
                        self.remember_history(&messages);
                        self.fire_stop_hook(None).await;
                        let _ = tx.send(Ok(AgentEvent::Done { reason: DoneReason::Complete })).await;
                        return Ok(());
                    }
                    let verification = self.verify_goal(&messages, &goal, cancel, tx).await;
                    if cancel.is_cancelled() { return Err(AgentError::Cancelled); }
                    // Stop, pause or replacement during the verification wins.
                    if self.active_goal()?.is_some() {
                        let store = self.goal.as_ref().expect("active goal has store");
                        match verification {
                            Ok(verification) if verification.passed && !todos.unfinished() => {
                                let changed = store.change_if_current(&goal.id, Some(goal.revision), crate::goal::GoalChange::Complete { reason: verification.reason }).map_err(AgentError::Host)?;
                                if changed.is_none() && self.active_goal()?.is_some() { continue; }
                            }
                            Ok(verification) => {
                                let next = if todos.unfinished() {
                                    format!("Complete the remaining todos before concluding. If genuinely blocked, cancelled, or out of scope, record a TodoWrite resolution with a specific reason and explain it; the goal will pause.\n{}", todos.render())
                                } else { verification.next_action.unwrap_or_default() };
                                let changed = store.change_if_current(&goal.id, Some(goal.revision), crate::goal::GoalChange::Advance { reason: verification.reason }).map_err(AgentError::Host)?;
                                if changed.is_none() { continue; }
                                let prompt = format!("{}\nNext action: {next}", crate::goal::reminder(changed.as_ref().unwrap_or(&goal)));
                                if let Some(sink) = &self.goal_continuation {
                                    self.remember_history(&messages);
                                    *sink.lock().unwrap() = Some(crate::goal::GoalContinuation { goal_id: goal.id.clone(), revision: goal.revision, prompt });
                                    self.fire_stop_hook(None).await;
                                    let _ = tx.send(Ok(AgentEvent::Done { reason: DoneReason::Complete })).await;
                                    return Ok(());
                                }
                                messages.push(Message::user(prompt));
                                let _ = tx.send(Ok(AgentEvent::TextDelta { text: "\n\n".into() })).await;
                                continue;
                            }
                            Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
                            Err(error) => {
                                let changed = store.change_if_current(&goal.id, Some(goal.revision), crate::goal::GoalChange::Pause { reason: error.to_string() }).map_err(AgentError::Host)?;
                                if changed.is_none() && self.active_goal()?.is_some() { continue; }
                            }
                        }
                    }
                } else if todos.unfinished() && todos.resolution().is_none() && self.tools.get("TodoWrite").is_some() {
                    // 普通结束与目标续跑互斥，不能分别注入提醒并启动两次执行。
                    let reminder = todos.record("closure");
                    self.record_message(&reminder).await?;
                    messages.push(reminder);
                    self.remember_history(&messages);
                    continue;
                }
                if cancel.is_cancelled() { return Err(AgentError::Cancelled); }
                self.remember_history(&messages);
                self.fire_stop_hook(None).await;
                let _ = tx
                    .send(Ok(AgentEvent::Done {
                        reason: DoneReason::Complete,
                    }))
                    .await;
                return Ok(());
            }

            let mut results = self.run_tools(&calls, cancel, tx).await?;
            if cancel.is_cancelled() {
                // 恢复子 Agent 前保留已完成的副作用，并补齐被取消的工具配对。
                // 否则中断后的历史会丢掉执行结果，模型可能重复执行已完成操作。
                let resolved: std::collections::HashSet<_> = results.iter().filter_map(|block| match block {
                    ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.clone()), _ => None,
                }).collect();
                for (id, _, _) in &calls {
                    if !resolved.contains(id) { results.push(ContentBlock::ToolResult {
                        tool_use_id: id.clone(), content: "Cancelled before completion. Inspect current state before retrying side effects.".into(), is_error: true,
                    }); }
                }
                messages.push(Message::tool_results(results));
                self.remember_history(&messages);
                return Err(AgentError::Cancelled);
            }
            messages.push(Message::tool_results(results));
            self.remember_history(&messages);
            if self.yield_turn(&messages, tx).await { return Ok(()); }
        }

        // Only reachable when a ceiling was configured — the default runs
        // until a final answer. This is a real outcome, not an error: the model
        // kept asking for tools and we stopped it.
        Err(AgentError::Model(format!("stopped after {ceiling} steps without a final answer")))
    }

    async fn verify_goal(
        &self,
        messages: &[Message],
        goal: &rcode_proto::goal::SessionGoal,
        cancel: &CancellationToken,
        tx: &mpsc::Sender<Result<AgentEvent, AgentError>>,
    ) -> Result<crate::goal::GoalVerification, AgentError> {
        let todos = crate::todo::State::from_history(messages);
        let evidence = serde_json::to_string(&serde_json::json!({"goal":goal,"currentTodos":todos.current,"transcript":messages}))
            .map_err(|error| AgentError::Host(error.to_string()))?;
        let request = [Message::system(crate::goal::VERIFICATION_PROMPT), Message::user(evidence)];
        let started = std::time::Instant::now();
        let mut stream = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            result = self.host.verify_goal(&request) => result?,
        };
        let mut response = None;
        while let Some(chunk) = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            chunk = stream.next() => chunk,
        } {
            if let ReasoningChunk::Done { message, usage } = chunk? {
                if !message.content.tool_calls().is_empty() { return Err(AgentError::Model("Goal verifier attempted to call a tool".into())); }
                let statistics = rcode_proto::ModelCallStatistics {
                    model: self.config.model.clone(), input_tokens: usage.map(|usage| usage.input_tokens),
                    output_tokens: usage.map(|usage| usage.output_tokens), cache_read_tokens: usage.and_then(|usage| usage.cache_read_tokens),
                    cache_write_tokens: usage.and_then(|usage| usage.cache_write_tokens),
                    duration_ms: started.elapsed().as_millis() as u64, first_token_ms: None,
                };
                self.account_goal(&statistics)?;
                let _ = tx.send(Ok(AgentEvent::ModelCall { id: uuid::Uuid::new_v4().to_string(), statistics })).await;
                response = Some(message.content.text_content());
            }
        }
        crate::goal::GoalVerification::parse(response.as_deref().ok_or_else(|| AgentError::Model("Goal verifier ended without a result".into()))?)
            .map_err(AgentError::Model)
    }

    /// Ask the model once, forwarding deltas, and return the message it built.
    async fn reason(
        &self,
        messages: &[Message],
        cancel: &CancellationToken,
        tx: &mpsc::Sender<Result<AgentEvent, AgentError>>,
    ) -> Result<Option<Message>, AgentError> {
        let tools = self.tools.all();
        let started = std::time::Instant::now();
        let mut first_token_ms = None;
        let mut stream = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            result = self.host.reason(messages, &tools) => result?,
        };
        let mut built = None;
        while let Some(chunk) = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            chunk = stream.next() => chunk,
        } {
            match chunk? {
                ReasoningChunk::Activity => {
                    first_token_ms.get_or_insert(started.elapsed().as_millis() as u64);
                }
                ReasoningChunk::Text(text) => {
                    if !text.is_empty() {
                        first_token_ms.get_or_insert(started.elapsed().as_millis() as u64);
                    }
                    let _ = tx.send(Ok(AgentEvent::TextDelta { text })).await;
                }
                ReasoningChunk::Reasoning(text) => {
                    if !text.is_empty() {
                        first_token_ms.get_or_insert(started.elapsed().as_millis() as u64);
                    }
                    let _ = tx.send(Ok(AgentEvent::ReasoningDelta { text })).await;
                }
                ReasoningChunk::Done { message, usage } => {
                    let statistics = rcode_proto::ModelCallStatistics {
                        model: self.config.model.clone(),
                        input_tokens: usage.map(|u| u.input_tokens),
                        output_tokens: usage.map(|u| u.output_tokens),
                        cache_read_tokens: usage.and_then(|u| u.cache_read_tokens),
                        cache_write_tokens: usage.and_then(|u| u.cache_write_tokens),
                        duration_ms: started.elapsed().as_millis() as u64,
                        first_token_ms,
                    };
                    self.account_goal(&statistics)?;
                    let _ = tx
                        .send(Ok(AgentEvent::ModelCall {
                            id: uuid::Uuid::new_v4().to_string(),
                            statistics,
                        }))
                        .await;
                    if let Some(usage) = usage {
                        let _ = tx
                            .send(Ok(AgentEvent::ContextUsage {
                                tokens: usage.input_tokens,
                                // The window arrives with the event's other
                                // half below; the host fills it when it maps.
                                window: None,
                            }))
                            .await;
                    }
                    built = Some(message)
                }
            }
        }
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        Ok(built)
    }

    /// Safe consecutive calls overlap; every unsafe call is an ordering barrier.
    async fn run_tools(
        &self,
        calls: &[(String, String, serde_json::Value)],
        cancel: &CancellationToken,
        tx: &mpsc::Sender<Result<AgentEvent, AgentError>>,
    ) -> Result<Vec<ContentBlock>, AgentError> {
        const CONCURRENCY: usize = 8;
        let safe = |call: &(String, String, serde_json::Value)| {
            self.hooks.allows_concurrent_tools()
                && call.1 != crate::tools::ASK_USER_QUESTION
                && self.tools.get(&call.1).is_some_and(|tool| tool.is_concurrency_safe(&call.2))
        };
        let mut results = Vec::with_capacity(calls.len());
        let mut start = 0;
        while start < calls.len() && !cancel.is_cancelled() {
            let mut end = start + 1;
            if safe(&calls[start]) {
                while end < calls.len() && safe(&calls[end]) { end += 1; }
            }
            // Drain started work even after one recording error. Dropping sibling
            // futures could lose side effects or leave a child process running.
            let jobs: Vec<futures::future::BoxFuture<'_, _>> = calls[start..end].iter().enumerate()
                .map(|(index, (id, name, input))| async move {
                    (index, self.run_tool(id, name, input, cancel, tx).await)
                }.boxed()).collect();
            let mut batch = futures::stream::iter(jobs).buffer_unordered(CONCURRENCY);
            let mut completed = Vec::new();
            let mut failure = None;
            while let Some((index, result)) = batch.next().await {
                match result {
                    Ok(Some(result)) => completed.push((index, result)),
                    Ok(None) => {},
                    Err(error) => { if failure.is_none() { failure = Some(error); } }
                }
            }
            if let Some(error) = failure { return Err(error); }
            completed.sort_by_key(|(index, _)| *index);
            results.extend(completed.into_iter().map(|(_, result)| result));
            start = end;
        }
        Ok(results)
    }

    async fn run_tool(
        &self,
        id: &str,
        name: &str,
        input: &serde_json::Value,
        cancel: &CancellationToken,
        tx: &mpsc::Sender<Result<AgentEvent, AgentError>>,
    ) -> Result<Option<ContentBlock>, AgentError> {
        if cancel.is_cancelled() { return Ok(None); }
        let _ = tx.send(Ok(AgentEvent::ToolCall {
            id: id.to_owned(), name: name.to_owned(), input: input.clone(),
        })).await;
        if cancel.is_cancelled() { return Ok(None); }
        let context = ToolContext {
            cwd: self.config.cwd.clone(), cancel: cancel.clone(),
            nested: Some(crate::tools::NestedEventSender::new(id.to_owned(), tx.clone())),
        };
        let pre = self.hooks.fire(crate::hook::HookEvent::PreToolUse,
            crate::hook::HookContext::for_tool(name, input, &self.config.cwd)).await;
        self.report_hook_failures(&pre);
        if cancel.is_cancelled() { return Ok(None); }
        let (output, is_error, diff) = if let Some(blocked) = pre.outcome.block() {
            (format!("A hook refused this call: {blocked}"), true, None)
        } else if name == crate::tools::ASK_USER_QUESTION {
            let (output, is_error) = self.ask_user(id, input, cancel, tx).await;
            (output, is_error, None)
        } else {
            let (output, is_error, diff) = match self.tools.call(name, input.clone(), &context).await {
                Ok(result) => (result.output, result.is_error, result.diff),
                Err(error) => (error.to_string(), true, None),
            };
            let post = self.fire_post_hooks(name, input, &output, is_error).await;
            let output = match post.outcome.context() {
                Some(note) => format!("{output}\n\n[hook] {note}"),
                None => output,
            };
            (output, is_error, diff)
        };
        let result = ContentBlock::ToolResult {
            tool_use_id: id.to_owned(), content: output.clone(), is_error,
        };
        // Keep completed side effects available to an in-process resume even
        // when the durable recorder fails. The settled batch replaces these
        // individual results in the normal path below.
        if let Some(sink) = &self.history_sink {
            sink.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(Message::tool_results(vec![result.clone()]));
        }
        self.record_message(&Message::tool_results(vec![result.clone()])).await?;
        let _ = tx.send(Ok(AgentEvent::ToolResult {
            id: id.to_owned(), is_error, output: Some(output), diff,
        })).await;
        Ok(Some(result))
    }

    /// Shrink the conversation when it is running out of room.
    ///
    /// Two stages, and the order is the point. Micro truncates oversized tool
    /// output without a model call, so it runs first and often. Full replaces
    /// the older half with a summary, which costs a call and cannot be undone
    /// from here — so it runs only when micro did not free enough, and only
    /// once the conversation is nearly full.
    ///
    /// A compaction that fails is reported and then ignored: the run continues
    /// with what it has, and the provider's own error is a better description
    /// of the problem than anything invented here. Refusing to continue would
    /// turn "the summary call failed" into "your conversation is dead".
    async fn compact_if_needed(
        &self,
        messages: &mut Vec<Message>,
        tx: &mpsc::Sender<Result<AgentEvent, AgentError>>,
    ) {
        let Some(config) = self.config.compact.as_ref() else {
            return;
        };
        if !crate::compact::should_micro_compact(messages, config) {
            return;
        }

        // Micro first, always: it is free, and on the conversations that are
        // long because of tool output it is the whole fix.
        let shortened = crate::compact::micro_compact(messages, config);
        if shortened > 0 {
            let _ = tx
                .send(Ok(AgentEvent::Compacted {
                    stage: crate::CompactionStage::Micro,
                    shortened,
                }))
                .await;
        }

        // Only escalate if the conversation is still nearly full after micro
        // did its work. This is what keeps a tool-heavy conversation from
        // spending a summarization call it did not need.
        if !crate::compact::should_compact(messages, config) {
            return;
        }

        // Rendering the older half as the text a summary is built from. The
        // system prompt is excluded: it is the host's standing instruction and
        // is re-sent on every request, so summarizing it would both waste the
        // call and risk losing it.
        let without_system: Vec<Message> = messages
            .iter()
            .filter(|message| message.role != crate::message::MessageRole::System)
            .cloned()
            .collect();
        let outcome = crate::compact::full_compact(without_system, config, |transcript| async {
            self.host.summarize(transcript).await
                .map_err(|error| crate::compact::CompactError::Summarize(error.to_string()))
        }).await;
        let outcome = match outcome {
            Ok(outcome) if outcome.compacted => outcome,
            Ok(_) => {
                let _ = tx.send(Ok(AgentEvent::Compacted {
                    stage: crate::CompactionStage::Exhausted, shortened: 0,
                })).await;
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "compaction failed; continuing with current history");
                let _ = tx.send(Ok(AgentEvent::Compacted {
                    stage: crate::CompactionStage::Failed, shortened: 0,
                })).await;
                return;
            }
        };
        let keep_from = outcome.replaced;
        let mut rebuilt: Vec<Message> = messages.iter()
            .filter(|message| message.role == crate::message::MessageRole::System)
            .cloned().collect();
        rebuilt.extend(outcome.messages);
        *messages = rebuilt;
        let result = self.hooks.fire(crate::hook::HookEvent::SessionStart, crate::hook::HookContext {
            cwd: self.config.cwd.clone(), source: Some("compact".into()), ..Default::default()
        }).await;
        self.report_hook_failures(&result);
        if let Some(context) = result.outcome.context() { messages.push(Message::user(context)); }


        let _ = tx
            .send(Ok(AgentEvent::Compacted {
                stage: crate::CompactionStage::Full,
                shortened: keep_from,
            }))
            .await;
    }

    /// Fire the hooks that run before the first model call.
    ///
    /// `SessionStart` first, then `UserPromptSubmit`: a session-scoped hook
    /// (project conventions, an environment probe) should have run before a
    /// prompt-scoped one reads what it produced.
    ///
    /// Returns `false` when a hook refused, which ends the run before the model
    /// is called. Reporting the refusal without stopping would be the worst
    /// outcome: the user sees "refused" and the model answers anyway.
    async fn fire_start_hooks(
        &self,
        messages: &mut Vec<Message>,
        tx: &mpsc::Sender<Result<AgentEvent, AgentError>>,
    ) -> bool {
        let prompt = messages
            .iter()
            .rev()
            .find(|message| message.is_user_input())
            .and_then(|message| {
                let text = message.content.text_content();
                (!text.is_empty()).then_some(text)
            });

        for event in [
            crate::hook::HookEvent::SessionStart,
            crate::hook::HookEvent::UserPromptSubmit,
        ] {
            let result = self
                .hooks
                .fire(
                    event,
                    crate::hook::HookContext {
                        prompt: prompt.clone(),
                        cwd: self.config.cwd.clone(),
                        ..Default::default()
                    },
                )
                .await;
            self.report_hook_failures(&result);
            if let Some(blocked) = result.outcome.block() {
                // Surfaced as text the user reads rather than a tool error:
                // there is no tool call to attach a refusal to.
                let _ = tx
                    .send(Ok(AgentEvent::TextDelta {
                        text: format!("A hook refused this message: {blocked}"),
                    }))
                    .await;
                return false;
            }
            if let Some(context) = result.outcome.context() {
                messages.push(Message::user(format!("<hook-context event=\"{}\">\n{context}\n</hook-context>", event.as_str())));
            }
        }
        true
    }

    /// Fire `PostToolUse`, plus `PostToolUseFailure` when the call failed.
    async fn fire_post_hooks(
        &self,
        name: &str,
        input: &serde_json::Value,
        output: &str,
        failed: bool,
    ) -> crate::hook::HookResult {
        let context = crate::hook::HookContext::for_tool(name, input, &self.config.cwd)
            .with_result(output, failed);

        let mut result = self
            .hooks
            .fire(crate::hook::HookEvent::PostToolUse, context.clone())
            .await;
        self.report_hook_failures(&result);

        if failed {
            let failure_result = self
                .hooks
                .fire(crate::hook::HookEvent::PostToolUseFailure, context)
                .await;
            self.report_hook_failures(&failure_result);
            result.outcome = result.outcome.merge(failure_result.outcome);
        }
        result
    }

    /// Fire `Stop` as the run ends.
    ///
    /// A failure here is reported and otherwise ignored: the run is over, so
    /// there is nothing left for a refusal to prevent.
    async fn fire_stop_hook(&self, reason: Option<String>) {
        let result = self
            .hooks
            .fire(
                crate::hook::HookEvent::Stop,
                crate::hook::HookContext {
                    cwd: self.config.cwd.clone(),
                    stop_reason: reason,
                    ..Default::default()
                },
            )
            .await;
        self.report_hook_failures(&result);
    }

    /// Log the hooks that did not run.
    ///
    /// Deliberately not surfaced to the model: a hook that failed to start is
    /// the user's configuration problem, not something the model can act on,
    /// and telling it would invite it to try to "fix" a shell script.
    fn report_hook_failures(&self, result: &crate::hook::HookResult) {
        for failure in &result.failures {
            tracing::warn!(
                event = failure.event.as_str(),
                message = %failure.message,
                "a hook did not run"
            );
        }
    }

    /// Raise the model's questions on the host's bridge and render the answers.
    async fn ask_user(
        &self,
        call_id: &str,
        input: &serde_json::Value,
        cancel: &CancellationToken,
        tx: &mpsc::Sender<Result<AgentEvent, AgentError>>,
    ) -> (String, bool) {
        if let Err(error) = crate::tools::validate_questions(input) {
            return (error.to_string(), true);
        }
        // Typed at the source: the tool knows what it asked, so it decodes its
        // own payload rather than the loop guessing at the JSON.
        let questions = crate::tools::ask_user::decode(input);
        if questions.is_empty() {
            return ("the question payload could not be decoded".to_owned(), true);
        }

        let request = InputRequest::Questions {
            questions: questions.clone(),
        };
        let _ = tx
            .send(Ok(AgentEvent::InputRequested {
                id: call_id.to_owned(),
                request: request.clone(),
            }))
            .await;

        let answer = tokio::select! {
            biased;
            _ = cancel.cancelled() => return ("the run was cancelled".to_owned(), true),
            answer = self.host.ask(request) => answer,
        };
        match answer {
            Ok(InputAnswer::Questions { answers }) => (
                crate::tools::ask_user::render_answers(&questions, &answers),
                false,
            ),
            // The panel was dismissed, or the run ended while it was open.
            // "No answer" is the honest result; it is not a success.
            Ok(InputAnswer::Dismissed) => ("the user did not answer the question".to_owned(), true),
            Err(error) => (format!("the question could not be asked: {error}"), true),
        }
    }
}

fn receiver_stream<T: Send + 'static>(
    mut rx: mpsc::Receiver<T>,
) -> impl Stream<Item = T> + Send + 'static {
    futures::stream::poll_fn(move |cx| rx.poll_recv(cx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{MessageContent, MessageId};
    use crate::tool::{Tool, ToolError, ToolResult};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A tool that records its calls.
    struct EchoTool {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "Echo"
        }
        fn description(&self) -> &str {
            "echoes its input"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }
        async fn call(
            &self,
            input: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolResult, ToolError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ToolResult {
                output: input.to_string(),
                is_error: false,
                diff: None,
            })
        }
    }

    /// A file-writing tool used to exercise direct execution.
    struct WriteTool;

    #[async_trait::async_trait]
    impl Tool for WriteTool {
        fn name(&self) -> &str {
            "Write"
        }
        fn description(&self) -> &str {
            "writes a file"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }
        async fn call(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult {
                output: "wrote".to_owned(),
                is_error: false,
                diff: None,
            })
        }
    }

    /// Answers a scripted list of model turns, then stops.
    struct ScriptedHost {
        turns: std::sync::Mutex<std::collections::VecDeque<Message>>,
        answers: std::sync::Mutex<std::collections::VecDeque<InputAnswer>>,
        asked: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl AgentHost for ScriptedHost {
        async fn reason(
            &self,
            _messages: &[Message],
            _tools: &[Arc<dyn Tool>],
        ) -> Result<BoxStream<'static, Result<ReasoningChunk, AgentError>>, AgentError> {
            let next = self.turns.lock().unwrap().pop_front();
            Ok(Box::pin(futures::stream::iter(match next {
                Some(message) => vec![
                    Ok(ReasoningChunk::Text("hi".to_owned())),
                    Ok(ReasoningChunk::Done {
                        message,
                        usage: None,
                    }),
                ],
                // Exhausted: end without a message, which the loop reports.
                None => vec![],
            })))
        }

        async fn ask(&self, _request: InputRequest) -> Result<InputAnswer, AgentError> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .answers
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(InputAnswer::Dismissed))
        }
    }

    fn assistant_with_tool(id: &str, call_id: &str) -> Message {
        Message {
            context: None,
            id: MessageId::new(),
            role: crate::message::MessageRole::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: call_id.to_owned(),
                name: id.to_owned(),
                input: serde_json::json!({ "file_path": "src/main.rs" }),
            }]),
        }
    }

    fn assistant_text() -> Message {
        Message {
            context: None,
            id: MessageId::new(),
            role: crate::message::MessageRole::Assistant,
            content: MessageContent::text("all done"),
        }
    }

    async fn collect(runner: Arc<AgentRunner>) -> Vec<Result<AgentEvent, AgentError>> {
        runner
            .run("go".to_owned(), CancellationToken::new())
            .collect()
            .await
    }

    #[tokio::test]
    async fn dropping_the_event_consumer_cancels_an_active_tool_and_allows_cleanup() {
        struct WaitingTool {
            started: Arc<tokio::sync::Notify>,
            cleaned: Arc<tokio::sync::Notify>,
        }
        #[async_trait::async_trait]
        impl Tool for WaitingTool {
            fn name(&self) -> &str { "Wait" }
            fn description(&self) -> &str { "wait for cancellation" }
            fn input_schema(&self) -> serde_json::Value { serde_json::json!({"type":"object"}) }
            async fn call(&self, _: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
                self.started.notify_one();
                ctx.cancel.cancelled().await;
                // Asynchronous cleanup still runs after the consumer disappears.
                tokio::task::yield_now().await;
                self.cleaned.notify_one();
                Err(ToolError::Cancelled)
            }
        }
        let started = Arc::new(tokio::sync::Notify::new());
        let cleaned = Arc::new(tokio::sync::Notify::new());
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(WaitingTool { started: started.clone(), cleaned: cleaned.clone() }));
        let host = Arc::new(ScriptedHost {
            turns: std::sync::Mutex::new([assistant_with_tool("Wait", "wait")].into()),
            answers: Default::default(),
            asked: AtomicUsize::new(0),
        });
        let runner = Arc::new(AgentRunner::new(host, Arc::new(tools), AgentConfig::default()));
        let caller_cancel = CancellationToken::new();
        let stream = runner.run("go".into(), caller_cancel.clone());
        tokio::time::timeout(std::time::Duration::from_secs(2), started.notified()).await.unwrap();
        drop(stream);
        tokio::time::timeout(std::time::Duration::from_secs(2), cleaned.notified()).await.expect("orphaned tool did not cancel and finish cleanup");
        assert!(!caller_cancel.is_cancelled(), "a disappeared consumer cancels only its own run");
    }

    /// A host that reports token counts on its first call only.
    struct UsageHost;

    #[async_trait::async_trait]
    impl AgentHost for UsageHost {
        async fn reason(
            &self,
            _messages: &[Message],
            _tools: &[Arc<dyn Tool>],
        ) -> Result<BoxStream<'static, Result<ReasoningChunk, AgentError>>, AgentError> {
            Ok(Box::pin(futures::stream::iter(vec![Ok(
                ReasoningChunk::Done {
                    message: assistant_text(),
                    usage: Some(crate::CallUsage {
                        input_tokens: 1_234,
                        output_tokens: 56,
                        ..Default::default()
                    }),
                },
            )])))
        }

        async fn ask(&self, _request: InputRequest) -> Result<InputAnswer, AgentError> {
            Ok(InputAnswer::Dismissed)
        }
    }

    #[tokio::test]
    async fn statistics_emits_billing_separately_from_context() {
        let runner = Arc::new(AgentRunner::new(Arc::new(UsageHost), Arc::new(ToolRegistry::new()), AgentConfig {
            model: "provider/model".into(), ..Default::default()
        }));
        let events = collect(runner).await;
        let calls: Vec<_> = events.iter().filter_map(|event| match event {
            Ok(AgentEvent::ModelCall { id, statistics }) => Some((id, statistics)), _ => None,
        }).collect();
        assert_eq!(calls.len(), 1);
        assert!(!calls[0].0.is_empty());
        assert_eq!(calls[0].1.model, "provider/model");
        assert_eq!(calls[0].1.input_tokens, Some(1234));
        assert_eq!(calls[0].1.output_tokens, Some(56));
        assert_eq!(calls[0].1.cache_read_tokens, None);
        assert_eq!(calls[0].1.first_token_ms, None);
    }

    #[tokio::test]
    async fn statistics_times_tool_output_from_request_start_to_completion() {
        struct TimedHost;
        #[async_trait::async_trait]
        impl AgentHost for TimedHost {
            async fn reason(&self, _: &[Message], _: &[Arc<dyn Tool>]) -> Result<BoxStream<'static, Result<ReasoningChunk, AgentError>>, AgentError> {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                Ok(Box::pin(futures::stream::iter([
                    ReasoningChunk::Activity,
                    ReasoningChunk::Done { message: assistant_text(), usage: Some(crate::CallUsage { input_tokens: 100, output_tokens: 10, ..Default::default() }) },
                ]).then(|chunk| async move {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    Ok(chunk)
                })))
            }
            async fn ask(&self, _: InputRequest) -> Result<InputAnswer, AgentError> { Ok(InputAnswer::Dismissed) }
        }
        let runner = Arc::new(AgentRunner::new(Arc::new(TimedHost), Arc::new(ToolRegistry::new()), AgentConfig::default()));
        let events = collect(runner).await;
        let stats = events.iter().find_map(|event| match event { Ok(AgentEvent::ModelCall { statistics, .. }) => Some(statistics), _ => None }).unwrap();
        assert!(stats.first_token_ms.unwrap() >= 5, "the timer must include request setup");
        assert!(stats.duration_ms > stats.first_token_ms.unwrap(), "decoding finishes after the first output");
    }

    #[tokio::test]
    async fn a_call_reports_its_context_usage() {
        // The meter and the compaction thresholds read this number, so a run
        // that quietly drops it leaves the UI blank and the loop guessing.
        let runner = Arc::new(AgentRunner::new(
            Arc::new(UsageHost),
            Arc::new(ToolRegistry::new()),
            AgentConfig::default(),
        ));

        let events = collect(runner).await;
        let usage: Vec<&AgentEvent> = events
            .iter()
            .filter_map(|item| match item {
                Ok(event @ AgentEvent::ContextUsage { .. }) => Some(event),
                _ => None,
            })
            .collect();
        assert_eq!(usage.len(), 1, "{events:?}");
        match usage[0] {
            AgentEvent::ContextUsage { tokens, window } => {
                assert_eq!(*tokens, 1_234);
                // The loop does not know the model's window; the host fills it.
                assert_eq!(*window, None);
            }
            other => panic!("not a usage event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_plain_answer_finishes_without_calling_tools() {
        let host = Arc::new(ScriptedHost {
            turns: std::sync::Mutex::new([assistant_text()].into()),
            answers: Default::default(),
            asked: AtomicUsize::new(0),
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            calls: calls.clone(),
        }));
        let runner = Arc::new(AgentRunner::new(
            host,
            Arc::new(registry),
            AgentConfig::default(),
        ));

        let events = collect(runner).await;
        assert!(matches!(
            events.last(),
            Some(Ok(AgentEvent::Done {
                reason: DoneReason::Complete
            }))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_tool_call_runs_and_the_loop_continues() {
        let host = Arc::new(ScriptedHost {
            turns: std::sync::Mutex::new(
                [assistant_with_tool("Echo", "call-1"), assistant_text()].into(),
            ),
            answers: Default::default(),
            asked: AtomicUsize::new(0),
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            calls: calls.clone(),
        }));
        let runner = Arc::new(AgentRunner::new(
            host,
            Arc::new(registry),
            AgentConfig::default(),
        ));

        let events = collect(runner).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the tool ran once");
        assert!(events.iter().any(|event| matches!(
            event,
            Ok(AgentEvent::ToolCall { name, .. }) if name == "Echo"
        )));
        assert!(matches!(
            events.last(),
            Some(Ok(AgentEvent::Done {
                reason: DoneReason::Complete
            }))
        ));
    }

    #[tokio::test]
    async fn tools_execute_without_requesting_user_input() {
        let host = Arc::new(ScriptedHost {
            turns: std::sync::Mutex::new(
                [assistant_with_tool("Write", "call-1"), assistant_text()].into(),
            ),
            answers: Default::default(),
            asked: AtomicUsize::new(0),
        });
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(WriteTool));
        let config = AgentConfig::default();
        let runner = Arc::new(AgentRunner::new(host.clone(), Arc::new(registry), config));

        let events = collect(runner).await;
        assert_eq!(host.asked.load(Ordering::SeqCst), 0, "nothing asked");
        assert!(events.iter().any(|event| matches!(
            event,
            Ok(AgentEvent::ToolResult {
                is_error: false,
                ..
            })
        )));
    }

    struct QuestionHost {
        scripted: ScriptedHost,
        requested: mpsc::UnboundedSender<InputRequest>,
        answer: std::sync::Mutex<Option<tokio::sync::oneshot::Receiver<InputAnswer>>>,
    }

    #[async_trait::async_trait]
    impl AgentHost for QuestionHost {
        async fn reason(
            &self,
            messages: &[Message],
            tools: &[Arc<dyn Tool>],
        ) -> Result<BoxStream<'static, Result<ReasoningChunk, AgentError>>, AgentError> {
            self.scripted.reason(messages, tools).await
        }

        async fn ask(&self, request: InputRequest) -> Result<InputAnswer, AgentError> {
            self.requested.send(request).expect("question receiver");
            let answer = self.answer.lock().unwrap().take().expect("one question");
            Ok(answer.await.unwrap_or(InputAnswer::Dismissed))
        }
    }

    async fn question_round_trip(answer: InputAnswer) -> Vec<Result<AgentEvent, AgentError>> {
        question_payload_round_trip(serde_json::json!({
            "questions": [{"id":"approach", "question":"Which approach?", "options":["First", "Second"]}]
        }), answer).await
    }

    async fn question_payload_round_trip(payload: serde_json::Value, answer: InputAnswer) -> Vec<Result<AgentEvent, AgentError>> {
        let mut question = assistant_with_tool(crate::tools::ASK_USER_QUESTION, "question-1");
        question.content = MessageContent::Blocks(vec![ContentBlock::ToolUse {
            id: "question-1".into(),
            name: crate::tools::ASK_USER_QUESTION.into(),
            input: payload,
        }]);
        let (requested, mut requests) = mpsc::unbounded_channel();
        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel();
        let host = Arc::new(QuestionHost {
            scripted: ScriptedHost {
                turns: std::sync::Mutex::new([question, assistant_text()].into()),
                answers: Default::default(),
                asked: AtomicUsize::new(0),
            },
            requested,
            answer: std::sync::Mutex::new(Some(answer_rx)),
        });
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(crate::tools::AskUserQuestionTool));
        let runner = Arc::new(AgentRunner::new(
            host,
            Arc::new(tools),
            AgentConfig::default(),
        ));
        let run = tokio::spawn(collect(runner));
        let request = tokio::time::timeout(std::time::Duration::from_secs(2), requests.recv())
            .await
            .expect("question surfaced")
            .expect("request");
        let InputRequest::Questions { questions } = request;
        assert_eq!(questions[0].question, "Which approach?");
        assert!(!run.is_finished(), "the loop must wait for the user");
        answer_tx
            .send(answer)
            .expect("the question is still pending");
        tokio::time::timeout(std::time::Duration::from_secs(2), run)
            .await
            .expect("the answer resumes the loop")
            .expect("run task")
    }

    #[tokio::test]
    async fn observed_object_choice_calls_wait_for_input_and_resume_with_the_answer() {
        for payload in [
            serde_json::json!({"questions":[{"question":"Which approach?", "options":[
                {"description":"First"}, {"description":"Second"}]}]}),
            serde_json::json!({"questions":[{"options":[
                {"question":"Which approach?", "prefill":"First", "description":"Keep data"},
                {"question":"Which approach?", "prefill":"Second", "description":"Remove data"}]}]}),
            serde_json::json!({"questions":[{"question":"Which approach?", "multiSelect":true, "options":[
                {"label":"First", "description":"Keep data"}, {"label":"Second", "description":"Remove data"}]}]}),
        ] {
            let events = question_payload_round_trip(payload, InputAnswer::Questions {
                answers: vec![rcode_proto::UserInputAnswer {
                    question_id:"q0".into(), selected_option_ids:vec!["0:1".into()], text:None,
                }],
            }).await;
            assert!(events.iter().any(|event| matches!(event,
                Ok(AgentEvent::ToolResult { is_error:false, output:Some(output), .. }) if output.contains("Second")
            )));
            assert!(matches!(events.last(), Some(Ok(AgentEvent::Done { reason:DoneReason::Complete }))));
        }
    }

    #[tokio::test]
    async fn a_question_waits_for_the_user_and_returns_their_answer() {
        let events = question_round_trip(InputAnswer::Questions {
            answers: vec![rcode_proto::UserInputAnswer {
                question_id: "q0".into(),
                selected_option_ids: vec!["0:1".into()],
                text: Some("with my changes".into()),
            }],
        })
        .await;
        assert!(events.iter().any(|event| matches!(event,
            Ok(AgentEvent::ToolResult { id, output: Some(output), is_error: false, .. })
                if id == "question-1" && output.contains("Second") && output.contains("with my changes")
        )));
        assert!(matches!(
            events.last(),
            Some(Ok(AgentEvent::Done {
                reason: DoneReason::Complete
            }))
        ));
    }

    #[tokio::test]
    async fn dismissing_a_question_returns_no_answer_and_resumes() {
        let events = question_round_trip(InputAnswer::Dismissed).await;
        assert!(events.iter().any(|event| matches!(event,
            Ok(AgentEvent::ToolResult { id, output: Some(output), is_error: true, .. })
                if id == "question-1" && output.contains("did not answer")
        )));
        assert!(matches!(
            events.last(),
            Some(Ok(AgentEvent::Done {
                reason: DoneReason::Complete
            }))
        ));
    }

    #[tokio::test]
    async fn cancelling_before_the_first_step_stops_immediately() {
        let host = Arc::new(ScriptedHost {
            turns: std::sync::Mutex::new([assistant_text()].into()),
            answers: Default::default(),
            asked: AtomicUsize::new(0),
        });
        let runner = Arc::new(AgentRunner::new(
            host,
            Arc::new(ToolRegistry::new()),
            AgentConfig::default(),
        ));

        let cancel = CancellationToken::new();
        cancel.cancel();
        let events: Vec<_> = runner.run("go".to_owned(), cancel).collect().await;
        assert!(matches!(
            events.last(),
            Some(Ok(AgentEvent::Done {
                reason: DoneReason::Interrupted
            }))
        ));
    }

    enum CancelPoint {
        Reason,
        Stream,
        Question,
        Eof,
        Error,
    }

    struct CancellationHost {
        point: CancelPoint,
        entered: Arc<tokio::sync::Notify>,
        cancel: CancellationToken,
    }

    #[async_trait::async_trait]
    impl AgentHost for CancellationHost {
        async fn reason(
            &self,
            _messages: &[Message],
            _tools: &[Arc<dyn Tool>],
        ) -> Result<BoxStream<'static, Result<ReasoningChunk, AgentError>>, AgentError> {
            match self.point {
                CancelPoint::Reason => {
                    self.entered.notify_one();
                    futures::future::pending().await
                }
                CancelPoint::Stream | CancelPoint::Eof | CancelPoint::Error => {
                    let entered = self.entered.clone();
                    let cancel = self.cancel.clone();
                    let eof = matches!(self.point, CancelPoint::Eof);
                    let error = matches!(self.point, CancelPoint::Error);
                    Ok(Box::pin(futures::stream::poll_fn(move |_cx| {
                        entered.notify_one();
                        if eof || error {
                            cancel.cancel();
                            if error {
                                return std::task::Poll::Ready(Some(Err(AgentError::Model(
                                    "cancelled transport".into(),
                                ))));
                            }
                            return std::task::Poll::Ready(None);
                        }
                        std::task::Poll::Pending
                    })))
                }
                CancelPoint::Question => {
                    let mut message = assistant_with_tool("Write", "call-1");
                    if matches!(self.point, CancelPoint::Question) {
                        message.content = MessageContent::Blocks(vec![ContentBlock::ToolUse {
                            id: "question-1".into(),
                            name: crate::tools::ASK_USER_QUESTION.into(),
                            input: serde_json::json!({
                                "questions": [{
                                    "id": "continue", "question": "Continue?",
                                    "options": ["Continue", "Stop"]
                                }]
                            }),
                        }]);
                    }
                    Ok(Box::pin(futures::stream::iter([Ok(
                        ReasoningChunk::Done {
                            message,
                            usage: None,
                        },
                    )])))
                }
            }
        }

        async fn ask(&self, _request: InputRequest) -> Result<InputAnswer, AgentError> {
            self.entered.notify_one();
            futures::future::pending().await
        }
    }

    async fn cancellation_at(point: CancelPoint) {
        let cancel = CancellationToken::new();
        let entered = Arc::new(tokio::sync::Notify::new());
        let host = Arc::new(CancellationHost {
            point,
            entered: entered.clone(),
            cancel: cancel.clone(),
        });
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(WriteTool));
        tools.register(Arc::new(crate::tools::AskUserQuestionTool));
        let runner = Arc::new(AgentRunner::new(
            host,
            Arc::new(tools),
            AgentConfig::default(),
        ));
        let mut stream = runner.run("go".into(), cancel.clone());
        tokio::time::timeout(std::time::Duration::from_secs(2), entered.notified())
            .await
            .expect("the intended wait was reached");
        cancel.cancel();
        let events: Vec<_> = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let mut events = Vec::new();
            while let Some(event) = stream.next().await {
                events.push(event.expect("readable"));
            }
            events
        })
        .await
        .expect("cancellation must reach Done and EOF");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, AgentEvent::Done { .. }))
                .count(),
            1
        );
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Done {
                reason: DoneReason::Interrupted
            })
        ));
        assert!(!events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolResult {
                is_error: false,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn cancelling_a_pending_model_start_finishes() {
        cancellation_at(CancelPoint::Reason).await;
    }

    #[tokio::test]
    async fn cancelling_a_pending_model_stream_finishes() {
        cancellation_at(CancelPoint::Stream).await;
    }

    #[tokio::test]
    async fn cancelling_a_pending_question_finishes() {
        cancellation_at(CancelPoint::Question).await;
    }

    #[tokio::test]
    async fn cancellation_with_model_eof_is_interrupted() {
        cancellation_at(CancelPoint::Eof).await;
    }

    #[tokio::test]
    async fn cancellation_with_model_error_is_interrupted() {
        cancellation_at(CancelPoint::Error).await;
    }

    // === Hooks: the loop must honour what the contract promises ===

    /// Records every event it was asked about, and answers from a script.
    ///
    /// The answers are keyed by event so a test can make `PreToolUse` block
    /// while everything else allows — proving the loop consults the right
    /// event rather than merely firing one.
    struct RecordingHooks {
        seen: std::sync::Mutex<Vec<crate::hook::HookEvent>>,
        answers: std::sync::Mutex<
            std::collections::HashMap<crate::hook::HookEvent, crate::hook::HookOutcome>,
        >,
    }

    impl RecordingHooks {
        fn new(answers: &[(crate::hook::HookEvent, crate::hook::HookOutcome)]) -> Arc<Self> {
            Arc::new(Self {
                seen: std::sync::Mutex::new(Vec::new()),
                answers: std::sync::Mutex::new(answers.iter().cloned().collect()),
            })
        }

        fn events(&self) -> Vec<crate::hook::HookEvent> {
            self.seen.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl crate::hook::HookRunner for RecordingHooks {
        async fn fire(
            &self,
            event: crate::hook::HookEvent,
            _context: crate::hook::HookContext,
        ) -> crate::hook::HookResult {
            self.seen.lock().unwrap().push(event);
            crate::hook::HookResult {
                outcome: self
                    .answers
                    .lock()
                    .unwrap()
                    .get(&event)
                    .cloned()
                    .unwrap_or_default(),
                failures: Vec::new(),
            }
        }
    }

    /// A runner over one scripted turn that calls `Echo` once, then answers.
    fn hooked_runner(
        hooks: Arc<dyn crate::hook::HookRunner>,
    ) -> (Arc<AgentRunner>, Arc<AtomicUsize>) {
        let host = Arc::new(ScriptedHost {
            turns: std::sync::Mutex::new(
                [assistant_with_tool("Echo", "call-1"), assistant_text()].into(),
            ),
            answers: Default::default(),
            asked: AtomicUsize::new(0),
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            calls: Arc::clone(&calls),
        }));
        let runner =
            AgentRunner::new(host, Arc::new(registry), AgentConfig::default()).with_hooks(hooks);
        (Arc::new(runner), calls)
    }

    #[tokio::test]
    async fn every_mount_point_fires() {
        let hooks = RecordingHooks::new(&[]);
        let (runner, _) = hooked_runner(hooks.clone());
        let _ = collect(runner).await;

        let events = hooks.events();
        for expected in [
            crate::hook::HookEvent::SessionStart,
            crate::hook::HookEvent::UserPromptSubmit,
            crate::hook::HookEvent::PreToolUse,
            crate::hook::HookEvent::PostToolUse,
            crate::hook::HookEvent::Stop,
        ] {
            assert!(
                events.contains(&expected),
                "{} did not fire; saw {events:?}",
                expected.as_str()
            );
        }
        // The call succeeded, so the failure event must not have fired.
        assert!(!events.contains(&crate::hook::HookEvent::PostToolUseFailure));
    }

    #[tokio::test]
    async fn a_pre_tool_use_block_stops_the_call_before_it_runs() {
        let hooks = RecordingHooks::new(&[(
            crate::hook::HookEvent::PreToolUse,
            crate::hook::HookOutcome::Block("no writes today".to_owned()),
        )]);
        let (runner, calls) = hooked_runner(hooks);
        let events = collect(runner).await;

        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "the tool ran despite a blocking PreToolUse hook"
        );
        let denied = events.iter().find_map(|event| match event {
            Ok(AgentEvent::ToolResult {
                is_error: true,
                output,
                ..
            }) => output.clone(),
            _ => None,
        });
        assert!(
            denied.is_some_and(|message| message.contains("no writes today")),
            "the model was not told why the call was refused: {events:?}"
        );
    }

    #[tokio::test]
    async fn a_post_tool_use_note_rides_the_result_the_model_sees() {
        let hooks = RecordingHooks::new(&[(
            crate::hook::HookEvent::PostToolUse,
            crate::hook::HookOutcome::Context("run the formatter".to_owned()),
        )]);
        let (runner, _) = hooked_runner(hooks);
        let events = collect(runner).await;

        let result = events.iter().find_map(|event| match event {
            Ok(AgentEvent::ToolResult { output, .. }) => output.clone(),
            _ => None,
        });
        assert!(
            result.is_some_and(|text| text.contains("run the formatter")),
            "the hook's note did not reach the tool result: {events:?}"
        );
    }

    #[tokio::test]
    async fn a_failed_call_fires_both_post_events() {
        // A tool that is not registered fails, which is the cheapest way to
        // produce a failure without a bespoke tool.
        let host = Arc::new(ScriptedHost {
            turns: std::sync::Mutex::new(
                [
                    assistant_with_tool("NoSuchTool", "call-1"),
                    assistant_text(),
                ]
                .into(),
            ),
            answers: Default::default(),
            asked: AtomicUsize::new(0),
        });
        let hooks = RecordingHooks::new(&[]);
        let runner = Arc::new(
            AgentRunner::new(host, Arc::new(ToolRegistry::new()), AgentConfig::default())
                .with_hooks(hooks.clone()),
        );
        let _ = collect(runner).await;

        let events = hooks.events();
        assert!(events.contains(&crate::hook::HookEvent::PostToolUse));
        assert!(
            events.contains(&crate::hook::HookEvent::PostToolUseFailure),
            "a failed call must fire the failure event too: {events:?}"
        );
    }

    #[tokio::test]
    async fn a_user_prompt_submit_block_stops_the_run_before_the_model() {
        let hooks = RecordingHooks::new(&[(
            crate::hook::HookEvent::UserPromptSubmit,
            crate::hook::HookOutcome::Block("not now".to_owned()),
        )]);
        let (runner, calls) = hooked_runner(hooks);
        let events = collect(runner).await;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Ok(AgentEvent::ToolCall { .. }))),
            "the run continued past a blocking UserPromptSubmit hook"
        );
    }

    #[tokio::test]
    async fn a_hook_failure_is_logged_and_does_not_stop_the_call() {
        // Failing open is the contract: a broken hook must not make the agent
        // unusable, so a tool call proceeds when the hook cannot run.
        struct BrokenHooks;
        #[async_trait::async_trait]
        impl crate::hook::HookRunner for BrokenHooks {
            async fn fire(
                &self,
                event: crate::hook::HookEvent,
                _context: crate::hook::HookContext,
            ) -> crate::hook::HookResult {
                crate::hook::HookResult {
                    outcome: crate::hook::HookOutcome::Allow,
                    failures: vec![crate::hook::HookFailure {
                        event,
                        message: "no such binary".to_owned(),
                    }],
                }
            }
        }

        let (runner, calls) = hooked_runner(Arc::new(BrokenHooks));
        let _ = collect(runner).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a failed hook blocked a tool call it had no opinion about"
        );
    }
}

#[cfg(test)]
#[path = "runner_compaction_tests.rs"]
mod compaction_tests;

#[cfg(test)]
#[path = "runner_tool_tests.rs"]
mod tool_tests;

#[cfg(test)]
#[path = "runner_context_tests.rs"]
mod context_tests;
