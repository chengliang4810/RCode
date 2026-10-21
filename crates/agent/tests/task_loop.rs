//! The Task tool through the loop: a delegation streams its child's events
//! into the parent run's event stream, tagged with the spawning tool call's
//! id, and the tool result carries the child's final message.

use std::sync::Arc;

use futures::StreamExt;
use rcode_agent::message::MessageRole;
use rcode_agent::subagents::{ModelAlias, SubagentDefinition};
use rcode_agent::tools::{SubagentOutcome, SubagentRuntime, SubagentSpawn, AGENT, AgentTool};
use rcode_agent::{
    AgentConfig, AgentEvent, AgentHost, AgentRunner, ContentBlock, DoneReason, InputAnswer,
    InputRequest, Message, MessageContent, MessageId, ReasoningChunk, ToolRegistry,
};
use tokio_util::sync::CancellationToken;

/// A runtime that streams its child's events through the sink it is handed,
/// then reports the result — the shape the harness runtime has.
struct StreamingRuntime;

#[async_trait::async_trait]
impl SubagentRuntime for StreamingRuntime {
    fn definitions(&self, _cwd: &std::path::Path) -> Vec<SubagentDefinition> {
        vec![SubagentDefinition {
            name: "reviewer".to_owned(),
            description: "Reviews a diff.".to_owned(),
            tools: Vec::new(),
            disallowed_tools: vec![AGENT.to_owned()],
            model: ModelAlias::Inherit,
            system_prompt: "You review.".to_owned(),
            path: std::path::PathBuf::from("/cfg/agents/reviewer.md"),
            source: rcode_agent::skills::SkillSource::Project,
        }]
    }

    async fn run(&self, spawn: SubagentSpawn) -> Result<SubagentOutcome, String> {
        spawn
            .events
            .send(AgentEvent::TextDelta {
                text: "working hard".to_owned(),
            })
            .await;
        spawn
            .events
            .send(AgentEvent::Done {
                reason: DoneReason::Complete,
            })
            .await;
        Ok(SubagentOutcome {
            history: Vec::new(),
            output: "the review".to_owned(),
        })
    }
}

/// A host whose model calls Task once, then answers.
///
/// The loop replays the whole conversation on every step, so a host that
/// always asked for the same tool would delegate forever. The second call is
/// the answer that ends the run — which is also what proves the child's `Done`
/// did not end the parent.
struct TaskCallingHost {
    call_id: String,
    /// How many model calls this host has served. The first delegates; every
    /// later one answers, so a regression that re-delegates cannot hang the
    /// test on an unbounded event stream.
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl AgentHost for TaskCallingHost {
    async fn reason(
        &self,
        _messages: &[Message],
        _tools: &[Arc<dyn rcode_agent::Tool>],
    ) -> Result<
        futures::stream::BoxStream<'static, Result<ReasoningChunk, rcode_agent::AgentError>>,
        rcode_agent::AgentError,
    > {
        let step = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let content = if step == 0 {
            MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: self.call_id.clone(),
                name: AGENT.to_owned(),
                input: serde_json::json!({
                    "prompt": "review the diff",
                    "subagent_type": "reviewer"
                }),
            }])
        } else {
            MessageContent::text("the review is in")
        };
        let message = Message {
            id: MessageId::new(),
            role: MessageRole::Assistant,
            content,
        };
        Ok(Box::pin(futures::stream::iter([Ok(
            ReasoningChunk::Done {
                message,
                usage: None,
            },
        )])))
    }

    async fn ask(&self, _request: InputRequest) -> Result<InputAnswer, rcode_agent::AgentError> {
        Ok(InputAnswer::Dismissed)
    }
}

#[tokio::test]
async fn the_loop_streams_nested_events_tagged_with_the_tool_call() {
    let tool = AgentTool::new(Arc::new(StreamingRuntime));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(tool));

    let host = Arc::new(TaskCallingHost {
        call_id: "toolu_spawn".to_owned(),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let runner = Arc::new(AgentRunner::new(
        host,
        Arc::new(registry),
        AgentConfig::default(),
    ));

    let events: Vec<_> = runner
        .run("go".to_owned(), CancellationToken::new())
        .collect()
        .await;

    // The child's traffic arrived tagged with the spawning call, and the
    // child's Done did NOT end the parent run — the parent ended with its own
    // Done after the tool result.
    let nested: Vec<&AgentEvent> = events
        .iter()
        .filter_map(|item| match item {
            Ok(AgentEvent::Nested {
                parent_tool_use_id,
                event,
            }) if parent_tool_use_id == "toolu_spawn" => Some(event.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(nested.len(), 2, "text and Done, tagged: {events:?}");
    assert!(matches!(
        nested[0],
        AgentEvent::TextDelta { text } if text == "working hard"
    ));
    assert!(matches!(nested[1], AgentEvent::Done { .. }));

    // The tool result the parent model reads carries the child's final
    // message.
    let result = events.iter().find_map(|item| match item {
        Ok(AgentEvent::ToolResult {
            output, is_error, ..
        }) => (!is_error).then(|| output.clone().unwrap_or_default()),
        _ => None,
    });
    assert!(
        result.is_some_and(|text| text.contains("the review")),
        "{events:?}"
    );
    assert!(events.iter().any(|item| matches!(
        item,
        Ok(AgentEvent::Done {
            reason: DoneReason::Complete
        })
    )));
}
