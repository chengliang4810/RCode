//! In-memory message model for the ReAct loop.
//!
//! Deliberately small. A [`Message`] is what the loop appends to its transcript
//! and hands to the model. The visible conversation lives in the Loro session
//! doc (`crates/doc`); full execution messages also round-trip through local
//! model-history records. New optional metadata must deserialize older records.

use std::fmt;

use serde::{Deserialize, Serialize};

// ─── MessageId ────────────────────────────────────────────────────────────────

/// Opaque identity for one message.
///
/// Newtype over a UUID string: the loop only ever compares, hashes and prints
/// these, and the host round-trips them through the session doc as text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MessageId(String);

impl MessageId {
    /// Mint a fresh random id.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for MessageId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for MessageId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

// ─── MessageRole ──────────────────────────────────────────────────────────────

/// Who produced a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

impl MessageRole {
    /// The lowercase name used on the wire, for provider conversion.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
            Self::Tool => "tool",
        }
    }
}

// ─── ImageSource ──────────────────────────────────────────────────────────────

/// Where an image's bytes come from.
///
/// The original project keeps `DocumentSource` and a provider-native `Unknown` block alongside
/// this; both are dropped here — documents are not a RCode input yet, and an
/// unrecognised block is not something the loop needs to preserve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    /// Inline base64 data (`image/png`, `image/jpeg`, ...).
    Base64 { media_type: String, data: String },
    /// A remote URL the provider fetches itself.
    Url { url: String },
}

// ─── ContentBlock ─────────────────────────────────────────────────────────────

/// One structured piece of a message.
///
/// The five variants the loop actually produces and consumes. The original project's
/// `Document`, `Unknown` and per-block `signature`/`id` extras are dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text.
    Text { text: String },

    /// An image, for multimodal input.
    Image { source: ImageSource },

    /// A tool call the assistant requested.
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },

    /// The result of one [`ContentBlock::ToolUse`].
    ToolResult {
        /// The `id` of the `ToolUse` this answers.
        tool_use_id: String,
        /// The result as text.
        content: String,
        /// Whether the tool failed.
        is_error: bool,
    },

    /// The model's reasoning, when the provider exposes it.
    ///
    /// The signature is what the provider returned with the reasoning;
    /// extended-thinking APIs require it verbatim when the block is replayed
    /// in a later turn, so it rides with the text rather than being
    /// re-derived.
    Reasoning {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
}

impl ContentBlock {
    /// A text block.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// A tool call block.
    pub fn tool_use(
        id: impl Into<String>,
        name: impl Into<String>,
        input: serde_json::Value,
    ) -> Self {
        Self::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }
    }

    /// A tool result block.
    pub fn tool_result(
        tool_use_id: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self::ToolResult {
            tool_use_id: tool_use_id.into(),
            content: content.into(),
            is_error,
        }
    }

    /// A reasoning block.
    pub fn reasoning(text: impl Into<String>) -> Self {
        Self::Reasoning {
            text: text.into(),
            signature: None,
        }
    }

    /// An image block from a base64 payload.
    pub fn image_base64(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self::Image {
            source: ImageSource::Base64 {
                media_type: media_type.into(),
                data: data.into(),
            },
        }
    }

    /// An image block from a URL.
    pub fn image_url(url: impl Into<String>) -> Self {
        Self::Image {
            source: ImageSource::Url { url: url.into() },
        }
    }

    /// The text, when this is a [`ContentBlock::Text`].
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            _ => None,
        }
    }

    /// `(id, name, input)`, when this is a [`ContentBlock::ToolUse`].
    pub fn as_tool_use(&self) -> Option<(&str, &str, &serde_json::Value)> {
        match self {
            Self::ToolUse { id, name, input } => Some((id, name, input)),
            _ => None,
        }
    }

    /// The text, when this is a [`ContentBlock::Reasoning`].
    pub fn as_reasoning(&self) -> Option<&str> {
        match self {
            Self::Reasoning { text, .. } => Some(text),
            _ => None,
        }
    }
}

// ─── MessageContent ───────────────────────────────────────────────────────────

/// What a message carries.
///
/// Two shapes, not the original project's three: a plain string for the common case, or blocks
/// for anything structured. The original project's `Raw(Vec<serde_json::Value>)` pass-through
/// variant is gone — the loop normalises provider output into blocks, so a
/// second untyped representation would only be a place for drift.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Plain text.
    Text(String),
    /// Structured blocks: multimodal input, tool calls, reasoning.
    Blocks(Vec<ContentBlock>),
}

