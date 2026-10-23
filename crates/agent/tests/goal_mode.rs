use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};
use rcode_agent::goal::{GoalChange, GoalStore};
use rcode_agent::{
    AgentConfig, AgentError, AgentEvent, AgentHost, AgentRunner, CallUsage, DoneReason,
    InputAnswer, InputRequest, Message, ReasoningChunk, Tool, ToolRegistry,
};
use rcode_proto::goal::{GoalStatus, SessionGoal};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio_util::sync::CancellationToken;

struct Store(Mutex<Option<SessionGoal>>);
impl Store {
    fn active() -> Arc<Self> {
        Arc::new(Self(Mutex::new(Some(
            SessionGoal::new(
                "goal-1".into(),
                "Implement and validate the feature".into(),
                0,
            )
            .unwrap(),
        ))))
    }
}
impl GoalStore for Store {
    fn read(&self) -> Result<Option<SessionGoal>, String> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn change_if_current(
        &self,
        id: &str,
        revision: Option<u64>,
        change: GoalChange,
    ) -> Result<Option<SessionGoal>, String> {
        let mut state = self.0.lock().unwrap();
        let Some(goal) = state.as_mut().filter(|goal| {
            goal.id == id && revision.is_none_or(|revision| revision == goal.revision)
        }) else {
            return Ok(None);
        };
        match change {
            GoalChange::Usage { tokens, .. } => goal.tokens_used += tokens,
            GoalChange::Advance { reason } if goal.status == GoalStatus::Active => {
                goal.iteration += 1;
                goal.reason = Some(reason);
            }
            GoalChange::Complete { reason } if goal.status == GoalStatus::Active => {
                goal.status = GoalStatus::Complete;
                goal.reason = Some(reason);
            }
            GoalChange::Pause { reason } if goal.status == GoalStatus::Active => {
                goal.status = GoalStatus::Paused;
                goal.reason = Some(reason);
            }
            _ => {}
        }
        Ok(Some(goal.clone()))
    }
}

