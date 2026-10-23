//! `TodoWrite` — the agent's own task list.
//!
//! Successful calls in execution history own the list. Internal checkpoints
//! retain that state across compaction; the tool owns no separate mutable list.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolContext, ToolError, ToolResult};

/// Tasks accepted in one call. A longer list is a sign the model is not
/// actually tracking work, just dumping it.
use crate::todo::{TodoStatus, Update};

pub struct TodoWriteTool;

#[async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "TodoWrite"
    }

    fn description(&self) -> &str {
        "Record or update your task list. Send the whole list every time — it \
         replaces the previous one, so include items you have not finished. \
         Use it for work with several steps; skip it for a single action. Update \
         status as work progresses and immediately after completion. Before ending, \
         complete the remaining work, or preserve unfinished statuses and provide \
         resolution={kind:blocked|cancelled|scope_changed,reason:<specific reason>}. \
         Explain that resolution to the user. Never mark unfinished work completed."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "resolution": {
                    "type": "object",
                    "description": "Explicitly explain why remaining work cannot continue. Applies to this full list only; omit on a later update to resume normal completion checks. Does not complete a session goal.",
                    "properties": {
                        "kind": {"type":"string", "enum":["blocked","cancelled","scope_changed"]},
                        "reason": {"type":"string", "minLength":1}
                    },
                    "required": ["kind", "reason"],
                    "additionalProperties": false
                },
                "todos": {
                    "type": "array",
                    "description": "The complete list, in the order you intend to work.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "text": {
                                "type": "string",
                                "description": "What to do, in the imperative."
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"],
                                "description": "Where this item stands. Exactly one item should be in_progress."
                            }
                        },
                        "required": ["text", "status"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let update = Update::parse(input)?;
        let items = &update.todos;

        let mut rendered = String::new();
        let mut counts = (0usize, 0usize, 0usize);
        for item in items {
            let text = item.text.trim();
            let (mark, slot) = match item.status {
                TodoStatus::Completed => ("[x]", 0),
                TodoStatus::InProgress => ("[~]", 1),
                TodoStatus::Pending => ("[ ]", 2),
            };
            match slot {
                0 => counts.0 += 1,
                1 => counts.1 += 1,
                _ => counts.2 += 1,
            }
            use std::fmt::Write as _;
            let _ = writeln!(rendered, "{mark} {text}");
        }

        if items.is_empty() {
            return Ok(ToolResult::ok("The task list is now empty."));
        }
        if counts.1 > 1 {
            // Not an error — the model may legitimately be mid-replan — but it
            // is the one shape that makes the list useless, so say so.
            rendered.push_str("\n(note: more than one item is in_progress)\n");
        }
        if let Some(resolution) = &update.resolution {
            use std::fmt::Write as _;
            let _ = writeln!(rendered, "\nUnfinished work ({:?}): {}. Preserve its status and explain this to the user; it does not complete a session goal.", resolution.kind, resolution.reason);
        }
        Ok(ToolResult::ok(format!(
            "{} done, {} in progress, {} pending:\n{rendered}",
            counts.0, counts.1, counts.2
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::todo::MAX_ITEMS;
    use tokio_util::sync::CancellationToken;

    fn context() -> ToolContext {
        ToolContext {
            cwd: std::env::temp_dir(),
            cancel: CancellationToken::new(),
            nested: None,
        }
    }

    #[tokio::test]
    async fn a_list_renders_with_marks_and_counts() {
        let result = TodoWriteTool
            .call(
                json!({ "todos": [
                    { "text": "read the spec", "status": "completed" },
                    { "text": "write the parser", "status": "in_progress" },
                    { "text": "add tests", "status": "pending" }
                ]}),
                &context(),
            )
            .await
            .expect("renders");

        assert!(result.output.contains("1 done, 1 in progress, 1 pending"));
        assert!(result.output.contains("[x] read the spec"));
        assert!(result.output.contains("[~] write the parser"));
        assert!(result.output.contains("[ ] add tests"));
    }

    #[tokio::test]
    async fn an_empty_list_clears_it() {
        let result = TodoWriteTool
            .call(json!({ "todos": [] }), &context())
            .await
            .expect("clears");
        assert!(result.output.contains("empty"));
    }

    #[tokio::test]
    async fn an_unknown_status_is_refused() {
        let error = TodoWriteTool
            .call(
                json!({ "todos": [{ "text": "x", "status": "nearly" }] }),
                &context(),
            )
            .await
            .expect_err("bad status");
        assert!(error.to_string().contains("nearly"));
    }

    #[tokio::test]
    async fn a_missing_text_is_refused() {
        let error = TodoWriteTool
            .call(json!({ "todos": [{ "status": "pending" }] }), &context())
            .await
            .expect_err("no text");
        assert!(error.to_string().contains("text"));
    }

    #[tokio::test]
    async fn too_many_items_are_refused() {
        let items: Vec<Value> = (0..MAX_ITEMS + 1)
            .map(|index| json!({ "text": format!("task {index}"), "status": "pending" }))
            .collect();
        let error = TodoWriteTool
            .call(json!({ "todos": items }), &context())
            .await
            .expect_err("too many");
        assert!(error.to_string().contains("at most"));
    }

    #[tokio::test]
    async fn more_than_one_in_progress_is_called_out() {
        let result = TodoWriteTool
            .call(
                json!({ "todos": [
                    { "text": "a", "status": "in_progress" },
                    { "text": "b", "status": "in_progress" }
                ]}),
                &context(),
            )
            .await
            .expect("renders");
        assert!(result.output.contains("more than one item is in_progress"));
    }
}