impl MessageContent {
    /// Plain text content.
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }

    /// Content from structured blocks.
    pub fn blocks(blocks: Vec<ContentBlock>) -> Self {
        Self::Blocks(blocks)
    }

    /// Every text block joined into one string.
    pub fn text_content(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Blocks(blocks) => blocks
                .iter()
                .filter_map(ContentBlock::as_text)
                .collect::<Vec<_>>()
                .join(""),
        }
    }

    /// The content as blocks, cloning the structured case.
    pub fn content_blocks(&self) -> Vec<ContentBlock> {
        match self {
            Self::Text(s) if s.is_empty() => Vec::new(),
            Self::Text(s) => vec![ContentBlock::text(s.clone())],
            Self::Blocks(blocks) => blocks.clone(),
        }
    }

    /// Every tool call as an owned `(id, name, input)` triple.
    pub fn tool_calls(&self) -> Vec<(String, String, serde_json::Value)> {
        match self {
            Self::Text(_) => Vec::new(),
            Self::Blocks(blocks) => blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::ToolUse { id, name, input } => {
                        Some((id.clone(), name.clone(), input.clone()))
                    }
                    _ => None,
                })
                .collect(),
        }
    }

    /// Whether this carries no content.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Text(s) => s.is_empty(),
            Self::Blocks(blocks) => blocks.is_empty(),
        }
    }
}

impl Default for MessageContent {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

impl From<String> for MessageContent {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for MessageContent {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<Vec<ContentBlock>> for MessageContent {
    fn from(blocks: Vec<ContentBlock>) -> Self {
        Self::Blocks(blocks)
    }
}

// ─── Message ──────────────────────────────────────────────────────────────────

/// One turn in the transcript.
///
/// Fields are public: the loop builds and rewrites these in place, and a
/// separate builders for each field would just be indirection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub role: MessageRole,
    pub content: MessageContent,
    /// Host context, separate from user text and from the visible transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<crate::context::ContextRecord>,
}

impl Message {
    /// A message with a fresh id.
    fn new(role: MessageRole, content: impl Into<MessageContent>) -> Self {
        Self {
            id: MessageId::new(),
            role,
            content: content.into(),
            context: None,
        }
    }

    /// A user turn.
    pub fn user(content: impl Into<MessageContent>) -> Self {
        Self::new(MessageRole::User, content)
    }

    pub fn is_user_input(&self) -> bool {
        self.role == MessageRole::User && self.context.is_none()
    }

    /// An assistant turn.
    pub fn assistant(content: impl Into<MessageContent>) -> Self {
        Self::new(MessageRole::Assistant, content)
    }

    /// A system prompt.
    pub fn system(content: impl Into<MessageContent>) -> Self {
        Self::new(MessageRole::System, content)
    }

    /// A tool turn carrying exactly one result.
    pub fn tool_result(
        tool_use_id: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self::tool_results(vec![ContentBlock::tool_result(
            tool_use_id,
            content,
            is_error,
        )])
    }

    /// A tool turn carrying the results of one assistant turn's tool calls.
    ///
    /// A provider may ask for several tools at once, and all their results go
    /// back as a single message — one message per call would not round-trip.
    pub fn tool_results(blocks: Vec<ContentBlock>) -> Self {
        Self::new(MessageRole::Tool, blocks)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn message_id_is_unique_and_prints_itself() {
        let a = MessageId::new();
        let b = MessageId::new();
        assert_ne!(a, b);
        assert_eq!(a.to_string(), a.as_str());
        assert_eq!(
            MessageId::from("fixed"),
            MessageId::from("fixed".to_string())
        );
        assert_eq!(MessageId::from("fixed").as_str(), "fixed");
    }

    #[test]
    fn constructors_set_the_role_and_content() {
        let user = Message::user("hello");
        assert_eq!(user.role, MessageRole::User);
        assert_eq!(user.content, MessageContent::Text("hello".into()));

        let assistant = Message::assistant(MessageContent::text("hi"));
        assert_eq!(assistant.role, MessageRole::Assistant);

        let system = Message::system("rules");
        assert_eq!(system.role, MessageRole::System);
        assert_eq!(system.content.text_content(), "rules");

        // Every message gets its own id.
        assert_ne!(user.id, assistant.id);
    }

    #[test]
    fn tool_result_builders_produce_tool_role() {
        let one = Message::tool_result("call_1", "ok", false);
        assert_eq!(one.role, MessageRole::Tool);
        assert_eq!(
            one.content,
            MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".into(),
                content: "ok".into(),
                is_error: false,
            }])
        );

