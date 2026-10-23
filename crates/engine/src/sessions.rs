//! SessionsEngine — per-chat agent runs: dispatch, steering, interrupts, input bridging,
//! journal + broadcast fan-out, and 120ms coalesced doc streaming.
//!
//! Pragmatic port of the original app's `sessions.ts` (spec: ARCHITECTURE.md):
//! - every `AgentEvent` is (a) appended to the on-disk run journal, (b) broadcast to
//!   in-process subscribers, (c) folded via `fold_event_into_parts` and diffed into the
//!   chat's `SessionDoc` through `SegmentWriter` on a coalesced `STREAM_COMMIT_MS` timer;
//! - the user message entry is pushed to the doc immediately on dispatch (id = the
//!   command's client-minted message id, so optimistic echoes never flicker);
//! - a `Steered` event splits the assistant entry at the exact boundary;
//! - recovery (interrupt or a stale journal at boot) stamps the streaming entry `aborted`.
//!
//! Scope notes: sessions are keyed by chat id (one live run per chat). RCode's pulse
//! loop is ported as the 15s liveness heartbeat in `drive_run`; its stall watchdog is
//! deliberately NOT ported (rejected in review — agents may legitimately wait on
//! something for far longer than any timeout, and a live child IS the working signal).
//! Every dying path must instead carry its own visible error (child crash with stderr,
//! spawn failure, stream error, engine-restart recovery).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use chrono::Utc;
use futures::{FutureExt, StreamExt};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use rcode_doc::{
    DocError, MessagePart, MessageRole, MessageStatus, STREAM_COMMIT_MS, SegmentWriter, SessionDoc,
    SessionMessageEntry, fold_event_into_parts, sanitize_tool_call,
};
use rcode_harness::{CancellationToken, Harness, RunControls, SteerMessage, TurnControl};
use rcode_proto::{AgentEvent, DoneStatus, HarnessId, RunRequest, Session, SessionStatus};

use crate::doc_host::{ChatDocHandle, DocHost};
use crate::registry::{HarnessDescriptor, HarnessRegistry};
use crate::run_journal::RunJournal;
use crate::{EngineError, new_id, now_ms};

/// One journaled event: the durable seq plus the event, as broadcast to subscribers.
#[derive(Debug, Clone)]
pub struct JournaledEvent {
    pub seq: u64,
    pub event: AgentEvent,
}

/// How many of the most recent messages `/compact` leaves alone.
///
/// The point of compacting is to keep working, so the turns the user is
/// actively thinking about must survive. Six is a guess at "the last exchange
/// or two plus what it referenced"; it is a constant rather than a setting
/// because a wrong value costs a little context, not a broken conversation.
const COMPACT_KEEP_RECENT_TURNS: usize = 6;

/// Outcome of a steer attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteerOutcome {
    /// Delivered into the live run's steering mailbox.
    Accepted,
    /// No live steerable run — the caller should dispatch the prompt as a new turn.
    NotSteerable,
}

/// One question the run is parked on, waiting for the user.
///
/// Carries more than the channel the answer arrives on, and each field is for
/// a failure that has actually happened or would be silent otherwise:
///
/// - `run_id` — an answer for a run that has ended must not be applied to the
///   one that replaced it. The chat's runs map holds the *current* run, so a
///   stale answer would find a live run and resolve its request by id, which
///   cannot collide but can be *believed*: the id would simply not be found,
///   unless a new run re-minted the same id.
/// - `request` — an answer must match a pending question before consuming it.
/// - `resolver` — one-shot, so a second answer for the same request finds
///   nothing to send on.
struct PendingInput {
    run_id: String,
    request: rcode_proto::InteractionRequest,
    resolver: oneshot::Sender<rcode_proto::InteractionResponse>,
}

type PendingInputs = Arc<Mutex<HashMap<String, PendingInput>>>;

/// Whether `answer` is a reply to `request`.
///
/// A dismissal is handled by cancellation rather than consumed as an answer.
fn answers_request(
    request: &rcode_proto::InteractionRequest,
    answer: &rcode_proto::InteractionResponse,
) -> bool {
    matches!(
        (request, answer),
        (
            rcode_proto::InteractionRequest::Questions { .. },
            rcode_proto::InteractionResponse::Questions { .. }
        )
    )
}

/// A harness-native session id plus the cwd it was created under. Harness
/// session stores are cwd-scoped (claude keys conversations by project
/// directory — the original app sessions.ts:563 "harness session stores are keyed by
/// cwd"), so resume is only injected for runs launched from the same cwd.
#[derive(Debug, Clone)]
struct HarnessSessionRef {
    session_id: String,
    cwd: String,
}

/// The configuration a live harness runtime launched with. The steering
/// mailbox carries only the prompt (and its images), so a request whose
/// launch settings differ would silently run with the old process's — it
/// must replace the runtime and resume its harness-native session instead.
/// The driver decides what "differ" means ([`Harness::same_runtime`]): a
/// replacement kills the runtime's background work, so a setting the
/// launch never reads, or the same value spelled differently by another
/// client, must never force one.
#[derive(Clone)]
struct RuntimeConfig {
    harness_id: HarnessId,
    request: RunRequest,
    harness: Arc<dyn Harness>,
}

impl RuntimeConfig {
    fn new(harness_id: HarnessId, request: &RunRequest, harness: Arc<dyn Harness>) -> Self {
        Self {
            harness_id,
            request: request.clone(),
            harness,
        }
    }

    fn can_route(&self, harness_id: HarnessId, request: &RunRequest) -> bool {
        self.harness_id == harness_id && self.harness.same_runtime(&self.request, request)
    }

    /// How `request` reaches this runtime: `Some(false)` as is, `Some(true)`
    /// with its configuration adopted in place first
    /// ([`Harness::reconfigures_in_place`]), `None` only by replacing it.
    fn route(&self, harness_id: HarnessId, request: &RunRequest) -> Option<bool> {
        if self.can_route(harness_id, request) {
            Some(false)
        } else if self.harness_id == harness_id
            && self.harness.reconfigures_in_place(&self.request, request)
        {
            Some(true)
        } else {
            None
        }
    }
}

struct RunHandle {
    run_id: String,
    steerable: bool,
    runtime_config: RuntimeConfig,
    steer_tx: mpsc::Sender<SteerMessage>,
    /// Harness-level cancellation (protocol interrupt + child teardown).
    interrupt_token: CancellationToken,
    /// Turn-level stop + the runtime's background-work report. Used for
    /// stopping a turn when the driver [`Harness::stops_turn_in_place`].
    turn: TurnControl,
    stops_turn_in_place: bool,
    /// The harness stream is open. Before that there is no runtime to keep:
    /// a stop tears the pending start down instead.
    started: Arc<std::sync::atomic::AtomicBool>,
    engine_tx: mpsc::UnboundedSender<AgentEvent>,
    pending_inputs: PendingInputs,
    /// Steers accepted into the mailbox but not yet confirmed by a `Steered`
    /// event — the at-least-once ledger. A run can die with accepted steers
    /// still in its mailbox (idle reaper vs. a routed send; a mid-turn error
    /// discarding queued boundary steers): the run task drains this at exit
    /// and re-dispatches each entry as a fresh turn, so an accepted message
    /// can never silently evaporate from a transcript that shows it as sent.
    routed_steers: Arc<Mutex<std::collections::VecDeque<RoutedSteer>>>,
    /// This runtime's provider session holds the fork's copied history (a
    /// side chat's bootstrap went out on its run or on one of its steers).
    fork_history_sent: Arc<std::sync::atomic::AtomicBool>,
    /// The user stopped this runtime: it is being torn down and takes no
    /// more prompts. Flipped under the ledger lock, so every accepted steer
    /// is unambiguously before the stop (cancelled with it) or after it
    /// (re-dispatched to a fresh runtime — never lost).
    retiring: Arc<std::sync::atomic::AtomicBool>,
}

/// Kept until the driver AND its runtime actors finish, even if the prompt
/// routing slot has already been released for a subsequent turn.
#[derive(Clone)]
struct RunTask {
    run_id: String,
    interrupt: CancellationToken,
    /// Arms the driver's grace deadline even when the harness ignores its token.
    cancel: watch::Sender<bool>,
    abort: futures::future::AbortHandle,
    completed: watch::Receiver<bool>,
}

/// One accepted-but-unconfirmed steer: enough to re-dispatch it verbatim.
#[derive(Debug, Clone)]
struct RoutedSteer {
    prompt: String,
    message_id: String,
    /// The delivered text carried the fork's copied history: its delivery
    /// is recorded only when the runtime confirms consuming it (`Steered`).
    /// An orphan re-dispatches the bare prompt, still owing the history.
    fork_history: bool,
    /// Accepted after the user's Stop began tearing the runtime down: the
    /// Stop did not cancel it, so the message still runs.
    after_stop: bool,
    /// The run configuration it was sent with, images included: an orphan
    /// re-dispatches exactly this, not whatever the chat's latest request
    /// became. `None` when the chat had no recorded request.
    request: Option<Box<RunRequest>>,
}

struct Inner {
    device_id: String,
    /// The engine is shutting down: dying runs re-dispatch nothing.
    shutting_down: std::sync::atomic::AtomicBool,
    /// Per chat: messages a Stop cancelled while they were still on their
    /// way (see [`SessionsEngine::cancel_sent_messages`]).
    cancelled_sends: Mutex<HashMap<String, std::collections::HashSet<String>>>,
    journal: Arc<RunJournal>,
    registry: Arc<HarnessRegistry>,
    /// Set-once (first wins), cleared on runtime retirement: sessions and
    /// doc-host reference each other through Arcs, so this back-edge must be
    /// severable for a replaced engine graph to drop.
    doc_host: Mutex<Option<DocHost>>,
    /// Back-edge to the workspace, for post-run auto-titling: a chat row is
    /// named here, and the sessions engine is what knows a turn finished.
    /// Severable for the same reason as `doc_host`.
    workspace: Mutex<Option<crate::workspace_host::WorkspaceHost>>,
    /// 每个会话同时最多一个标题请求；锁同时保护关闭与迟到提交之间的边界。
    title_jobs: Mutex<HashMap<String, CancellationToken>>,
    /// chat_id → live run.
    runs: Mutex<HashMap<String, RunHandle>>,
    run_tasks: Mutex<HashMap<String, Vec<RunTask>>>,
    deleting_chats: Mutex<std::collections::HashSet<String>>,
    dispatch_locks: Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>,
    /// chat_id → broadcast hub (retained across runs so subscribers survive turns).
    hubs: Mutex<HashMap<String, broadcast::Sender<JournaledEvent>>>,
    statuses: Mutex<HashMap<String, Session>>,
    sessions_tx: watch::Sender<Vec<Session>>,
    /// Last dispatched request per chat — the steer→new-turn fallback re-derives its
    /// run config from this (chat config rows land with the workspace doc in M4).
    last_requests: Mutex<HashMap<String, RunRequest>>,
    goal_stores: Mutex<HashMap<String, std::sync::Weak<crate::goals::DocGoalStore>>>,
    /// Harness-native session ids per chat (resume continuity across turns) —
    /// the live-process cache over the durable copy on the workspace chat row
    /// (the original app kept the same pair on `chats.harness_session_id`). An empty
    /// session id is the "do not resume" tombstone after a rejected resume.
    harness_sessions: Mutex<HashMap<String, HarnessSessionRef>>,
    /// Generated-image intake: (uploads store, allowed source root).
    generated_images: OnceLock<(crate::uploads::Uploads, std::path::PathBuf)>,
    /// [`crate::Repos::scratch_root`]; bare tests fall back to
    /// [`crate::repos::fallback_scratch_root`].
    scratch_root: OnceLock<std::path::PathBuf>,
    /// Fired with `(chat_id, cwd)` when a user prompt starts a turn (fresh
    /// dispatch or accepted steer) — the diff sync snapshots the checkout tree
    /// for the Changes pane's "Latest turn" scope. Absent in bare tests.
    turn_listener: OnceLock<TurnListener>,
}

/// Turn-start hook: called with `(chat_id, cwd)`.
pub type TurnListener = Arc<dyn Fn(&str, &str) + Send + Sync>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone)]
pub struct SessionsEngine {
    inner: Arc<Inner>,
}

impl SessionsEngine {
    /// Read a full local result for the authenticated lazy-output RPC. The
    /// caller checks the part against the chat document before using it.
    pub(crate) fn journal_tool_result(
        &self,
        chat_id: &str,
        part_id: &str,
    ) -> Result<Option<AgentEvent>, EngineError> {
        for event in self.inner.journal.events_rev(chat_id)? {
            let (_, event) = event?;
            if matches!(&event, AgentEvent::ToolResult { id, .. } if id == part_id) {
                return Ok(Some(event));
            }
        }
        Ok(None)
    }

    pub fn new(
        device_id: String,
        journal: Arc<RunJournal>,
        registry: Arc<HarnessRegistry>,
    ) -> Self {
        let (sessions_tx, _) = watch::channel(Vec::new());
        Self {
            inner: Arc::new(Inner {
                device_id,
                shutting_down: std::sync::atomic::AtomicBool::new(false),
                cancelled_sends: Mutex::new(HashMap::new()),
                journal,
                registry,
                doc_host: Mutex::new(None),
                workspace: Mutex::new(None),
                title_jobs: Mutex::new(HashMap::new()),
                runs: Mutex::new(HashMap::new()),
                run_tasks: Mutex::new(HashMap::new()),
                deleting_chats: Mutex::new(Default::default()),
                dispatch_locks: Mutex::new(HashMap::new()),
                hubs: Mutex::new(HashMap::new()),
                statuses: Mutex::new(HashMap::new()),
                sessions_tx,
                last_requests: Mutex::new(HashMap::new()),
                goal_stores: Mutex::new(HashMap::new()),
                harness_sessions: Mutex::new(HashMap::new()),
                generated_images: OnceLock::new(),
                scratch_root: OnceLock::new(),
                turn_listener: OnceLock::new(),
            }),
        }
    }

    /// Wire the doc host (called once at engine assembly; the two services are mutually
    /// referential by design — sessions stream into docs, docs execute commands here).
    pub fn set_doc_host(&self, host: DocHost) {
        // First set wins (the OnceLock contract this slot replaced).
        let mut slot = lock(&self.inner.doc_host);
        if slot.is_none() {
            *slot = Some(host);
        }
    }

    /// Sever the doc-host back-edge (runtime retirement; the doc host's
    /// `shutdown_workers` clears its own sessions edge). Every access site
    /// already treats a missing doc host as "not wired".
    pub fn clear_doc_host(&self) {
        lock(&self.inner.doc_host).take();
    }

    /// Wire the workspace (called once at engine assembly). Post-run
    /// auto-titling needs it: the sessions engine sees the turn finish, the
    /// workspace owns the chat row the title lands on.
    pub fn set_workspace(&self, workspace: crate::workspace_host::WorkspaceHost) {
        let mut slot = lock(&self.inner.workspace);
        if slot.is_none() {
            *slot = Some(workspace);
        }
    }

    /// Sever the workspace back-edge (runtime retirement; same shape as
    /// [`Self::clear_doc_host`]).
    pub fn clear_workspace(&self) {
        let jobs = lock(&self.inner.title_jobs);
        for cancel in jobs.values() {
            cancel.cancel();
        }
        lock(&self.inner.workspace).take();
    }

    /// Bind generated-image intake to the same profile store used by attachment RPCs.
    /// Where projectless chats' scratch folders resolve (set once at boot).
    pub(crate) fn set_scratch_root(&self, root: std::path::PathBuf) {
        let _ = self.inner.scratch_root.set(root);
    }

    pub fn set_generated_images(
        &self,
        uploads: crate::uploads::Uploads,
        allowed_root: std::path::PathBuf,
    ) {
        let _ = self.inner.generated_images.set((uploads, allowed_root));
    }

    /// Wire the turn-start listener (called once at engine assembly).
    pub fn set_turn_listener(&self, listener: TurnListener) {
        let _ = self.inner.turn_listener.set(listener);
    }

    fn note_turn_start(&self, chat_id: &str, cwd: &str) {
        if let Some(listener) = self.inner.turn_listener.get() {
            listener(chat_id, cwd);
        }
    }

    fn doc_handle(&self, chat_id: &str) -> Result<Arc<ChatDocHandle>, EngineError> {
        let host = self
            .inner
            .doc_host()
            .ok_or_else(|| EngineError::Other("doc host not wired into sessions engine".into()))?;
        host.open(chat_id)
    }

    fn dispatch_lock(&self, chat_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = lock(&self.inner.dispatch_locks);
        if let Some(existing) = locks.get(chat_id).and_then(std::sync::Weak::upgrade) {
            return existing;
        }
        let current = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(chat_id.to_owned(), Arc::downgrade(&current));
        current
    }

