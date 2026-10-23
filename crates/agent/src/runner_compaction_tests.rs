//! Automatic compaction: the loop must shrink a growing conversation before
//! the provider is asked to accept something that cannot fit.
//!
//! These tests drive the real loop with a host that reports a tiny window, so
//! the thresholds are crossed with a handful of messages rather than hundreds.
//! The numbers are small on purpose — the behaviour under test is *when* the
//! loop acts, not how well tokens are estimated (that is `compact`'s own suite).
use super::*;
use crate::AgentEvent;
use crate::CompactionStage;
use crate::compact::CompactConfig;
use crate::tool::Tool;

/// A host that answers with text, records what it was asked to summarize, and
/// can be told whether summarizing fails.
struct CompactHost {
    /// What each `reason` call saw, so a test can assert on the request the
    /// loop actually built.
    seen: std::sync::Mutex<Vec<Vec<Message>>>,
    summaries: std::sync::Mutex<Vec<String>>,
    fail_summarize: bool,
    window: u32,
}

#[async_trait::async_trait]
impl AgentHost for CompactHost {
    async fn reason(
        &self,
        messages: &[Message],
        _tools: &[Arc<dyn Tool>],
    ) -> Result<BoxStream<'static, Result<ReasoningChunk, AgentError>>, AgentError> {
        self.seen.lock().unwrap().push(messages.to_vec());
        // Always answer with text and no tool call, so the run ends after one
        // step. The conversation growth these tests need comes from the
        // replayed history, not from a tool loop.
        Ok(Box::pin(futures::stream::iter(vec![
            Ok(ReasoningChunk::Text("done".to_owned())),
            Ok(ReasoningChunk::Done {
                usage: None,
                message: Message::assistant(crate::message::MessageContent::text("done")),
            }),
        ])))
    }

    async fn ask(&self, _request: InputRequest) -> Result<InputAnswer, AgentError> {
        Ok(InputAnswer::Dismissed)
    }

    async fn summarize(&self, transcript: String) -> Result<String, AgentError> {
        if self.fail_summarize {
            return Err(AgentError::Model("summarizer is down".to_owned()));
        }
        self.summaries.lock().unwrap().push(transcript);
        Ok("THE-SUMMARY".to_owned())
    }

    fn context_window(&self) -> Option<u32> {
        Some(self.window)
    }
}

/// A conversation long enough to cross `full_ratio` on a small window.
///
/// Each turn is a wordy user message and a reply, so the estimate climbs
/// quickly — the point is to reach the threshold in a test-sized input.
fn long_history(turns: usize) -> Vec<Message> {
    let mut messages = Vec::new();
    for index in 0..turns {
        messages.push(Message::user(format!(
            "turn {index}: {}",
            "some reasonably long user text ".repeat(20)
        )));
        messages.push(Message::assistant(crate::message::MessageContent::text(
            "an assistant reply that is also fairly long ".repeat(20),
        )));
    }
    messages
}

fn host(window: u32, fail_summarize: bool) -> Arc<CompactHost> {
    Arc::new(CompactHost {
        seen: std::sync::Mutex::new(Vec::new()),
        summaries: std::sync::Mutex::new(Vec::new()),
        fail_summarize,
        window,
    })
}

fn config(window: u32) -> CompactConfig {
    CompactConfig {
        context_window: window,
        keep_recent_turns: 2,
        ..CompactConfig::for_window(window)
    }
}

/// Run the loop once with the given history and compaction config.
async fn run_once(
    host: Arc<CompactHost>,
    history: Vec<Message>,
    compact: Option<CompactConfig>,
) -> Vec<AgentEvent> {
    let runner = Arc::new(AgentRunner::new(
        host,
        Arc::new(ToolRegistry::new()),
        AgentConfig {
            history,
            compact,
            ..AgentConfig::default()
        },
    ));
    runner
        .run("go".to_owned(), CancellationToken::new())
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter_map(Result::ok)
        .collect()
}

