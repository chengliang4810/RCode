//! rcode-agent — the native agent: a ReAct loop over `rcode-model`.
//!
//! The caller supplies the model client, tool set, conversation history and user
//! interaction callback. The loop is independent of the engine and UI;
//! `rcode-harness` supplies its production host. See `ARCHITECTURE.md` §5.
//!
//! One iteration asks the model, streams output, executes requested tools and
//! appends their results. Iteration continues until an answer, cancellation or
//! failure.

pub mod command;
pub mod builtin_subagent_prompts;
pub mod commands;
pub mod custom_commands;
pub mod compact;
pub mod hook;
pub mod memory;
pub mod message;
pub mod runner;
pub mod goal;
pub mod agent_tasks;
pub mod skills;
pub mod subagents;
pub mod tool;
pub mod tools;

pub use command::{
    ArgsSchema, CommandFeedback, CommandName, CommandOutcome, CommandRegistry, CommandRoute,
    ParsedInput,
};
pub use commands::BuiltinCommand;
pub use compact::{CompactConfig, CompactError, CompactOutcome};
pub use hook::{HookContext, HookEvent, HookOutcome, HookResult, HookRunner, NoHooks};
pub use message::{ContentBlock, ImageSource, Message, MessageContent, MessageId, MessageRole};
pub use runner::{AgentConfig, AgentRunner};
pub use tool::{Tool, ToolContext, ToolError, ToolRegistry, ToolResult};
pub use tools::register_core_tools;

use std::sync::Arc;

use futures::stream::BoxStream;

/// One step of the loop, as the host sees it.
///
/// This is intentionally close to the wire shape the host already consumes;
/// the harness adapter maps it onto `rcode_proto::AgentEvent`. Keeping the
/// mapping in one place is what lets the engine, the Loro doc and the UI stay
/// untouched by this rewrite.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// The run started; carries the model that answered for it.
    Started { model: String },
    /// A chunk of assistant text.
    TextDelta { text: String },
    /// A coordinator message delivered to a managed child at a step boundary.
    UserMessage { text: String },
    /// A chunk of the model's reasoning, when the provider exposes it.
    ReasoningDelta { text: String },
    /// The assistant turn is complete.
    MessageComplete { message_id: MessageId },
    /// The model asked for a tool.
    ToolCall {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// A tool finished.
    ///
    /// `diff` is the edit an edit-shaped tool produced (unified or
    /// ACP-style), carried for the UI's diff pane. The model never sees it —
    /// it reads `output` — so a tool that returns a diff must still describe
    /// the change in words. Dropping it here would leave the right pane blank
    /// for every file the agent edits.
    ToolResult {
        id: String,
        is_error: bool,
        output: Option<String>,
        diff: Option<rcode_proto::ToolDiff>,
    },
    /// The loop is waiting for the user to answer a question.
    InputRequested { id: String, request: InputRequest },
    /// The loop shrank the conversation to stay inside the context window.
    ///
    /// Surfaced because compaction changes what the model can see, and a user
    /// watching a long run deserves to know why their earlier messages stopped
    /// being referenced — a silent compaction looks like the model forgetting.
    Compacted {
        stage: CompactionStage,
        shortened: usize,
    },
    /// Latest context occupancy, reported after each model call.
    ///
    /// `tokens` is what the provider counted as input for the call — the
    /// conversation as it will be replayed next turn, which is what fills the
    /// window. `window` is the model's own limit when the host knows it; the
    /// loop does not, so it reports `None` and the host fills it in.
    ContextUsage { tokens: u64, window: Option<u64> },
    ModelCall {
        id: String,
        statistics: rcode_proto::ModelCallStatistics,
    },
    /// An event from a tool-driven nested run — the `Agent` tool's subagent —
    /// attributed to the tool call that spawned it. The loop itself never
    /// produces one; a tool forwards its child's events through the
    /// [`crate::tools::NestedEventSender`] on the [`crate::ToolContext`], and
    /// the loop tags them here so a host can route them to the child's own
    /// transcript instead of folding them into the parent's.
    Nested {
        parent_tool_use_id: String,
        event: Box<AgentEvent>,
    },
    /// The run finished.
    Done { reason: DoneReason },
}

