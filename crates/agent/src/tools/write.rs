//! `Write` — a whole file, creating parents as needed.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::resolve_path;
use crate::tool::{Tool, ToolContext, ToolError, ToolResult};

pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    fn description(&self) -> &str {
        "Write a file, replacing anything already there. Use Write for whole \
         files (and for new files) rather than echo >/sed/awk; parent \
         directories are created. Prefer Edit for a targeted change to a file \
         that already exists."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to write, absolute or relative to the working directory."
                },
                "content": {
                    "type": "string",
                    "description": "The complete contents to write."
                }
            },
            "required": ["file_path", "content"]
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let path = input
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("`file_path` is required".to_owned()))?;
        let content = input
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("`content` is required".to_owned()))?;

        let resolved = resolve_path(&ctx.cwd, path)?;
        // Read before writing: the diff pane needs the old text, and after the
        // write there is nowhere to get it. A missing file is a creation, which
        // the wire spells as `old_text: None`.
        let previous = tokio::fs::read_to_string(&resolved).await.ok();
        if let Some(parent) = resolved.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                ToolError::Execution(format!("cannot create {}: {error}", parent.display()))
            })?;
        }

        tokio::fs::write(&resolved, content)
            .await
            .map_err(|error| {
                ToolError::Execution(format!("cannot write {}: {error}", resolved.display()))
            })?;

        Ok(ToolResult::ok(format!(
            "Wrote {} ({} bytes)",
            resolved.display(),
            content.len()
        ))
        .with_diff(rcode_proto::ToolDiff {
            path: super::relative_to(&ctx.cwd, &resolved),
            old_text: previous,
            new_text: content.to_owned(),
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

    #[tokio::test]
    async fn writes_a_file_and_creates_parents() {
        let dir = tempfile::tempdir().unwrap();
        let result = WriteTool
            .call(
                json!({ "file_path": "nested/deep/a.txt", "content": "hello" }),
                &context(dir.path()),
            )
            .await
            .expect("writes");

        assert!(!result.is_error);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("nested/deep/a.txt")).unwrap(),
            "hello"
        );
    }

    #[tokio::test]
    async fn replaces_existing_contents() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "old").unwrap();

        WriteTool
            .call(
                json!({ "file_path": "a.txt", "content": "new" }),
                &context(dir.path()),
            )
            .await
            .expect("writes");

        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "new"
        );
    }

    #[tokio::test]
    async fn missing_content_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let error = WriteTool
            .call(json!({ "file_path": "a.txt" }), &context(dir.path()))
            .await
            .expect_err("no content");
        assert!(matches!(error, ToolError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn parent_path_can_write_outside_the_working_directory() {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("project");
        std::fs::create_dir(&cwd).unwrap();
        WriteTool
            .call(
                json!({"file_path":"../fact.md","content":"remembered"}),
                &context(&cwd),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("fact.md")).unwrap(),
            "remembered"
        );
    }
}
