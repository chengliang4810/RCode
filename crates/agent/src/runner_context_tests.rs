use super::*;
use crate::context::{ContextSource, Domain, Operation, Snapshot, Status};
use std::collections::BTreeMap;
use std::sync::Mutex;

struct FileContext(std::path::PathBuf);
#[async_trait::async_trait]
impl ContextSource for FileContext {
    async fn snapshots(&self) -> Vec<Snapshot> {
        vec![crate::memory::index::load(&self.0).facts]
    }
}

struct Host {
    seen: Mutex<Vec<Vec<Message>>>,
    write_to: Option<std::path::PathBuf>,
}
#[async_trait::async_trait]
impl AgentHost for Host {
    async fn reason(
        &self,
        messages: &[Message],
        _: &[Arc<dyn crate::Tool>],
    ) -> Result<BoxStream<'static, Result<ReasoningChunk, AgentError>>, AgentError> {
        let mut seen = self.seen.lock().unwrap();
        seen.push(messages.to_vec());
        let reply = if seen.len() == 1 && self.write_to.is_some() {
            Message::assistant(vec![ContentBlock::tool_use(
                "write-index",
                "Write",
                serde_json::json!({
                    "file_path": self.write_to, "content": "- [Fact](fact.md) — NEW VALUE"
                }),
            )])
        } else {
            Message::assistant("done")
        };
        Ok(Box::pin(futures::stream::iter([Ok(
            ReasoningChunk::Done {
                message: reply,
                usage: None,
            },
        )])))
    }
    async fn ask(&self, _: InputRequest) -> Result<InputAnswer, AgentError> {
        Ok(InputAnswer::Dismissed)
    }
    async fn summarize(&self, _: String) -> Result<String, AgentError> {
        Ok("OLD VALUE in the historical summary".into())
    }
}

fn files() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("fact.md"), "---\ntype: project\n---\nbody").unwrap();
    std::fs::write(
        root.path().join("MEMORY.md"),
        "- [Fact](fact.md) — OLD VALUE",
    )
    .unwrap();
    root
}

#[tokio::test]
async fn actual_write_is_followed_by_a_context_delta_without_a_visible_user_message() {
    let root = files();
    let host = Arc::new(Host {
        seen: Mutex::default(),
        write_to: Some(root.path().join("MEMORY.md")),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(crate::tools::WriteTool));
    let runner = Arc::new(
        AgentRunner::new(
            host.clone(),
            Arc::new(tools),
            AgentConfig {
                cwd: root.path().to_owned(),
                ..Default::default()
            },
        )
        .with_context_source(Arc::new(FileContext(root.path().to_owned()))),
    );
    let events: Vec<_> = runner
        .run("update the fact".into(), CancellationToken::new())
        .collect()
        .await;
    assert!(events.iter().any(|e| matches!(
        e,
        Ok(AgentEvent::Done {
            reason: DoneReason::Complete
        })
    )));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Ok(AgentEvent::UserMessage { .. })))
    );
    let seen = host.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].iter().filter(|m| m.context.is_some()).count(), 1);
    let last = seen[1].last().unwrap();
    assert_eq!(last.context.as_ref().unwrap().operation, Operation::Patch);
    assert!(last.content.text_content().contains("NEW VALUE"));
    assert_eq!(&seen[1][..seen[0].len()], seen[0].as_slice());
}

struct FailingRecorder;
#[async_trait::async_trait]
impl crate::HistoryRecorder for FailingRecorder {
    async fn record(&self, _: &Message) -> Result<(), AgentError> {
        Err(AgentError::Host("disk unavailable".into()))
    }
}

#[tokio::test]
async fn failed_persistence_and_cancellation_do_not_advance_delivery() {
    let root = files();
    let host = Arc::new(Host {
        seen: Mutex::default(),
        write_to: None,
    });
    let source: Arc<dyn ContextSource> = Arc::new(FileContext(root.path().to_owned()));
    let runner = AgentRunner::new(
        host.clone(),
        Arc::new(ToolRegistry::new()),
        AgentConfig::default(),
    )
    .with_context_source(source.clone())
    .with_history_recorder(Arc::new(FailingRecorder));
    let mut messages = vec![Message::user("real user")];
    assert!(
        runner
            .refresh_context(&mut messages, &CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(messages.len(), 1);
    let runner = AgentRunner::new(host, Arc::new(ToolRegistry::new()), AgentConfig::default())
        .with_context_source(source);
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(
        runner
            .refresh_context(&mut messages, &cancelled)
            .await
            .is_err()
    );
    assert_eq!(messages.len(), 1);
    runner
        .refresh_context(&mut messages, &CancellationToken::new())
        .await
        .unwrap();
    runner
        .refresh_context(&mut messages, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(messages.len(), 2);
}

#[tokio::test]
async fn full_compaction_keeps_real_user_task_and_reestablishes_current_context() {
    let root = files();
    let host = Arc::new(Host {
        seen: Mutex::default(),
        write_to: None,
    });
    let old = Snapshot {
        domain: Domain::MemoryIndex,
        scope: "old".into(),
        status: Status::Ready,
        entries: BTreeMap::from([("fact".into(), "OLD".into())]),
    };
    let mut history = vec![crate::context::next_record(&[], old).unwrap().message()];
    for _ in 0..10 {
        history.push(Message::user("long previous request ".repeat(100)));
        history.push(Message::assistant("long previous response ".repeat(100)));
    }
    let runner = Arc::new(
        AgentRunner::new(
            host.clone(),
            Arc::new(ToolRegistry::new()),
            AgentConfig {
                cwd: root.path().to_owned(),
                history,
                compact: Some(crate::CompactConfig {
                    context_window: 4000,
                    keep_recent_turns: 1,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .with_context_source(Arc::new(FileContext(root.path().to_owned()))),
    );
    let events: Vec<_> = runner
        .run("CURRENT REAL USER TASK".into(), CancellationToken::new())
        .collect()
        .await;
    assert!(events.iter().any(|e| matches!(
        e,
        Ok(AgentEvent::Compacted {
            stage: crate::CompactionStage::Full,
            ..
        })
    )));
    let seen = host.seen.lock().unwrap();
    let messages = &seen[0];
    assert!(
        messages
            .iter()
            .any(|m| m.is_user_input() && m.content.text_content() == "CURRENT REAL USER TASK")
    );
    let record = messages
        .iter()
        .rev()
        .find_map(|m| m.context.as_ref())
        .unwrap();
    assert_eq!(record.operation, Operation::Replace);
    assert!(
        record
            .upsert
            .values()
            .any(|value| value.contains("OLD VALUE"))
    );
}
