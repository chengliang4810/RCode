//! `Read` — file contents, optionally a line range.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{resolve_path, truncate};
use crate::tool::{Tool, ToolContext, ToolError, ToolResult};

/// Characters returned before the tool truncates. Roughly 2,000 lines of
/// ordinary source; past this the model is better served by a range read.
const MAX_CHARS: usize = 200_000;

pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "Read"
    }

    fn description(&self) -> &str {
        "Read a file. Use Read for file contents rather than cat/head/tail. \
         Returns the file with 1-based line numbers; pass offset/limit for a \
         large file. Reading a directory fails — use FolderOperations."
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { true }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file, absolute or relative to the working directory."
                },
                "offset": {
                    "type": "integer",
                    "description": "1-based line to start at. Omit to start at the beginning."
                },
                "limit": {
                    "type": "integer",
                    "description": "How many lines to read. Omit to read to the end."
                }
            },
            "required": ["file_path"]
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let path = input
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("`file_path` is required".to_owned()))?;
        let offset = input
            .get("offset")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1) as usize;
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| value as usize);

        let resolved = resolve_path(&ctx.cwd, path)?;
        let metadata = tokio::fs::metadata(&resolved).await.map_err(|error| {
            ToolError::Execution(format!("cannot read {}: {error}", resolved.display()))
        })?;
        if metadata.is_dir() {
            return Err(ToolError::InvalidInput(format!(
                "{} is a directory; use FolderOperations to list it",
                resolved.display()
            )));
        }

        let bytes = tokio::fs::read(&resolved).await.map_err(|error| {
            ToolError::Execution(format!("cannot read {}: {error}", resolved.display()))
        })?;
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                return Err(ToolError::InvalidInput(format!(
                    "{} is not UTF-8 text ({} bytes); this tool reads text files only",
                    resolved.display(),
                    metadata.len()
                )));
            }
        };

        let total_lines = text.lines().count();
        let selected: String = text
            .lines()
            .skip(offset - 1)
            .take(limit.unwrap_or(usize::MAX))
            .enumerate()
            .map(|(index, line)| format!("{:>6}\t{line}\n", offset + index))
            .collect();

        if selected.is_empty() {
            return Ok(ToolResult::ok(format!(
                "{} has {total_lines} lines; offset {offset} is past the end",
                resolved.display()
            )));
        }

        let (body, truncated) = truncate(&selected, MAX_CHARS);
        let header = if truncated {
            format!(
                "{} (lines {offset}–{}, of {total_lines})\n",
                resolved.display(),
                offset + body.lines().count().saturating_sub(2)
            )
        } else {
            format!(
                "{} (lines {offset}–{}, of {total_lines})\n",
                resolved.display(),
                offset + selected.lines().count() - 1
            )
        };
        Ok(ToolResult::ok(format!("{header}{body}")))
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
    async fn reads_a_file_with_line_numbers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();

        let result = ReadTool
            .call(json!({ "file_path": "a.txt" }), &context(dir.path()))
            .await
            .expect("reads");

        assert!(result.output.contains("one"));
        assert!(result.output.contains("three"));
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn an_offset_and_limit_select_a_range() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\nfour\n").unwrap();

        let result = ReadTool
            .call(
                json!({ "file_path": "a.txt", "offset": 2, "limit": 2 }),
                &context(dir.path()),
            )
            .await
            .expect("reads");

        assert!(result.output.contains("two"));
        assert!(result.output.contains("three"));
        assert!(!result.output.contains("one"));
        assert!(!result.output.contains("four"));
    }

    #[tokio::test]
    async fn a_missing_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let error = ReadTool
            .call(json!({ "file_path": "nope.txt" }), &context(dir.path()))
            .await
            .expect_err("missing");
        assert!(matches!(error, ToolError::Execution(_)));
    }

    #[tokio::test]
    async fn a_directory_is_refused_with_a_hint() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let error = ReadTool
            .call(json!({ "file_path": "sub" }), &context(dir.path()))
            .await
            .expect_err("dir");
        assert!(error.to_string().contains("FolderOperations"));
    }

    #[tokio::test]
    async fn parent_path_can_read_outside_the_working_directory() {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("project");
        std::fs::create_dir(&cwd).unwrap();
        std::fs::write(root.path().join("fact.md"), "remembered").unwrap();
        let result = ReadTool
            .call(json!({"file_path":"../fact.md"}), &context(&cwd))
            .await
            .unwrap();
        assert!(result.output.contains("remembered"));
    }
}
