//! Keeping a long conversation inside the model's context window.
//!
//! Two reductions, applied in order:
//!
//! - [`micro_compact`] shrinks oversized tool output in place. It touches no
//!   message boundaries, so it can run on every turn for free.
//! - [`full_compact`] replaces the older half of the conversation with a
//!   summary, keeping the recent turns verbatim. It needs an LLM call, so the
//!   caller injects that as a callback.
//!
//! The invariant both must preserve: **a tool result never outlives its tool
//! call.** Providers reject a `tool_result` with no matching `tool_use`, and a
//! summary that ate one but not the other is worse than a larger prompt.

use crate::message::{ContentBlock, Message, MessageContent, MessageRole};

/// When to compact, and how much to keep.
///
/// Two thresholds, matching the original project's, because the two reductions cost very
/// different things:
///
/// - **micro** truncates oversized tool output, needs no model call, and is
///   safe to run often — so it triggers early, at [`Self::micro_ratio`].
/// - **full** replaces the older half with a summary, costs a model call, and
///   is not reversible from the loop's side — so it triggers late, at
///   [`Self::full_ratio`], and only when micro did not free enough.
///
/// The gap between them is the point: most long conversations are long because
/// of tool output, and micro alone keeps them running without ever spending a
/// summarization call.
#[derive(Debug, Clone)]
pub struct CompactConfig {
    /// The model's context window, in tokens.
    pub context_window: u32,
    /// Run micro compaction once the estimate reaches this share of the window.
    pub micro_ratio: f32,
    /// Summarize once the estimate reaches this share. Below 1.0 so there is
    /// room for the reply, which is what would otherwise be the thing that
    /// fails.
    pub full_ratio: f32,
    /// Recent exchanges kept verbatim. A user prompt plus its first response,
    /// or a subsequent assistant/tool-result cycle, is one exchange.
    pub keep_recent_turns: usize,
    /// Characters of a single tool result kept by micro compaction.
    pub max_tool_output_chars: usize,
}

impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            context_window: 200_000,
            // The original project's defaults, kept as-is: they were tuned against real long
            // conversations, and a different pair would be a guess.
            micro_ratio: 0.75,
            full_ratio: 0.95,
            keep_recent_turns: 4,
            max_tool_output_chars: 4_000,
        }
    }
}

impl CompactConfig {
    /// The config for a model of this window size.
    pub fn for_window(context_window: u32) -> Self {
        Self {
            context_window,
            ..Self::default()
        }
    }

    /// Whether the estimate has reached a share of the window.
    fn reached(&self, estimate: u64, ratio: f32) -> bool {
        estimate >= (self.context_window as f32 * ratio) as u64
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CompactError {
    #[error("summarize: {0}")]
    Summarize(String),
    /// Compaction cannot help: the recent turns alone already exceed the
    /// window, so dropping the older ones would not make the request fit.
    #[error(
        "the last {turns} turns need ~{tokens} tokens, over the {window}-token window; \
         start a new conversation or trim the recent messages"
    )]
    RecentTurnsTooLarge {
        turns: usize,
        tokens: u64,
        window: u32,
    },
}

/// What a compaction did.
#[derive(Debug, Clone)]
pub struct CompactOutcome {
    pub messages: Vec<Message>,
    /// Whether anything actually changed.
    pub compacted: bool,
    /// Messages replaced by the summary.
    pub replaced: usize,
}

/// Rough token count.
///
/// Deliberately not tiktoken: the exact number is not needed, only a figure
/// that crosses the trigger threshold at about the right time. ASCII runs about
/// four characters per token; CJK is closer to one, so those characters are
/// counted whole.
pub fn estimate_tokens(messages: &[Message]) -> u64 {
    messages
        .iter()
        .map(|message| estimate_message_tokens(message))
        .sum()
}

fn estimate_message_tokens(message: &Message) -> u64 {
    // Per-message framing overhead (role, delimiters) is small but real, and it
    // matters when a conversation is thousands of short messages.
    const FRAMING: u64 = 4;
    FRAMING + estimate_content_tokens(&message.content)
}

fn estimate_content_tokens(content: &MessageContent) -> u64 {
    match content {
        MessageContent::Text(text) => estimate_text_tokens(text),
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } | ContentBlock::Reasoning { text, .. } => {
                    estimate_text_tokens(text)
                }
                ContentBlock::ToolUse { name, input, .. } => {
                    estimate_text_tokens(name) + estimate_text_tokens(&input.to_string())
                }
                ContentBlock::ToolResult { content, .. } => estimate_text_tokens(content),
                ContentBlock::Image { .. } => {
                    // Vision tokens are provider-specific; a flat figure keeps
                    // the estimate from wildly undercounting.
                    1_000
                }
            })
            .sum(),
    }
}

