//! `FolderOperations` — list a directory as structured data.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::resolve_path;
use crate::tool::{Tool, ToolContext, ToolError, ToolResult};

/// Entries listed before the output is cut short.
const MAX_ENTRIES: usize = 500;

pub struct FolderTool;

#[async_trait]
impl Tool for FolderTool {
    fn name(&self) -> &str {
        "FolderOperations"
    }

    fn description(&self) -> &str {
        "List a directory. Use FolderOperations when directory entries are needed \
         as data; `ls -la` in Bash is the quicker choice for a one-off human look. \
         Directories are marked with a trailing `/`."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory to list, relative to the working directory. Defaults to the working directory."
                }
            },
            "required": []
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let root = match input.get("path").and_then(Value::as_str) {
            Some(path) if !path.trim().is_empty() => resolve_path(&ctx.cwd, path)?,
            _ => resolve_path(&ctx.cwd, ".")?,
        };

        let mut entries = tokio::fs::read_dir(&root).await.map_err(|error| {
            ToolError::Execution(format!("cannot list {}: {error}", root.display()))
        })?;

        let mut directories: Vec<(String, u64)> = Vec::new();
        let mut files: Vec<(String, u64)> = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| ToolError::Execution(error.to_string()))?
        {
            if ctx.cancel.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(metadata) = entry.metadata().await else {
                continue;
            };
            if metadata.is_dir() {
                directories.push((format!("{name}/"), metadata.len()));
            } else {
                files.push((name, metadata.len()));
            }
        }

        if directories.is_empty() && files.is_empty() {
            return Ok(ToolResult::ok(format!("{} is empty", root.display())));
        }

        // Directories first, then files; each alphabetical. That is the order a
        // person scans a listing in.
        directories.sort_by(|left, right| left.0.cmp(&right.0));
        files.sort_by(|left, right| left.0.cmp(&right.0));

        let total = directories.len() + files.len();
        let mut lines: Vec<String> = Vec::with_capacity(total);
        for (name, _) in &directories {
            lines.push(name.clone());
        }
        for (name, bytes) in &files {
            lines.push(format!("{name}  ({bytes} bytes)"));
        }

        let shown = lines.len().min(MAX_ENTRIES);
        let body = lines[..shown].join("\n");
        let note = if total > shown {
            format!("\n… {} more entries not shown", total - shown)
        } else {
            String::new()
        };
        Ok(ToolResult::ok(format!(
            "{} — {} director{}, {} file{}\n{body}{note}",
            root.display(),
            directories.len(),
            if directories.len() == 1 { "y" } else { "ies" },
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        )))
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
    async fn lists_directories_before_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("z.txt"), "x").unwrap();
        std::fs::create_dir(dir.path().join("a-dir")).unwrap();

        let result = FolderTool
            .call(json!({}), &context(dir.path()))
            .await
            .expect("lists");

        let body = &result.output;
        let dir_at = body.find("a-dir/").expect("directory listed");
        let file_at = body.find("z.txt").expect("file listed");
        assert!(dir_at < file_at, "directories sort before files");
    }

    #[tokio::test]
    async fn an_empty_directory_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let result = FolderTool
            .call(json!({}), &context(dir.path()))
            .await
            .expect("lists");
        assert!(result.output.contains("is empty"));
    }

    #[tokio::test]
    async fn a_missing_directory_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let error = FolderTool
            .call(json!({ "path": "nope" }), &context(dir.path()))
            .await
            .expect_err("missing");
        assert!(matches!(error, ToolError::Execution(_)));
    }

    #[tokio::test]
    async fn parent_directory_can_be_listed() {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("project");
        std::fs::create_dir(&cwd).unwrap();
        std::fs::write(root.path().join("fact.md"), "remembered").unwrap();
        let result = FolderTool
            .call(json!({"path":".."}), &context(&cwd))
            .await
            .unwrap();
        assert!(result.output.contains("fact.md"));
    }
}