    fn ensure_chat_open(&self, chat_id: &str) -> Result<(), EngineError> {
        if lock(&self.inner.deleting_chats).contains(chat_id)
            || self.inner.shutting_down.load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(EngineError::Other(format!("chat `{chat_id}` is closing or deleted")));
        }
        if self.inner.workspace().is_some_and(|workspace| workspace.chat_is_closing(chat_id).unwrap_or(true)) {
            return Err(EngineError::Other(format!("chat `{chat_id}` or its project is closing or deleted")));
        }
        Ok(())
    }

    /// Closing is permanent for this chat id. It precedes every await, so a
    /// queued prompt, retry, background wake or late steer cannot reopen it.
    pub(crate) fn begin_chat_deletion(&self, chat_id: &str) {
        lock(&self.inner.deleting_chats).insert(chat_id.to_owned());
        if let Some(cancel) = lock(&self.inner.title_jobs).get(chat_id) {
            cancel.cancel();
        }
        if let Some(tasks) = lock(&self.inner.run_tasks).get(chat_id) {
            for task in tasks {
                let _ = task.cancel.send(true);
                task.interrupt.cancel();
            }
        }
    }

    pub(crate) async fn settle_chat_deletion(
        &self,
        chat_id: &str,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, EngineError> {
        self.begin_chat_deletion(chat_id);
        let gate = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.dispatch_lock(chat_id).lock_owned(),
        )
        .await
        .map_err(|_| EngineError::Other(format!("chat deletion incomplete: `{chat_id}` still has a pending dispatch")))?;
        self.terminate(chat_id).await?;
        Ok(gate)
    }

    pub(crate) fn forget_deleted_chat(&self, chat_id: &str) {
        lock(&self.inner.last_requests).remove(chat_id);
        lock(&self.inner.harness_sessions).remove(chat_id);
        lock(&self.inner.goal_stores).remove(chat_id);
        let mut statuses = lock(&self.inner.statuses);
        statuses.remove(chat_id);
        let mut rows: Vec<_> = statuses.values().cloned().collect();
        rows.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
        self.inner.sessions_tx.send_replace(rows);
    }

    /// The conversation so far, for a harness that replays history itself.
    ///
    /// The native agent is stateless: rather than resuming a provider-side
    /// session it is handed the whole conversation and sends it every turn
    /// (the original project does the same — see the design note on `ConversationTurn`). This
    /// is where that conversation comes from: the chat's own document, which
    /// is already the authority for what the user sees.
    ///
    /// `exclude` is the message being dispatched. It was written to the doc
    /// just before this call, and it is passed to the model separately as the
    /// prompt — replaying it here as well would show it twice.
    ///
    /// Returns `None` when the chat has no prior turns, which lets the harness
    /// skip the replay path entirely rather than build an empty list.
    async fn conversation_history(
        &self,
        chat_id: &str,
        exclude: Option<&str>,
    ) -> Option<Vec<rcode_proto::ConversationTurn>> {
        let handle = self.doc_handle(chat_id).ok()?;
        let mut entries = handle.doc().read_entries().ok()?;
        // Never restore a complete local record when part of its display group
        // has been excluded by compaction or removed from the visible history.
        let roots: std::collections::HashSet<_> = entries.iter()
            .filter(|entry| !entry.excluded && entry.continuation_of.is_none())
            .map(|entry| entry.id.clone()).collect();
        let excluded_groups: std::collections::HashSet<_> = entries.iter()
            .filter(|entry| entry.excluded)
            .map(|entry| entry.continuation_of.clone().unwrap_or_else(|| entry.id.clone())).collect();
        // 手动压缩只恢复待办状态引用，不能因此把已排除的完整工具历史重新送给模型。
        let mut todo_history_ids = Vec::new();
        for entry in entries.iter().filter(|entry| entry.role == rcode_doc::MessageRole::Assistant) {
            let root = entry.continuation_of.as_ref().unwrap_or(&entry.id);
            if excluded_groups.contains(root) && !todo_history_ids.contains(root) {
                todo_history_ids.push(root.clone());
            }
        }
        hydrate_history_outputs(self.inner.journal.clone(), chat_id.to_owned(), &mut entries).await;

        let mut turns: Vec<rcode_proto::ConversationTurn> = entries
            .into_iter()
            .filter(|entry| Some(entry.id.as_str()) != exclude)
            // Compacted-out entries stay in the document and on screen, but the
            // model no longer sees them. This is the one place that decision is
            // honoured — see `SessionDoc::set_excluded`.
            .filter(|entry| !entry.excluded)
            .filter_map(|entry| {
                let role = match entry.role {
                    rcode_doc::MessageRole::User => "user",
                    rcode_doc::MessageRole::Assistant => "assistant",
                    // System and tool rows are not conversation turns. Their
                    // content is already folded into the assistant turn that
                    // produced it (see the flattening below).
                    _ => return None,
                };
                let text = flatten_parts(&entry.parts);
                let text = handle.doc().command_prompt(&entry.id, &text).unwrap_or(text);
                (!text.trim().is_empty()).then(|| rcode_proto::ConversationTurn {
                    role: role.to_owned(),
                    text,
                    native_history_id: {
                        let root = entry.continuation_of.as_ref().unwrap_or(&entry.id);
                        (role == "assistant" && roots.contains(root) && !excluded_groups.contains(root))
                            .then(|| root.clone())
                    },
                    todo_history_ids: Vec::new(),
                })
            })
            .collect();

        if let Some(first) = turns.first_mut() { first.todo_history_ids = todo_history_ids; }

        (!turns.is_empty()).then_some(turns)
    }

    /// Status watch: the full session list, re-sent on every transition.
    pub fn watch_sessions(&self) -> watch::Receiver<Vec<Session>> {
        self.inner.sessions_tx.subscribe()
    }

    pub fn session_status(&self, chat_id: &str) -> Option<Session> {
        lock(&self.inner.statuses).get(chat_id).cloned()
    }

    /// Whether this harness takes a prompt *during* a turn, rather than only at
    /// the boundary between turns. The queue drain's central question: a
    /// mid-turn agent gets the message now, everything else holds it until the
    /// turn ends. Catalog lookup only — never spawns.
    pub fn steers_mid_turn(&self, harness: HarnessId) -> bool {
        self.inner
            .registry
            .descriptors()
            .iter()
            .find(|d| d.id == harness)
            .is_some_and(HarnessDescriptor::steers_mid_turn)
    }

    /// Whether this chat has a turn in flight — streaming, or parked on a
    /// question it is still owed an answer to (see [`is_active`]). A persistent
    /// session parked BETWEEN turns is not: it holds a warm child with nothing
    /// outstanding, and takes the next prompt straight through its mailbox.
    pub fn turn_in_flight(&self, chat_id: &str) -> bool {
        lock(&self.inner.statuses)
            .get(chat_id)
            .is_some_and(is_active)
    }

    /// Whether the chat's live runtime reports background work (subagents,
    /// background shells) still running — parked or not.
    pub fn holds_background_work(&self, chat_id: &str) -> bool {
        lock(&self.inner.runs)
            .get(chat_id)
            .is_some_and(|h| h.turn.background_live())
    }

    /// Any run currently working or blocked on input — the auto-updater's
    /// "don't restart from under a session" gate.
    pub fn any_active(&self) -> bool {
        lock(&self.inner.statuses).values().any(is_active)
    }

    /// A text prompt for `chat_id` would land in the mailbox of a live
    /// turn-boundary agent mid-turn. The agent reads it only after the turn,
    /// but a mailbox delivery writes the user message now — above the reply
    /// still streaming for the message before it. Such prompts belong in the
    /// visible queue. `request` = a Run that may differ from the live config
    /// (one the runtime adopts in place still lands in its mailbox; any other
    /// config restarts the runtime instead, which is no hold).
    pub fn defers_to_turn_end(
        &self,
        chat_id: &str,
        request: Option<(HarnessId, &RunRequest)>,
    ) -> bool {
        if !self.turn_in_flight(chat_id) {
            return false;
        }
        let live = lock(&self.inner.runs).get(chat_id).map(|h| {
            (
                h.runtime_config.harness_id,
                h.steerable
                    && request.is_none_or(|(id, r)| h.runtime_config.route(id, r).is_some()),
            )
        });
        live.is_some_and(|(harness, routable)| routable && !self.steers_mid_turn(harness))
    }


    /// The chat's live run accepts steering into its mailbox.
    pub fn live_run_steerable(&self, chat_id: &str) -> bool {
        lock(&self.inner.runs)
            .get(chat_id)
            .is_some_and(|h| h.steerable && !h.retiring.load(std::sync::atomic::Ordering::Acquire))
    }

    /// The last request dispatched for a chat (steer→new-turn fallback).
    pub fn last_request(&self, chat_id: &str) -> Option<RunRequest> {
        lock(&self.inner.last_requests).get(chat_id).cloned()
    }

    /// Subscribe to a chat's live event stream: returns the journal replay after
    /// `after_seq` plus a live receiver. Subscribe-then-replay ordering means overlap
    /// (dedupe by seq) rather than gaps.
    pub fn subscribe(
        &self,
        chat_id: &str,
        after_seq: u64,
    ) -> Result<(Vec<JournaledEvent>, broadcast::Receiver<JournaledEvent>), EngineError> {
        let rx = {
            let mut hubs = lock(&self.inner.hubs);
            hubs.entry(chat_id.to_string())
                .or_insert_with(|| broadcast::channel(1024).0)
                .subscribe()
        };
        let replay = self
            .inner
            .journal
            .replay(chat_id, after_seq)?
            .into_iter()
            .map(|(seq, event)| JournaledEvent { seq, event })
            .collect();
        Ok((replay, rx))
    }

    /// Local experimental bootstrap. No Submit, empty user entry or title request.
    pub async fn start_idle(
        &self,
        chat_id: &str,
        request: RunRequest,
    ) -> Result<String, EngineError> {
        self.dispatch_inner(chat_id, HarnessId::Codex, request, None, false, true)
            .await
    }

    /// Start (or route) a run for `chat_id`.
    ///
    /// - The user message entry is written to the doc immediately (id = `message_id`).
    /// - A live steerable run receives the prompt as its next turn via the mailbox
    ///   (the original app's persistent-session routing); otherwise any live run is interrupted
    ///   first — never two runtimes driving one chat.
    pub async fn dispatch(
        &self,
        chat_id: &str,
        harness_id: HarnessId,
        mut request: RunRequest,
        message_id: Option<String>,
    ) -> Result<String, EngineError> {
        if let Some(command) = crate::goals::parse_command(&request.prompt) {
            if harness_id != HarnessId::Native { return Err(EngineError::Other("Goal mode requires the native agent".into())); }
            let (action, objective) = command.map_err(EngineError::Other)?;
            if matches!(action, "create" | "replace") {
                if self.turn_in_flight(chat_id) { return Err(EngineError::Other("Stop the current turn before setting a goal".into())); }
                let goal = self.goal_store(chat_id)?.user_action(action, Some(&objective)).map_err(EngineError::Other)?
                    .expect("created goal");
                request.prompt = format!("Work toward this session goal: {}", goal.objective);
            } else if action == "resume" {
                if self.turn_in_flight(chat_id) { return Err(EngineError::Other("The previous turn is still running or stopping".into())); }
                let goal = self.goal_store(chat_id)?.user_action("resume", None).map_err(EngineError::Other)?.expect("resumed goal");
                request.prompt = format!("Continue working toward the session goal: {}", goal.objective);
            } else {
                self.goal_action(chat_id, action).await?;
                return Ok(format!("goal-{action}"));
            }
        }
        // A slash command is handled here, before anything is written or
        // dispatched. It is not a prompt: `/compact` does not ask the model to
        // compact, it compacts. Sending it on would make the outcome depend on
        // the model agreeing, which is exactly what a command exists to avoid.
        let harness = self.inner.registry.resolve(harness_id).ok();
        if let Some(outcome) = self
            .run_command(chat_id, &request.prompt, harness.clone())
            .await
        {
            return outcome;
        }

        // Compact before dispatching when the conversation has outgrown the
        // model's window. Doing it here rather than in the loop is what makes
        // it stick: the summary and the `excluded` marks go into the document,
        // so the next turn replays a shorter conversation instead of paying for
        // the same summary again.
        if let Some(harness) = harness.as_ref() {
            self.compact_if_full(chat_id, &request, harness).await;
        }

        let active = if self.inner.doc_host().is_some() {
            let store = self.goal_store(chat_id)?;
            store.user_action("read", None).map_err(EngineError::Other)?.map(|goal| (store, goal.id))
        } else { None };
        let result = self.dispatch_with(chat_id, harness_id, request, message_id, false).await;
        if let Err(error) = &result {
            if let Some((store, id)) = active {
                use rcode_agent::goal::GoalStore;
                store.change(&id, rcode_agent::goal::GoalChange::Pause { reason: error.to_string() }).map_err(EngineError::Other)?;
            }
        }
        result
    }

    /// One bounded, local-only recovery for the most recent legacy blank checklist.
    pub(crate) fn repair_todo_labels_from_journal(&self, doc: &SessionDoc, chat_id: &str) -> Result<(), EngineError> {
        let entries = doc.read_opening_tail(256)?;
        let Some((id, items)) = entries.iter().rev().flat_map(|entry| entry.parts.iter().rev()).find_map(|part| match part {
            MessagePart::Tool { id, call: rcode_proto::ToolCall::Todo { items }, .. } => Some((id, items)), _ => None,
        }) else { return Ok(()); };
        if !items.iter().any(|item| item.text.trim().is_empty()) { return Ok(()); }
        for event in self.inner.journal.events_rev(chat_id)?.take(4096) {
            match event?.1 {
                AgentEvent::ToolResult { id: found, output: Some(output), is_error: false, .. } if found == *id => {
                    if let Some(repaired) = rcode_doc::todo::recover_blank_todo_labels(items, &output) { doc.restore_todo_labels(id, items, &repaired)?; }
                    break;
                }
                AgentEvent::ToolCall { id: found, .. } if found == *id => break,
                _ => {}
            }
        }
        Ok(())
    }

    fn goal_store(&self, chat_id: &str) -> Result<Arc<crate::goals::DocGoalStore>, EngineError> {
        let mut stores = lock(&self.inner.goal_stores);
        if let Some(store) = stores.get(chat_id).and_then(std::sync::Weak::upgrade) { return Ok(store); }
        stores.retain(|_, store| store.strong_count() > 0);
        let store = Arc::new(crate::goals::DocGoalStore::new(self.doc_handle(chat_id)?));
        stores.insert(chat_id.into(), Arc::downgrade(&store));
        Ok(store)
    }

    pub async fn goal_action(&self, chat_id: &str, action: &str) -> Result<Option<rcode_proto::goal::SessionGoal>, EngineError> {
        self.goal_action_if_current(chat_id, action, None).await
    }

    pub async fn goal_action_if_current(&self, chat_id: &str, action: &str, expected_id: Option<&str>) -> Result<Option<rcode_proto::goal::SessionGoal>, EngineError> {
        self.update_goal_if_current(chat_id, action, expected_id, None, None).await
    }

    pub async fn update_goal_if_current(&self, chat_id: &str, action: &str, expected_id: Option<&str>, objective: Option<&str>, revision: Option<u64>) -> Result<Option<rcode_proto::goal::SessionGoal>, EngineError> {
        if action == "edit" && (expected_id.is_none() || revision.is_none()) {
            return Err(EngineError::Other("Editing requires the current goal id and revision".into()));
        }
        if !matches!(action, "read" | "pause" | "resume" | "clear" | "edit") {
            return Err(EngineError::Other("Unknown goal action".into()));
        }
        let store = self.goal_store(chat_id)?;
        if action == "resume" && self.turn_in_flight(chat_id) {
            return Err(EngineError::Other("The previous turn is still running or stopping".into()));
        }
        let was_active = store.user_action("read", None).map_err(EngineError::Other)?.is_some_and(|goal| goal.status == rcode_proto::goal::GoalStatus::Active);
        let goal = store.user_action_if_revision(action, objective, expected_id, revision).map_err(EngineError::Other)?;
        if was_active && matches!(action, "pause" | "clear") { self.stop_turn(chat_id).await?; }
        if action == "resume" && !self.turn_in_flight(chat_id) {
            let request = self.inner.doc_host().and_then(|host| host.request_from_chat_row(chat_id, ""))
                .or_else(|| self.last_request(chat_id));
            let resumed_id = &goal.as_ref().expect("resumed goal").id;
            let Some(mut request) = request else {
                let _ = store.user_action_if_current("pause", None, Some(resumed_id));
                return Err(EngineError::Other("Select a model and send /goal resume to resume this restored goal".into()));
            };
            request.attachments.clear();
            request.worktree = None;
            request.prompt = format!("Continue working toward the session goal: {}", goal.as_ref().expect("resumed goal").objective);
            if let Err(error) = self.dispatch_with(chat_id, HarnessId::Native, request, None, false).await {
                let _ = store.user_action_if_current("pause", None, Some(resumed_id));
                return Err(error);
            }
        }
        Ok(goal)
    }

    /// Compact when the conversation has outgrown the model's window.
    ///
    /// Silent by design: this is housekeeping the user did not ask for, and a
    /// failure must not stop the turn they did ask for. The run itself will
    /// report a genuine problem — if the conversation really does not fit, the
    /// provider says so, and that message is more truthful than one invented
    /// here.
    async fn compact_if_full(
        &self,
        chat_id: &str,
        request: &RunRequest,
        harness: &Arc<dyn Harness>,
    ) {
        // The model this turn will use, or the harness's default when the
        // request left it unset. Passed through unresolved: the harness
        // resolves it the same way its run would.
        let model = request.model.as_deref();

        let Ok(handle) = self.doc_handle(chat_id) else {
            return;
        };
        let Ok(entries) = handle.doc().read_entries() else {
            return;
        };
        let visible: Vec<&rcode_doc::SessionMessageEntry> =
            entries.iter().filter(|entry| !entry.excluded).collect();
        if visible.is_empty() {
            return;
        }

        // Rendered in the shape a replay carries, because that is the text the
        // model will actually be sent — measuring anything else would be
        // measuring a conversation that does not exist.
        let transcript = visible
            .iter()
            .map(|entry| {
                let role = match entry.role {
                    rcode_doc::MessageRole::User => "User",
                    rcode_doc::MessageRole::Assistant => "Assistant",
                    rcode_doc::MessageRole::System => "System",
                };
                format!("{role}: {}", flatten_parts(&entry.parts))
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        if !harness.needs_compaction_with_options(model, &request.model_options, &transcript) {
            return;
        }

        tracing::info!(
            %chat_id,
            model = model.unwrap_or("(default)"),
            messages = visible.len(),
            "the conversation is over the window; compacting before dispatch"
        );
        // A failure here is logged and otherwise ignored: the turn proceeds,
        // and the provider is the authority on whether it fits.
        if let Err(error) = self.compact_now(chat_id, "", harness.clone()).await {
            tracing::warn!(%chat_id, %error, "pre-dispatch compaction failed; dispatching anyway");
        }
    }

    /// Handle a slash command, or return `None` to dispatch normally.
    ///
    /// A command that runs here produces no run, so the returned id is a
    /// synthetic one — the caller uses it only to track the turn, and a
    /// command turn has no stream to track.
    ///
    /// The name is matched as text rather than through `rcode_agent`'s types:
    /// the engine does not depend on the agent crate (the dependency runs the
    /// other way — a harness adapts the agent, not the reverse), and a command
    /// *effect* is a document operation the engine owns anyway. The registry
    /// that *advertises* commands still lives with the agent, so the composer
    /// and this match read the same list.
    async fn run_command(
        &self,
        chat_id: &str,
        prompt: &str,
        harness: Option<Arc<dyn Harness>>,
    ) -> Option<Result<String, EngineError>> {
        let delivered = rcode_proto::invocation::harness_prompt(prompt, HarnessId::Native);
        let trimmed = delivered.trim_start();
        let rest = trimmed.strip_prefix('/')?;
        // `/` alone, `//path`, or a path in prose is not a command.
        if rest.is_empty() || rest.starts_with('/') {
            return None;
        }
        let (name, args) = match rest.split_once(char::is_whitespace) {
            Some((name, args)) => (name, args.trim()),
            None => (rest, ""),
        };
        // Accept both spellings: the user types `/compact`, and the domain is
        // an implementation detail they should not have to know.
        let is_compact = name.eq_ignore_ascii_case("compact")
            || name.eq_ignore_ascii_case("c")
            || name.eq_ignore_ascii_case("core:compact");

        if !is_compact {
            return None;
        }
        let Some(harness) = harness else {
            return Some(Err(EngineError::Other(
                "the agent is not available to summarize with".into(),
            )));
        };
        Some(self.compact_now(chat_id, args, harness).await)
    }

    /// Summarize the older half of the conversation and mark it out of view.
    ///
    /// Marks rather than deletes: the transcript is the user's record of what
    /// happened, and a compaction is a decision about what the *model* sees,
    /// not a claim that the earlier turns did not occur. That also makes it
    /// reversible and crash-safe — an interrupted compaction leaves a document
    /// that is still valid, just not yet compacted.
    ///
    /// The order matters. The summary is produced *first*, and nothing is
    /// marked until it exists: a compaction that failed halfway must leave the
    /// conversation exactly as it was, not with history hidden behind a
    /// summary that was never written.
    async fn compact_now(
        &self,
        chat_id: &str,
        focus: &str,
        harness: Arc<dyn Harness>,
    ) -> Result<String, EngineError> {
        let handle = self.doc_handle(chat_id)?;
        let mut entries = handle.doc().read_entries()?;
        hydrate_history_outputs(self.inner.journal.clone(), chat_id.to_owned(), &mut entries).await;

        // Only entries the model currently sees are candidates: compacting
        // twice must not re-mark what is already out of view, and the summary
        // of a summary is worthless.
        let visible: Vec<&rcode_doc::SessionMessageEntry> =
            entries.iter().filter(|entry| !entry.excluded).collect();

        // Nothing worth compacting: report rather than spend a model call
        // summarizing two messages.
        if visible.len() <= COMPACT_KEEP_RECENT_TURNS {
            return Err(EngineError::Other(
                "This conversation is short enough that compacting would not free \
                 meaningful context."
                    .into(),
            ));
        }

        let keep_from = visible.len() - COMPACT_KEEP_RECENT_TURNS;
        let to_compact = &visible[..keep_from];
        let ids: Vec<String> = to_compact.iter().map(|entry| entry.id.clone()).collect();

        // Render what is about to leave the model's view, in the same shape a
        // replay would have used — so the summary describes exactly what is
        // being taken away.
        let transcript = to_compact
            .iter()
            .map(|entry| {
                let role = match entry.role {
                    rcode_doc::MessageRole::User => "User",
                    rcode_doc::MessageRole::Assistant => "Assistant",
                    rcode_doc::MessageRole::System => "System",
                };
                let text = flatten_parts(&entry.parts);
                let text = handle.doc().command_prompt(&entry.id, &text).unwrap_or(text);
                format!("{role}: {text}")
            })
            .filter(|line| !line.trim_end().ends_with(':'))
            .collect::<Vec<_>>()
            .join("\n\n");

        let summary = harness
            .summarize(transcript, focus.to_owned())
            .await
            .map_err(|error| EngineError::Other(error.to_string()))?;

        // Now that the summary exists, take the old turns out of view and put
        // the summary in their place.
        let marked = handle.doc().set_excluded(&ids, true)?;
        handle.write_user_message(
            &format!("compact:{}", new_id()),
            &format!(
                "[Compacted] The earlier part of this conversation ({marked} messages) \
                 was replaced by this summary:\n\n{summary}"
            ),
            now_ms(),
        )?;

        let session_id = lock(&self.inner.harness_sessions).get(chat_id).map(|known| known.session_id.clone())
            .or_else(|| self.inner.journal_harness_session(chat_id).map(|(id, _)| id));
        if let Some(session_id) = session_id {
            harness.reset_prompt_cache(&session_id).await;
        }
        Ok(format!("compacted:{chat_id}"))
    }

    /// [`Self::dispatch`] with the startup-crash retry marker: the retry
    /// re-dispatches with `startup_retry = true`, which makes that attempt
    /// final (its own startup death surfaces instead of retrying again).
    /// Boxed future: `drive_run` re-enters this for that retry, and the
    /// erasure breaks the opaque-type cycle the recursion would otherwise form.
    fn dispatch_with<'a>(
        &'a self,
        chat_id: &'a str,
        harness_id: HarnessId,
        request: RunRequest,
        message_id: Option<String>,
        startup_retry: bool,
    ) -> futures::future::BoxFuture<'a, Result<String, EngineError>> {
        Box::pin(self.dispatch_inner(
            chat_id,
            harness_id,
            request,
            message_id,
            startup_retry,
            false,
        ))
    }

    async fn dispatch_inner(
        &self,
        chat_id: &str,
        harness_id: HarnessId,
        mut request: RunRequest,
        mut message_id: Option<String>,
        startup_retry: bool,
        idle: bool,
    ) -> Result<String, EngineError> {
        let _dispatch = self.dispatch_lock(chat_id).lock_owned().await;
        self.ensure_chat_open(chat_id)?;
        // Project-less chats store a home-relative cwd (the creating device
        // can't know the host's home); expand it here, on the host, where the
        // run spawns, creating a new chat's scratch folder.
        let scratch_root = self
            .inner
            .scratch_root
            .get()
            .cloned()
            .unwrap_or_else(crate::repos::fallback_scratch_root);
        request.cwd = crate::repos::resolve_chat_cwd(&request.cwd, &scratch_root)
            .map_err(EngineError::Other)?;
        // Native-only catalog entries have no portable file fallback. Reject
        // cross-harness delivery before recording or routing the user turn.
        rcode_proto::invocation::validate_harness_invocations(&request.prompt, harness_id)
            .map_err(EngineError::Other)?;
        if !idle && harness_id == HarnessId::Native {
            let user_id = message_id.get_or_insert_with(new_id);
            let handle = self.doc_handle(chat_id)?;
            if handle.doc().command_prompt(user_id, &request.prompt).is_none() {
                let harness = self.inner.registry.resolve(harness_id).map_err(|e| EngineError::Other(e.to_string()))?;
                if let Some(prompt) = harness.expand_custom_command(std::path::Path::new(&request.cwd), &request.prompt).await
                    .map_err(|e| EngineError::Other(e.to_string()))? {
                    handle.doc().remember_command_prompt(user_id, &request.prompt, &prompt)?;
                }
            }
        }
        let routed = lock(&self.inner.runs).get(chat_id).map(|h| {
            (
                h.run_id.clone(),
                h.steerable && !h.retiring.load(std::sync::atomic::Ordering::Acquire),
                h.runtime_config.route(harness_id, &request),
                h.steer_tx.clone(),
                h.routed_steers.clone(),
                h.fork_history_sent.clone(),
                h.retiring.clone(),
            )
        });
        if let Some((run_id, steerable, route, steer_tx, ledger, history_sent, retiring)) = routed {
            if idle {
                // No prompt rides an idle start, so nothing could carry an
                // in-place reconfiguration: only the runtime as is will do.
                return if route == Some(false) && harness_id == HarnessId::Codex {
                    Ok(run_id)
                } else {
                    Err(EngineError::Other("incompatible live runtime".into()))
                };
            }
            let user_id = message_id.clone().unwrap_or_else(new_id);
            let mut bootstrap = None;
            let same_runtime = route.is_some();
            let reconfigure = route == Some(true);
            let accepted = if steerable && same_runtime {
                bootstrap =
                    self.warm_fork_history(chat_id, harness_id, &request.prompt, &history_sent);
                let delivered = bootstrap.as_deref().unwrap_or(&request.prompt);
                // Warm dispatch uses the same mailbox as explicit steering.
                // Register acceptance before a fast boundary can retire it.
                let message = SteerMessage {
                    // OpenCode must see the canonical selection before it
                    // decodes the provider command: a project-scoped command
                    // can disappear between composer discovery and delivery.
                    prompt: if harness_id == HarnessId::Opencode {
                        delivered.to_owned()
                    } else {
                        rcode_proto::invocation::harness_prompt(delivered, harness_id)
                    },
                    message_id: Some(user_id.clone()),
                    // Images ride the mailbox too: replacing the runtime just
                    // to inline them killed its background work.
                    attachments: request.attachments.clone(),
                    // A changed model/effort/option the live process adopts
                    // in place: replacing it killed its background work.
                    config: reconfigure.then(|| Box::new(request.clone())),
                };
                if let Ok(permit) = steer_tx.reserve().await {
                    self.ensure_chat_open(chat_id)?;
                    let mut pending = lock(&ledger);
                    pending.push_back(RoutedSteer {
                        prompt: request.prompt.clone(),
                        message_id: user_id.clone(),
                        fork_history: bootstrap.is_some(),
                        after_stop: retiring.load(std::sync::atomic::Ordering::Acquire),
                        request: Some(Box::new(request.clone())),
                    });
                    permit.send(message);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if accepted {
                if reconfigure {
                    // The runtime runs this configuration from now on: later
                    // sends compare against it, and a re-dispatch of an
                    // orphaned steer starts from it.
                    if let Some(h) = lock(&self.inner.runs)
                        .get_mut(chat_id)
                        .filter(|h| h.run_id == run_id)
                    {
                        h.runtime_config.request = request.clone();
                    }
                    lock(&self.inner.last_requests).insert(chat_id.to_string(), request.clone());
                }
                let handle = self.doc_handle(chat_id)?;
                handle.write_user_message(&user_id, &request.prompt, now_ms())?;
                self.note_turn_start(chat_id, &request.cwd);
                if self.is_live(chat_id, &run_id) {
                    if bootstrap.is_some() {
                        history_sent.store(true, std::sync::atomic::Ordering::Release);
                    }
                    // Working BEFORE the lastMessageAt bump: both ride the
                    // workspace doc from this one peer, so causal order makes it
                    // impossible for an observer to hold [new message, old status]
                    // — that gap read as unseen-with-no-live-run = a phantom
                    // "completed" flash on every remote send (2026-07-31).
                    self.set_status(chat_id, SessionStatus::Working, false);
                    self.inner.note_message(chat_id, &request.prompt);
                    return Ok(run_id);
                }
                // The run died around the send. If its exit drain already
                // claimed the entry, that re-dispatch owns the message —
                // otherwise reclaim it and fall through to a fresh run.
                let reclaimed = {
                    let mut ledger = lock(&ledger);
                    let before = ledger.len();
                    ledger.retain(|s| s.message_id != user_id);
                    ledger.len() != before
                };
                if !reclaimed {
                    self.inner.note_message(chat_id, &request.prompt);
                    return Ok(run_id);
                }
                // Keep the already-written doc entry's id for the fresh run
                // below (write_user_message dedupes by id).
                message_id = Some(user_id);
            }
            if !same_runtime {
                tracing::info!(
                    chat = %chat_id,
                    "restarting live harness to apply changed run configuration"
                );
            }
            // Mailbox closed (runtime mid-teardown / non-steering harness) or
            // the routed run died with the message reclaimed, or the launch
            // configuration changed: replace the runtime.
            self.terminate(chat_id).await?;
        }

        // A Stop sent after this message cancelled it while it was on its
        // way: never start the stopped work again in a fresh runtime.
        if let Some(id) = &message_id
            && lock(&self.inner.cancelled_sends)
                .get_mut(chat_id)
                .is_some_and(|ids| ids.remove(id))
        {
            tracing::info!(chat = %chat_id, "message cancelled by a later Stop: not dispatched");
            return Ok(String::new());
        }
        let harness = self.inner.registry.resolve(harness_id)?;
        let handle = self.doc_handle(chat_id)?;
        let user_id = message_id.unwrap_or_else(new_id);
        if !idle {
            handle.write_user_message(&user_id, &request.prompt, now_ms())?;
        }

        // Engine-owned resume (the original app sessions.ts:736 — every dispatch read the
        // chat's stored harness session): callers always send `resume: None`;
        // the engine threads the chat's prior harness session back in so a new
        // process (app restart) continues the same harness conversation. The
        // startup-crash retry injects too — a stale id is the harness's
        // problem now (`session/load` falls back to `session/new` internally),
        // and starting the retry fresh silently dropped a good conversation.
        let mut resume_injected = false;
        if request.resume.is_none() {
            request.resume = self.inner.resume_for(chat_id, &request.cwd);
            resume_injected = request.resume.is_some();
        }
        lock(&self.inner.last_requests).insert(chat_id.to_string(), request.clone());

        let run_id = new_id();
        let (steer_tx, steer_rx) = mpsc::channel::<SteerMessage>(32);
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let (engine_tx, engine_rx) = mpsc::unbounded_channel::<AgentEvent>();
        let pending_inputs: PendingInputs = Arc::new(Mutex::new(HashMap::new()));

        // Input bridge: the harness asks questions; we mint the request id, park the
        // resolver for `respond_input`, and surface the event through the run pipeline.
        let request_input = {
            let pending = pending_inputs.clone();
            let engine_tx = engine_tx.clone();
            let run_id = run_id.clone();
            Box::new(move |request: rcode_proto::InteractionRequest| {
                let (mut tx, rx) = oneshot::channel();
                let (answer_tx, answer_rx) = oneshot::channel();
                let request_id = new_id();
                lock(&pending).insert(
                    request_id.clone(),
                    PendingInput {
                        run_id: run_id.clone(),
                        request: request.clone(),
                        resolver: answer_tx,
                    },
                );
                let _ = engine_tx.send(AgentEvent::InputRequested {
                    request_id: request_id.clone(),
                    request,
                });
                let pending = pending.clone();
                let engine_tx = engine_tx.clone();
                tokio::spawn(async move {
                    tokio::select! {
                        // A dropped resolver stays an error for the harness, as before.
                        answer = answer_rx => if let Ok(answer) = answer { let _ = tx.send(answer); },
                        _ = tx.closed() => {
                            // The run went away without answering. Unpark the
                            // record so a later answer is not accepted for a
                            // request nobody is waiting on.
                            lock(&pending).remove(&request_id);
                            let _ = engine_tx.send(AgentEvent::InputResolved { request_id });
                        }
                    }
                });
                rx
            })
        };
        let interrupt_token = CancellationToken::new();
        let turn = TurnControl::default();
        let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fork_history_sent = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // The conversation before this turn, for a stateless harness that
        // replays it. Read here rather than in the harness because the doc is
        // the engine's, and because the message being dispatched must be
        // excluded — it was written above and rides `request.prompt`.
        let history = self.conversation_history(chat_id, Some(&user_id)).await;
        self.ensure_chat_open(chat_id)?;
        let writer = handle.writer()?;
        let controls = RunControls {
            conversation_id: Some(chat_id.to_owned()),
            goal: if self.inner.doc_host().is_some() { Some(self.goal_store(chat_id)?) } else { None },
            request_input,
            steering: steer_rx,
            interrupt: interrupt_token.clone(),
            turn: turn.clone(),
            history,
        };

        let (abort, abort_registration) = futures::future::AbortHandle::new_pair();
        let (completed_tx, completed) = watch::channel(false);
        let task = RunTask {
            run_id: run_id.clone(),
            interrupt: interrupt_token.clone(),
            cancel: cancel_tx.clone(),
            abort,
            completed,
        };
        // Publication and the closing intent share this boundary. Deletion
        // either observes this task, or prevents its insertion altogether.
        let closing = lock(&self.inner.deleting_chats);
        if closing.contains(chat_id) {
            return Err(EngineError::Other(format!("chat `{chat_id}` is closing or deleted")));
        }
        lock(&self.inner.run_tasks).entry(chat_id.to_owned()).or_default().push(task);
        lock(&self.inner.runs).insert(
            chat_id.to_string(),
            RunHandle {
                run_id: run_id.clone(),
                steerable: harness.supports_steering(),
                runtime_config: RuntimeConfig::new(harness_id, &request, harness.clone()),
                steer_tx,
                interrupt_token,
                stops_turn_in_place: harness.supports_steering() && harness.stops_turn_in_place(),
                turn: turn.clone(),
                started: started.clone(),
                engine_tx,
                pending_inputs,
                routed_steers: Arc::new(Mutex::new(std::collections::VecDeque::new())),
                fork_history_sent: fork_history_sent.clone(),
                retiring: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            },
        );
        drop(closing);
        self.set_status(
            chat_id,
            if idle {
                SessionStatus::Idle
            } else {
                SessionStatus::Working
            },
            true,
        );
        // AFTER Working (same causal-order guarantee as the steer path): the
        // lastMessageAt bump must never be observable ahead of the live run.
        if !idle {
            self.inner.note_message(chat_id, &request.prompt);
        }

        // Title generation ran off the first prompt in the external-agent
        // design; the native agent titles within its own turn.

        let driver = drive_run(
            self.inner.clone(),
            chat_id.to_string(),
            run_id.clone(),
            harness,
            request,
            writer,
            controls,
            engine_rx,
            cancel_rx,
            RunResumeState {
                idle,
                user_message_id: user_id,
                resume_injected,
                startup_retry,
                fork_history_sent,
                started,
            },
            turn.clone(),
        );
        let inner = self.inner.clone();
        let chat = chat_id.to_owned();
        let task_run_id = run_id.clone();
        tokio::spawn(async move {
            let result = std::panic::AssertUnwindSafe(futures::future::Abortable::new(driver, abort_registration))
                .catch_unwind().await;
            if result.is_err() {
                tracing::error!(chat = %chat, "session driver panicked during execution");
            }
            // Dropping the stream requests shutdown; the actor lease confirms
            // that the runtime has actually reaped its background children.
            turn.wait_for_runtime_exit().await;
            let still_current = lock(&inner.runs).get(&chat)
                .is_some_and(|run| run.run_id == task_run_id);
            if !matches!(result, Ok(Ok(())))
                && still_current
            {
                inner.set_status(&chat, SessionStatus::Idle, false);
            }
            inner.remove_run(&chat, &task_run_id);
            {
                let mut all_tasks = lock(&inner.run_tasks);
                if let Some(tasks) = all_tasks.get_mut(&chat) {
                    tasks.retain(|task| task.run_id != task_run_id);
                    if tasks.is_empty() { all_tasks.remove(&chat); }
                }
            }
            completed_tx.send_replace(true);
        });
        Ok(run_id)
    }

    /// A warm send's prompt with the fork's copied history in front, when the
    /// live provider session never received it (the fork's first turns were
    /// native commands) and no earlier send in this runtime carries it
    /// (`sent`). `None` = send the prompt as is.
    fn warm_fork_history(
        &self,
        chat_id: &str,
        harness_id: HarnessId,
        prompt: &str,
        sent: &std::sync::atomic::AtomicBool,
    ) -> Option<String> {
        if sent.load(std::sync::atomic::Ordering::Acquire) {
            return None;
        }
        let session = lock(&self.inner.harness_sessions)
            .get(chat_id)
            .map(|known| known.session_id.clone())
            .filter(|id| !id.is_empty());
        let handle = self.doc_handle(chat_id).ok()?;
        self.inner.fork_history_prompt(
            chat_id,
            handle.doc(),
            harness_id,
            prompt,
            None,
            ProviderSession::Continued(session.as_deref()),
        )
    }

    /// Push a steer prompt into the live run's mailbox. `NotSteerable` when no live
    /// steerable run exists — the caller (command executor) dispatches a new turn.
    pub async fn steer(
        &self,
        chat_id: &str,
        prompt: &str,
        message_id: Option<String>,
    ) -> Result<SteerOutcome, EngineError> {
        self.steer_at(chat_id, prompt, message_id, now_ms()).await
    }

    pub(crate) async fn steer_at(
        &self,
        chat_id: &str,
        prompt: &str,
        message_id: Option<String>,
        issued_at: i64,
    ) -> Result<SteerOutcome, EngineError> {
        let _dispatch = self.dispatch_lock(chat_id).lock_owned().await;
        self.ensure_chat_open(chat_id)?;
        let target = lock(&self.inner.runs)
            .get(chat_id)
            .filter(|h| h.steerable && !h.retiring.load(std::sync::atomic::Ordering::Acquire))
            .map(|h| {
                (
                    h.run_id.clone(),
                    h.runtime_config.harness_id,
                    h.steer_tx.clone(),
                    h.routed_steers.clone(),
                    h.fork_history_sent.clone(),
                    h.retiring.clone(),
                )
            });
        let Some((run_id, harness_id, steer_tx, ledger, history_sent, retiring)) = target else {
            return Ok(SteerOutcome::NotSteerable);
        };
        rcode_proto::invocation::validate_harness_invocations(prompt, harness_id)
            .map_err(EngineError::Other)?;
        let user_id = message_id.unwrap_or_else(new_id);
        let bootstrap = self.warm_fork_history(chat_id, harness_id, prompt, &history_sent);
        let delivered = bootstrap.as_deref().unwrap_or(prompt);
        // A steer runs on the configuration live now; record it with the
        // message so a re-dispatch keeps it.
        let sent_with = self.last_request(chat_id).map(|mut request| {
            request.prompt = prompt.to_string();
            request.attachments = Vec::new();
            Box::new(request)
        });
        let message = SteerMessage {
            prompt: if harness_id == HarnessId::Opencode {
                delivered.to_owned()
            } else {
                rcode_proto::invocation::harness_prompt(delivered, harness_id)
            },
            message_id: Some(user_id.clone()),
            attachments: Vec::new(),
            config: None,
        };
        // Saturation is backpressure, not a dead runtime. Waiting for room
        // preserves the live process and every accepted message in a burst.
        let Ok(permit) = steer_tx.reserve().await else {
            return Ok(SteerOutcome::NotSteerable);
        };
        self.ensure_chat_open(chat_id)?;
        {
            // Serialize mailbox acceptance with confirmation and Done-time
            // inspection: a fast consumer must never outrun its ledger entry.
            let mut pending = lock(&ledger);
            pending.push_back(RoutedSteer {
                prompt: prompt.to_string(),
                message_id: user_id.clone(),
                fork_history: bootstrap.is_some(),
                after_stop: retiring.load(std::sync::atomic::Ordering::Acquire),
                request: sent_with,
            });
            permit.send(message);
        }
        let handle = self.doc_handle(chat_id)?;
        handle.write_user_message(&user_id, prompt, issued_at.min(now_ms()))?;
        // A routed steer is a turn too. Fired here (not only on the confirmed
        // path) — a reclaim falls back to dispatch, which just re-snapshots.
        if let Some(request) = self.last_request(chat_id) {
            self.note_turn_start(chat_id, &request.cwd);
        }
        if self.is_live(chat_id, &run_id) {
            if bootstrap.is_some() {
                history_sent.store(true, std::sync::atomic::Ordering::Release);
            }
            self.set_status(chat_id, SessionStatus::Working, false);
            self.inner.note_message(chat_id, prompt);
            return Ok(SteerOutcome::Accepted);
        }
        // The run died around the send. Exit drain claimed the entry → its
        // re-dispatch owns the message; still ours → reclaim and report
        // NotSteerable so the executor falls back to a fresh dispatch
        // (same message id — the doc entry dedupes).
        let reclaimed = {
            let mut ledger = lock(&ledger);
            let before = ledger.len();
            ledger.retain(|s| s.message_id != user_id);
            ledger.len() != before
        };
        if reclaimed {
            return Ok(SteerOutcome::NotSteerable);
        }
        self.inner.note_message(chat_id, prompt);
        Ok(SteerOutcome::Accepted)
    }

    /// A Stop acting now cancels these messages, sent before it but still
    /// on their way: one that reaches dispatch after the runtime went down
    /// is dropped rather than run in a fresh runtime.
    pub fn cancel_sent_messages(&self, chat_id: &str, message_ids: Vec<String>) {
        if message_ids.is_empty() {
            return;
        }
        lock(&self.inner.cancelled_sends)
            .entry(chat_id.to_string())
            .or_default()
            .extend(message_ids);
    }

    /// Native Stop ends the current turn while preserving detached tasks.
    /// A stop with no foreground turn, or a legacy driver, tears down the runtime.
    pub async fn interrupt(&self, chat_id: &str) -> Result<bool, EngineError> {
        let native = lock(&self.inner.runs).get(chat_id)
            .is_some_and(|run| run.runtime_config.harness_id == HarnessId::Native);
        if native && self.turn_in_flight(chat_id) {
            self.stop_turn(chat_id).await
        } else {
            self.terminate(chat_id).await
        }
    }

    /// End the in-flight turn so another prompt can take its place ("send
    /// now"), keeping the runtime and its background work. Its streaming
    /// entry is stamped `aborted`; this waits (bounded) for that settlement
    /// so callers observe a consistent doc. Returns whether there was a turn
    /// (or runtime) to stop.
    ///
    /// A driver that [`Harness::stops_turn_in_place`] ends only the turn: the
    /// runtime parks with its background work — subagents, background shells
    /// — still running, and the next prompt routes into it. Drivers without
    /// that, runs whose harness has not started yet, and stops that do not
    /// settle fall back to [`Self::terminate`].
    pub async fn stop_turn(&self, chat_id: &str) -> Result<bool, EngineError> {
        const STOP_SETTLE: std::time::Duration = std::time::Duration::from_secs(15);
        let target = lock(&self.inner.runs)
            .get(chat_id)
            .filter(|h| {
                h.stops_turn_in_place && h.started.load(std::sync::atomic::Ordering::Acquire)
            })
            .map(|h| (h.run_id.clone(), h.turn.clone(), h.pending_inputs.clone()));
        let Some((run_id, turn, pending)) = target else {
            return self.terminate(chat_id).await;
        };
        if !self.turn_in_flight(chat_id) {
            return Ok(false); // parked between turns: nothing to stop
        }
        // Unpark questions first: the stopped turn can never be answered.
        // `Dismissed` grants nothing and answers nothing — the safe reading
        // for a prompt the user never got to reply to.
        let parked: Vec<_> = lock(&pending).drain().map(|(_, entry)| entry).collect();
        for entry in parked {
            let _ = entry
                .resolver
                .send(rcode_proto::InteractionResponse::Dismissed);
        }
        turn.stop_turn();
        let deadline = tokio::time::Instant::now() + STOP_SETTLE;
        while tokio::time::Instant::now() < deadline {
            if !self.is_live(chat_id, &run_id) || !self.turn_in_flight(chat_id) {
                return Ok(true);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        tracing::warn!(chat = %chat_id, "turn stop did not settle; tearing the runtime down");
        self.terminate(chat_id).await
    }

    /// Tear the chat's live runtime down (protocol interrupt, then the
    /// child). The run settles with a synthetic `Done{interrupted}` and its
    /// streaming entry stamped `aborted`; this waits (bounded) for that
    /// settlement. Background work the runtime held dies with it.
    pub async fn terminate(&self, chat_id: &str) -> Result<bool, EngineError> {
        self.terminate_with_deadlines(
            chat_id,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(3),
        ).await
    }

    async fn terminate_with_deadlines(
        &self,
        chat_id: &str,
        grace: std::time::Duration,
        forced: std::time::Duration,
    ) -> Result<bool, EngineError> {
        if let Some(handle) = lock(&self.inner.runs).get(chat_id) {
            let _order = lock(&handle.routed_steers);
            handle.retiring.store(true, std::sync::atomic::Ordering::Release);
            let parked: Vec<_> = lock(&handle.pending_inputs).drain().map(|(_, entry)| entry).collect();
            for entry in parked {
                let _ = entry.resolver.send(rcode_proto::InteractionResponse::Dismissed);
            }
        }
        let tasks = lock(&self.inner.run_tasks).get(chat_id).cloned().unwrap_or_default();
        if tasks.is_empty() {
            return Ok(false);
        }
        // Cancellation is a request, not acknowledgement that the process or
        // writer has exited. Retain each task until its completion signal.
        for task in &tasks {
            let _ = task.cancel.send(true);
            task.interrupt.cancel();
        }
        let wait = || futures::future::try_join_all(tasks.iter().map(|task| async move {
            let mut completed = task.completed.clone();
            completed.wait_for(|done| *done).await.map(|_| ())
                .map_err(|_| EngineError::Other(format!("runtime `{}` closed without confirming shutdown", task.run_id)))
        }));
        match tokio::time::timeout(grace, wait()).await {
            Ok(result) => { result?; }
            Err(_) => {
                for task in &tasks {
                    task.abort.abort();
                }
                tokio::time::timeout(forced, wait()).await
                    .map_err(|_| EngineError::Other(format!("chat shutdown incomplete: `{chat_id}` still has runtime or background work")))??;
            }
        }
        Ok(true)
    }

    /// Fold an engine-made event (a `show_visual` page) into the chat's live
    /// run, after whatever the harness streamed so far. Returns `false` when
    /// the chat has no run on this device.
    /// Whether the chat has a run on this device (one `inject_event` can reach).
    pub fn has_run(&self, chat_id: &str) -> bool {
        lock(&self.inner.runs).contains_key(chat_id)
    }

    pub fn inject_event(&self, chat_id: &str, event: AgentEvent) -> bool {
        lock(&self.inner.runs)
            .get(chat_id)
            .is_some_and(|h| h.engine_tx.send(event).is_ok())
    }

    /// Resolve a pending interaction. Returns `false` when nothing is pending
    /// for `request_id` — an unknown id, or a run that already settled.
    ///
    /// The checks are ordered so that a *wrong* answer costs nothing: the
    /// request stays parked and can still be answered correctly. Consuming the
    /// record first and validating afterwards would leave the run suspended
    /// with no way to reach it, because the only thing that resumes it is an
    /// answer to this id.
    pub fn respond_input(
        &self,
        chat_id: &str,
        request_id: &str,
        answer: rcode_proto::InteractionResponse,
    ) -> Result<bool, EngineError> {
        let target = lock(&self.inner.runs).get(chat_id).map(|h| {
            (
                h.run_id.clone(),
                h.pending_inputs.clone(),
                h.engine_tx.clone(),
            )
        });
        let Some((run_id, pending, engine_tx)) = target else {
            // The run is gone. Its request went with it — a late answer must
            // not reach whatever run replaced it.
            return Ok(false);
        };

        let entry = {
            let mut guard = lock(&pending);
            let Some(entry) = guard.get(request_id) else {
                return Ok(false);
            };
            if entry.run_id != run_id {
                // The chat is on a different run than the one that asked.
                return Ok(false);
            }
            if !answers_request(&entry.request, &answer) {
                // Leave the question parked until it has an answer.
                return Err(EngineError::Other(format!(
                    "the answer for `{request_id}` does not match what was asked"
                )));
            }
            guard
                .remove(request_id)
                .expect("the entry was found under the same lock")
        };

        let _ = entry.resolver.send(answer);
        let _ = engine_tx.send(AgentEvent::InputResolved {
            request_id: request_id.to_string(),
        });
        Ok(true)
    }

    /// Boot recovery: for every journal whose last event is not `Done` (a run died
    /// mid-stream), stamp this device's abandoned `streaming` doc entries `aborted`
    /// with a VISIBLE "Run interrupted by engine restart" error part, close the
    /// journal with a synthetic `Done{interrupted}` — and then PICK THE RUN BACK
    /// UP: a fresh crashed turn with revival budget left is re-dispatched against
    /// the remembered harness session (the original app: "not just eulogized";
    /// `MAX_AUTO_RESUME` = 3 consecutive revivals, fresh = crashed < 12h ago).
    pub fn recover_stale(&self) -> Result<usize, EngineError> {
        const MAX_AUTO_RESUME: u32 = 3;
        const RESUME_FRESH_MS: i64 = 12 * 60 * 60 * 1000;

        let stale = self.inner.journal.stale_sessions()?;
        let mut recovered = 0usize;
        for chat_id in stale {
            if lock(&self.inner.runs).contains_key(&chat_id) {
                continue; // a live run owns this journal
            }
            let handle = self.doc_handle(&chat_id)?;
            let interrupted_goal = handle.doc().goal().filter(|goal| goal.status == rcode_proto::goal::GoalStatus::Active);
            if let Some(goal) = &interrupted_goal {
                self.goal_store(&chat_id)?.pause_interrupted(&goal.id).map_err(EngineError::Other)?;
            }
            // Harness continuity first: the crashed run's session id may only
            // exist in the journal (the debounced workspace-row write may
            // never have landed) — remember it so the revived run resumes the
            // same harness conversation (the original app recoverDraft, sessions.ts:538).
            if let Some((session_id, cwd)) = self.inner.journal_harness_session(&chat_id) {
                self.inner
                    .remember_harness_session(&chat_id, &session_id, &cwd);
            }
            // The revival prompt: the last user message (idempotent re-dispatch
            // under the SAME id — `write_user_message` dedupes by id, so the
            // transcript never shows a duplicate).
            let prompt = handle.doc().read_entries().ok().and_then(|entries| {
                entries
                    .iter()
                    .rev()
                    .find(|e| e.role == MessageRole::User)
                    .and_then(|e| {
                        e.parts.iter().find_map(|p| match p {
                            MessagePart::Text { text, .. } => Some((e.id.clone(), text.clone())),
                            _ => None,
                        })
                    })
            });
            let attempts = self.inner.journal.resume_attempts(&chat_id);
            // Fresh: the crashed reply's streaming entry is recent — or no
            // reply entry reached disk at all (the doc saves on a debounce,
            // and a kill moments into the turn leaves the user's message as
            // the transcript's last word), and that message is recent.
            let fresh = handle
                .doc()
                .read_entries()
                .ok()
                .and_then(|entries| {
                    entries
                        .iter()
                        .rev()
                        .find(|e| e.status == Some(MessageStatus::Streaming))
                        .or_else(|| entries.last().filter(|e| e.role == MessageRole::User))
                        .map(|e| now_ms() - e.created_at < RESUME_FRESH_MS)
                })
                .unwrap_or(false);
            let will_resume = interrupted_goal.is_none() && fresh && prompt.is_some() && attempts < MAX_AUTO_RESUME;

            let note = if will_resume {
                "Run interrupted by engine restart — resuming"
            } else {
                "Run interrupted by engine restart"
            };
            let done = AgentEvent::Done {
                status: DoneStatus::Interrupted,
                result: None,
                error: Some(note.into()),
                session_id: None,
            };
            self.inner.publish(&chat_id, &done);
            let stamped = handle.mark_abandoned_streams(note)?.len();
            self.set_status(&chat_id, SessionStatus::Idle, false);
            tracing::info!(chat = %chat_id, stamped, will_resume, attempts, "recovered stale session journal");
            recovered += 1;

            if !will_resume {
                continue;
            }
            let attempt = self.inner.journal.note_resume_attempt(&chat_id);
            let (user_id, prompt_text) = prompt.expect("gated by will_resume");
            let sessions = self.clone();
            tokio::spawn(async move {
                let Some(host) = sessions.inner.doc_host() else {
                    return;
                };
                let request = sessions
                    .last_request(&chat_id)
                    .or_else(|| host.request_from_chat_row(&chat_id, &prompt_text))
                    // Last resort: the journal's own cwd (the original app's draft config)
                    // — a crash can predate the debounced workspace-row write.
                    .or_else(|| {
                        let (_, cwd) = sessions.inner.journal_harness_session(&chat_id)?;
                        Some(RunRequest {
                            prompt: String::new(),
                            harness: None,
                            model: None,
                            reasoning: None,
                            model_options: Default::default(),
                            cwd,
                            attachments: Vec::new(),
                            resume: None,
                            worktree: None,
                        })
                    });
                let Some(mut request) = request else {
                    tracing::warn!(chat = %chat_id, "auto-resume skipped: no run config");
                    return;
                };
                request.prompt = prompt_text;
                request.resume = None; // dispatch re-injects the remembered session
                request.attachments = Vec::new();
                let harness_id = host.harness_for_request(&chat_id, &request);
                match host
                    .dispatch_with_source_context(
                        &sessions,
                        &chat_id,
                        harness_id,
                        request,
                        Some(user_id),
                    )
                    .await
                {
                    Ok(_) => {
                        tracing::info!(chat = %chat_id, attempt, "auto-resumed crashed run")
                    }
                    Err(err) => {
                        tracing::warn!(chat = %chat_id, error = %err, "auto-resume dispatch failed")
                    }
                }
            });
        }
        Ok(recovered)
    }

    /// Graceful shutdown: interrupt every live run so streaming entries settle.
    pub async fn shutdown(&self) {
        {
            // 与自动标题提交共用锁，关闭开始后不得出现迟到标题写入。
            let jobs = lock(&self.inner.title_jobs);
            self.inner
                .shutting_down
                .store(true, std::sync::atomic::Ordering::Release);
            for cancel in jobs.values() {
                cancel.cancel();
            }
        }
        let chats: Vec<String> = lock(&self.inner.run_tasks).keys().cloned().collect();
        for chat_id in chats {
            if let Err(err) = self.terminate(&chat_id).await {
                tracing::warn!(chat = %chat_id, error = %err, "shutdown interrupt failed");
            }
        }
    }

    fn is_live(&self, chat_id: &str, run_id: &str) -> bool {
        lock(&self.inner.runs)
            .get(chat_id)
            .is_some_and(|h| h.run_id == run_id)
    }

    fn set_status(&self, chat_id: &str, status: SessionStatus, fresh_start: bool) {
        self.inner.set_status(chat_id, status, fresh_start);
    }
}

impl Inner {
    /// Journal + broadcast one event (the two unconditional legs of the pipeline).
    /// Sanitize before ANY journal/publish path, including nested subagent events.
    async fn prepare_generated_image(
        &self,
        chat_id: &str,
        event: AgentEvent,
        seen: &mut std::collections::HashSet<String>,
    ) -> Vec<AgentEvent> {
        let mut parents = Vec::new();
        let mut leaf = event;
        while let AgentEvent::Subagent {
            parent_tool_use_id,
            event,
        } = leaf
        {
            parents.push(parent_tool_use_id);
            leaf = *event;
        }
        let events = if let AgentEvent::GeneratedImage { id, path, .. } = leaf {
            let key = std::iter::once(chat_id)
                .chain(parents.iter().map(String::as_str))
                .chain(std::iter::once(id.as_str()))
                .collect::<Vec<_>>()
                .join("\0");
            if seen.contains(&key) {
                return vec![];
            }
            let result = if let Some((uploads, allowed_root)) = self.generated_images.get() {
                let uploads = uploads.clone();
                let root = allowed_root.clone();
                let stable_key = key.clone();
                tokio::task::spawn_blocking(move || {
                    uploads.import_generated_image(std::path::Path::new(&path), &root, &stable_key)
                })
                .await
                .unwrap_or_else(|err| Err(EngineError::Other(err.to_string())))
            } else {
                Err(EngineError::Other(
                    "Generated image intake is not configured".into(),
                ))
            };
            match result {
                Ok(image) => {
                    seen.insert(key);
                    vec![AgentEvent::GeneratedImage {
                        id,
                        path: image.path,
                        name: image.name,
                        mime_type: image.mime_type,
                    }]
                }
                Err(err) => {
                    tracing::warn!(chat = %chat_id, error = %err, "generated image import failed");
                    vec![
                        AgentEvent::ToolResult {
                            id: id.strip_suffix(":image").unwrap_or(&id).into(),
                            is_error: true,
                            output: None,
                            diff: None,
                        },
                        AgentEvent::Error {
                            message: "Generated image unavailable".into(),
                        },
                    ]
                }
            }
        } else {
            vec![leaf]
        };
        events
            .into_iter()
            .map(|mut event| {
                for parent in parents.iter().rev() {
                    event = AgentEvent::Subagent {
                        parent_tool_use_id: parent.clone(),
                        event: Box::new(event),
                    };
                }
                event
            })
            .collect()
    }

    fn publish(&self, chat_id: &str, event: &AgentEvent) -> u64 {
        let seq = match self.journal.append(chat_id, event) {
            Ok(seq) => seq,
            Err(err) => {
                tracing::error!(chat = %chat_id, error = %err, "journal append failed");
                0
            }
        };
        if let Some(hub) = lock(&self.hubs).get(chat_id) {
            let _ = hub.send(JournaledEvent {
                seq,
                event: event.clone(),
            });
        }
        seq
    }

    /// Bump the session's freshness on stream activity WITHOUT a status
    /// transition. Long silent-LOOKING stretches (thinking heartbeats, a big
    /// tool input being generated) still carry events — the UI's 45s
    /// staleness gate must not flip "Working" off mid-run. Throttled: a
    /// workspace-doc mirror per delta would be far too chatty.
    fn touch_session(&self, chat_id: &str) {
        const TOUCH_THROTTLE_MS: i64 = 10_000;
        let now = Utc::now();
        let session = {
            let mut statuses = lock(&self.statuses);
            let Some(entry) = statuses.get_mut(chat_id) else {
                return;
            };
            let age = now
                .signed_duration_since(entry.updated_at)
                .num_milliseconds();
            if age < TOUCH_THROTTLE_MS {
                return;
            }
            entry.updated_at = now;
            let session = entry.clone();
            let mut list: Vec<Session> = statuses.values().cloned().collect();
            list.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
            self.sessions_tx.send_replace(list);
            session
        };
        if let Some(ws) = self.workspace() {
            ws.record_session(&session);
        }
    }

    fn set_status(&self, chat_id: &str, status: SessionStatus, fresh_start: bool) {
        self.set_status_with_completion(chat_id, status, fresh_start, None);
    }

    /// Publish how many subagents of `chat_id` are streaming. A change is
    /// mirrored like any status transition (it doubles as a heartbeat); an
    /// unchanged count is a no-op, and a chat without a status row has
    /// nothing to badge yet.
    fn set_running_subagents(&self, chat_id: &str, running: usize) {
        let running = u32::try_from(running).unwrap_or(u32::MAX);
        let session = {
            let mut statuses = lock(&self.statuses);
            let Some(entry) = statuses.get_mut(chat_id) else {
                return;
            };
            if entry.running_subagents == running {
                return;
            }
            entry.running_subagents = running;
            entry.updated_at = Utc::now();
            let session = entry.clone();
            let mut list: Vec<Session> = statuses.values().cloned().collect();
            list.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
            self.sessions_tx.send_replace(list);
            session
        };
        if let Some(ws) = self.workspace() {
            ws.record_session(&session);
        }
    }

    fn has_pending_steers(&self, chat_id: &str, run_id: &str) -> bool {
        lock(&self.runs)
            .get(chat_id)
            .filter(|h| h.run_id == run_id)
            .is_some_and(|h| !lock(&h.routed_steers).is_empty())
    }

    /// Whether this chat still wants a generated title.
    ///
    /// True only while the chat is unnamed. A user- or agent-set title is
    /// never overwritten — that is the whole reason the check reads the row
    /// rather than keeping a "we titled this" flag: a title arriving from
    /// anywhere else settles the question.
    fn should_auto_title(&self, chat_id: &str) -> bool {
        lock(&self.workspace)
            .as_ref()
            .and_then(|workspace| workspace.chat(chat_id).ok().flatten())
            .is_some_and(|chat| !chat.archived && chat.title.is_none())
    }

    /// 标题任务独立于主回合，失败不改变主回合状态；同一会话不会重复启动。
    fn spawn_auto_title(
        self: &Arc<Self>,
        chat_id: String,
        harness_id: HarnessId,
        mut request: RunRequest,
    ) {
        let Ok(harness) = self.registry.resolve(harness_id) else {
            return;
        };
        let cancel = CancellationToken::new();
        {
            let mut jobs = lock(&self.title_jobs);
            if self
                .shutting_down
                .load(std::sync::atomic::Ordering::Acquire)
                || jobs.contains_key(&chat_id)
                || !self.should_auto_title(&chat_id)
            {
                return;
            }
            jobs.insert(chat_id.clone(), cancel.clone());
        }
        // 沿用该会话实际选择的模型，不重新选择设备默认模型。
        request.resume = None;
        request.attachments.clear();
        request.worktree = None;
        let inner = self.clone();
        // 占位在 spawn 之前就写入（拒绝重复），但清理只能由真正持有它的任务做：
        // lease 记录自己拿到的 token，取消/重试后新任务不会被旧任务误删。
        let task_chat_id = chat_id.clone();
        let spawned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tokio::spawn(async move {
                let _lease = TitleJobLease {
                    inner: inner.clone(),
                    chat_id: task_chat_id.clone(),
                    cancel: cancel.clone(),
                };
                let generated = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    result = tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        collect_title(harness, request, cancel.clone()),
                    ) => result,
                };
                let title = match generated {
                    Ok(Ok(title)) => title,
                    _ => {
                        cancel.cancel();
                        tracing::debug!(chat = %task_chat_id, "auto-title request failed or timed out");
                        return;
                    }
                };
                // 提交屏障：锁保护 shutdown，文档锁内再次检查人工标题、归档和删除。
                let _jobs = lock(&inner.title_jobs);
                if cancel.is_cancelled()
                    || inner
                        .shutting_down
                        .load(std::sync::atomic::Ordering::Acquire)
                {
                    return;
                }
                if let Some(workspace) = lock(&inner.workspace).as_ref() {
                    if let Err(error) = workspace.rename_untitled_chat(&task_chat_id, &title) {
                        tracing::debug!(chat = %task_chat_id, %error, "auto-title write failed");
                    }
                }
            })
        }));
        if spawned.is_err() {
            // 没有运行时（或 spawn 失败）：占位必须回退，否则这个会话再也不能自动命名。
            lock(&self.title_jobs).remove(&chat_id);
        }
    }

    /// Publish the completion marker atomically with the settled status. Keep
    /// it on subsequent Working/heartbeat rows for coalesced and remote watches.
    fn set_status_with_completion(
        &self,
        chat_id: &str,
        status: SessionStatus,
        fresh_start: bool,
        completed_turn: Option<String>,
    ) {
        if lock(&self.deleting_chats).contains(chat_id) {
            return;
        }
        let now = Utc::now();
        let session = {
            let mut statuses = lock(&self.statuses);
            let entry = statuses
                .entry(chat_id.to_string())
                .or_insert_with(|| Session {
                    last_completed_turn: None,
                    chat_id: chat_id.to_string(),
                    device_id: self.device_id.clone(),
                    status,
                    started_at: None,
                    updated_at: now,
                    running_subagents: 0,
                });
            // `started_at` is the elapsed-timer base and must only ever mean
            // "this turn". Entering Working from a settled state always
            // restamps (a steer into a parked session is a NEW turn — reusing
            // the old base showed the previous turn's 30min on send), and a
            // settled session drops its base entirely so no later reader can
            // resurrect a stale elapsed.
            let was_active = matches!(
                entry.status,
                SessionStatus::Working | SessionStatus::AwaitingInput
            );
            if let Some(turn) = completed_turn {
                entry.last_completed_turn = Some(turn);
            }
            entry.status = status;
            entry.updated_at = now;
            match status {
                SessionStatus::Working if fresh_start || !was_active => {
                    entry.started_at = Some(now);
                }
                SessionStatus::Working | SessionStatus::AwaitingInput => {}
                SessionStatus::Idle | SessionStatus::Errored => {
                    entry.started_at = None;
                }
            }
            let session = entry.clone();
            let mut list: Vec<Session> = statuses.values().cloned().collect();
            list.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
            // send_replace: keep the current value fresh even with no receivers,
            // so late WatchSessions subscribers see the last transition.
            self.sessions_tx.send_replace(list);
            session
        };
        // Mirror the transition into the workspace doc's session-status row so
        // remote devices' sidebars show this run (staleness-checked client-side).
        if let Some(ws) = self.workspace() {
            ws.record_session(&session);
        }
    }

    /// The doc host, once wired. `None` before assembly or after retirement.
    fn doc_host(&self) -> Option<DocHost> {
        lock(&self.doc_host).clone()
    }

    /// A message the user already sent follows the turn that is ending
    /// (see [`crate::doc_host::ChatDocHandle::continues_with_sent_message`]).
    fn continues_with_sent_message(&self, chat_id: &str) -> bool {
        self.doc_host()
            .and_then(|host| host.open(chat_id).ok())
            .is_some_and(|handle| handle.continues_with_sent_message())
    }

    fn workspace(&self) -> Option<crate::workspace_host::WorkspaceHost> {
        self.doc_host().and_then(|host| host.workspace().cloned())
    }

    /// Sidebar freshness: push a message-persist preview into the chat's workspace row.
    fn note_message(&self, chat_id: &str, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(ws) = self.workspace() {
            ws.note_message(chat_id, text);
        }
    }

    /// Record the chat's harness-native session id (and its cwd): live-process
    /// cache plus the durable workspace chat row — the row is what survives an
    /// engine restart (the original app sessions.ts:1039).
    fn remember_harness_session(&self, chat_id: &str, session_id: &str, cwd: &str) {
        if session_id.is_empty() {
            return;
        }
        lock(&self.harness_sessions).insert(
            chat_id.to_string(),
            HarnessSessionRef {
                session_id: session_id.to_string(),
                cwd: cwd.to_string(),
            },
        );
        if let Some(ws) = self.workspace() {
            ws.set_chat_harness_session(chat_id, session_id, cwd);
        }
    }

    // NB: there is deliberately no `forget_harness_session` anymore. The old
    // tombstone fired on "run died before SessionStarted", which — since the
    // ACP conversion made stale ids a harness-internal fallback — only ever
    // meant a child STARTUP failure, and permanently severed good
    // conversations (user incident 2026-08-13). A truly stale id simply
    // yields a fresh session whose SessionStarted overwrites the row.

    /// The session id to resume for a run in `chat_id` launching from `cwd`
    /// (the original app sessions.ts:736, looked up on every dispatch):
    /// live-process cache → workspace chat row → journal scan (the crash path
    /// where the debounced row write never landed — SessionStarted/Done events
    /// are journaled per event, flushed immediately). Cwd-gated throughout:
    /// harness session stores are keyed by cwd, so a session created elsewhere
    /// never rides `--resume`. An empty stored id is the explicit tombstone —
    /// no resume, no falling through to staler sources.
    fn resume_for(&self, chat_id: &str, cwd: &str) -> Option<String> {
        // The same directory, however it is spelled: an agent reports where
        // it runs with symlinks resolved (`/private/var/…` for `/var/…` on
        // macOS), and until a turn's Done restores the request's spelling a
        // mid-turn crash leaves that one stored — a string compare then
        // started the resumed run in a fresh session, its history gone.
        let cwd_ok = |session_cwd: &str| {
            session_cwd.is_empty()
                || session_cwd == cwd
                || std::fs::canonicalize(session_cwd)
                    .is_ok_and(|a| std::fs::canonicalize(cwd).is_ok_and(|b| a == b))
        };
        if let Some(known) = lock(&self.harness_sessions).get(chat_id).cloned() {
            return (!known.session_id.is_empty() && cwd_ok(&known.cwd))
                .then_some(known.session_id);
        }
        if let Some(ws) = self.workspace()
            && let Some((session_id, session_cwd)) = ws.chat_harness_session(chat_id)
        {
            return (!session_id.is_empty() && cwd_ok(session_cwd.as_deref().unwrap_or("")))
                .then_some(session_id);
        }
        let (session_id, session_cwd) = self.journal_harness_session(chat_id)?;
        // Cache the journal hit (memory + row) so later dispatches skip the scan.
        self.remember_harness_session(chat_id, &session_id, &session_cwd);
        cwd_ok(&session_cwd).then_some(session_id)
    }

    /// The last harness session id named anywhere in the chat's journal, with
    /// the cwd of the `SessionStarted` that governs it. `Done.session_id`
    /// inherits the cwd of the most recent `SessionStarted` (same run).
    ///
    /// Walks the journal newest-first and stops at the answer, so boot recovery
    /// doesn't load whole multi-MB journals: the newest non-empty id wins, and a
    /// `Done` id takes the cwd of the nearest earlier `SessionStarted` (any id).
    fn journal_harness_session(&self, chat_id: &str) -> Option<(String, String)> {
        let warn = |err: crate::run_journal::JournalError| {
            tracing::warn!(chat = %chat_id, error = %err, "journal scan for harness session failed");
        };
        let events = match self.journal.events_rev(chat_id) {
            Ok(events) => events,
            Err(err) => {
                warn(err);
                return None;
            }
        };
        // A `Done` id seen first, still waiting for its run's `SessionStarted` cwd.
        let mut done_id: Option<String> = None;
        for item in events {
            let (seq, event) = match item {
                Ok(item) => item,
                Err(err) => {
                    warn(err);
                    return None;
                }
            };
            if seq == 0 {
                continue; // `replay(_, 0)` (the old full scan) never yielded seq 0
            }
            match event {
                AgentEvent::SessionStarted {
                    session_id, cwd, ..
                } => {
                    if let Some(id) = done_id {
                        return Some((id, cwd));
                    }
                    if !session_id.is_empty() {
                        return Some((session_id, cwd));
                    }
                }
                AgentEvent::Done {
                    session_id: Some(session_id),
                    ..
                } if done_id.is_none() && !session_id.is_empty() => {
                    done_id = Some(session_id);
                }
                _ => {}
            }
        }
        done_id.map(|id| (id, String::new()))
    }

    fn remove_run(&self, chat_id: &str, run_id: &str) {
        let mut runs = lock(&self.runs);
        if runs.get(chat_id).is_some_and(|h| h.run_id == run_id) {
            runs.remove(chat_id);
        }
    }
}