fn estimate_text_tokens(text: &str) -> u64 {
    let mut tokens = 0u64;
    let mut ascii_run = 0u64;
    for character in text.chars() {
        if character.is_ascii() {
            ascii_run += 1;
        } else {
            // Flush the pending ASCII run at four characters per token.
            tokens += ascii_run.div_ceil(4);
            ascii_run = 0;
            // Non-ASCII (CJK, emoji, accented Latin) is roughly one token each.
            tokens += 1;
        }
    }
    tokens + ascii_run.div_ceil(4)
}

/// Whether the conversation should be compacted.
/// How much of the window a conversation is using, as a share.
///
/// Returned instead of a bare `bool` so the caller can apply both thresholds
/// from one measurement, and so a log line can say *how* full it was rather
/// than only that a line was crossed.
pub fn pressure(messages: &[Message], config: &CompactConfig) -> f32 {
    if config.context_window == 0 {
        // A zero window is a configuration error, not a runaway conversation.
        // Reporting full pressure would compact everything; reporting zero
        // would never compact. Neither is right, so say "empty" and let the
        // caller's own limits stop the run.
        return 0.0;
    }
    estimate_tokens(messages) as f32 / config.context_window as f32
}

/// Whether a full compaction is due.
///
/// Reaching the window itself always counts, whatever the ratio says: at that
/// point the next request cannot fit, so the ratio is no longer the binding
/// constraint.
pub fn should_compact(messages: &[Message], config: &CompactConfig) -> bool {
    let now = pressure(messages, config);
    now >= config.full_ratio || now >= 1.0
}

/// Whether micro compaction is due: tool output is worth trimming, but the
/// conversation is not yet full enough to spend a summarization call on.
pub fn should_micro_compact(messages: &[Message], config: &CompactConfig) -> bool {
    pressure(messages, config) >= config.micro_ratio
}

/// Whether a conversation rendered as plain text is full enough to compact.
///
/// For a caller that holds the transcript as text rather than as messages —
/// the engine, which reads turns out of the document. It measures the same way
/// [`should_compact`] does, so the two agree on what "full" means.
pub fn should_compact_for_text(transcript: &str, config: &CompactConfig) -> bool {
    if config.context_window == 0 {
        return false;
    }
    let estimate = estimate_text_tokens(transcript) as f32 / config.context_window as f32;
    estimate >= config.full_ratio || estimate >= 1.0
}

/// Shrink oversized tool results in place. Returns how many were shortened.
///
/// Safe at any time: message roles and tool-call pairing are untouched, so the
/// conversation stays valid.
pub fn micro_compact(messages: &mut [Message], config: &CompactConfig) -> usize {
    let boundaries = exchange_boundaries(messages);
    let keep = config.keep_recent_turns.max(1);
    if boundaries.len() <= keep || config.max_tool_output_chars < 80 { return 0; }
    let stale_end = boundaries[boundaries.len() - keep];
    let mut shortened = 0;
    for message in &mut messages[..stale_end] {
        let MessageContent::Blocks(blocks) = &mut message.content else { continue; };
        for block in blocks {
            let ContentBlock::ToolResult { content, is_error: false, .. } = block else { continue; };
            let chars: Vec<char> = content.chars().collect();
            if chars.len() <= config.max_tool_output_chars { continue; }
            // Reserve the marker inside the budget so a second pass is a no-op.
            // Keep the tail too: errors/conclusions commonly appear at the end.
            let budget = config.max_tool_output_chars - 64;
            let head = budget * 3 / 4;
            let tail = budget - head;
            let dropped = chars.len() - budget;
            *content = format!("{}\n\n… ({dropped} more characters truncated)\n\n{}",
                chars[..head].iter().collect::<String>(),
                chars[chars.len() - tail..].iter().collect::<String>());
            shortened += 1;
        }
    }
    shortened
}