/// Everything the model was sent on one call, as one string.
fn request_text(seen: &[Vec<Message>], index: usize) -> String {
    seen[index]
        .iter()
        .map(|message| message.content.text_content())
        .collect::<Vec<_>>()
        .join("\n")
}

struct BootstrapHook;
#[async_trait::async_trait]
impl crate::hook::HookRunner for BootstrapHook {
    async fn fire(&self, event: crate::hook::HookEvent, context: crate::hook::HookContext) -> crate::hook::HookResult {
        let outcome = if event == crate::hook::HookEvent::SessionStart {
            crate::hook::HookOutcome::Context(format!("BOOTSTRAP-{}", context.source.as_deref().unwrap_or("startup")))
        } else { crate::hook::HookOutcome::Allow };
        crate::hook::HookResult { outcome, failures: vec![] }
    }
}

#[tokio::test]
async fn bootstrap_context_reaches_the_model_and_is_reinjected_after_compaction() {
    for compact in [false, true] {
        let host = host(if compact { 500 } else { 200_000 }, false);
        let runner = Arc::new(AgentRunner::new(host.clone(), Arc::new(ToolRegistry::new()), AgentConfig {
            history: if compact { long_history(6) } else { vec![] },
            compact: compact.then(|| config(500)), ..Default::default()
        }).with_hooks(Arc::new(BootstrapHook)));
        let events = runner.run("go".into(), CancellationToken::new()).collect::<Vec<_>>().await;
        let requests = host.seen.lock().unwrap();
        let sent = request_text(&requests, 0);
        assert!(sent.contains(if compact { "BOOTSTRAP-compact" } else { "BOOTSTRAP-startup" }), "the model never received the plugin bootstrap");
        assert!(!events.iter().any(|event| matches!(event, Ok(AgentEvent::TextDelta { text }) if text.contains("BOOTSTRAP"))), "internal hook context leaked into the assistant reply");
    }
}

#[tokio::test]
async fn a_small_conversation_is_left_alone() {
    let host = host(200_000, false);
    let events = run_once(host.clone(), long_history(2), Some(config(200_000))).await;

    assert!(
        host.summaries.lock().unwrap().is_empty(),
        "a conversation well under the window must not be summarized"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AgentEvent::Compacted { .. })),
        "nothing should have been reported as compacted: {events:?}"
    );
}

#[tokio::test]
async fn a_full_conversation_is_summarized_before_the_request() {
    // A window small enough that the replayed history crosses full_ratio.
    let host = host(500, false);
    let history = long_history(6);

    // The history alone must actually be over the threshold, or the test
    // proves nothing.
    let before = crate::compact::pressure(&history, &config(500));
    assert!(
        before >= 0.95,
        "the fixture is not full enough to trigger compaction ({before}); \
         adjust the window rather than trusting a test that cannot fail"
    );

    let events = run_once(host.clone(), history, Some(config(500))).await;

    assert_eq!(
        host.summaries.lock().unwrap().len(),
        1,
        "the loop must summarize exactly once"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentEvent::Compacted {
                stage: CompactionStage::Full,
                ..
            }
        )),
        "a full compaction must be reported: {events:?}"
    );

    // The model must have seen the summary in place of the older turns.
    let seen = host.seen.lock().unwrap();
    assert!(!seen.is_empty(), "the model was called");
    let text = request_text(&seen, 0);
    assert!(
        text.contains("THE-SUMMARY"),
        "the summary never reached the model: {text}"
    );
    assert!(
        !text.contains("turn 0:"),
        "a summarized-away turn was still sent: {text}"
    );
    // The recent turns survive verbatim.
    assert!(
        text.contains("turn 5:"),
        "the most recent turn must be kept: {text}"
    );
}

#[tokio::test]
async fn a_failed_summary_does_not_lose_the_conversation() {
    let host = host(500, true);
    let events = run_once(host.clone(), long_history(6), Some(config(500))).await;

    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentEvent::Compacted {
                stage: CompactionStage::Failed,
                ..
            }
        )),
        "a failed summary must be reported: {events:?}"
    );

    // The run still happened, with the whole conversation intact: a summary
    // that did not happen must not drop anything.
    let seen = host.seen.lock().unwrap();
    assert!(!seen.is_empty(), "the model was still called");
    let text = request_text(&seen, 0);
    assert!(
        text.contains("turn 0:"),
        "a failed summary dropped history: {text}"
    );
    assert!(
        !text.contains("THE-SUMMARY"),
        "the summary was inserted even though it failed"
    );
}

