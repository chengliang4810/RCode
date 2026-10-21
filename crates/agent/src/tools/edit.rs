//! `Edit` — a targeted replacement, refused unless the target is unambiguous.
//!
//! The uniqueness check is the point of this tool: a model that sends a
//! `old_string` matching several places would otherwise silently rewrite the
//! wrong one. Refusing and naming the count is recoverable; a wrong edit is not.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{relative_to, resolve_path};
use crate::tool::{Tool, ToolContext, ToolError, ToolResult};

pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "Edit"
    }

    fn description(&self) -> &str {
        "Replace an exact string in a file. `old_string` must appear exactly \
         once unless `replace_all` is true, so include enough surrounding \
         context to be unambiguous. Use Edit for targeted changes rather than \
         rewriting the file with Write."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file to change."
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact text to replace. Must match once (including whitespace)."
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement text."
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace every occurrence instead of requiring a unique match."
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let path = input
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("`file_path` is required".to_owned()))?;
        let old = input
            .get("old_string")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("`old_string` is required".to_owned()))?;
        let new = input
            .get("new_string")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("`new_string` is required".to_owned()))?;
        let replace_all = input
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if old == new {
            return Err(ToolError::InvalidInput(
                "`old_string` and `new_string` are identical; nothing to do".to_owned(),
            ));
        }
        if old.is_empty() {
            return Err(ToolError::InvalidInput(
                "`old_string` is empty; give the text to replace".to_owned(),
            ));
        }

        let resolved = resolve_path(&ctx.cwd, path)?;
        let text = tokio::fs::read_to_string(&resolved)
            .await
            .map_err(|error| {
                ToolError::Execution(format!("cannot read {}: {error}", resolved.display()))
            })?;

        let matches = text.matches(old).count();
        if matches == 0 {
            return Err(ToolError::InvalidInput(format!(
                "`old_string` does not appear in {}. Read the file and match it exactly, \
                 including whitespace.",
                resolved.display()
            )));
        }
        if matches > 1 && !replace_all {
            return Err(ToolError::InvalidInput(format!(
                "`old_string` appears {matches} times in {}. Add surrounding context to make \
                 it unique, or pass `replace_all: true` to change every occurrence.",
                resolved.display()
            )));
        }

        let updated = if replace_all {
            text.replace(old, new)
        } else {
            text.replacen(old, new, 1)
        };
        tokio::fs::write(&resolved, &updated)
            .await
            .map_err(|error| {
                ToolError::Execution(format!("cannot write {}: {error}", resolved.display()))
            })?;

        let changed = if replace_all { matches } else { 1 };
        Ok(ToolResult::ok(format!(
            "Edited {} ({changed} replacement{})",
            resolved.display(),
            if changed == 1 { "" } else { "s" }
        ))
        // The pane shows before and after; the model reads the line above.
        .with_diff(rcode_proto::ToolDiff {
            path: relative_to(&ctx.cwd, &resolved),
            old_text: Some(text),
            new_text: updated,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    fn context(cwd: &std::path::Path) -> ToolContext {
        ToolContext {
            cwd: cwd.to_path_buf(),
            cancel: CancellationToken::new(),
            nested: None,
        }
    }

    fn write(dir: &std::path::Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[tokio::test]
    async fn replaces_a_unique_match() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "let x = 1;\nlet y = 2;\n");

        EditTool
            .call(
                json!({ "file_path": "a.rs", "old_string": "let x = 1;", "new_string": "let x = 42;" }),
                &context(dir.path()),
            )
            .await
            .expect("edits");

        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
            "let x = 42;\nlet y = 2;\n"
        );
    }

    /// The diff pane needs both sides of the change and the file it belongs
    /// to; a tool that reports only "edited" leaves the pane blank.
    #[tokio::test]
    async fn an_edit_reports_the_before_and_after_text() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.rs",
            "let x = 1;
",
        );

        let result = EditTool
            .call(
                json!({ "file_path": "a.rs", "old_string": "1", "new_string": "2" }),
                &context(dir.path()),
            )
            .await
            .expect("edits");

        let diff = result.diff.expect("an edit carries its diff");
        assert_eq!(diff.path, "a.rs");
        assert_eq!(
            diff.old_text.as_deref(),
            Some(
                "let x = 1;
"
            )
        );
        assert_eq!(
            diff.new_text,
            "let x = 2;
"
        );
    }

    #[tokio::test]
    async fn an_ambiguous_match_is_refused_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "foo\nfoo\n");

        let error = EditTool
            .call(
                json!({ "file_path": "a.rs", "old_string": "foo", "new_string": "bar" }),
                &context(dir.path()),
            )
            .await
            .expect_err("ambiguous");

        assert!(error.to_string().contains("appears 2 times"));
        // The file is untouched.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
            "foo\nfoo\n"
        );
    }

    #[tokio::test]
    async fn replace_all_changes_every_occurrence() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "foo\nfoo\n");

        let result = EditTool
            .call(
                json!({
                    "file_path": "a.rs",
                    "old_string": "foo",
                    "new_string": "bar",
                    "replace_all": true
                }),
                &context(dir.path()),
            )
            .await
            .expect("edits");

        assert!(result.output.contains("2 replacements"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
            "bar\nbar\n"
        );
    }

    #[tokio::test]
    async fn a_missing_match_says_so() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "hello\n");

        let error = EditTool
            .call(
                json!({ "file_path": "a.rs", "old_string": "absent", "new_string": "x" }),
                &context(dir.path()),
            )
            .await
            .expect_err("absent");

        assert!(error.to_string().contains("does not appear"));
    }

    #[tokio::test]
    async fn identical_strings_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "hello\n");

        let error = EditTool
            .call(
                json!({ "file_path": "a.rs", "old_string": "hello", "new_string": "hello" }),
                &context(dir.path()),
            )
            .await
            .expect_err("no-op");

        assert!(error.to_string().contains("identical"));
    }

    #[tokio::test]
    async fn parent_path_can_edit_outside_the_working_directory() {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("project");
        std::fs::create_dir(&cwd).unwrap();
        std::fs::write(root.path().join("fact.md"), "old fact").unwrap();
        EditTool.call(json!({"file_path":"../fact.md","old_string":"old fact","new_string":"corrected fact"}), &context(&cwd)).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("fact.md")).unwrap(),
            "corrected fact"
        );
    }
}