/// Replace the older part of the conversation with a summary.
///
/// `summarize` is given the text of the messages being dropped and returns a
/// summary. It is a callback because summarising means an LLM call, and this
/// crate does not own a model client — keeping it injectable is also what makes
/// the function testable without a network.
pub async fn full_compact<F, Fut>(
    messages: Vec<Message>,
    config: &CompactConfig,
    summarize: F,
) -> Result<CompactOutcome, CompactError>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, CompactError>>,
{
    let boundaries = exchange_boundaries(&messages);
    let keep = config.keep_recent_turns.max(1);
    if boundaries.len() <= keep {
        // Not enough turns to be worth summarising.
        return Ok(CompactOutcome {
            messages,
            compacted: false,
            replaced: 0,
        });
    }

    let keep_from = boundaries[boundaries.len() - keep];

    // Never split a tool call from its result: if the first kept message is a
    // tool result, walk back to its call. Walking *back* keeps the summary
    // shorter than dropping the pair forward would.
    let keep_from = match messages[keep_from..].first().map(|message| message.role) {
        Some(MessageRole::Tool) => {
            let mut start = keep_from;
            while start > 0 && matches!(messages[start - 1].role, MessageRole::Assistant) {
                start -= 1;
                if !messages[start].content.tool_calls().is_empty() {
                    break;
                }
            }
            start
        }
        _ => keep_from,
    };
    let (older, recent) = messages.split_at(keep_from);
    // When splitting inside one user task, keep its latest instruction verbatim
    // alongside the summary and the recent tool exchanges.
    let task = (!recent.iter().any(|message| message.role == MessageRole::User))
        .then(|| older.iter().rev().find(|message| message.role == MessageRole::User))
        .flatten();

    if older.is_empty() {
        return Ok(CompactOutcome {
            messages,
            compacted: false,
            replaced: 0,
        });
    }

    // If the kept tail alone already overflows, summarizing cannot fix it.
    let recent_tokens = estimate_tokens(recent) + task.map(|message| estimate_tokens(std::slice::from_ref(message))).unwrap_or(0);
    if recent_tokens >= config.context_window as u64 {
        return Err(CompactError::RecentTurnsTooLarge {
            turns: config.keep_recent_turns,
            tokens: recent_tokens,
            window: config.context_window,
        });
    }

    let transcript = render_for_summary(older);
    let summary = summarize(transcript).await?;

    let mut compacted = Vec::with_capacity(recent.len() + 1);
    compacted.push(Message {
        id: crate::message::MessageId::new(),
        role: MessageRole::User,
        content: MessageContent::text(format!(
            "[Summary of the earlier conversation, which has been dropped to fit the \
             context window]\n\n{summary}"
        )),
    });
    if let Some(task) = task { compacted.push(task.clone()); }
    compacted.extend_from_slice(recent);

    Ok(CompactOutcome {
        messages: compacted,
        compacted: true,
        replaced: older.len(),
    })
}

/// Safe cuts between exchanges. Never split an assistant's tool calls from
/// their results, even if an internal user message is interleaved.
pub fn exchange_boundaries(messages: &[Message]) -> Vec<usize> {
    let mut boundaries = Vec::new();
    let mut pending = std::collections::HashSet::new();
    let mut previous = None;
    for (index, message) in messages.iter().enumerate() {
        if pending.is_empty() && (message.role == MessageRole::User
            || (message.role == MessageRole::Assistant && previous != Some(MessageRole::User))) {
            boundaries.push(index);
        }
        for (id, _, _) in message.content.tool_calls() { pending.insert(id); }
        if let MessageContent::Blocks(blocks) = &message.content {
            for block in blocks {
                if let ContentBlock::ToolResult { tool_use_id, .. } = block { pending.remove(tool_use_id); }
            }
        }
        if message.role != MessageRole::System { previous = Some(message.role); }
    }
    boundaries
}

/// The index of each user message: the start of a turn.
pub fn turn_boundaries(messages: &[Message]) -> Vec<usize> {
    messages
        .iter()
        .enumerate()
        .filter(|(_, message)| matches!(message.role, MessageRole::User))
        .map(|(index, _)| index)
        .collect()
}

/// Flatten messages into the text a summariser reads.
///
/// Public because the loop renders the same shape when it compacts mid-run,
/// and two spellings of "what the summarizer sees" would drift.
pub fn render_transcript(messages: &[Message]) -> String {
    render_for_summary(messages)
}

/// The turn a summary is delivered as.
///
/// Framed as a user turn, because it is context being handed *to* the model
/// rather than something the model said. Labelled so a reader (and the model)
/// can tell it apart from a real message the user typed.
pub fn summary_message(summary: &str) -> String {
    format!(
        "[Compacted] The earlier part of this conversation was replaced by this \n         summary:

{summary}"
    )
}