// ── run task ────────────────────────────────────────────────────────────────

/// Which provider session a side chat's prompt goes to.
#[derive(Clone, Copy)]
enum ProviderSession<'a> {
    /// A new provider session: it holds none of the transcript.
    Fresh,
    /// An existing one (id when known): it holds the fork's own turns.
    Continued(Option<&'a str>),
}

/// A run's provider session now holds the fork history its own prompt
/// carried (steers record theirs when confirmed, not here).
fn note_fork_history_session(doc: &SessionDoc, carried: bool, session_id: &str) {
    if carried && !session_id.is_empty() {
        let _ = doc.set_fork_history_session(session_id);
    }
}

impl Inner {
    /// `prompt` with the conversation a side chat's provider session lacks in
    /// front of it, or `None` when it lacks nothing. A fresh session gets the
    /// whole transcript (`current` excluded); a continued one only the fork's
    /// copied history before the seam, until that went out to that session.
    /// Native commands are never wrapped: providers route a leading `/name`.
    fn fork_history_prompt(
        &self,
        chat_id: &str,
        doc: &SessionDoc,
        harness_id: HarnessId,
        prompt: &str,
        current: Option<&str>,
        provider: ProviderSession<'_>,
    ) -> Option<String> {
        if native_command(prompt, harness_id)
            || !self
                .workspace()
                .and_then(|ws| ws.chat(chat_id).ok().flatten())
                .is_some_and(|chat| chat.parent_chat_id.is_some())
        {
            return None;
        }
        let entries: Vec<_> = doc
            .read_entries()
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| Some(entry.id.as_str()) != current)
            .collect();
        let end = match provider {
            ProviderSession::Fresh => entries.len(),
            ProviderSession::Continued(session) => {
                let delivered = doc.fork_history_session();
                if session.is_some() && delivered.as_deref() == session {
                    return None;
                }
                entries.iter().position(|entry| {
                    entry
                        .parts
                        .iter()
                        .any(|part| matches!(part, rcode_doc::MessagePart::Fork { .. }))
                })?
            }
        };
        let history: Vec<_> = entries[..end]
            .iter()
            .map(|entry| {
                let text = entry
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        rcode_doc::MessagePart::Text { text, .. } => Some(text.clone()),
                        rcode_doc::MessagePart::Tool { call, output, .. } => Some(format!(
                            "Tool: {}\n{}",
                            serde_json::to_string(call).unwrap_or_default(),
                            output.clone().unwrap_or_default()
                        )),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let text = doc.command_prompt(&entry.id, &text).unwrap_or(text);
                (entry.role, text)
            })
            .filter(|(_, text)| !text.is_empty())
            .map(|(role, text)| serde_json::json!({ "role": role, "text": text }))
            .collect();
        (!history.is_empty()).then(|| {
            format!(
                "Continue this side conversation using the following prior conversation as context.\n<conversation>\n{}\n</conversation>\n\n{}",
                serde_json::to_string(&history).unwrap_or_default(),
                prompt
            )
        })
    }
}

