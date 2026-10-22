//! Persistent native session: parent turns and detached work share one event stream.
use crate::native::NativeHarness;
use crate::{HarnessError, RunControls, SteerMessage, TurnControl};
use futures::{
    StreamExt,
    stream::{BoxStream, SelectAll},
};
use rcode_proto::{AgentEvent, RunRequest};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub(super) struct SessionState {
    pub cancel: CancellationToken,
    pub turn: TurnControl,
    pub history: Arc<Mutex<Vec<rcode_agent::Message>>>,
    pub expected_goal: Mutex<Option<String>>,
    pub goal_continuation: Arc<Mutex<Option<rcode_agent::goal::GoalContinuation>>>,
    pub tasks: Mutex<Option<Arc<rcode_agent::agent_tasks::AgentTasks>>>,
}
impl SessionState {
    fn manager(&self) -> Option<Arc<rcode_agent::agent_tasks::AgentTasks>> {
        self.tasks.lock().unwrap().clone()
    }
}

pub(super) async fn start(
    harness: NativeHarness,
    mut request: RunRequest,
    controls: RunControls,
) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
    let RunControls {
        conversation_id,
        goal,
        request_input,
        mut steering,
        interrupt,
        turn,
        history,
    } = controls;
    let state = Arc::new(SessionState {
        cancel: interrupt.child_token(),
        turn,
        history: Arc::default(),
        goal_continuation: Arc::default(),
        expected_goal: Mutex::default(),
        tasks: Mutex::default(),
    });
    let input: Arc<
        dyn Fn(
                rcode_proto::InteractionRequest,
            ) -> tokio::sync::oneshot::Receiver<rcode_proto::InteractionResponse>
            + Send
            + Sync,
    > = Arc::from(request_input);
    let session_goal = goal.clone();
    let make_controls = {
        let input = input.clone();
        let state = state.clone();
        move |history, cancel| {
            let input = input.clone();
            RunControls {
                conversation_id: conversation_id.clone(),
                goal: goal.clone(),
                request_input: Box::new(move |request| input(request)),
                steering: mpsc::channel(1).1,
                interrupt: cancel,
                turn: state.turn.clone(),
                history,
            }
        }
    };
    let mut current_cancel = state.cancel.child_token();
    let first = harness
        .run_once(
            request.clone(),
            make_controls(history, current_cancel.clone()),
            Some(state.clone()),
        )
        .await?;
    let (tx, rx) = mpsc::channel(128);
    let state_worker = state.clone();
    tokio::spawn(async move {
        let state = state_worker;
        let mut streams = SelectAll::new();
        streams.push(first);
        let mut active = true;
        let mut suppressed = false;
        let mut steering_open = true;
        let mut pending_prompt = None;
        let mut session_id = request.resume.clone();
        loop {
            // User assignments have priority over completion wakes at every turn boundary.
            let mut next: Option<SteerMessage> = pending_prompt.take();
            let user_waiting = session_goal.as_ref().is_some_and(|store| store.should_yield().unwrap_or(true));
            let mut wake = false;
            let mut goal_prompt = None;
            if !active {
                if next.is_none() {
                    match steering.try_recv() {
                        Ok(prompt) => next = Some(prompt),
                        Err(mpsc::error::TryRecvError::Disconnected) => steering_open = false,
                        Err(_) => {}
                    }
                }
                if next.is_none()
                    && !suppressed
                    && !user_waiting
                    && state
                        .manager()
                        .is_some_and(|tasks| tasks.has_notifications())
                {
                    wake = true;
                }
            }
            if !active
                && next.is_none()
                && !suppressed
                && !user_waiting
                && !state.manager().is_some_and(|tasks| tasks.has_running())
            {
                if let Some(pending) = state.goal_continuation.lock().unwrap().take() {
                    if let Some(goal) = session_goal
                        .as_ref()
                        .and_then(|store| store.read().ok().flatten())
                        .filter(|goal| {
                            goal.id == pending.goal_id
                                && goal.status == rcode_proto::goal::GoalStatus::Active
                        })
                    {
                        goal_prompt = Some((
                            goal.id.clone(),
                            if goal.revision == pending.revision {
                                pending.prompt
                            } else {
                                rcode_agent::goal::reminder(&goal)
                            },
                        ));
                    }
                }
            }
            if next.is_some() || wake || goal_prompt.is_some() {
                current_cancel = state.cancel.child_token();
                *state.expected_goal.lock().unwrap() = None;
                if let Some(prompt) = next {
                    if let Some(config) = prompt.config {
                        request = *config;
                    }
                    request.prompt = prompt.prompt;
                    request.attachments = prompt.attachments;
                    let _ = tx
                        .send(Ok(AgentEvent::Steered {
                            assistant_message_id: None,
                            // The mailbox id belongs to the user, not the new assistant turn.
                            next_assistant_message_id: None,
                        }))
                        .await;
                    suppressed = false;
                } else if let Some((id, prompt)) = goal_prompt {
                    *state.expected_goal.lock().unwrap() = Some(id);
                    request.prompt = prompt;
                    request.attachments.clear();
                    let _ = tx.send(Ok(AgentEvent::GoalWake)).await;
                } else {
                    request.prompt = "Process the completed background task notifications. Report relevant results and continue only work authorized by the user. Do not resume a paused or cleared goal.".into();
                    request.attachments.clear();
                    let _ = tx.send(Ok(AgentEvent::BackgroundWake)).await;
                }
                request.resume = session_id.clone();
                let prepare = harness.run_once(
                    request.clone(),
                    make_controls(None, current_cancel.clone()),
                    Some(state.clone()),
                );
                let prepared = tokio::select! { biased; _ = state.cancel.cancelled() => break, result = prepare => result };
                match prepared {
                    Ok(stream) => {
                        streams.push(stream);
                        active = true;
                    }
                    Err(error) => {
                        suppressed = true;
                        let _ = tx
                            .send(Ok(AgentEvent::Done {
                                status: rcode_proto::DoneStatus::Errored,
                                result: None,
                                error: Some(error.to_string()),
                                session_id: None,
                            }))
                            .await;
                    }
                }
                continue;
            }
            let manager = state.manager();
            if !active
                && !steering_open
                && manager.as_ref().is_none_or(|tasks| {
                    !tasks.has_running() && (suppressed || !tasks.has_notifications())
                })
            {
                break;
            }
            let changed = async {
                if let Some(tasks) = &manager {
                    let _ = tasks.wait_for_completion(&state.cancel).await;
                } else {
                    futures::future::pending::<()>().await;
                }
            };
            let activity = session_goal.as_ref().and_then(|store| store.activity());
            let activity_changed = async {
                if let Some(activity) = &activity { activity.notified().await; }
                else { futures::future::pending::<()>().await; }
            };
            tokio::select! { biased;
                _ = state.cancel.cancelled() => break,
                _ = tx.closed() => { state.cancel.cancel(); break; }
                _ = state.turn.stop_requested() => {
                    suppressed = true;
                    current_cancel.cancel();
                    if let Some(tasks) = &manager { tasks.cancel_foreground(); }
                }
                prompt = steering.recv(), if !active && steering_open => {
                    match prompt {
                        Some(prompt) => pending_prompt = Some(prompt),
                        None => steering_open = false,
                    }
                }
                event = streams.next(), if !streams.is_empty() => {
                    if let Some(event) = event {
                        if let Ok(AgentEvent::SessionStarted {session_id: id,..}) = &event { session_id = Some(id.clone()); }
                        if let Ok(AgentEvent::Done { status, .. }) = &event { active = false; if *status != rcode_proto::DoneStatus::Completed { suppressed = true; } }
                        if tx.send(event).await.is_err() { state.cancel.cancel(); break; }
                    }
                }
                _ = activity_changed, if !active => {},
                _ = changed, if !active && !suppressed && !user_waiting && manager.as_ref().is_some_and(|tasks| tasks.has_running()) => {}
            }
            if !active
                && !steering_open
                && !state
                    .manager()
                    .is_some_and(|tasks| tasks.has_running() || tasks.has_notifications())
            {
                break;
            }
        }
        state.cancel.cancel();
        if let Some(tasks) = state.manager() {
            tasks.cancel_running();
            // Drain nested terminal events while jobs reap, avoiding channel backpressure.
            while tasks.has_running() {
                tokio::select! {
                    _ = streams.next(), if !streams.is_empty() => {},
                    _ = tasks.wait_for_idle() => { tokio::task::yield_now().await; }
                }
            }
        }
        state.turn.set_background(0);
    });
    Ok(Box::pin(super::native::DropShutdown {
        inner: futures::stream::unfold(rx, |mut rx| async {
            rx.recv().await.map(|event| (event, rx))
        })
        .boxed(),
        cancel: state.cancel.clone(),
    }))
}