fn render_for_summary(messages: &[Message]) -> String {
    let mut out = String::new();
    for message in messages {
        let role = match message.role {
            MessageRole::User => "User",
            MessageRole::Assistant => "Assistant",
            MessageRole::System => "System",
            MessageRole::Tool => "Tool result",
        };
        use std::fmt::Write as _;
        let _ = writeln!(out, "## {role}");
        match &message.content {
            MessageContent::Text(text) => {
                let _ = writeln!(out, "{text}");
            }
            MessageContent::Blocks(blocks) => {
                for block in blocks {
                    match block {
                        ContentBlock::Text { text } | ContentBlock::Reasoning { text, .. } => {
                            let _ = writeln!(out, "{text}");
                        }
                        ContentBlock::ToolUse { name, input, .. } => {
                            let _ = writeln!(out, "[tool call: {name} {input}]");
                        }
                        ContentBlock::ToolResult { content, .. } => {
                            let _ = writeln!(out, "[tool result: {content}]");
                        }
                        ContentBlock::Image { .. } => {
                            let _ = writeln!(out, "[image]");
                        }
                    }
                }
            }
        }
        out.push('\n');
    }
    out
}

/// The prompt a summariser should be given, for hosts that need one.
pub const SUMMARY_INSTRUCTION: &str = "Summarise the conversation above so work can continue \
     without it. Keep, in this order: the user's goal and any constraints they stated; what has \
     been done so far and what it changed; the files that matter and why; anything that failed \
     and how; and what remains. Be specific — name files, functions and commands. Omit \
     pleasantries and restatements of the code you saw.";