/// Whether the provider routes this prompt as a native command: its delivered
/// text leads with `/name` (Codex `command_request`, OpenCode commands, Claude
/// slash commands). Anything put in front of it would make it model input.
fn native_command(prompt: &str, harness: HarnessId) -> bool {
    let delivered = if harness == HarnessId::Codex {
        rcode_proto::invocation::invocation_prompt(prompt)
    } else {
        rcode_proto::invocation::harness_prompt(prompt, harness)
    };
    rcode_proto::invocation::leading_command(&delivered).is_some()
}

/// Hydrate legacy display entries from local results without executing tools or
/// mutating the shared document. Missing remote-only data retains its summary.
async fn hydrate_history_outputs(
    journal: Arc<RunJournal>,
    chat_id: String,
    entries: &mut [rcode_doc::SessionMessageEntry],
) {
    let wanted: std::collections::HashSet<String> = entries.iter().filter(|e| !e.excluded)
        .flat_map(|e| &e.parts).filter_map(|part| match part {
            MessagePart::Tool { id, .. } => Some(id.clone()), _ => None,
        }).collect();
    if wanted.is_empty() { return; }
    let outputs = tokio::task::spawn_blocking(move || journal.tool_outputs(&chat_id, wanted)).await;
    let outputs = match outputs {
        Ok(Ok(outputs)) => outputs,
        other => { tracing::warn!(error = ?other, "cannot hydrate local tool history"); return; }
    };
    for entry in entries.iter_mut().filter(|e| !e.excluded) {
        for part in &mut entry.parts {
            if let MessagePart::Tool { id, output, .. } = part {
                if let Some(full) = outputs.get(id) { *output = Some(full.clone()); }
            }
        }
    }
}