/// How far a compaction got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionStage {
    /// Oversized tool output was truncated. Costs no model call.
    Micro,
    /// The older half was replaced by a summary. Costs a model call.
    Full,
    /// The conversation is too full to compact further — the recent turns
    /// alone exceed the window, so there is nothing safe to drop.
    Exhausted,
    /// A summarization call failed; the conversation continues as it was.
    Failed,
}

/// Something the loop needs a human to answer before it can continue.
///
/// Re-exported from `rcode-proto` rather than defined here: the engine, the
/// wire and the UI all describe the same interaction, and a second definition
/// is a second thing to keep in step.
pub use rcode_proto::InteractionRequest as InputRequest;

/// The user's answer to an [`InputRequest`], likewise from `rcode-proto`.
pub use rcode_proto::InteractionResponse as InputAnswer;

/// Why a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoneReason {
    /// The model answered without asking for another tool.
    Complete,
    /// The caller cancelled.
    Interrupted,
    /// A step failed; the message is user-facing.
    Failed(String),
}

/// What the loop needs from its host.
///
/// A trait rather than a struct so the harness can drive it without this crate
/// learning about RPC, GPUI or the session doc.
#[async_trait::async_trait]
pub trait AgentHost: Send + Sync {
    /// Ask the model. The host adapts `rcode-model` to whatever provider the
    /// conversation selected.
    async fn reason(
        &self,
        messages: &[Message],
        tools: &[Arc<dyn Tool>],
    ) -> Result<BoxStream<'static, Result<ReasoningChunk, AgentError>>, AgentError>;

    /// A separate, tool-free completion check, without the coding system prompt.
    async fn verify_goal(&self, messages: &[Message]) -> Result<BoxStream<'static, Result<ReasoningChunk, AgentError>>, AgentError> {
        self.reason(messages, &[]).await
    }

    /// Present a request to the user and await the answer. Resolves once they
    /// respond; cancellation is the caller's concern.
    async fn ask(&self, request: InputRequest) -> Result<InputAnswer, AgentError>;

    /// Summarize a transcript, for compaction.
    ///
    /// On the host because summarizing means calling a model, and this crate
    /// owns no model client. The loop needs it because compaction happens
    /// *inside* a run: the moment the context is full is the moment the loop
    /// has to act, and it cannot wait for the caller to notice.
    ///
    /// The default refuses, which disables automatic compaction for a host
    /// that has not implemented it. That is a working configuration — the
    /// loop then runs until the provider itself rejects the request — and it
    /// keeps the trait implementable by a test double that only needs
    /// `reason`.
    async fn summarize(&self, _transcript: String) -> Result<String, AgentError> {
        Err(AgentError::Model(
            "this host cannot summarize, so compaction is unavailable".to_owned(),
        ))
    }

    /// The context window of the model this run is talking to, in tokens.
    ///
    /// `None` means unknown, and the loop then never compacts on its own:
    /// guessing a window is worse than not acting, because a wrong guess
    /// either compacts far too early (throwing away context the model could
    /// have used) or far too late (the request fails at the provider).
    fn context_window(&self) -> Option<u32> {
        None
    }
}

/// Token counts one model call reported.
///
/// Kept separate from [`ReasoningChunk::Done`]'s message because the loop needs
/// the numbers for the context meter and compaction, and neither belongs to the
/// assistant text the doc persists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CallUsage {
    /// Everything the provider counted as input for this call. That is what
    /// occupies the window next turn: the replayed history plus this turn.
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
}

/// A streamed piece of one model call.
#[derive(Debug, Clone)]
pub enum ReasoningChunk {
    /// A tool-call delta is also model output, even though its body is buffered.
    Activity,
    Text(String),
    Reasoning(String),
    /// The model finished; carries the full assistant message it produced and,
    /// when the provider reported them, the call's token counts.
    Done {
        message: Message,
        usage: Option<CallUsage>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("model: {0}")]
    Model(String),
    #[error("tool: {0}")]
    Tool(String),
    #[error("cancelled")]
    Cancelled,
    #[error("host: {0}")]
    Host(String),
}