struct Host {
    replies: Mutex<VecDeque<Message>>,
    verifications: Mutex<VecDeque<String>>,
    calls: AtomicUsize,
    checks: AtomicUsize,
    before_verify: Option<Box<dyn Fn() + Send + Sync>>,
    before_reason: Option<Box<dyn Fn(&[Message]) + Send + Sync>>,
}
fn response(message: Message) -> BoxStream<'static, Result<ReasoningChunk, AgentError>> {
    Box::pin(futures::stream::iter([Ok(ReasoningChunk::Done {
        message,
        usage: Some(CallUsage {
            input_tokens: 100,
            output_tokens: 5,
            cache_read_tokens: Some(80),
            cache_write_tokens: None,
        }),
    })]))
}
#[async_trait]
impl AgentHost for Host {
    async fn reason(
        &self,
        messages: &[Message],
        _: &[Arc<dyn Tool>],
    ) -> Result<BoxStream<'static, Result<ReasoningChunk, AgentError>>, AgentError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(callback) = &self.before_reason {
            callback(messages);
        }
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| AgentError::Model("unexpected extra goal iteration".into()))?;
        Ok(response(reply))
    }
    async fn verify_goal(
        &self,
        messages: &[Message],
    ) -> Result<BoxStream<'static, Result<ReasoningChunk, AgentError>>, AgentError> {
        let evidence: serde_json::Value = serde_json::from_str(&messages[1].content.text_content()).unwrap();
        assert!(evidence.get("currentTodos").is_some(), "verifier must receive explicit current checklist state");
        assert!(
            messages[0]
                .content
                .text_content()
                .contains("Independently verify")
        );
        self.checks.fetch_add(1, Ordering::SeqCst);
        if let Some(callback) = &self.before_verify {
            callback();
        }
        let text = self.verifications.lock().unwrap().pop_front().unwrap();
        Ok(response(Message::assistant(text)))
    }
    async fn ask(&self, _: InputRequest) -> Result<InputAnswer, AgentError> {
        Ok(InputAnswer::Dismissed)
    }
}
fn host(replies: &[&str], verifications: &[&str]) -> Arc<Host> {
    Arc::new(Host {
        replies: Mutex::new(
            replies
                .iter()
                .map(|text| Message::assistant(*text))
                .collect(),
        ),
        verifications: Mutex::new(verifications.iter().map(|text| (*text).into()).collect()),
        calls: AtomicUsize::new(0),
        checks: AtomicUsize::new(0),
        before_verify: None,
        before_reason: None,
    })
}
async fn run(host: Arc<Host>, store: Option<Arc<Store>>) -> Vec<Result<AgentEvent, AgentError>> {
    Arc::new(
        AgentRunner::new(host, Arc::new(ToolRegistry::new()), AgentConfig::default())
            .with_goal(store.map(|store| store as Arc<dyn GoalStore>)),
    )
    .run("Start".into(), CancellationToken::new())
    .collect()
    .await
}
#[tokio::test]
async fn goal_continues_until_independent_verification_passes_and_counts_cache_once() {
    let store = Store::active();
    let host = host(
        &["Partial implementation", "Implemented and tested"],
        &[
            r#"{"passed":false,"reason":"validation missing","nextAction":"Run the focused tests"}"#,
            r#"{"passed":true,"reason":"implementation and test evidence present"}"#,
        ],
    );
    let events = run(host.clone(), Some(store.clone())).await;
    let goal = store.read().unwrap().unwrap();
    assert_eq!(goal.status, GoalStatus::Complete);
    assert_eq!(goal.iteration, 2);
    assert_eq!(
        goal.tokens_used, 420,
        "cache tokens are subsets, not additional spending"
    );
    assert_eq!(host.calls.load(Ordering::SeqCst), 2);
    assert_eq!(host.checks.load(Ordering::SeqCst), 2);
    assert!(matches!(
        events.last(),
        Some(Ok(AgentEvent::Done {
            reason: DoneReason::Complete
        }))
    ));
}
#[tokio::test]
async fn ordinary_requests_do_not_create_or_verify_goals() {
    let host = host(&["Done"], &[]);
    run(host.clone(), None).await;
    assert_eq!(host.checks.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn editing_during_a_model_call_uses_latest_objective_on_next_call_without_cancelling() {
    let store = Store::active();
    let captured = store.clone();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured_requests = requests.clone();
    let mut host = host(
        &["Old objective answer", "New objective finished"],
        &[r#"{"passed":true,"reason":"new objective validated"}"#],
    );
    Arc::get_mut(&mut host).unwrap().before_reason = Some(Box::new(move |messages| {
        let mut requests = captured_requests.lock().unwrap();
        requests.push(
            messages
                .iter()
                .map(|m| m.content.text_content())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        if requests.len() == 1 {
            let mut state = captured.0.lock().unwrap();
            let goal = state.as_mut().unwrap();
            // Two saves during this call: only the latest is sent next time.
            goal.objective = "Intermediate edit".into();
            goal.revision += 1;
            goal.objective = "New objective".into();
            goal.revision += 1;
        }
    }));
    let events = run(host.clone(), Some(store.clone())).await;
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].contains("New objective"));
    assert!(requests[1].contains("New objective"));
    assert!(!requests[1].contains("Intermediate edit"));
    assert_eq!(
        host.checks.load(Ordering::SeqCst),
        1,
        "skip verifying the old answer"
    );
    let goal = store.read().unwrap().unwrap();
    assert_eq!(goal.id, "goal-1");
    assert_eq!(goal.revision, 2);
    assert_eq!(goal.status, GoalStatus::Complete);
    assert_eq!(
        goal.tokens_used, 315,
        "all calls remain attributed to this goal"
    );
    assert!(events.iter().all(Result::is_ok));
}

#[tokio::test]
async fn edits_during_verification_discard_both_old_passes_and_errors_and_continue() {
    for old_result in [
        r#"{"passed":true,"reason":"old target passed"}"#,
        "invalid JSON",
    ] {
        let store = Store::active();
        let captured = store.clone();
        let mut host = host(
            &["Old answer", "Updated answer"],
            &[
                old_result,
                r#"{"passed":true,"reason":"updated target passed"}"#,
            ],
        );
        Arc::get_mut(&mut host).unwrap().before_verify = Some(Box::new(move || {
            let mut state = captured.0.lock().unwrap();
            let goal = state.as_mut().unwrap();
            if goal.revision == 0 {
                goal.objective = "Updated objective".into();
                goal.revision = 1;
            }
        }));
        let events = run(host.clone(), Some(store.clone())).await;
        assert!(events.iter().all(Result::is_ok));
        assert_eq!(host.calls.load(Ordering::SeqCst), 2);
        assert_eq!(host.checks.load(Ordering::SeqCst), 2);
        let goal = store.read().unwrap().unwrap();
        assert_eq!(goal.status, GoalStatus::Complete);
        assert_eq!(goal.reason.as_deref(), Some("updated target passed"));
    }
}
#[tokio::test]
async fn malformed_or_unactionable_verification_pauses_instead_of_looping() {
    for verification in ["not JSON", r#"{"passed":false,"reason":"not done"}"#] {
        let store = Store::active();
        let host = host(&["Done"], &[verification]);
        run(host.clone(), Some(store.clone())).await;
        assert_eq!(store.read().unwrap().unwrap().status, GoalStatus::Paused);
        assert_eq!(host.calls.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn pause_or_replacement_while_verifier_runs_wins_over_its_result() {
    for replace in [false, true] {
        let store = Store::active();
        let captured = store.clone();
        let mut host = host(&["Done"], &[r#"{"passed":true,"reason":"done"}"#]);
        Arc::get_mut(&mut host).unwrap().before_verify = Some(Box::new(move || {
            if replace {
                *captured.0.lock().unwrap() = Some(
                    SessionGoal::new("new-goal".into(), "A different goal".into(), 1).unwrap(),
                );
            } else {
                captured
                    .change(
                        "goal-1",
                        GoalChange::Pause {
                            reason: "User pause".into(),
                        },
                    )
                    .unwrap();
            }
        }));
        run(host, Some(store.clone())).await;
        let goal = store.read().unwrap().unwrap();
        assert_eq!(
            goal.status,
            if replace {
                GoalStatus::Active
            } else {
                GoalStatus::Paused
            }
        );
        if replace {
            assert_eq!(goal.tokens_used, 0);
        }
    }
}
#[tokio::test]
async fn unfinished_todos_prevent_goal_completion_even_if_verifier_passes() {
    use rcode_agent::{ContentBlock, MessageContent};
    let store = Store::active();
    let mut host = host(
        &[],
        &[
            r#"{"passed":true,"reason":"claimed complete"}"#,
            r#"{"passed":true,"reason":"all done"}"#,
        ],
    );
    let messages = ["pending", "completed"]
        .into_iter()
        .enumerate()
        .flat_map(|(index, status)| {
            [
                Message::assistant(MessageContent::blocks(vec![ContentBlock::ToolUse {
                    id: format!("todo-{index}"),
                    name: "TodoWrite".into(),
                    input: serde_json::json!({"todos":[{"text":"Validate","status":status}]}),
                }])),
                Message::assistant("Ready"),
            ]
        })
        .collect();
    *Arc::get_mut(&mut host).unwrap().replies.lock().unwrap() = messages;
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(rcode_agent::tools::TodoWriteTool));
    let events: Vec<_> = Arc::new(
        AgentRunner::new(host.clone(), Arc::new(tools), AgentConfig::default())
            .with_goal(Some(store.clone())),
    )
    .run("Start".into(), CancellationToken::new())
    .collect()
    .await;
    assert!(matches!(
        events.last(),
        Some(Ok(AgentEvent::Done {
            reason: DoneReason::Complete
        }))
    ));
    assert_eq!(store.read().unwrap().unwrap().iteration, 2);
    assert_eq!(host.calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn unfinished_todo_with_explicit_blocker_pauses_instead_of_completing_goal() {
    let store = Store::active();
    let mut host = host(&[], &[]);
    *Arc::get_mut(&mut host).unwrap().replies.lock().unwrap() = VecDeque::from([
        Message::assistant(vec![rcode_agent::ContentBlock::tool_use("blocked", "TodoWrite", serde_json::json!({
            "todos":[{"text":"Validate in staging", "status":"pending"}],
            "resolution":{"kind":"blocked", "reason":"Staging credentials are unavailable"}
        }))]),
        Message::assistant("Validation is blocked because staging credentials are unavailable"),
    ]);
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(rcode_agent::tools::TodoWriteTool));
    let events: Vec<_> = Arc::new(AgentRunner::new(host.clone(), Arc::new(tools), AgentConfig::default())
        .with_goal(Some(store.clone())))
        .run("Implement and validate".into(), CancellationToken::new()).collect().await;
    assert!(events.iter().all(Result::is_ok));
    let goal = store.read().unwrap().unwrap();
    assert_eq!(goal.status, GoalStatus::Paused);
    assert!(goal.reason.unwrap().contains("Staging credentials are unavailable"));
    assert_eq!(host.checks.load(Ordering::SeqCst), 0);
    assert_eq!(host.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn compacted_todos_still_block_a_false_goal_pass_without_duplicate_ordinary_steering() {
    let store = Store::active();
    let mut history = vec![Message::user("Implement and validate"),
        Message::assistant(vec![rcode_agent::ContentBlock::tool_use("old", "TodoWrite", serde_json::json!({
            "todos":[{"text":"Run validation", "status":"pending"}]
        }))]), Message::tool_result("old", "ok", false)];
    history.extend((0..6).map(|_| Message::assistant("Working")));
    let compacted = rcode_agent::compact::full_compact(history, &rcode_agent::CompactConfig {
        context_window: 100_000, keep_recent_turns: 2, ..Default::default()
    }, |_| async { Ok("The assistant claimed everything was completed".into()) }).await.unwrap();
    let mut host = host(&[], &[r#"{"passed":true,"reason":"claimed complete"}"#,
        r#"{"passed":true,"reason":"validation recorded"}"#]);
    *Arc::get_mut(&mut host).unwrap().replies.lock().unwrap() = VecDeque::from([
        Message::assistant("Ready"),
        Message::assistant(vec![rcode_agent::ContentBlock::tool_use("finish", "TodoWrite", serde_json::json!({
            "todos":[{"text":"Run validation", "status":"completed"}]
        }))]), Message::assistant("Validated"),
    ]);
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(rcode_agent::tools::TodoWriteTool));
    let sink = Arc::new(Mutex::new(Vec::new()));
    let events: Vec<_> = Arc::new(AgentRunner::new(host.clone(), Arc::new(tools), AgentConfig {
        history: compacted.messages, max_steps: Some(10), ..Default::default()
    }).with_goal(Some(store.clone())).with_history_sink(sink.clone()))
        .run("Continue".into(), CancellationToken::new()).collect().await;
    assert!(events.iter().all(Result::is_ok));
    assert_eq!(store.read().unwrap().unwrap().status, GoalStatus::Complete);
    assert_eq!(host.checks.load(Ordering::SeqCst), 2);
    assert_eq!(host.calls.load(Ordering::SeqCst), 3);
    assert!(!sink.lock().unwrap().iter().any(|m| m.context.as_ref().is_some_and(|c|
        c.domain == rcode_agent::context::Domain::Todo && c.upsert.get("kind").is_some_and(|kind| kind == "closure"))));
}

#[tokio::test]
async fn cancellation_before_start_or_during_verification_pauses_the_goal() {
    for during_verify in [false, true] {
        let store = Store::active();
        let cancel = CancellationToken::new();
        let mut host = host(&["Done"], &[r#"{"passed":true,"reason":"done"}"#]);
        if during_verify {
            let cancel = cancel.clone();
            Arc::get_mut(&mut host).unwrap().before_verify =
                Some(Box::new(move || cancel.cancel()));
        } else {
            cancel.cancel();
        }
        let events: Vec<_> = Arc::new(
            AgentRunner::new(
                host.clone(),
                Arc::new(ToolRegistry::new()),
                AgentConfig::default(),
            )
            .with_goal(Some(store.clone())),
        )
        .run("Start".into(), cancel)
        .collect()
        .await;
        assert_eq!(store.read().unwrap().unwrap().status, GoalStatus::Paused);
        assert!(matches!(
            events.last(),
            Some(Ok(AgentEvent::Done {
                reason: DoneReason::Interrupted
            }))
        ));
        assert_eq!(
            host.calls.load(Ordering::SeqCst),
            usize::from(during_verify)
        );
    }
}

#[tokio::test]
async fn goal_turn_boundary_preserves_history_and_defers_next_work_to_the_host() {
    let store=Store::active();
    let host=host(&["First checkpoint","Second checkpoint"],&[r#"{"passed":false,"reason":"not verified","nextAction":"Run validation"}"#,r#"{"passed":true,"reason":"validated"}"#]);
    let sink=Arc::new(Mutex::new(None));let history=Arc::new(Mutex::new(Vec::new()));
    let runner=Arc::new(AgentRunner::new(host.clone(),Arc::new(ToolRegistry::new()),AgentConfig::default()).with_goal(Some(store.clone())).with_history_sink(history.clone()).with_goal_continuation(sink.clone()));
    let events:Vec<_>=runner.run("Start".into(),CancellationToken::new()).collect().await;
    assert_eq!(host.calls.load(Ordering::SeqCst),1);assert_eq!(store.read().unwrap().unwrap().status,GoalStatus::Active);
    assert!(matches!(events.last(),Some(Ok(AgentEvent::Done{reason:DoneReason::Complete}))));
    let pending=sink.lock().unwrap().take().unwrap();assert!(pending.prompt.contains("Run validation"));
    assert!(serde_json::to_string(&*history.lock().unwrap()).unwrap().contains("First checkpoint"));
    // A pause during host preparation must not turn an old goal prompt into an ordinary request.
    store.change("goal-1",GoalChange::Pause{reason:"user paused".into()}).unwrap();
    let runner=Arc::new(AgentRunner::new(host.clone(),Arc::new(ToolRegistry::new()),AgentConfig::default()).with_goal(Some(store)).with_expected_goal(pending.goal_id));
    let _:Vec<_>=runner.run(pending.prompt,CancellationToken::new()).collect().await;
    assert_eq!(host.calls.load(Ordering::SeqCst),1,"no model request after goal pause");
}