        let many = Message::tool_results(vec![
            ContentBlock::tool_result("call_1", "ok", false),
            ContentBlock::tool_result("call_2", "boom", true),
        ]);
        assert_eq!(many.role, MessageRole::Tool);
        let MessageContent::Blocks(blocks) = &many.content else {
            panic!("expected blocks");
        };
        assert_eq!(blocks.len(), 2);
        assert!(matches!(
            &blocks[1],
            ContentBlock::ToolResult { tool_use_id, is_error: true, .. } if tool_use_id == "call_2"
        ));
    }

    #[test]
    fn text_content_joins_text_blocks_and_skips_the_rest() {
        assert_eq!(MessageContent::text("abc").text_content(), "abc");

        let blocks = MessageContent::blocks(vec![
            ContentBlock::text("foo"),
            ContentBlock::reasoning("ignored"),
            ContentBlock::text("bar"),
        ]);
        assert_eq!(blocks.text_content(), "foobar");
    }

    #[test]
    fn tool_calls_extracts_owned_triples() {
        let content = MessageContent::blocks(vec![
            ContentBlock::text("let me look"),
            ContentBlock::tool_use("call_1", "Read", json!({ "path": "src/main.rs" })),
            ContentBlock::reasoning("thinking"),
            ContentBlock::tool_use("call_2", "Bash", json!({ "command": "ls" })),
        ]);

        let calls = content.tool_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "call_1");
        assert_eq!(calls[0].1, "Read");
        assert_eq!(calls[0].2, json!({ "path": "src/main.rs" }));
        assert_eq!(calls[1].1, "Bash");

        assert!(
            MessageContent::text("no tools here")
                .tool_calls()
                .is_empty()
        );
    }

    #[test]
    fn content_blocks_and_emptiness() {
        assert_eq!(MessageContent::text("").content_blocks(), Vec::new());
        assert_eq!(
            MessageContent::text("x").content_blocks(),
            vec![ContentBlock::text("x")]
        );
        assert!(MessageContent::text("").is_empty());
        assert!(MessageContent::blocks(Vec::new()).is_empty());
        assert!(!MessageContent::text("x").is_empty());
    }

    #[test]
    fn content_block_accessors() {
        assert_eq!(ContentBlock::text("t").as_text(), Some("t"));
        assert_eq!(ContentBlock::reasoning("r").as_reasoning(), Some("r"));

        let call = ContentBlock::tool_use("id", "Read", json!({ "a": 1 }));
        let (id, name, input) = call.as_tool_use().expect("tool_use");
        assert_eq!((id, name), ("id", "Read"));
        assert_eq!(input, &json!({ "a": 1 }));
        assert!(call.as_text().is_none());
    }

    #[test]
    fn messages_round_trip_through_json() {
        let original = Message::assistant(MessageContent::blocks(vec![
            ContentBlock::text("reading it"),
            ContentBlock::tool_use("call_1", "Read", json!({ "path": "a.txt" })),
            ContentBlock::image_base64("image/png", "AAAA"),
        ]));

        let json = serde_json::to_string(&original).expect("serialize");
        let back: Message = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, original);
    }

    #[test]
    fn message_ids_and_text_content_round_trip_as_plain_values() {
        let id: MessageId = serde_json::from_str("\"abc\"").expect("id");
        assert_eq!(id.as_str(), "abc");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"abc\"");

        let content: MessageContent = serde_json::from_str("\"hi\"").expect("content");
        assert_eq!(content, MessageContent::Text("hi".into()));
    }

    #[test]
    fn role_wire_names() {
        assert_eq!(MessageRole::User.as_str(), "user");
        assert_eq!(MessageRole::Assistant.as_str(), "assistant");
        assert_eq!(MessageRole::System.as_str(), "system");
        assert_eq!(MessageRole::Tool.as_str(), "tool");
        assert_eq!(
            serde_json::to_string(&MessageRole::Tool).unwrap(),
            "\"tool\""
        );
    }
}
