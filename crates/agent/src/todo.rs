//! Successful TodoWrite calls and internal checkpoints are the planning state.
//! User text and summaries never supply authoritative checklist state.
use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context::{ContextRecord, Domain, Operation, Status};
use crate::{ContentBlock, Message, MessageRole, ToolError};

pub const REMINDER_INTERVAL: usize = 10;
pub const MAX_ITEMS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub text: String,
    pub status: TodoStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionKind {
    Blocked,
    Cancelled,
    ScopeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub kind: ResolutionKind,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    pub todos: Vec<Item>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<Resolution>,
}

impl Update {
    pub fn parse(input: Value) -> Result<Self, ToolError> {
        let update: Self = serde_json::from_value(input).map_err(|error| {
            ToolError::InvalidInput(format!("Invalid TodoWrite input: {error}"))
        })?;
        if update.todos.len() > MAX_ITEMS {
            return Err(ToolError::InvalidInput(format!(
                "TodoWrite accepts at most {MAX_ITEMS} items"
            )));
        }
        if update.todos.iter().any(|item| item.text.trim().is_empty()) {
            return Err(ToolError::InvalidInput(
                "every item needs a non-empty `text`".into(),
            ));
        }
        if update
            .resolution
            .as_ref()
            .is_some_and(|r| r.reason.trim().is_empty())
        {
            return Err(ToolError::InvalidInput(
                "a todo resolution needs a non-empty reason".into(),
            ));
        }
        Ok(update)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub current: Option<Update>,
    pub turns_since_write: usize,
    pub turns_since_reminder: usize,
}

impl State {
    pub fn from_history(messages: &[Message]) -> Self {
        let mut state = Self::default();
        let mut calls = HashMap::new();
        let mut next_call = 0usize;
        let mut latest_success = 0usize;
        for message in messages {
            if let Some(record) = message
                .context
                .as_ref()
                .filter(|r| r.domain == Domain::Todo)
            {
                if record.version == 1 {
                    if let Some(restored) = record
                        .upsert
                        .get("state")
                        .and_then(|json| serde_json::from_str::<Self>(json).ok())
                    {
                        state = restored;
                    }
                }
                continue;
            }
            if message.role == MessageRole::Assistant {
                state.turns_since_write = state.turns_since_write.saturating_add(1);
                state.turns_since_reminder = state.turns_since_reminder.saturating_add(1);
            }
            if message.is_user_input() {
                // 新请求可重新推进先前阻塞的工作；内部提醒不解除也不伪造这次用户意图。
                if let Some(update) = &mut state.current {
                    update.resolution = None;
                }
            }
            let crate::MessageContent::Blocks(blocks) = &message.content else {
                continue;
            };
            for block in blocks {
                if message.role == MessageRole::Assistant {
                    if let ContentBlock::ToolUse { id, name, input } = block {
                        if name == "TodoWrite" {
                            next_call = next_call.saturating_add(1);
                            calls.insert(id.as_str(), (next_call, input));
                        }
                    }
                }
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    is_error,
                    ..
                } = block
                    && message.role == MessageRole::Tool
                {
                    if let Some((order, input)) = calls.remove(tool_use_id.as_str()) {
                        // 并发工具可能乱序返回；较早调用的晚到结果不能覆盖模型最后一次成功更新。
                        if !*is_error && order > latest_success {
                            if let Ok(update) = Update::parse(input.clone()) {
                                state.current = Some(update);
                                state.turns_since_write = 0;
                                latest_success = order;
                            }
                        }
                    }
                }
            }
        }
        state
    }

    pub fn unfinished(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|u| u.todos.iter().any(|i| i.status != TodoStatus::Completed))
    }

    pub fn resolution(&self) -> Option<&Resolution> {
        self.current
            .as_ref()
            .and_then(|u| u.resolution.as_ref())
            .filter(|_| self.unfinished())
    }

    pub fn reminder_due(&self) -> bool {
        self.unfinished()
            && self.resolution().is_none()
            && self.turns_since_write >= REMINDER_INTERVAL
            && self.turns_since_reminder >= REMINDER_INTERVAL
    }

    pub fn render(&self) -> String {
        serde_json::to_string_pretty(&self.current).expect("todo state serializes")
    }

    pub fn record(&self, kind: &str) -> Message {
        let mut snapshot = self.clone();
        if kind == "reminder" || kind == "closure" {
            snapshot.turns_since_reminder = 0;
        }
        let instruction = match kind {
            "reminder" => {
                "The todo list has not been updated recently. Keep it aligned with your actual progress: mark finished items completed and the current item in_progress. Use TodoWrite only when relevant. Do not mention this internal reminder to the user."
            }
            "closure" => {
                "You stopped with unfinished todos. Continue the work and update TodoWrite before concluding. If genuinely blocked, cancelled, or out of scope, keep unfinished statuses and call TodoWrite with resolution={kind:blocked|cancelled|scope_changed,reason:<specific reason>}, then explain the remaining work to the user. Never mark unfinished work completed just to end the turn."
            }
            _ => {
                "Current todo state retained across compaction. This supersedes older todo lists; later successful TodoWrite calls replace it."
            }
        };
        let body = format!("{instruction}\nCurrent todo list:\n{}", self.render())
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        let mut message = Message::user(format!("<system-reminder>\n{body}\n</system-reminder>"));
        message.context = Some(ContextRecord {
            version: 1,
            domain: Domain::Todo,
            scope: "session".into(),
            revision: 1,
            base_revision: None,
            operation: Operation::Replace,
            status: Status::Ready,
            upsert: BTreeMap::from([
                (
                    "state".into(),
                    serde_json::to_string(&snapshot).expect("todo state serializes"),
                ),
                ("kind".into(), kind.into()),
            ]),
            remove: Vec::new(),
        });
        message
    }
}

#[cfg(test)]
mod tests;