#[cfg(test)]
#[path = "compact_regression_tests.rs"]
mod regression_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MessageId;

    fn user(text: &str) -> Message {
        Message {
            id: MessageId::new(),
            role: MessageRole::User,
            content: MessageContent::text(text),
        }
    }

    fn assistant(text: &str) -> Message {
        Message {
            id: MessageId::new(),
            role: MessageRole::Assistant,
            content: MessageContent::text(text),
        }
    }

    fn tool_call(id: &str) -> Message {
        Message {
            id: MessageId::new(),
            role: MessageRole::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: id.to_owned(),
                name: "Read".to_owned(),
                input: serde_json::json!({ "file_path": "a.rs" }),
            }]),
        }
    }

    fn tool_result(id: &str, content: &str) -> Message {
        Message {
            id: MessageId::new(),
            role: MessageRole::Tool,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: id.to_owned(),
                content: content.to_owned(),
                is_error: false,
            }]),
        }
    }

    fn config(keep: usize) -> CompactConfig {
        CompactConfig {
            context_window: 100_000,
            micro_ratio: 0.75,
            full_ratio: 0.95,
            keep_recent_turns: keep,
            max_tool_output_chars: 100,
        }
    }

    #[test]
    fn ascii_text_estimates_at_about_four_characters_per_token() {
        let text = "a".repeat(400);
        let tokens = estimate_text_tokens(&text);
        assert_eq!(tokens, 100);
    }

    #[test]
    fn cjk_text_estimates_at_about_one_token_per_character() {
        let tokens = estimate_text_tokens("日本語テキスト");
        assert_eq!(tokens, 7);
    }

    #[test]
    fn a_mixed_string_counts_both_categories() {
        // 4 ASCII characters (1 token) + 2 CJK characters (2 tokens).
        let tokens = estimate_text_tokens("abcd日本");
        assert_eq!(tokens, 3);
    }

    #[test]
    fn an_empty_conversation_estimates_to_zero() {
        assert_eq!(estimate_tokens(&[]), 0);
    }

    #[test]
    fn should_compact_trips_at_the_threshold() {
        let mut messages = vec![user("hi")];
        assert!(!should_compact(&messages, &config(4)));

        // Enough text to cross 80% of a small window.
        messages.push(user(&"x".repeat(400_000)));
        assert!(should_compact(&messages, &config(4)));
    }

    #[test]
    fn micro_compact_shrinks_only_oversized_results() {
        let mut messages = vec![
            user("read"),
            tool_call("a"),
            tool_result("a", &"x".repeat(500)),
            user("next"),
            tool_result("b", "short"),
        ];
        let shortened = micro_compact(&mut messages, &config(1));

        assert_eq!(shortened, 1);
        let MessageContent::Blocks(blocks) = &messages[2].content else {
            panic!("expected blocks");
        };
        let ContentBlock::ToolResult { content, .. } = &blocks[0] else {
            panic!("expected a tool result");
        };
        assert!(content.contains("more characters truncated"));
        assert!(content.chars().count() <= 100);
        assert!(content.ends_with('x'));
    }

    #[test]
    fn micro_compact_leaves_small_results_alone() {
        let mut messages = vec![tool_result("a", "short")];
        assert_eq!(micro_compact(&mut messages, &config(1)), 0);
    }

    #[tokio::test]
    async fn full_compact_keeps_the_recent_turns_and_summarises_the_rest() {
        let messages = vec![
            user("first question"),
            assistant("first answer"),
            user("second question"),
            assistant("second answer"),
            user("third question"),
            assistant("third answer"),
        ];

        let outcome = full_compact(messages, &config(2), |transcript| async move {
            assert!(transcript.contains("first question"));
            assert!(!transcript.contains("third question"));
            Ok("the earlier work was about something".to_owned())
        })
        .await
        .expect("compacts");

        assert!(outcome.compacted);
        assert_eq!(outcome.replaced, 2);
        // Summary + the last two turns (second and third question).
        assert_eq!(outcome.messages.len(), 5);
        assert!(
            outcome.messages[0]
                .content
                .text_content()
                .contains("Summary of the earlier conversation")
        );
        assert!(
            outcome.messages[1]
                .content
                .text_content()
                .contains("second question")
        );
        assert!(
            outcome.messages[3]
                .content
                .text_content()
                .contains("third question")
        );
    }

    #[tokio::test]
    async fn full_compact_does_nothing_when_there_is_little_to_drop() {
        let messages = vec![user("a"), assistant("b")];
        let outcome = full_compact(messages, &config(4), |_| async {
            panic!("must not summarise");
        })
        .await
        .expect("no-op");

        assert!(!outcome.compacted);
    }

    #[tokio::test]
    async fn a_summary_never_strands_a_tool_result() {
        // The keep window starts *inside* an assistant tool call: the call and
        // its result must both survive, not be split.
        let messages = vec![
            user("early"),
            assistant("early answer"),
            user("do a thing"),
            tool_call("call-1"),
            tool_result("call-1", "file contents"),
            assistant("done"),
        ];

        let outcome = full_compact(messages, &config(1), |_| async { Ok("summary".to_owned()) })
            .await
            .expect("compacts");

        // Every surviving tool result has its call.
        let kept = &outcome.messages;
        for (index, message) in kept.iter().enumerate() {
            if message.role != MessageRole::Tool {
                continue;
            }
            let calls: Vec<String> = kept[..index]
                .iter()
                .flat_map(|earlier| {
                    earlier
                        .content
                        .tool_calls()
                        .into_iter()
                        .map(|(id, _, _)| id)
                        .collect::<Vec<_>>()
                })
                .collect();
            let ids: Vec<String> = message
                .content
                .tool_calls()
                .into_iter()
                .map(|(id, _, _)| id)
                .collect();
            let _ = ids;
            if let MessageContent::Blocks(blocks) = &message.content {
                for block in blocks {
                    if let ContentBlock::ToolResult { tool_use_id, .. } = block {
                        assert!(
                            calls.contains(tool_use_id),
                            "tool result {tool_use_id} outlived its call"
                        );
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn recent_turns_that_do_not_fit_are_an_error() {
        let huge = "x".repeat(500_000);
        let messages = vec![user("early"), assistant("early"), user(&huge)];

        let config = CompactConfig {
            context_window: 1_000,
            ..config(1)
        };
        let error = full_compact(messages, &config, |_| async { Ok("summary".to_owned()) })
            .await
            .expect_err("does not fit");

        assert!(matches!(error, CompactError::RecentTurnsTooLarge { .. }));
    }

    #[tokio::test]
    async fn a_failing_summariser_propagates() {
        let messages = vec![user("a"), assistant("b"), user("c"), assistant("d")];
        let error = full_compact(messages, &config(1), |_| async {
            Err(CompactError::Summarize("model unavailable".to_owned()))
        })
        .await
        .expect_err("propagates");

        assert!(error.to_string().contains("model unavailable"));
    }

    #[test]
    fn the_summary_prompt_asks_for_the_things_that_matter() {
        for expected in ["goal", "files", "failed", "remains"] {
            assert!(
                SUMMARY_INSTRUCTION.contains(expected),
                "the instruction should mention {expected}"
            );
        }
    }
}