#[tokio::test]
async fn without_a_configured_window_nothing_is_compacted() {
    // A host that cannot say how big its window is must not be guessed at.
    let host = host(500, false);
    let events = run_once(host.clone(), long_history(6), None).await;

    assert!(
        host.summaries.lock().unwrap().is_empty(),
        "compaction ran without a known window"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AgentEvent::Compacted { .. }))
    );
}

#[tokio::test]
async fn micro_compaction_fires_before_a_summary_is_spent() {
    // Tool output is what makes most long conversations long. Micro must trim
    // it and, if that is enough, never spend a summarization call.
    let mut history = long_history(3);
    // One enormous tool result pushes the conversation over micro_ratio. It
    // must be well over `max_tool_output_chars` (4000) or micro would find
    // nothing to trim — exactly the trap the first version of this test fell
    // into.
    history.push(Message {
        context: None,
        id: crate::message::MessageId::new(),
        role: crate::message::MessageRole::Tool,
        content: crate::message::MessageContent::Blocks(vec![
            crate::message::ContentBlock::ToolResult {
                tool_use_id: "call-1".to_owned(),
                content: "x".repeat(20_000),
                is_error: false,
            },
        ]),
    });
    history.push(Message::user("go on"));

    // Chosen so that trimming alone brings the conversation back under
    // full_ratio: the point of the test is that micro suffices.
    let window = 6_000;
    let host = host(window, false);
    let events = run_once(host.clone(), history, Some(config(window))).await;

    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentEvent::Compacted {
                stage: CompactionStage::Micro,
                shortened, ..
            } if *shortened > 0
        )),
        "the oversized tool result should have been trimmed: {events:?}"
    );
    assert!(
        host.summaries.lock().unwrap().is_empty(),
        "micro freed enough; a summarization call should not have been spent"
    );
}

#[tokio::test]
async fn the_summary_is_sent_as_a_user_turn_not_an_assistant_one() {
    // It is context being handed *to* the model. Framing it as something the
    // assistant said would put words in its mouth.
    let host = host(500, false);
    let _ = run_once(host.clone(), long_history(6), Some(config(500))).await;

    let seen = host.seen.lock().unwrap();
    assert!(!seen.is_empty(), "the model was called");
    let carrying = seen[0]
        .iter()
        .find(|message| message.content.text_content().contains("THE-SUMMARY"))
        .expect("the summary is in the request");
    assert_eq!(
        carrying.role,
        crate::message::MessageRole::User,
        "the summary must arrive as a user turn"
    );
}

#[test]
fn the_two_thresholds_gate_the_two_stages_independently() {
    let config = CompactConfig::for_window(1_000);
    // Just over half: neither stage.
    let light = vec![Message::user("x".repeat(1_600))];
    assert!(!crate::compact::should_micro_compact(&light, &config));
    assert!(!crate::compact::should_compact(&light, &config));

    // Between the ratios: micro only.
    let medium = vec![Message::user("x".repeat(3_200))];
    assert!(crate::compact::should_micro_compact(&medium, &config));
    assert!(!crate::compact::should_compact(&medium, &config));

    // Past the window: both.
    let heavy = vec![Message::user("x".repeat(4_000))];
    assert!(crate::compact::should_micro_compact(&heavy, &config));
    assert!(crate::compact::should_compact(&heavy, &config));
}

#[test]
fn a_zero_window_reports_no_pressure_rather_than_everything() {
    // A zero window is a configuration error. Reporting full pressure would
    // compact the whole conversation away.
    let config = CompactConfig::for_window(0);
    let messages = vec![Message::user("something")];
    assert_eq!(crate::compact::pressure(&messages, &config), 0.0);
    assert!(!crate::compact::should_compact(&messages, &config));
}
