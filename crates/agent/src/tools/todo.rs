//! `TodoWrite` — the agent's own task list.
//!
//! The list is **stateless on this side**: the tool takes the whole list and
//! returns a rendered view of it, and the agent carries the state in the
//! conversation. That is deliberate rather than a shortcut — a list owned by
//! the tool would need somewhere to live (a file? the session doc?), and every
//! one of those choices makes the list outlive the task that created it. The
//! conversation is already the agent's memory; putting the list there keeps one
//! source of truth and lets the user see it in the transcript.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolContext, ToolError, ToolResult};

/// Tasks accepted in one call. A longer list is a sign the model is not
/// actually tracking work, just dumping it.
const MAX_ITEMS: usize = 100;

pub struct TodoWriteTool;

#[async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "TodoWrite"
    }

    fn description(&self) -> &str {
        "Record or update your task list. Send the whole list every time — it \
         replaces the previous one, so include items you have not finished. \
         Use it for work with several steps; skip it for a single action."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
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
        let items = input
            .get("todos")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ToolError::InvalidInput("`todos` is required and must be an array".to_owned())
            })?;
        if items.len() > MAX_ITEMS {
            return Err(ToolError::InvalidInput(format!(
                "{} items is more than the {MAX_ITEMS} this tool accepts; a list that long is \
                 not tracking work",
                items.len()
            )));
        }

        let mut rendered = String::new();
        let mut counts = (0usize, 0usize, 0usize);
        for item in items {
            let text = item
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .ok_or_else(|| {
                    ToolError::InvalidInput("every item needs a non-empty `text`".to_owned())
                })?;
            let status = item
                .get("status")
                .and_then(Value::as_str)
                .ok_or_else(|| ToolError::InvalidInput(format!("`{text}` needs a `status`")))?;
            let (mark, slot) = match status {
                "completed" => ("[x]", 0),
                "in_progress" => ("[~]", 1),
                "pending" => ("[ ]", 2),
                other => {
                    return Err(ToolError::InvalidInput(format!(
                        "`{other}` is not a status; use pending, in_progress or completed"
                    )));
                }
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
        Ok(ToolResult::ok(format!(
            "{} done, {} in progress, {} pending:\n{rendered}",
            counts.0, counts.1, counts.2
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(error.to_string().contains("not a status"));
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
        assert!(error.to_string().contains("more than"));
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
