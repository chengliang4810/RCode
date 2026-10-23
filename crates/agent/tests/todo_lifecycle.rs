use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};
use rcode_agent::{
    AgentConfig, AgentError, AgentEvent, AgentHost, AgentRunner, ContentBlock, DoneReason,
    HistoryRecorder, InputAnswer, InputRequest, Message, ReasoningChunk, Tool, ToolRegistry,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Recording(Mutex<Vec<Message>>);
#[async_trait]
impl HistoryRecorder for Recording {
    async fn record(&self, message: &Message) -> Result<(), AgentError> {
        self.0.lock().unwrap().push(message.clone());
        Ok(())
    }
}

struct Host {
    replies: Mutex<VecDeque<Message>>,
    seen: Mutex<Vec<Vec<Message>>>,
    cancel_on_closure: Option<CancellationToken>,
}
#[async_trait]
impl AgentHost for Host {
    async fn reason(
        &self,
        messages: &[Message],
        _: &[Arc<dyn Tool>],
    ) -> Result<BoxStream<'static, Result<ReasoningChunk, AgentError>>, AgentError> {
        self.seen.lock().unwrap().push(messages.to_vec());
        if messages.iter().any(|m| kind(m, "closure")) {
            if let Some(cancel) = &self.cancel_on_closure {
                cancel.cancel();
                return Err(AgentError::Cancelled);
            }
        }
        let message = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| AgentError::Model("unexpected extra model call".into()))?;
        Ok(Box::pin(futures::stream::iter([Ok(
            ReasoningChunk::Done {
                message,
                usage: None,
            },
        )])))
    }
    async fn ask(&self, _: InputRequest) -> Result<InputAnswer, AgentError> {
        Ok(InputAnswer::Dismissed)
    }
}

fn write(id: &str, status: &str) -> Message {
    Message::assistant(vec![ContentBlock::tool_use(
        id,
        "TodoWrite",
        json!({
            "todos":[{"text":"Validate the behavior", "status":status}]
        }),
    )])
}

async fn run(
    replies: Vec<Message>,
    history: Vec<Message>,
    tool_available: bool,
) -> (
    Arc<Host>,
    Arc<Recording>,
    Vec<Result<AgentEvent, AgentError>>,
) {
    let host = Arc::new(Host {
        replies: Mutex::new(replies.into()),
        seen: Mutex::default(),
        cancel_on_closure: None,
    });
    let recording = Arc::new(Recording::default());
    let mut tools = ToolRegistry::new();
    if tool_available {
        tools.register(Arc::new(rcode_agent::tools::TodoWriteTool));
    }
    let runner = Arc::new(
        AgentRunner::new(
            host.clone(),
            Arc::new(tools),
            AgentConfig {
                history,
                max_steps: Some(20),
                ..Default::default()
            },
        )
        .with_history_recorder(recording.clone()),
    );
    let events = runner
        .run("Implement and validate".into(), CancellationToken::new())
        .collect()
        .await;
    (host, recording, events)
}

fn kind(message: &Message, kind: &str) -> bool {
    message.context.as_ref().is_some_and(|record| {
        record.domain == rcode_agent::context::Domain::Todo
            && record.upsert.get("kind").is_some_and(|value| value == kind)
    })
}

#[tokio::test]
async fn premature_answer_continues_once_then_finishes_after_successful_update() {
    let (host, recording, events) = run(
        vec![
            write("start", "pending"),
            Message::assistant("Done"),
            write("finish", "completed"),
            Message::assistant("Implemented and validated"),
        ],
        vec![],
        true,
    )
    .await;
    assert!(events.iter().all(Result::is_ok));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Ok(AgentEvent::Done { .. })))
            .count(),
        1
    );
    assert!(matches!(
        events.last(),
        Some(Ok(AgentEvent::Done {
            reason: DoneReason::Complete
        }))
    ));
    let seen = host.seen.lock().unwrap();
    assert_eq!(seen.len(), 4);
    let reminder = seen[2].iter().find(|m| kind(m, "closure")).unwrap();
    assert!(!reminder.is_user_input());
    assert!(
        reminder
            .content
            .text_content()
            .contains("Validate the behavior")
    );
    assert_eq!(
        recording
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|m| kind(m, "closure"))
            .count(),
        1
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Ok(AgentEvent::UserMessage { .. })))
    );
}