/// Render one entry's parts as the text a replayed turn carries.
///
/// Tool activity is flattened into prose rather than kept as `tool_use` /
/// `tool_result` blocks, because a replayed turn must not reference a tool
/// call whose result has since been compacted away — providers reject a
/// `tool_result` with no matching `tool_use`, and the rejection fails the
/// whole request. Flattening keeps every turn self-contained.
///
/// Reasoning is deliberately dropped: it is the model's private scratch, the
/// user does not see it in the transcript, and replaying it would put words in
/// the model's mouth that it never committed to.
///
/// The shape matches [`SessionsEngine::fork_history_prompt`]'s, so a forked
/// chat and a resumed one see the same history — two spellings of the same
/// conversation would be a difference nobody could explain.
fn flatten_parts(parts: &[rcode_doc::MessagePart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            rcode_doc::MessagePart::Text { text, .. } => Some(text.clone()),
            rcode_doc::MessagePart::Tool { call, output, .. } => Some(format!(
                "Tool: {}\n{}",
                serde_json::to_string(call).unwrap_or_default(),
                output.clone().unwrap_or_default()
            )),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A turn is in flight: streaming, or parked on a question it is still owed an
/// answer to. Notably NOT a persistent session that has parked between turns —
/// that holds a warm child with nothing outstanding.
fn is_active(session: &Session) -> bool {
    matches!(
        session.status,
        SessionStatus::Working | SessionStatus::AwaitingInput
    )
}

// ── subagent docs ───────────────────────────────────────────────────────────

/// The per-subagent doc id: `{chatId}--sub--{suffix}`. Constrained by the
/// edge's `ID_RE` (`^[A-Za-z0-9_-]{1,128}$` — the same id names the ChatRoom
/// `chat2/{id}/ws` and the frozen blob `blob/{chatId}/{id}`): a clean, short
/// tool-use id rides verbatim; anything unclean or over budget hashes.
pub(crate) fn subagent_doc_id(chat_id: &str, tool_use_id: &str) -> String {
    let clean = tool_use_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    let budget = 128usize.saturating_sub(chat_id.len() + "--sub--".len());
    if clean && !tool_use_id.is_empty() && tool_use_id.len() <= budget {
        return format!("{chat_id}--sub--{tool_use_id}");
    }
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(tool_use_id.as_bytes());
    let mut hex = String::with_capacity(16);
    for b in &digest[..8] {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    format!("{chat_id}--sub--{hex}")
}

/// Test-only instrumentation: how often `drive_run`'s coalesced commit branch
/// fires. The sink-dirty regression test proves the branch ticks once per
/// commit window instead of spinning on a deadline left in the past.
/// Thread-local so parallel tests' runs can't bleed into the count: the
/// test's current-thread runtime runs its spawned `drive_run` on its own
/// thread.
#[cfg(test)]
thread_local! {
    static FLUSH_TICKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A live subagent transcript sink: its own doc (opened by id — the room
/// `chat2/{docId}/ws` dials automatically, so viewers sync it like a chat),
/// one streaming assistant entry folded from the tagged events. The held
/// doc Arc pins the doc warm for the LRU while the subagent runs.
struct SubagentSink {
    doc_id: String,
    doc: crate::doc_host::DocWriter,
    entry_id: String,
    started_at: i64,
    entry_index: Option<usize>,
    written: Vec<MessagePart>,
    folded: Vec<MessagePart>,
    dirty: bool,
}

impl SubagentSink {
    fn flush(&mut self, device_id: &str) {
        if !self.dirty {
            return;
        }
        // An empty fold still counts as flushed: leaving dirty set would hold
        // the run loop's coalesced-commit guard open on a deadline already in
        // the past, spinning it until the subagent emits real content.
        self.dirty = false;
        if self.folded.is_empty() {
            return;
        }
        let rendered = render_parts(&self.folded);
        let result = match self.entry_index {
            Some(ix) => {
                let mut w = SegmentWriter::resume(&self.doc, ix, std::mem::take(&mut self.written));
                let r = w.sync(&rendered);
                let (ix, written) = w.into_state();
                self.entry_index = Some(ix);
                self.written = written;
                r
            }
            None => {
                match SegmentWriter::begin(&self.doc, &self.entry_id, device_id, self.started_at) {
                    Ok(mut w) => {
                        let r = w.sync(&rendered);
                        let (ix, written) = w.into_state();
                        self.entry_index = Some(ix);
                        self.written = written;
                        r
                    }
                    Err(e) => Err(e),
                }
            }
        };
        if let Err(err) = result {
            // Fail soft: a broken subagent doc degrades to chip-only, never
            // errors the chat.
            tracing::warn!(doc = %self.doc_id, error = %err, "subagent sink flush failed");
        }
    }

    /// A parent→subagent steer ([`AgentEvent::UserMessage`], tagged): close
    /// the open assistant segment (it reads Complete above the steer), write
    /// the steer as its own USER entry, and reset so the next tagged delta
    /// opens a fresh assistant entry below it — the subagent transcript then
    /// reads like any steered chat.
    fn push_user(&mut self, device_id: &str, text: &str) {
        let rendered = render_parts(&self.folded);
        let closed = match self.entry_index.take() {
            Some(ix) => SegmentWriter::resume(&self.doc, ix, std::mem::take(&mut self.written))
                .finish(&rendered, MessageStatus::Complete),
            None if !self.folded.is_empty() => {
                match SegmentWriter::begin(&self.doc, &self.entry_id, device_id, self.started_at) {
                    Ok(w) => w.finish(&rendered, MessageStatus::Complete),
                    Err(e) => Err(e),
                }
            }
            None => Ok(()),
        };
        if let Err(err) = closed {
            tracing::warn!(doc = %self.doc_id, error = %err, "subagent segment close failed");
        }
        let entry = SessionMessageEntry {
            id: new_id(),
            role: MessageRole::User,
            parts: vec![MessagePart::Text {
                id: "t0".into(),
                text: text.to_owned(),
            }],
            created_at: now_ms(),
            device_id: device_id.to_owned(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
            duration_ms: None,
            excluded: false,
        };
        if let Err(err) = self.doc.push_message(&entry) {
            tracing::warn!(doc = %self.doc_id, error = %err, "subagent steer write failed");
        }
        self.entry_id = new_id();
        self.started_at = now_ms();
        self.written = Vec::new();
        self.folded = Vec::new();
        self.dirty = false;
    }

    /// Final flush + entry finalize; returns the transcript JSON for the
    /// frozen blob (entries as the client renders them).
    fn finish(mut self, device_id: &str, status: MessageStatus) -> Option<String> {
        let rendered = render_parts(&self.folded);
        let finished = match self.entry_index {
            Some(ix) => SegmentWriter::resume(&self.doc, ix, std::mem::take(&mut self.written))
                .finish(&rendered, status),
            None if !self.folded.is_empty() => {
                match SegmentWriter::begin(&self.doc, &self.entry_id, device_id, self.started_at) {
                    Ok(w) => w.finish(&rendered, status),
                    Err(e) => Err(e),
                }
            }
            None => Ok(()),
        };
        if let Err(err) = finished {
            tracing::warn!(doc = %self.doc_id, error = %err, "subagent sink finish failed");
        }
        let entries = rcode_doc::join_continuation_entries(self.doc.read_entries().ok()?);
        serde_json::to_string(&entries).ok()
    }
}

/// The parent-chip refresh a tagged event implies, for the in-place (parked)
/// path — LIFECYCLE ONLY, mirroring the fold (live tails were rejected:
/// per-delta chip rewrites grew the parent doc for the whole run).
fn subagent_chip_update(event: &AgentEvent) -> Option<&'static str> {
    match event {
        AgentEvent::Done { status, .. } => Some(match status {
            DoneStatus::Errored => "failed",
            _ => "done",
        }),
        _ => Some("running"),
    }
}

/// 占位随任务结束释放，取消或失败后下一轮可以重试。
///
/// 持有自己拿到的 token：只有仍是当前占位的那个任务能移除它，旧任务不会误删
/// 后来者（例如取消后立刻重试时新任务已插入的槽位）。
struct TitleJobLease {
    inner: Arc<Inner>,
    chat_id: String,
    cancel: CancellationToken,
}

impl Drop for TitleJobLease {
    fn drop(&mut self) {
        let mut jobs = lock(&self.inner.title_jobs);
        if jobs
            .get(&self.chat_id)
            .is_some_and(|current| current == &self.cancel)
        {
            jobs.remove(&self.chat_id);
        }
    }
}

async fn collect_title(
    harness: Arc<dyn Harness>,
    request: RunRequest,
    cancel: CancellationToken,
) -> Result<String, rcode_harness::HarnessError> {
    let controls = RunControls {
        conversation_id: None,
        goal: None,
        request_input: Box::new(|_| {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let _ = tx.send(rcode_proto::InteractionResponse::Dismissed);
            rx
        }),
        steering: tokio::sync::mpsc::channel(1).1,
        interrupt: cancel,
        turn: TurnControl::default(),
        history: None,
    };
    let mut stream = harness.run_title(request, controls).await?;
    let mut text = String::new();
    let mut completed = false;
    while let Some(event) = stream.next().await {
        if completed {
            return Err(rcode_harness::HarnessError::Protocol(
                "event after title completion".into(),
            ));
        }
        match event? {
            AgentEvent::TextDelta { text: delta } => {
                text.push_str(&delta);
                if text.chars().count() > 512 {
                    return Err(rcode_harness::HarnessError::Protocol(
                        "title response too long".into(),
                    ));
                }
            }
            AgentEvent::Done {
                status: DoneStatus::Completed,
                ..
            } => completed = true,
            AgentEvent::SessionStarted { .. }
            | AgentEvent::ReasoningDelta { .. }
            | AgentEvent::ContextUsage { .. }
            | AgentEvent::ModelCall { .. }
            | AgentEvent::Usage { .. } => {}
            _ => {
                return Err(rcode_harness::HarnessError::Protocol(
                    "title request failed or used tools".into(),
                ));
            }
        }
    }
    if !completed {
        return Err(rcode_harness::HarnessError::Protocol(
            "title stream ended without completion".into(),
        ));
    }
    clean_title(&text)
        .ok_or_else(|| rcode_harness::HarnessError::Protocol("invalid short title".into()))
}

/// 只接受完整的单行短标题，不把解释段落截成看似成功的标题。
fn clean_title(raw: &str) -> Option<String> {
    let title = raw.trim();
    let title = title
        .strip_prefix("Title:")
        .or_else(|| title.strip_prefix("title:"))
        .unwrap_or(title)
        .trim();
    let title = title
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '“' | '”'))
        .trim();
    if title.is_empty()
        || title.chars().count() > 60
        || title.chars().any(char::is_control)
        || title.contains('`')
    {
        return None;
    }
    Some(title.to_owned())
}

/// Apply the render-parts privacy policy: strip heavy/sensitive tool inputs before doc
/// entry. Full inputs live only in the local run journal.
fn render_parts(parts: &[MessagePart]) -> Vec<MessagePart> {
    parts
        .iter()
        .map(|part| match part {
            MessagePart::Tool {
                id,
                call,
                is_error,
                resolved,
                output,
                diff,
                output_ref,
                output_bytes,
                diff_ref,
                diff_stats,
                subagent_ref,
                subagent_status,
                subagent_tail,
            } => MessagePart::Tool {
                id: id.clone(),
                call: sanitize_tool_call(call),
                is_error: *is_error,
                resolved: *resolved,
                // Output summaries, diff stats, and sidecar refs are
                // deliberately kept: unlike raw tool inputs they are the
                // transcript's record of what happened, and the strip already
                // bounded them (docs/chat2-sync.md).
                output: output.clone(),
                diff: diff.clone(),
                output_ref: output_ref.clone(),
                output_bytes: *output_bytes,
                diff_ref: diff_ref.clone(),
                diff_stats: diff_stats.clone(),
                subagent_ref: subagent_ref.clone(),
                subagent_status: *subagent_status,
                subagent_tail: subagent_tail.clone(),
            },
            other => other.clone(),
        })
        .collect()
}

/// The persisted assistant text of a folded segment (workspace preview source).
fn folded_text(parts: &[MessagePart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn sync_segment<'a>(
    doc: &'a SessionDoc,
    writer: &mut Option<SegmentWriter<'a>>,
    entry_id: &str,
    device_id: &str,
    started_at: i64,
    folded: &[MessagePart],
) -> Result<(), DocError> {
    if folded.is_empty() {
        return Ok(());
    }
    let rendered = render_parts(folded);
    if writer.is_none() {
        *writer = Some(SegmentWriter::begin(doc, entry_id, device_id, started_at)?);
    }
    if let Some(w) = writer.as_mut() {
        w.sync(&rendered)?;
    }
    Ok(())
}

fn finish_segment<'a>(
    doc: &'a SessionDoc,
    writer: Option<SegmentWriter<'a>>,
    entry_id: &str,
    device_id: &str,
    started_at: i64,
    folded: &[MessagePart],
    status: MessageStatus,
) -> Result<(), DocError> {
    let rendered = render_parts(folded);
    match writer {
        Some(w) => w.finish(&rendered, status),
        None if !folded.is_empty() => {
            SegmentWriter::begin(doc, entry_id, device_id, started_at)?.finish(&rendered, status)
        }
        None => Ok(()),
    }
}

/// Resume bookkeeping for one run task: which user entry the run answers (so
/// the startup-crash retry re-dispatches idempotently against the same doc
/// entry), whether `dispatch` injected the resume id itself (only
/// engine-injected resumes retry — a caller-specified resume fails loudly),
/// and whether this run already IS the retry (one attempt only).
struct RunResumeState {
    idle: bool,
    user_message_id: String,
    resume_injected: bool,
    startup_retry: bool,
    fork_history_sent: Arc<std::sync::atomic::AtomicBool>,
    /// Set once the harness stream is open (see [`RunHandle::started`]).
    started: Arc<std::sync::atomic::AtomicBool>,
}

fn cursor_unstarted_history(
    doc: &SessionDoc,
    current_id: &str,
    prompt: &str,
    has_session: bool,
) -> Result<String, DocError> {
    // Convert each message before JSON encoding. Rewriting canonical chips in
    // the encoded envelope can introduce unescaped quotes or newlines and can
    // cause Cursor's current message to be converted twice.
    let prompt = rcode_proto::invocation::harness_prompt(prompt, HarnessId::Cursor);
    let entries = doc.read_entries()?;
    let preceding: Vec<_> = entries
        .iter()
        .take_while(|entry| entry.id != current_id)
        .collect();
    // With an existing session, only bridge the tail whose assistant never
    // produced content. A process can die before writing its SDK-side receipt,
    // even though an older session ID still exists.
    let after = if has_session {
        preceding
            .iter()
            .rposition(|entry| {
                entry.role == MessageRole::Assistant
                    && entry.parts.iter().any(|part| match part {
                        MessagePart::Text { text, .. } | MessagePart::Reasoning { text, .. } => {
                            !text.is_empty()
                        }
                        MessagePart::Tool { .. } | MessagePart::Input { .. } => true,
                        _ => false,
                    })
            })
            .map_or(0, |i| i + 1)
    } else {
        0
    };
    let previous: Vec<String> = preceding
        .iter()
        .skip(after)
        .filter(|entry| entry.role == MessageRole::User)
        .map(|entry| {
            entry
                .parts
                .iter()
                .filter_map(|part| match part {
                    MessagePart::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|text| !text.is_empty())
        .map(|text| rcode_proto::invocation::harness_prompt(&text, HarnessId::Cursor))
        .collect();
    if previous.is_empty() {
        return Ok(prompt);
    }
    Ok(format!(
        "The preceding user messages may not have reached a Cursor checkpoint before startup stopped. Retain this JSON as conversation history; do not rerun prior tools or side effects. Respond to the current message.\n{}",
        serde_json::json!({"previousUserMessages": previous, "currentUserMessage": prompt})
    ))
}

#[allow(clippy::too_many_arguments)]
async fn drive_run(
    inner: Arc<Inner>,
    chat_id: String,
    run_id: String,
    harness: Arc<dyn Harness>,
    mut request: RunRequest,
    doc: crate::doc_host::DocWriter,
    controls: RunControls,
    mut engine_rx: mpsc::UnboundedReceiver<AgentEvent>,
    mut cancel_rx: watch::Receiver<bool>,
    resume_state: RunResumeState,
    turn: TurnControl,
) {
    let device_id = inner.device_id.clone();
    // Captured for post-run auto-titling (the request moves into the harness).
    let harness_id = harness.id();
    let title_request = request.clone();
    let run_cwd = request.cwd.clone();
    if request.resume.is_none() {
        let _ = doc.clear_context_usage();
    }
    // Kept whole for the startup-crash retry (same user entry; dispatch
    // re-injects the stored resume id). Option so the retry branch (inside
    // the event loop) can take ownership.
    let mut retry_request = Some(RunRequest {
        resume: None,
        ..request.clone()
    });
    if harness_id == HarnessId::Native {
        if let Some(prompt) = doc.command_prompt(&resume_state.user_message_id, &request.prompt) {
            request.prompt = prompt;
        }
    }
    // A side chat owns a fresh provider session. Bootstrap it from the frozen
    // conversation, never resume (and mutate) the parent's provider session.
    let mut carried_history = false;
    let provider = match request.resume.as_deref() {
        None => ProviderSession::Fresh,
        resumed => ProviderSession::Continued(resumed),
    };
    if !resume_state.idle
        && let Some(prompt) = inner.fork_history_prompt(
            &chat_id,
            &doc,
            harness_id,
            &request.prompt,
            Some(&resume_state.user_message_id),
            provider,
        )
    {
        request.prompt = prompt;
        carried_history = true;
        resume_state
            .fork_history_sent
            .store(true, std::sync::atomic::Ordering::Release);
    }
    // Startup can stop before the SDK saves user text, with no new session
    // ID or receipt. Bridge that unacknowledged tail from our transcript;
    // a fresh session needs all prior user text, not just the latest tail.
    let prepared = if harness_id == HarnessId::Cursor {
        cursor_unstarted_history(
            &doc,
            &resume_state.user_message_id,
            &request.prompt,
            request.resume.is_some(),
        )
        .map(|prompt| request.prompt = prompt)
        .map_err(|e| rcode_harness::HarnessError::Protocol(e.to_string()))
    } else {
        Ok(())
    };
    let started = match prepared {
        Ok(()) => {
            if !controls.interrupt.is_cancelled() {
                if let Some(listener) = inner.turn_listener.get() {
                    listener(&chat_id, &request.cwd);
                }
                let mut wire_request = request;
                if !matches!(harness_id, HarnessId::Cursor | HarnessId::Opencode) {
                    wire_request.prompt =
                        rcode_proto::invocation::harness_prompt(&wire_request.prompt, harness_id);
                }
                if resume_state.idle {
                    harness.start_idle(wire_request, controls).await
                } else {
                    harness.run(wire_request, controls).await
                }
            } else {
                Ok(futures::stream::once(async {
                    Ok(AgentEvent::Done {
                        status: DoneStatus::Interrupted,
                        result: None,
                        error: None,
                        session_id: None,
                    })
                })
                .boxed())
            }
        }
        Err(error) => Err(error),
    };
    let mut stream = match started {
        Ok(stream) => {
            resume_state
                .started
                .store(true, std::sync::atomic::Ordering::Release);
            stream
        }
        Err(err) => {
            let message = err.to_string();
            tracing::warn!(chat = %chat_id, harness = ?harness_id, error = %message, "run failed to start");
            // The journal alone is live-only: without an entry the transcript
            // shows "Run failed" with no reason (an OpenCode server that never
            // booted looked exactly like that).
            let parts = [MessagePart::Error {
                id: "e0".into(),
                message: message.clone(),
            }];
            if let Err(err) = finish_segment(
                &doc,
                None,
                &new_id(),
                &device_id,
                now_ms(),
                &parts,
                MessageStatus::Complete,
            ) {
                tracing::warn!(chat = %chat_id, error = %err, "start failure entry failed");
            }
            inner.publish(
                &chat_id,
                &AgentEvent::Error {
                    message: message.clone(),
                },
            );
            inner.publish(
                &chat_id,
                &AgentEvent::Done {
                    status: DoneStatus::Errored,
                    result: None,
                    error: Some(message),
                    session_id: None,
                },
            );
            inner.remove_run(&chat_id, &run_id);
            inner.set_status(&chat_id, SessionStatus::Errored, false);
            return;
        }
    };

    let doc_ref: &SessionDoc = &doc;
    let mut folded: Vec<MessagePart> = Vec::new();
    // Every tool id this run has folded, across segment resets. Adapters
    // re-emit shape-bearing `tool_call_update`s (title/rawInput refreshes,
    // long-running completions) as full ToolCall events; once the fold has
    // reset at a steer/park boundary those ids are gone from `folded`, and
    // folding the echo would mint an orphan chip mid-text in the NEXT
    // segment — the mid-word transcript splits.
    let mut seen_tools: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_images = std::collections::HashSet::new();
    for entry in doc_ref.read_entries().unwrap_or_default() {
        for part in entry.parts {
            if let MessagePart::Image { id, .. } = part {
                seen_images.insert(format!("{chat_id}\0{id}"));
                if let Some(tool_id) = id.strip_suffix(":image") {
                    seen_tools.insert(tool_id.to_owned());
                }
            }
        }
    }
    let mut prepared_events = std::collections::VecDeque::new();
    let mut entry_id = new_id();
    let mut segment_started = now_ms();
    let mut writer: Option<SegmentWriter<'_>> = None;
    let mut dirty = false;
    let mut flush_at = tokio::time::Instant::now();
    // Set when the engine interrupts the run: the harness gets this long to end its own
    // stream (its token was cancelled); past it, a terminal Done is synthesized.
    let mut interrupt_deadline: Option<tokio::time::Instant> = None;
    let mut interrupted = false;
    let mut saw_session_started = false;
    // Liveness heartbeat: this loop RUNNING is proof the harness stream is
    // open, so freshness must not depend on events arriving. Silent stretches
    // are normal and UNBOUNDED — a long tool call, redacted thinking, an
    // agent waiting on an external process, a question parked for an hour —
    // and each starved the UI's 45s staleness gate in turn (working strip /
    // AwaitingInput dot vanishing mid-run, both user-reported). No stall
    // timeout here by design (a first port was rejected — agents may
    // legitimately be quiet for >10min): a live child means Working, dying
    // paths each carry their own error, and engine death stops these ticks
    // so the gate still catches real crashes. touch_session throttles at 10s.
    let mut live_heartbeat = tokio::time::interval(std::time::Duration::from_secs(15));
    live_heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // PERSISTENT SESSION (the original app runsBySession): a completed turn on a
    // steerable harness parks here instead of ending the run — the child and
    // its steering mailbox stay warm, and the next user message (dispatch
    // routes into a live run) starts the next turn with zero respawn/resume
    // latency. `Some(when)` = idle since then; the 30-min reaper below ends
    // a session nobody comes back to (the original app SESSION_IDLE_MS).
    // `RCODE_SESSION_IDLE_MS` overrides it (tests).
    let session_idle = std::env::var("RCODE_SESSION_IDLE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_millis)
        .unwrap_or(std::time::Duration::from_secs(30 * 60));
    // A live subagent stretches the window, but never unbounded: every
    // driver's subagent terminal is a best-effort vendor signal (claude's
    // untagged task_notification, grok's subagent_finished, codex thread
    // closure), and one lost signal would otherwise pin the child — and this
    // loop's registry heartbeat — until the chat is archived. A subagent
    // silent this long (4h by default) is presumed lost; the reap stamps its
    // chip failed.
    let subagent_silence = session_idle * 8;
    let mut idle_since: Option<tokio::time::Instant> =
        resume_state.idle.then(tokio::time::Instant::now);
    // A parked runtime whose agent reports background work (see
    // `TurnControl::background_live`) is not idle: reaping it killed that
    // work. The reaper re-checks a full idle window later instead.
    let mut reap_deferred_until: Option<tokio::time::Instant> = None;
    // The reaper last deferred for background work. Its end is not idleness
    // either: the agent reports the drained set a moment before the turn the
    // completion wakes, and reaping in that gap lost the wake (issue #831).
    let mut held_for_background = false;
    let steerable = harness.supports_steering();
    let stops_turn_in_place = steerable && harness.stops_turn_in_place();
    // TURN-QUIESCE WATCHDOG (2026-08-12 stuck-Working incident): a harness
    // that loses a turn's Done — the adapter never settles `session/prompt`
    // even though the agent finished — strands Working forever: the live
    // heartbeat above keeps the row fresh, and there is no per-turn timeout
    // by design. This is NOT that stall timeout: it never ends the run or
    // errors anything. When the stream has been silent past the window AND
    // the fold shows completed output with nothing in flight (no unresolved
    // tool, no open question), the turn parks exactly like a Done would —
    // segment finalized Complete, status Idle, child and mailbox warm. A
    // false trip (the agent was quietly waiting on something invisible)
    // costs a status dip: the parked-resume path below re-arms Working the
    // moment output flows again, and nothing is lost. `RCODE_TURN_QUIESCE_MS`
    // overrides the window; 0 disables.
    // RETIRED for native drivers: a harness whose every turn shape ends with
    // a deterministic wire Done (claude/codex/cursor native) needs no
    // quiesce backstop — arming one only risks false parks on long silent
    // work. The env knob can configure a diagnostic window, but a prompt
    // with an authoritative completion signal must still await that signal.
    // ACP retains the watchdog only for unowned self-continued activity.
    let deterministic_turn_end = harness.deterministic_turn_end();
    let authoritative_prompt_end = harness.authoritative_prompt_end();
    let quiesce_after: Option<std::time::Duration> = match std::env::var("RCODE_TURN_QUIESCE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        Some(0) => None,
        Some(ms) => Some(std::time::Duration::from_millis(ms)),
        None if deterministic_turn_end => None,
        None => Some(std::time::Duration::from_secs(120)),
    };
    let mut last_stream_activity = tokio::time::Instant::now();
    // SELF-CONTINUED turns get a much SHORTER quiesce window. A turn the
    // agent starts on its own (background-task wake) can never receive a
    // harness Done: the adapter has no `session/prompt` outstanding to
    // settle — verified in claude-agent-acp's autonomous-result lane, which
    // consumes the SDK's turn-end without emitting anything; codex shows
    // the same shape. The watchdog is that turn shape's ONLY settle path,
    // so the default 120s window read as 2min of stuck-Working after every
    // background notification (user report 2026-08-13). The in-flight
    // fold gate below still protects running tools; reasoning heartbeats
    // push the window during real thinking. `RCODE_SELF_TURN_QUIESCE_MS`
    // overrides; 0 falls back to the normal window. An explicit
    // `RCODE_TURN_QUIESCE_MS=0` still disables the watchdog entirely.
    let self_quiesce_after: Option<std::time::Duration> =
        match std::env::var("RCODE_SELF_TURN_QUIESCE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            Some(0) => None,
            Some(ms) => Some(std::time::Duration::from_millis(ms)),
            None => Some(std::time::Duration::from_secs(20)),
        };
    let mut self_continued_turn = false;
    // Spawn chips that have SETTLED (tagged Done seen). Content events for a
    // settled chip with no live sink are dropped — a straggler frame after
    // the freeze must not mint a new doc entry or wedge the transcript back
    // into Streaming. Only a steer (UserMessage) legitimately REOPENS a
    // settled subagent: it announces more work is coming.
    let mut settled_subagents: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Live subagent sinks, parent tool-use id → transcript doc state.
    let mut subagents: std::collections::HashMap<String, SubagentSink> =
        std::collections::HashMap::new();
    // Last tagged subagent event. A background subagent outlives the turn that
    // spawned it, and its traffic never un-parks the chat, so the idle reaper
    // measures from here as well as from the park.
    let mut last_subagent_activity: Option<tokio::time::Instant> = None;

    let mut final_completed_turn = None;
    let final_status = loop {
        let event: AgentEvent = if let Some(event) = prepared_events.pop_front() {
            event
        } else {
            let raw_event = tokio::select! {
                biased;
                changed = cancel_rx.changed(), if !interrupted => {
                    let _ = changed;
                    interrupted = true;
                    interrupt_deadline = Some(
                        tokio::time::Instant::now() + std::time::Duration::from_secs(3),
                    );
                    continue;
                }
                _ = tokio::time::sleep_until(
                    interrupt_deadline.unwrap_or_else(tokio::time::Instant::now)
                ), if interrupt_deadline.is_some() => AgentEvent::Done {
                    status: DoneStatus::Interrupted,
                    result: None,
                    error: None,
                    session_id: None,
                },
                _ = live_heartbeat.tick() => {
                    inner.touch_session(&chat_id);
                    continue;
                }
                // Idle reaper (the original app SESSION_IDLE_MS): a parked persistent session
                // nobody returned to within its window releases its child. The turn
                // was finalized at Done, so this end is clean — no aborted stamp.
                // A live background subagent is somebody still using the child:
                // the window counts from its last activity, and stretches to
                // `subagent_silence` while its sink is open.
                _ = tokio::time::sleep_until(
                    {
                        let window = if subagents.is_empty() { session_idle } else { subagent_silence };
                        let base = idle_since
                            .map(|at| at.max(last_subagent_activity.unwrap_or(at)) + window);
                        match (base, reap_deferred_until) {
                            (Some(base), Some(deferred)) => base.max(deferred),
                            (base, deferred) => base.or(deferred).unwrap_or_else(tokio::time::Instant::now),
                        }
                    }
                ), if idle_since.is_some() => {
                    // The driver's own report of background work (Claude's
                    // background agents and shells) is authoritative — it
                    // lists every task the process still runs — so it holds
                    // the runtime for as long as that work lives.
                    if turn.background_live() {
                        tracing::info!(chat = %chat_id, "idle reaper deferred: the agent reports background work");
                        reap_deferred_until = Some(tokio::time::Instant::now() + session_idle);
                        held_for_background = true;
                        continue;
                    }
                    if std::mem::take(&mut held_for_background) {
                        // The work just ended: a full idle window from here.
                        reap_deferred_until = Some(tokio::time::Instant::now() + session_idle);
                        continue;
                    }
                    tracing::info!(
                        chat = %chat_id,
                        live_subagents = subagents.len(),
                        "reaping idle persistent session"
                    );
                    if let Some(token) = lock(&inner.runs)
                        .get(&chat_id)
                        .filter(|h| h.run_id == run_id)
                        .map(|h| h.interrupt_token.clone())
                    {
                        token.cancel();
                    }
                    break SessionStatus::Idle;
                }
                Some(event) = engine_rx.recv() => event,
                next = stream.next() => match next {
                    Some(Ok(event)) => event,
                    // A stream error while PARKED is a post-turn child death —
                    // the turn was already finalized, so the run ends clean
                    // instead of stamping a completed session Errored.
                    Some(Err(err)) if idle_since.is_some() => {
                        tracing::warn!(chat = %chat_id, error = %err, "parked session child died; ending clean");
                        break SessionStatus::Idle;
                    }
                    Some(Err(err)) => AgentEvent::Done {
                        status: DoneStatus::Errored,
                        result: None,
                        error: Some(err.to_string()),
                        session_id: None,
                    },
                    None if interrupted => AgentEvent::Done {
                        status: DoneStatus::Interrupted,
                        result: None,
                        error: None,
                        session_id: None,
                    },
                    // Stream end while PARKED idle: a per-turn adapter closing
                    // after its final Done — a clean end, not a crash (the turn
                    // was already finalized). Persistent adapters keep the
                    // stream open and never hit this.
                    None if idle_since.is_some() => break SessionStatus::Idle,
                    None => AgentEvent::Done {
                        status: DoneStatus::Errored,
                        result: None,
                        error: Some("harness stream ended without Done".into()),
                        session_id: None,
                    },
                },
                _ = tokio::time::sleep_until(flush_at), if dirty || subagents.values().any(|s| s.dirty) => {
                    #[cfg(test)]
                    FLUSH_TICKS.with(|t| t.set(t.get() + 1));
                    // Coalesced STREAM_COMMIT_MS tick: one doc commit per window
                    // (parent + any dirty subagent docs).
                    if dirty {
                        if let Err(err) = sync_segment(
                            doc_ref, &mut writer, &entry_id, &device_id, segment_started, &folded,
                        ) {
                            tracing::warn!(chat = %chat_id, error = %err, "segment sync failed");
                        }
                        dirty = false;
                    }
                    for sink in subagents.values_mut() {
                        sink.flush(&device_id);
                    }
                    continue;
                }
                // Turn-quiesce watchdog (see the knob above). Armed only when the
                // fold says nothing is in flight: an unresolved tool part is a
                // command still running (legitimately silent for minutes — the
                // rejected stall-timeout case), an unresolved input part is a
                // question awaiting the user. The live-plan chip is exempt from
                // the tool check: it is a singleton that never resolves. An EMPTY
                // fold still arms — a Steered boundary that no output ever
                // follows is one of the wedge shapes — it just parks without
                // writing a segment (an empty finalize would leave a stub entry).
                _ = tokio::time::sleep_until({
                    let mut window = quiesce_after.unwrap_or_default();
                    if self_continued_turn && let Some(short) = self_quiesce_after {
                        window = window.min(short);
                    }
                    last_stream_activity + window
                }), if quiesce_after.is_some()
                    && (self_continued_turn || !authoritative_prompt_end)
                    && idle_since.is_none()
                    && !interrupted
                    && steerable
                    && !folded.iter().any(|p| match p {
                        MessagePart::Tool { id, resolved: false, .. } => {
                            id != rcode_proto::LIVE_PLAN_TOOL_ID
                        }
                        MessagePart::Input { resolved: false, .. } => true,
                        _ => false,
                    }) =>
                {
                    tracing::warn!(
                        chat = %chat_id,
                        quiet_ms = quiesce_after.unwrap_or_default().as_millis() as u64,
                        "turn quiesced: stream silent after completed output with no \
                         turn-end; parking (suspected missing harness Done)"
                    );
                    // Some adapters close a completed response through this
                    // engine watchdog instead of a native Done. Preserve that
                    // completion notice, but never notify for an empty boundary
                    // or while an accepted steer still awaits delivery.
                    let completed_turn = ((!folded.is_empty() || writer.is_some())
                        && !inner.has_pending_steers(&chat_id, &run_id)
                        && !inner.continues_with_sent_message(&chat_id))
                        .then(|| entry_id.clone());
                    if !folded.is_empty() || writer.is_some() {
                        if let Err(err) = finish_segment(
                            doc_ref,
                            writer.take(),
                            &entry_id,
                            &device_id,
                            segment_started,
                            &folded,
                            MessageStatus::Complete,
                        ) {
                            tracing::warn!(chat = %chat_id, error = %err, "quiesce segment finish failed");
                        }
                        inner.note_message(&chat_id, &folded_text(&folded));
                    }
                    folded.clear();
                    dirty = false;
                    entry_id = new_id();
                    segment_started = now_ms();
                    idle_since = Some(tokio::time::Instant::now());
                    reap_deferred_until = None;
                    self_continued_turn = false;
                    inner.set_status_with_completion(
                        &chat_id, SessionStatus::Idle, false, completed_turn,
                    );
                    continue;
                }
            };

            prepared_events.extend(
                inner
                    .prepare_generated_image(&chat_id, raw_event, &mut seen_images)
                    .await,
            );
            let Some(event) = prepared_events.pop_front() else {
                continue;
            };
            event
        };

        // ── subagent routing ───────────────────────────────────────────
        // Tagged events NEVER fold into the parent transcript: they stream
        // into the subagent's own doc, and the parent keeps only the spawn
        // chip (ref + status + one-line tail) — refreshed through the live
        // fold while its segment still streams, else stamped in place (the
        // eager-done norm: the chip's entry finished before the background
        // subagent spoke). Handled BEFORE the parked gate so a parked
        // session's background traffic still reaches the subagent doc
        // without un-parking the chat.
        if let AgentEvent::Subagent {
            parent_tool_use_id,
            event: sub_event,
        } = &event
        {
            inner.publish(&chat_id, &event);
            last_subagent_activity = Some(tokio::time::Instant::now());
            // Background subagents can outlive the turn and be the only
            // traffic for minutes: keep the row (and its count) inside the
            // UI's staleness window while any is live.
            if !subagents.is_empty() {
                inner.touch_session(&chat_id);
            }
            let is_steer = matches!(
                sub_event.as_ref(),
                AgentEvent::UserMessage { .. } | AgentEvent::Steered { .. }
            );
            if is_steer {
                settled_subagents.remove(parent_tool_use_id);
            } else if settled_subagents.contains(parent_tool_use_id)
                && !subagents.contains_key(parent_tool_use_id)
            {
                // Straggler after the freeze: chip-only silence (the old
                // pre-viz behavior), never a reopened doc.
                continue;
            }
            let sub_id = subagent_doc_id(&chat_id, parent_tool_use_id);
            let chip_streaming = folded
                .iter()
                .any(|p| matches!(p, MessagePart::Tool { id, .. } if id == parent_tool_use_id));
            let sink_known = subagents.contains_key(parent_tool_use_id);
            // A Done with NO sink (a subagent that never streamed — codex
            // turn ends can beat registration) is chip-only: minting a doc
            // just to freeze it empty helps no one, and stamping the ref
            // would link the chip to that never-created doc (an empty tab
            // on click).
            let done_only = !sink_known && matches!(sub_event.as_ref(), AgentEvent::Done { .. });
            if chip_streaming {
                if !sink_known && !done_only {
                    for p in folded.iter_mut() {
                        if let MessagePart::Tool {
                            id,
                            call,
                            subagent_ref,
                            ..
                        } = p
                            && id == parent_tool_use_id
                            // Genus gate: a subagent ref may only ever bind
                            // to a SPAWN call — mis-keyed tagged traffic
                            // must not turn an ordinary chip into a spawn
                            // link (empty tab on click).
                            && call.is_subagent_spawn()
                        {
                            *subagent_ref = Some(sub_id.clone());
                        }
                    }
                }
                rcode_doc::fold_event_into_parts(&mut folded, &event);
                if !dirty {
                    dirty = true;
                    flush_at = tokio::time::Instant::now()
                        + std::time::Duration::from_millis(STREAM_COMMIT_MS);
                }
            }
            // Open the sink lazily; an open failure degrades to chip-only.
            if !sink_known && !done_only {
                let opened = inner.doc_host().and_then(|host| match host.open(&sub_id) {
                    Ok(handle) => handle.writer().ok(),
                    Err(err) => {
                        tracing::warn!(doc = %sub_id, error = %err, "subagent doc open failed (chip-only)");
                        None
                    }
                });
                if let Some(sub_doc) = opened {
                    subagents.insert(
                        parent_tool_use_id.clone(),
                        SubagentSink {
                            doc_id: sub_id.clone(),
                            doc: sub_doc,
                            entry_id: new_id(),
                            started_at: now_ms(),
                            entry_index: None,
                            written: Vec::new(),
                            folded: Vec::new(),
                            dirty: false,
                        },
                    );
                    inner.set_running_subagents(&chat_id, subagents.len());
                    if !chip_streaming {
                        let _ = doc_ref.update_subagent_chip(
                            parent_tool_use_id,
                            Some(&sub_id),
                            Some("running"),
                            None,
                        );
                    }
                }
            }
            let done = matches!(sub_event.as_ref(), AgentEvent::Done { .. });
            if done {
                settled_subagents.insert(parent_tool_use_id.clone());
                if !chip_streaming {
                    // A resumed run can finish an older chip without ever
                    // opening a sink. The chip's lifecycle is independent of
                    // whether this run received transcript content.
                    let _ = doc_ref.update_subagent_chip(
                        parent_tool_use_id,
                        None,
                        subagent_chip_update(sub_event),
                        None,
                    );
                }
            }
            if let Some(sink) = subagents.get_mut(parent_tool_use_id) {
                if let AgentEvent::UserMessage { text } = sub_event.as_ref() {
                    // A steer splits ENTRIES, not parts — handled at the
                    // sink level (the fold ignores UserMessage).
                    sink.push_user(&device_id, text);
                    continue;
                }
                let was_clean = !sink.dirty;
                rcode_doc::fold_event_into_parts(&mut sink.folded, sub_event);
                sink.dirty = true;
                // A sink waking on its own must arm the same commit window
                // the parent's dirty flag does — `flush_at` is otherwise only
                // rescheduled on the parent's flip, so a past deadline would
                // fire the commit branch on every event instead of once per
                // STREAM_COMMIT_MS window.
                if was_clean && !dirty && flush_at <= tokio::time::Instant::now() {
                    flush_at = tokio::time::Instant::now()
                        + std::time::Duration::from_millis(STREAM_COMMIT_MS);
                }
                if done {
                    let status = match sub_event.as_ref() {
                        AgentEvent::Done {
                            status: DoneStatus::Errored,
                            ..
                        } => MessageStatus::Complete,
                        AgentEvent::Done {
                            status: DoneStatus::Interrupted,
                            ..
                        } => MessageStatus::Aborted,
                        _ => MessageStatus::Complete,
                    };
                    let sink = subagents.remove(parent_tool_use_id).expect("checked");
                    inner.set_running_subagents(&chat_id, subagents.len());
                    let doc_id = sink.doc_id.clone();
                    // FREEZE: the finished transcript uploads as a static endpoint
                    // blob (`blob/{chatId}/{subDocId}`) so viewers of
                    // finished subagents never wake the doc's room; dropping
                    // the sink unpins the doc for the LRU and the room
                    // idles. The live doc remains the fallback.
                    if let Some(json) = sink.finish(&device_id, status)
                        && let Some(host) = inner.doc_host()
                    {
                        host.upload_tool_sidecar(
                            &chat_id,
                            rcode_doc::SidecarPayload {
                                part_id: doc_id,
                                output: Some(json),
                                diff: None,
                            },
                        );
                    }
                }
            }
            continue;
        }

        // Any stream activity proves the run is alive — keep the session's
        // freshness inside the UI's 45s staleness window (throttled), and
        // push the quiesce watchdog's window out.
        inner.touch_session(&chat_id);
        last_stream_activity = tokio::time::Instant::now();
        // The engine's input bridge is the sole authority on input requests:
        // it mints the id and parks the resolver BEFORE emitting the event,
        // so a legitimate id is always pending here. A harness emitting its
        // own copy (a different id no resolver knows) would fold an
        // unanswerable twin chip into the doc — and answering the twin would
        // never resume the run. Dropped BEFORE the parked gate below: letting
        // it un-park the session and then dropping it would disarm the idle
        // reaper with no turn running (a leaked child no reap ever ends).
        if let AgentEvent::InputRequested { request_id, .. } = &event {
            let pending = lock(&inner.runs)
                .get(&chat_id)
                .map(|h| h.pending_inputs.clone());
            let known = pending.is_some_and(|p| lock(&p).contains_key(request_id));
            if !known {
                tracing::warn!(
                    chat = %chat_id,
                    request = %request_id,
                    "dropping harness-emitted InputRequested (unknown id; \
                     the engine input bridge owns this lifecycle)"
                );
                continue;
            }
        }
        // Capacity/occupancy can settle after Done; updating it must not reopen a turn.
        if let AgentEvent::ContextUsage { tokens, window } = &event {
            if let Err(err) = doc_ref.update_context_usage(*tokens, *window) {
                tracing::warn!(%chat_id, error = %err, "context usage write failed");
            }
            continue;
        }
        // Compaction is reported, not persisted: its effect is already in the
        // transcript (the summary turn the loop appended), and a run that
        // compacted silently is one the user cannot explain afterwards.
        if let AgentEvent::Compacted { stage, shortened } = &event {
            tracing::info!(%chat_id, stage, shortened, "the conversation was compacted");
            continue;
        }
        // PARKED: a steer boundary, a terminal Done, or SELF-CONTINUED OUTPUT
        // re-opens the session; everything else stays gated. The ACP child
        // keeps forwarding `session/update` frames after a turn completes,
        // and they split two ways:
        //
        // - Post-turn NOISE — late tool_call_updates for commands folded in a
        //   prior segment, command refreshes, reasoning heartbeats. Treating
        //   those as "the next turn" re-armed Working with no Done ever
        //   coming (the eternally-running session bug) and folded orphan
        //   parts into a phantom segment. Still dropped.
        // - SELF-CONTINUED WORK — Claude Code re-invokes itself when a
        //   background task finishes (turns no prompt started) and streams
        //   real output for them. Dropping those LOST transcript content
        //   (2026-08-12: "Build finished successfully…" streamed by the
        //   agent, absent from the doc). Fresh text or a genuinely new tool
        //   call resumes the session: new segment, Working, and the turn
        //   settles again via Done — or via the quiesce watchdog, which is
        //   what makes this resume safe where the naive version was not.
        //
        // The RESUME_GATE separates the two by arrival time: a finished
        // turn's tail flush lands within milliseconds of its Done, while a
        // self-continued turn starts a whole new agent round trip (seconds
        // at minimum, minutes in the incident). Inside the gate everything
        // non-boundary stays inert, exactly as before.
        let turn_was_active = idle_since.is_none();
        const RESUME_GATE: std::time::Duration = std::time::Duration::from_secs(1);
        if idle_since.is_some() {
            let self_continued = idle_since
                .is_some_and(|parked_at| parked_at.elapsed() >= RESUME_GATE)
                && (matches!(
                    &event,
                    AgentEvent::TextDelta { text } if !text.is_empty()
                ) || matches!(
                    &event,
                    AgentEvent::ToolCall { id, .. }
                        if id == rcode_proto::LIVE_PLAN_TOOL_ID || !seen_tools.contains(id)
                ));
            if self_continued {
                tracing::info!(
                    chat = %chat_id,
                    "parked session resumed by self-continued agent output"
                );
                idle_since = None;
                self_continued_turn = true;
                // The park cleared the fold; rotate to a fresh entry and
                // fall through — this event is the new segment's first part.
                entry_id = new_id();
                segment_started = now_ms();
                inner.set_status(&chat_id, SessionStatus::Working, true);
            } else {
                match &event {
                    AgentEvent::SessionStarted { .. }
                        if resume_state.idle && !saw_session_started => {}
                    AgentEvent::Steered { .. } | AgentEvent::BackgroundWake | AgentEvent::GoalWake => {
                        idle_since = None;
                        inner.set_status(&chat_id, SessionStatus::Working, true);
                    }
                    AgentEvent::Done { .. } => {
                        idle_since = None;
                    }
                    // A question with NO turn behind it (post-turn input
                    // noise): answer it empty right here. Un-parking into
                    // AwaitingInput would disarm the idle reaper with no Done
                    // ever coming — stranded status, leaked warm child.
                    AgentEvent::InputRequested { request_id, .. } => {
                        let resolver = lock(&inner.runs)
                            .get(&chat_id)
                            .and_then(|h| lock(&h.pending_inputs).remove(request_id));
                        if let Some(entry) = resolver {
                            let _ = entry
                                .resolver
                                .send(rcode_proto::InteractionResponse::Dismissed);
                        }
                        tracing::debug!(chat = %chat_id, "parked session: post-turn input request auto-declined");
                        continue;
                    }
                    // A stale answer settling after its turn already closed:
                    // nothing is running — stay parked.
                    AgentEvent::InputResolved { .. } => continue,
                    _ => continue,
                }
            }
        }
        // Empty reasoning deltas are PURE heartbeats: redacted thinking and
        // tool-input-generation windows stream them with no text. They fold
        // to nothing, so journaling/publishing them is only noise (hundreds
        // per long turn observed) — the touch above already did their job.
        if matches!(&event, AgentEvent::ReasoningDelta { text } if text.is_empty()) {
            continue;
        }

        // Stale tool echoes: a ToolCall/ToolResult naming an id folded in a
        // PRIOR segment (the fold reset at a steer or park since) belongs to
        // a chip that already rendered in its own entry. Folding the echo
        // would splice a phantom chip into the middle of the current
        // segment's streaming text (the mid-word transcript splits). Dropped;
        // same-segment refreshes still land in place.
        let in_segment = |folded: &[MessagePart], id: &str| {
            folded
                .iter()
                .any(|p| matches!(p, MessagePart::Tool { id: pid, .. } if pid == id))
        };
        match &event {
            // The live plan chip is a deliberate singleton (every update
            // reuses `LIVE_PLAN_TOOL_ID` so the fold refreshes in place):
            // treating its reappearance after a park/steer reset as a stale
            // echo dropped the todo list for the rest of the run — from the
            // first boundary on, plans never rendered again.
            AgentEvent::ToolCall { id, .. } if id == rcode_proto::LIVE_PLAN_TOOL_ID => {}
            AgentEvent::ToolResult { id, .. } if id == rcode_proto::LIVE_PLAN_TOOL_ID => {}
            AgentEvent::ToolCall { id, .. } => {
                if !in_segment(&folded, id) && seen_tools.contains(id) {
                    continue;
                }
                seen_tools.insert(id.clone());
            }
            AgentEvent::ToolResult { id, .. }
                if !in_segment(&folded, id) && seen_tools.contains(id) =>
            {
                continue;
            }
            _ => {}
        }

        // Startup-crash retry: a run that dies before ever starting (errored
        // Done, no SessionStarted, nothing streamed) means the AGENT CHILD
        // failed to come up — not that the injected resume id was bad. Since
        // the ACP conversion (2026-08-08) a stale id is handled inside the
        // harness (`session/load` falls back to `session/new`), so the old
        // guess here — tombstone the id, retry fresh — fired only on child
        // startup failures and permanently severed GOOD conversations (user
        // incident 2026-08-13). The id stays; retry ONCE against the same
        // user entry, resume and all, in case the crash was transient. A
        // helper that is down hard fails the retry too and surfaces its
        // crash text (the harness now appends exit status + stderr).
        if resume_state.resume_injected
            && !resume_state.idle
            && !resume_state.startup_retry
            && !saw_session_started
            && folded.is_empty()
            && !interrupted
            && matches!(
                &event,
                AgentEvent::Done {
                    status: DoneStatus::Errored,
                    ..
                }
            )
            && let Some(retry) = retry_request.take()
        {
            tracing::warn!(
                chat = %chat_id,
                "run died before session start; retrying once (resume kept)"
            );
            inner.remove_run(&chat_id, &run_id);
            let engine = SessionsEngine {
                inner: inner.clone(),
            };
            let chat = chat_id.clone();
            let message_id = resume_state.user_message_id.clone();
            tokio::spawn(async move {
                // The user entry write inside dispatch is idempotent by
                // message id; `startup_retry` makes this attempt final.
                if let Err(err) = engine
                    .dispatch_with(&chat, harness_id, retry, Some(message_id), true)
                    .await
                {
                    tracing::error!(chat = %chat, error = %err, "startup-crash retry dispatch failed");
                    // No run is coming: leaving the row Working would spin
                    // the session forever with nothing behind it.
                    engine
                        .inner
                        .set_status(&chat, SessionStatus::Errored, false);
                }
            });
            return;
        }

        if matches!(&event, AgentEvent::BackgroundWake | AgentEvent::GoalWake) {
            inner.publish(&chat_id, &event);
            if let Err(error) = finish_segment(
                doc_ref, writer.take(), &entry_id, &device_id, segment_started,
                &folded, MessageStatus::Complete,
            ) {
                tracing::warn!(chat = %chat_id, %error, "background wake segment finish failed");
            }
            folded.clear();
            dirty = false;
            entry_id = new_id();
            segment_started = now_ms();
            self_continued_turn = !deterministic_turn_end;
            inner.set_status(&chat_id, SessionStatus::Working, true);
            continue;
        }

        // A steer boundary splits the assistant entry exactly where the fold resets.
        if let AgentEvent::Steered {
            next_assistant_message_id,
            ..
        } = &event
        {
            inner.publish(&chat_id, &event);
            // A steer boundary means a real prompt owns the turn again — its
            // Done will come; the short self-continued window stands down.
            self_continued_turn = false;
            if let Err(err) = finish_segment(
                doc_ref,
                writer.take(),
                &entry_id,
                &device_id,
                segment_started,
                &folded,
                MessageStatus::Complete,
            ) {
                tracing::warn!(chat = %chat_id, error = %err, "segment finish failed");
            }
            inner.note_message(&chat_id, &folded_text(&folded));
            folded.clear();
            dirty = false;
            entry_id = next_assistant_message_id.clone()
                .filter(|id| !crate::message_ids::exists(&doc, id))
                .unwrap_or_else(new_id);
            segment_started = now_ms();
            // The elapsed timer is per user message, not per child process: a
            // steer boundary restarts it (matches the parked-resume path and
            // the composer's optimistic overlay, which already reads 0:00).
            inner.set_status(&chat_id, SessionStatus::Working, true);
            // The boundary confirms delivery of the oldest accepted steer —
            // retire its at-least-once ledger entry.
            let confirmed = lock(&inner.runs)
                .get(&chat_id)
                .filter(|h| h.run_id == run_id)
                .and_then(|h| lock(&h.routed_steers).pop_front());
            // Consumed: a steer carrying the fork history delivered it to
            // this runtime's provider session.
            if confirmed.is_some_and(|steer| steer.fork_history)
                && let Some(session) = lock(&inner.harness_sessions)
                    .get(&chat_id)
                    .map(|known| known.session_id.clone())
                    .filter(|id| !id.is_empty())
            {
                let _ = doc.set_fork_history_session(&session);
            }
            continue;
        }

        match &event {
            AgentEvent::SessionStarted {
                session_id, cwd, assistant_message_id, ..
            } => {
                if harness_id == HarnessId::Native && folded.is_empty() && writer.is_none()
                    && !crate::message_ids::exists(&doc, assistant_message_id) {
                    entry_id = assistant_message_id.clone();
                }
                saw_session_started = true;
                // The event's own cwd (where the harness actually created the
                // session) scopes the stored id, not the request's.
                inner.remember_harness_session(&chat_id, session_id, cwd);
                note_fork_history_session(&doc, carried_history, session_id);
            }
            AgentEvent::Done {
                session_id: Some(session_id),
                ..
            } => {
                inner.remember_harness_session(&chat_id, session_id, &run_cwd);
                note_fork_history_session(&doc, carried_history, session_id);
            }
            AgentEvent::InputRequested { .. } => {
                // Known-id guaranteed: the unknown-id twin was dropped above,
                // before the parked gate.
                inner.set_status(&chat_id, SessionStatus::AwaitingInput, false);
            }
            AgentEvent::InputResolved { .. } => {
                inner.set_status(&chat_id, SessionStatus::Working, false);
            }
            _ => {}
        }

        inner.publish(&chat_id, &event);

        // Defensive rule from the original app: a mid-run SessionStarted re-emission (Claude SDK
        // background re-invocations) must not wipe the segment being written.
        let skip_fold = matches!(&event, AgentEvent::SessionStarted { .. }) && !folded.is_empty();
        if !skip_fold {
            fold_event_into_parts(&mut folded, &event);
            // Tool sidecar uploads are disabled: the fold's
            // summary/stats ARE the doc's whole record — no refs stamped, no
            // uploads. Full outputs survive only in the host's local run
            // journal. To reintroduce: `rcode_doc::sidecar_payload(&event)`
            // → `apply_sidecar_refs` → `doc_host.upload_tool_sidecar`, all
            // still in place and tested.
        }

        if let AgentEvent::Done { status, .. } = &event {
            // A question still pending at turn end can never be legitimately
            // answered (its turn is over): drain the resolvers NOW, or a late
            // `respond_input` finds one, emits InputResolved, and un-parks
            // the session into Working with no turn behind it — stranded
            // Working, timer forever, reaper disarmed. Empty answers unblock
            // the harness-side bridge like an interrupt does.
            let pending = lock(&inner.runs)
                .get(&chat_id)
                .filter(|h| h.run_id == run_id)
                .map(|h| h.pending_inputs.clone());
            if let Some(pending) = pending {
                for (_, entry) in lock(&pending).drain() {
                    let _ = entry
                        .resolver
                        .send(rcode_proto::InteractionResponse::Dismissed);
                }
            }
            let message_status = match status {
                DoneStatus::Interrupted => MessageStatus::Aborted,
                DoneStatus::Completed | DoneStatus::Errored => MessageStatus::Complete,
            };
            // No dangling chips: a run that ends for ANY reason (completed,
            // errored, interrupted) terminally resolves its input parts — an
            // unresolved question must not outlive the run that asked it
            // (its resolver died with the run; an answer could never land).
            for part in folded.iter_mut() {
                if let MessagePart::Input { resolved, .. } = part {
                    *resolved = true;
                }
            }
            // A Done landing on a PARKED session with nothing streamed (the
            // idle reaper's or an interrupt's own teardown) has no entry to
            // finalize — writing one would leave an empty aborted stub.
            let nothing_streamed = writer.is_none() && folded.is_empty();
            if !nothing_streamed {
                if let Err(err) = finish_segment(
                    doc_ref,
                    writer.take(),
                    &entry_id,
                    &device_id,
                    segment_started,
                    &folded,
                    message_status,
                ) {
                    tracing::warn!(chat = %chat_id, error = %err, "final segment finish failed");
                }
                inner.note_message(&chat_id, &folded_text(&folded));
            }
            if *status == DoneStatus::Completed {
                // A cleanly completed turn resets the auto-resume revival
                // budget: only consecutive crash-revive-crash cycles spend it.
                inner.journal.clear_resume_attempts(&chat_id);
            }
            // A turn stopped in place: the runtime lives on, parked like a
            // completed turn. Steers it had accepted but not yet delivered
            // were cancelled with the turn (never delivered, never
            // confirmed) — drop them, or they would hold the parked session
            // Working forever.
            let stopped_in_place =
                *status == DoneStatus::Interrupted && stops_turn_in_place && !interrupted;
            if stopped_in_place
                && let Some(ledger) = lock(&inner.runs)
                    .get(&chat_id)
                    .filter(|h| h.run_id == run_id)
                    .map(|h| h.routed_steers.clone())
            {
                lock(&ledger).clear();
            }
            // An accepted steer awaiting its boundary owns the continuation:
            // the previous Done is an internal handoff, not a completion ping.
            // So does a message the user already sent that is still on its
            // way (see `continues_with_sent_message`); rows the user queued
            // themselves still notify.
            let pending_steer = inner.has_pending_steers(&chat_id, &run_id);
            let completed_turn = (*status == DoneStatus::Completed
                && !interrupted
                && turn_was_active
                && !pending_steer
                && !inner.continues_with_sent_message(&chat_id))
            .then(|| entry_id.clone());
            // Auto-title on the first completed turn of an unnamed chat —
            // the trigger the external-agent design ran off the first prompt,
            // now driven from the turn's own completion. `harness_id` and
            // `user_prompt` were captured at the top of this function for it.
            if completed_turn.is_some()
                && inner.should_auto_title(&chat_id)
                && rcode_harness::supports_titles(harness_id)
            {
                inner.spawn_auto_title(chat_id.clone(), harness_id, title_request.clone());
            }
            // PERSISTENT SESSION: a cleanly completed turn on a steerable
            // harness PARKS instead of ending — child + mailbox stay warm for
            // the next routed dispatch; per-turn state resets for it.
            if (*status == DoneStatus::Completed || stopped_in_place)
                && steerable
                && !interrupted
            {
                folded.clear();
                dirty = false;
                entry_id = new_id();
                segment_started = now_ms();
                // Resume-retry is strictly a first-turn concern.
                saw_session_started = true;
                idle_since = Some(tokio::time::Instant::now());
                reap_deferred_until = None;
                self_continued_turn = false;
                // Accepted steers still own this runtime. Publishing Idle
                // here lets the ordinary queue overtake that continuation.
                inner.set_status_with_completion(
                    &chat_id,
                    if pending_steer {
                        SessionStatus::Working
                    } else {
                        SessionStatus::Idle
                    },
                    false,
                    completed_turn,
                );
                continue;
            }
            final_completed_turn = completed_turn;
            break match status {
                DoneStatus::Errored => SessionStatus::Errored,
                _ => SessionStatus::Idle,
            };
        }

        if !folded.is_empty() && !dirty {
            dirty = true;
            flush_at =
                tokio::time::Instant::now() + std::time::Duration::from_millis(STREAM_COMMIT_MS);
        }
    };

    // Any subagent still streaming when the run ends freezes as-is: the
    // parent process is gone, so nothing more can arrive on this stream.
    for (parent_id, sink) in subagents.drain() {
        let doc_id = sink.doc_id.clone();
        let _ = doc_ref.update_subagent_chip(&parent_id, None, Some("failed"), None);
        if let Some(json) = sink.finish(&device_id, MessageStatus::Aborted)
            && let Some(host) = inner.doc_host()
        {
            host.upload_tool_sidecar(
                &chat_id,
                rcode_doc::SidecarPayload {
                    part_id: doc_id,
                    output: Some(json),
                    diff: None,
                },
            );
        }
    }
    inner.set_running_subagents(&chat_id, 0);

    // Claim any accepted-but-unconfirmed steers BEFORE the handle goes away:
    // a routed send that raced this exit either finds its entry gone (we own
    // it — re-dispatched below) or reclaims it and starts a fresh run itself.
    let orphans: Vec<RoutedSteer> = lock(&inner.runs)
        .get(&chat_id)
        .filter(|h| h.run_id == run_id)
        .map(|h| std::mem::take(&mut *lock(&h.routed_steers)).into())
        .unwrap_or_default();
    inner.remove_run(&chat_id, &run_id);
    inner.set_status_with_completion(&chat_id, final_status, false, final_completed_turn);
    // A Stop cancels the messages it found waiting; one accepted after it
    // began is still the user's next message and must run.
    let orphans: Vec<RoutedSteer> = if inner
        .shutting_down
        .load(std::sync::atomic::Ordering::Acquire)
        || lock(&inner.deleting_chats).contains(&chat_id)
    {
        Vec::new()
    } else if interrupted {
        orphans.into_iter().filter(|s| s.after_stop).collect()
    } else {
        orphans
    };
    if !orphans.is_empty() {
        // The dying run accepted these into its mailbox but never confirmed a
        // Steered boundary (idle-reaper race, a mid-turn error discarding
        // queued boundary steers, a parked child death). Their user entries
        // are already in the transcript — a message that shows as sent must
        // never silently not run. Re-dispatch each as a fresh turn
        // (write_user_message dedupes by id; resume is engine-injected).
        let engine = SessionsEngine {
            inner: inner.clone(),
        };
        let chat = chat_id.clone();
        tokio::spawn(async move {
            for steer in orphans {
                let Some(mut request) = steer
                    .request
                    .map(|request| *request)
                    .or_else(|| engine.last_request(&chat))
                else {
                    tracing::warn!(chat = %chat, "orphaned steer lost: no run config to re-dispatch");
                    break;
                };
                request.prompt = steer.prompt.clone();
                request.resume = None;
                tracing::info!(chat = %chat, "re-dispatching steer orphaned by a dying run");
                let Some(host) = engine.inner.doc_host() else {
                    tracing::warn!(chat = %chat, "orphaned steer lost: doc host unavailable");
                    break;
                };
                if let Err(err) = host
                    .dispatch_with_source_context(
                        &engine,
                        &chat,
                        harness_id,
                        request,
                        Some(steer.message_id.clone()),
                    )
                    .await
                {
                    tracing::warn!(chat = %chat, error = %err, "orphaned steer re-dispatch failed");
                    engine
                        .inner
                        .set_status(&chat, SessionStatus::Errored, false);
                    break;
                }
            }
        });
    }
}

#[cfg(test)]
#[path = "sessions_title_tests.rs"]
mod title_tests;

#[cfg(test)]
#[path = "sessions_history_tests.rs"]
mod history_tests;

#[cfg(test)]
#[path = "sessions_deletion_tests.rs"]
mod deletion_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_recovery_converts_rich_messages_before_json_encoding() {
        let doc = rcode_doc::SessionDoc::init("cursor-rich-recovery").unwrap();
        let skill = rcode_proto::invocation::Invocation::Skill {
            name: "review \"quoted\"".into(),
            path: "/repo/quoted \"path\"/SKILL.md".into(),
            command: None,
        }
        .link();
        let previous = format!("Previous {skill}\nSecond line with \\ and \"quotes\"");
        doc.push_message(&rcode_doc::SessionMessageEntry {
            id: "u1".into(),
            role: rcode_doc::MessageRole::User,
            parts: vec![rcode_doc::MessagePart::Text {
                id: "u1-text".into(),
                text: previous.clone(),
            }],
            created_at: 0,
            device_id: "test".into(),
            status: None,
            continuation_of: None,
            duration_ms: None,
            excluded: false,
        })
        .unwrap();
        let current = format!("Current {skill}\nKeep **Markdown**");
        let delivered = super::cursor_unstarted_history(&doc, "u2", &current, false).unwrap();
        let (_, json) = delivered.split_once('\n').unwrap();
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed["currentUserMessage"],
            rcode_proto::invocation::harness_prompt(&current, rcode_proto::HarnessId::Cursor)
        );
        assert_eq!(
            parsed["previousUserMessages"][0],
            rcode_proto::invocation::harness_prompt(&previous, rcode_proto::HarnessId::Cursor)
        );
    }

    #[test]
    fn cursor_without_a_session_id_retains_only_preceding_user_messages() {
        let doc = rcode_doc::SessionDoc::init("cursor-unstarted").unwrap();
        for (id, role, text) in [
            (
                "u1",
                rcode_doc::MessageRole::User,
                "first interrupted request",
            ),
            ("a1", rcode_doc::MessageRole::Assistant, "partial output"),
            ("u2", rcode_doc::MessageRole::User, "current request"),
            ("u3", rcode_doc::MessageRole::User, "future pending request"),
        ] {
            doc.push_message(&rcode_doc::SessionMessageEntry {
                id: id.into(),
                role,
                parts: vec![rcode_doc::MessagePart::Text {
                    id: format!("{id}-text"),
                    text: text.into(),
                }],
                created_at: 0,
                device_id: "test".into(),
                status: None,
                continuation_of: None,
                duration_ms: None,
                excluded: false,
            })
            .unwrap();
        }
        assert_eq!(
            super::cursor_unstarted_history(&doc, "u1", "first interrupted request", false)
                .unwrap(),
            "first interrupted request"
        );
        let prompt = super::cursor_unstarted_history(&doc, "u2", "current request", false).unwrap();
        assert!(prompt.contains("first interrupted request"));
        assert!(prompt.contains("current request"));
        assert!(!prompt.contains("partial output"));
        assert!(!prompt.contains("future pending request"));
        assert!(prompt.contains("do not rerun prior tools or side effects"));
        assert_eq!(
            super::cursor_unstarted_history(&doc, "u2", "current request", true).unwrap(),
            "current request",
            "content from the preceding turn is already in the native session"
        );
        assert!(
            super::cursor_unstarted_history(&doc, "u3", "future pending request", true)
                .unwrap()
                .contains("current request"),
            "an unacknowledged prompt after an older checkpoint must survive"
        );
        assert_eq!(doc.read_entries().unwrap().len(), 4);
    }

    use rcode_proto::{HarnessId, RunRequest};

    #[tokio::test]
    async fn generated_image_failure_is_sanitized_even_inside_subagents() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let sessions = SessionsEngine::new(
            "host".into(),
            Arc::new(RunJournal::open(dir.path().join("journals")).unwrap()),
            Arc::new(HarnessRegistry::new()),
        );
        let source = "/outside/private/source.png";
        let event = AgentEvent::Subagent {
            parent_tool_use_id: "child".into(),
            event: Box::new(AgentEvent::GeneratedImage {
                id: "i:image".into(),
                path: source.into(),
                name: "secret".into(),
                mime_type: "bad".into(),
            }),
        };
        let events = sessions
            .inner
            .prepare_generated_image("chat", event, &mut std::collections::HashSet::new())
            .await;
        assert_eq!(events.len(), 2);
        let mut parts = vec![];
        fold_event_into_parts(
            &mut parts,
            &AgentEvent::ToolCall {
                id: "i".into(),
                call: rcode_proto::ToolCall::Unknown {
                    name: "Generate image".into(),
                    input: None,
                },
            },
        );
        for event in &events {
            sessions.inner.publish("chat", event);
            let AgentEvent::Subagent {
                parent_tool_use_id,
                event,
            } = event
            else {
                panic!("lost owner")
            };
            assert_eq!(parent_tool_use_id, "child");
            fold_event_into_parts(&mut parts, event);
        }
        assert!(matches!(
            parts[0],
            MessagePart::Tool {
                is_error: true,
                resolved: true,
                ..
            }
        ));
        assert!(matches!(parts[1], MessagePart::Error { .. }));
        let journal =
            serde_json::to_string(&sessions.inner.journal.replay("chat", 0).unwrap()).unwrap();
        assert!(!journal.contains(source));
        assert!(!journal.contains("secret"));
    }

    /// The backwards journal scan must answer exactly what the old full forward
    /// replay did: newest non-empty id, cwd from the governing `SessionStarted`.
    #[tokio::test]
    async fn journal_harness_session_matches_forward_scan() {
        fn started(id: &str, cwd: &str) -> AgentEvent {
            AgentEvent::SessionStarted {
                harness: rcode_proto::HarnessId::ClaudeCode,
                model: "m".into(),
                tools: Vec::new(),
                cwd: cwd.into(),
                session_id: id.into(),
                assistant_message_id: "a".into(),
            }
        }
        fn done(id: Option<&str>) -> AgentEvent {
            AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: id.map(Into::into),
            }
        }
        fn text() -> AgentEvent {
            AgentEvent::TextDelta { text: "t".into() }
        }
        /// The pre-change implementation, verbatim, over a forward replay.
        fn forward(events: Vec<(u64, AgentEvent)>) -> Option<(String, String)> {
            let mut current_cwd = String::new();
            let mut found: Option<(String, String)> = None;
            for (_, event) in events {
                match event {
                    AgentEvent::SessionStarted {
                        session_id, cwd, ..
                    } => {
                        current_cwd = cwd;
                        if !session_id.is_empty() {
                            found = Some((session_id, current_cwd.clone()));
                        }
                    }
                    AgentEvent::Done {
                        session_id: Some(session_id),
                        ..
                    } if !session_id.is_empty() => {
                        found = Some((session_id, current_cwd.clone()));
                    }
                    _ => {}
                }
            }
            found
        }

        /// (chat id, journaled events, expected `(session id, cwd)`).
        type Case = (
            &'static str,
            Vec<AgentEvent>,
            Option<(&'static str, &'static str)>,
        );
        let cases: Vec<Case> = vec![
            ("empty", vec![], None),
            ("no-ids", vec![text(), done(None), done(Some(""))], None),
            (
                "started-only",
                vec![started("s1", "/a"), text()],
                Some(("s1", "/a")),
            ),
            (
                "done-inherits-run-cwd",
                vec![started("s1", "/a"), text(), done(Some("s2"))],
                Some(("s2", "/a")),
            ),
            (
                "done-before-any-started",
                vec![text(), done(Some("s1"))],
                Some(("s1", "")),
            ),
            (
                "empty-started-sets-cwd-for-later-done",
                vec![started("s1", "/a"), started("", "/b"), done(Some("s2"))],
                Some(("s2", "/b")),
            ),
            (
                "later-empty-started-does-not-override",
                vec![
                    started("s1", "/a"),
                    done(Some("s2")),
                    started("", "/b"),
                    text(),
                ],
                Some(("s2", "/a")),
            ),
            (
                "newest-run-wins",
                vec![
                    started("s1", "/a"),
                    done(Some("s1")),
                    started("s3", "/c"),
                    text(),
                ],
                Some(("s3", "/c")),
            ),
            (
                "empty-done-ids-ignored",
                vec![started("s1", "/a"), done(Some("")), done(None)],
                Some(("s1", "/a")),
            ),
            (
                "only-empty-started-then-done-from-older-run",
                vec![
                    started("s1", "/a"),
                    done(Some("s9")),
                    started("", "/b"),
                    done(None),
                ],
                Some(("s9", "/a")),
            ),
        ];

        let dir = tempfile::tempdir().unwrap();
        let sessions = SessionsEngine::new(
            "host".into(),
            Arc::new(RunJournal::open(dir.path().join("journals")).unwrap()),
            Arc::new(HarnessRegistry::new()),
        );
        for (chat, events, want) in cases {
            for event in &events {
                sessions.inner.journal.append(chat, event).unwrap();
            }
            let want = want.map(|(id, cwd)| (id.to_string(), cwd.to_string()));
            let replayed = sessions.inner.journal.replay(chat, 0).unwrap();
            assert_eq!(forward(replayed), want, "{chat}: reference");
            assert_eq!(sessions.inner.journal_harness_session(chat), want, "{chat}");
        }
        // Torn tail: the last complete SessionStarted still governs.
        let path = dir.path().join("journals").join("started-only.jsonl");
        let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        std::io::Write::write_all(&mut f, b"{\"seq\":3,\"event\":{\"type\":\"done\"").unwrap();
        assert_eq!(
            sessions.inner.journal_harness_session("started-only"),
            Some(("s1".into(), "/a".into()))
        );
    }

    fn request() -> RunRequest {
        RunRequest {
            prompt: "first".into(),
            harness: None,
            model: Some("grok-4.6".into()),
            reasoning: Some(rcode_proto::ReasoningLevel::High),
            model_options: serde_json::Map::new(),
            cwd: "/tmp".into(),
            resume: None,
            attachments: Vec::new(),
            worktree: None,
        }
    }

    struct IdleStartupFixture {
        idle_calls: Arc<std::sync::atomic::AtomicUsize>,
        inference_calls: Arc<std::sync::atomic::AtomicUsize>,
        fail: bool,
    }
    #[async_trait::async_trait]
    impl rcode_harness::Harness for IdleStartupFixture {
        fn id(&self) -> HarnessId {
            HarnessId::Codex
        }
        fn display_name(&self) -> &str {
            "Offline idle fixture"
        }
        fn supports_steering(&self) -> bool {
            false
        }
        fn steering_mode(&self) -> rcode_proto::SteeringMode {
            rcode_proto::SteeringMode::TurnBoundary
        }
        fn reasoning_levels(&self) -> &[rcode_proto::ReasoningLevel] {
            &[]
        }
        async fn models(&self) -> Result<Vec<rcode_proto::Model>, rcode_harness::HarnessError> {
            Ok(vec![])
        }
        async fn start_idle(
            &self,
            request: RunRequest,
            _controls: rcode_harness::RunControls,
        ) -> Result<
            futures::stream::BoxStream<'static, Result<AgentEvent, rcode_harness::HarnessError>>,
            rcode_harness::HarnessError,
        > {
            use futures::StreamExt;
            self.idle_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert!(request.prompt.is_empty());
            assert_eq!(request.resume.as_deref(), Some("old-native-thread"));
            let event = if self.fail {
                AgentEvent::Done {
                    status: rcode_proto::DoneStatus::Errored,
                    result: None,
                    error: Some("offline startup failed".into()),
                    session_id: None,
                }
            } else {
                AgentEvent::SessionStarted {
                    harness: HarnessId::Codex,
                    model: "fixture".into(),
                    tools: vec![],
                    cwd: request.cwd,
                    session_id: "new-native-thread".into(),
                    assistant_message_id: "fixture-empty".into(),
                }
            };
            Ok(futures::stream::iter([Ok(event)])
                .chain(futures::stream::pending())
                .boxed())
        }
        async fn run(
            &self,
            _request: RunRequest,
            _controls: rcode_harness::RunControls,
        ) -> Result<
            futures::stream::BoxStream<'static, Result<AgentEvent, rcode_harness::HarnessError>>,
            rcode_harness::HarnessError,
        > {
            self.inference_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(rcode_harness::HarnessError::Protocol(
                "fixture forbids inference".into(),
            ))
        }
    }

    async fn idle_startup_case(fail: bool) {
        use super::*;
        let temp = tempfile::tempdir().unwrap();
        let idle_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let inference_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let registry = crate::HarnessRegistry::new();
        registry.register(Arc::new(IdleStartupFixture {
            idle_calls: idle_calls.clone(),
            inference_calls: inference_calls.clone(),
            fail,
        }));
        let core = crate::EngineCore::assemble_with_profile(
            crate::EngineProfile::new(
                &crate::DataRoots::same(temp.path()),
                "test-org",
                "test-user",
            ),
            Arc::new(registry),
            HarnessId::Codex,
            None,
        )
        .unwrap();
        core.workspace
            .create_chat("idle", None, Some(&core.device_id), None, None)
            .unwrap();
        let mut request = request();
        request.prompt.clear();
        core.sessions
            .inner
            .remember_harness_session("idle", "old-native-thread", &request.cwd);
        core.sessions.start_idle("idle", request).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let completed = if fail {
                    core.sessions
                        .session_status("idle")
                        .is_some_and(|s| s.status == SessionStatus::Errored)
                } else {
                    core.sessions.inner.resume_for("idle", "/tmp").as_deref()
                        == Some("new-native-thread")
                };
                if completed {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(idle_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(inference_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        let entries = core
            .doc_host
            .open("idle")
            .unwrap()
            .doc()
            .read_entries()
            .unwrap();
        assert!(
            !entries
                .iter()
                .any(|entry| entry.role == rcode_doc::MessageRole::User)
        );
        if !fail {
            assert!(entries.is_empty());
            assert_eq!(
                core.sessions.session_status("idle").unwrap().status,
                SessionStatus::Idle
            );
            assert_eq!(
                core.workspace.chat_harness_session("idle").unwrap().0,
                "new-native-thread"
            );
            let events = core.sessions.inner.journal.replay("idle", 0).unwrap();
            assert!(events.iter().any(|(_, event)| matches!(event, AgentEvent::SessionStarted { session_id, .. } if session_id == "new-native-thread")));
        }
        core.sessions.shutdown().await;
    }
    #[tokio::test]
    async fn idle_startup_failure_never_retries_as_inference() {
        idle_startup_case(true).await;
    }
    #[tokio::test]
    async fn idle_startup_remembers_thread_without_creating_messages() {
        idle_startup_case(false).await;
    }

    #[test]
    fn clean_tool_ids_ride_verbatim_and_fit_id_re() {
        let id = subagent_doc_id("chat-abc123", "toolu_01EjFLnNhCiMR2PBNKcVvkT");
        assert_eq!(id, "chat-abc123--sub--toolu_01EjFLnNhCiMR2PBNKcVvkT");
        assert!(id.len() <= 128);
        assert!(
            id.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        );
    }

    #[test]
    fn unclean_or_oversized_ids_hash_deterministically() {
        let dirty = subagent_doc_id("chat", "call/with:odd chars");
        assert!(
            dirty
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "{dirty}"
        );
        assert_eq!(dirty, subagent_doc_id("chat", "call/with:odd chars"));
        let long_chat = "c".repeat(100);
        let long = subagent_doc_id(&long_chat, &"t".repeat(64));
        assert!(long.len() <= 128, "{}", long.len());
        // Distinct inputs stay distinct.
        assert_ne!(
            subagent_doc_id("chat", "a:b"),
            subagent_doc_id("chat", "a:c")
        );
    }

    #[tokio::test]
    async fn subagent_sink_flush_clears_dirty_when_nothing_folded() {
        use super::*;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(rcode_sync::DocsStore::open(dir.path()).unwrap());
        let host = DocHost::new(
            store,
            crate::doc_host::DocHostConfig {
                device_id: "dev-test".into(),
                default_harness: HarnessId::Mock,
                edge: None,
            },
        );
        let handle = host.open("chat-sub-flush--sub--t1").unwrap();
        let mut sink = SubagentSink {
            doc_id: "chat-sub-flush--sub--t1".into(),
            doc: handle.writer().unwrap(),
            entry_id: "entry-1".into(),
            started_at: 0,
            entry_index: None,
            written: Vec::new(),
            folded: Vec::new(),
            dirty: true,
        };
        // Nothing to write is still a completed flush — left set, the flag
        // holds the run loop's commit guard open on a past deadline.
        sink.flush("dev-test");
        assert!(!sink.dirty);
    }

    /// Feed-by-hand harness: the test pushes wire events through a channel
    /// and decides when the stream ends.
    struct FeedHarness {
        feed: Mutex<Option<mpsc::UnboundedReceiver<AgentEvent>>>,
    }

    #[async_trait::async_trait]
    impl Harness for FeedHarness {
        fn id(&self) -> HarnessId {
            HarnessId::Mock
        }
        fn display_name(&self) -> &str {
            "Feed"
        }
        fn supports_steering(&self) -> bool {
            true
        }
        fn steering_mode(&self) -> rcode_proto::SteeringMode {
            rcode_proto::SteeringMode::StepBoundary
        }
        fn reasoning_levels(&self) -> &[rcode_proto::ReasoningLevel] {
            &[rcode_proto::ReasoningLevel::Medium]
        }
        async fn models(&self) -> Result<Vec<rcode_proto::Model>, rcode_harness::HarnessError> {
            Ok(vec![])
        }
        async fn run(
            &self,
            _request: RunRequest,
            _controls: RunControls,
        ) -> Result<
            futures::stream::BoxStream<'static, Result<AgentEvent, rcode_harness::HarnessError>>,
            rcode_harness::HarnessError,
        > {
            let mut feed = self
                .feed
                .lock()
                .unwrap()
                .take()
                .expect("FeedHarness serves one run per test");
            Ok(futures::stream::poll_fn(move |cx| feed.poll_recv(cx).map(|e| e.map(Ok))).boxed())
        }
    }

    // A tagged subagent event that folds to NO parts used to leave its sink
    // dirty forever: the commit branch's guard stayed true on a deadline in
    // the past and the run loop burned a core until real content arrived.
    #[tokio::test]
    async fn empty_subagent_events_commit_once_per_window_not_per_tick() {
        use super::*;
        FLUSH_TICKS.with(|t| t.set(0));
        let (feed, rx) = mpsc::unbounded_channel();
        let registry = HarnessRegistry::new();
        registry.register(Arc::new(FeedHarness {
            feed: Mutex::new(Some(rx)),
        }));
        let dir = tempfile::tempdir().unwrap();
        let core =
            crate::EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
                .unwrap();
        let chat = "chat-sink-spin";
        core.sessions
            .dispatch(chat, HarnessId::Mock, request(), None)
            .await
            .unwrap();
        feed.send(AgentEvent::SessionStarted {
            harness: HarnessId::Mock,
            model: "mock-1".into(),
            tools: vec![],
            cwd: "/tmp".into(),
            session_id: "hs-1".into(),
            assistant_message_id: "a1".into(),
        })
        .unwrap();
        feed.send(AgentEvent::Subagent {
            parent_tool_use_id: "tool-1".into(),
            event: Box::new(AgentEvent::ReasoningDelta {
                text: String::new(),
            }),
        })
        .unwrap();
        // Well past several commit windows with the stream still open: the
        // loop must idle between commits, not spin on a past deadline.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let ticks = FLUSH_TICKS.with(|t| t.get());
        assert!(
            (1..=8).contains(&ticks),
            "commit branch fired {ticks} times in 1s for one empty subagent event"
        );
        assert_eq!(
            core.sessions.session_status(chat).map(|s| s.status),
            Some(SessionStatus::Working),
            "stream still open, turn still running"
        );
        feed.send(AgentEvent::Done {
            status: DoneStatus::Completed,
            result: None,
            error: None,
            session_id: None,
        })
        .unwrap();
        core.sessions.shutdown().await;
    }
}

#[cfg(test)]
#[path = "sessions_todo_tests.rs"]
mod todo_label_recovery_tests;