#[tokio::test]
async fn explicit_blocker_allows_honest_answer_without_falsely_completing_todos() {
    let blocked = Message::assistant(vec![ContentBlock::tool_use(
        "blocked",
        "TodoWrite",
        json!({
            "todos":[{"text":"Validate the behavior","status":"pending"}],
            "resolution":{"kind":"blocked","reason":"Test environment is unavailable"}
        }),
    )]);
    let (host, recording, events) = run(
        vec![
            write("start", "pending"),
            Message::assistant("Stopped"),
            blocked,
            Message::assistant("Validation remains blocked by the unavailable test environment"),
        ],
        vec![],
        true,
    )
    .await;
    assert!(events.iter().all(Result::is_ok));
    assert_eq!(host.seen.lock().unwrap().len(), 4);
    let state = rcode_agent::todo::State::from_history(&recording.0.lock().unwrap());
    assert!(state.unfinished());
    assert_eq!(
        state.resolution().unwrap().reason,
        "Test environment is unavailable"
    );
}

#[tokio::test]
async fn overdue_reminder_includes_real_list_and_does_not_repeat_after_update() {
    let mut history = vec![
        write("old", "in_progress"),
        Message::tool_result("old", "ok", false),
    ];
    history.extend((0..10).map(|_| Message::assistant("Working")));
    let (host, recording, events) = run(
        vec![write("finish", "completed"), Message::assistant("Done")],
        history,
        true,
    )
    .await;
    assert!(events.iter().all(Result::is_ok));
    let seen = host.seen.lock().unwrap();
    let reminder = seen[0].iter().find(|m| kind(m, "reminder")).unwrap();
    assert!(reminder.content.text_content().contains("in_progress"));
    assert!(
        reminder
            .content
            .text_content()
            .contains("Validate the behavior")
    );
    assert_eq!(
        recording
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|m| kind(m, "reminder"))
            .count(),
        1
    );
    assert!(
        !recording
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|m| kind(m, "closure"))
    );
}

#[tokio::test]
async fn unavailable_tool_does_not_create_an_unresolvable_ordinary_loop() {
    let mut history = vec![
        write("old", "pending"),
        Message::tool_result("old", "ok", false),
    ];
    history.extend((0..10).map(|_| Message::assistant("Working")));
    let (host, recording, events) =
        run(vec![Message::assistant("Current answer")], history, false).await;
    assert!(events.iter().all(Result::is_ok));
    assert_eq!(host.seen.lock().unwrap().len(), 1);
    assert!(
        !recording
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.context.is_some())
    );
}

#[tokio::test]
async fn user_cancellation_during_closure_stops_without_more_todo_work() {
    let cancel = CancellationToken::new();
    let host = Arc::new(Host {
        replies: Mutex::new(VecDeque::from([
            write("start", "pending"),
            Message::assistant("Done"),
        ])),
        seen: Mutex::default(),
        cancel_on_closure: Some(cancel.clone()),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(rcode_agent::tools::TodoWriteTool));
    let events: Vec<_> = Arc::new(AgentRunner::new(
        host.clone(),
        Arc::new(tools),
        AgentConfig {
            max_steps: Some(10),
            ..Default::default()
        },
    ))
    .run("Implement".into(), cancel)
    .collect()
    .await;
    assert!(matches!(
        events.last(),
        Some(Ok(AgentEvent::Done {
            reason: DoneReason::Interrupted
        }))
    ));
    assert_eq!(host.seen.lock().unwrap().len(), 3);
}
