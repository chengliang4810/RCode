//! `Glob` — find files by name.
//!
//! The pattern guard matters: `*` and `**/*` match the entire tree, and a model
//! that asks for one gets a list far too large to be useful (and expensive to
//! put in the next request). Refusing with a hint is the better failure.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::resolve_path;
use crate::tool::{Tool, ToolContext, ToolError, ToolResult};

/// Matches returned before the list is cut short.
const MAX_RESULTS: usize = 200;

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }

    fn description(&self) -> &str {
        "Find files by name, e.g. `**/*.rs` or `src/**/*.toml`. Use Glob for \
         name search rather than `Bash` with find. Do not pass `*` or `**/*` \
         alone — that lists the whole tree; use FolderOperations to see one \
         directory instead."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "A glob, e.g. `**/*.rs`. Must be more specific than `*`."
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search under, relative to the working directory. Defaults to the working directory."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let pattern = input
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("`pattern` is required".to_owned()))?;
        let trimmed = pattern.trim();
        if matches!(trimmed, "*" | "**" | "**/*" | "*/*") {
            return Err(ToolError::InvalidInput(format!(
                "`{trimmed}` would list the entire tree. Use a pattern with a name or \
                 extension (e.g. `**/*.rs`), or FolderOperations for one directory."
            )));
        }

        let root = match input.get("path").and_then(Value::as_str) {
            Some(path) => resolve_path(&ctx.cwd, path)?,
            None => resolve_path(&ctx.cwd, ".")?,
        };

        let full = root.join(trimmed).to_string_lossy().replace('\\', "/");

        let entries = glob::glob(&full).map_err(|error| {
            ToolError::InvalidInput(format!("`{trimmed}` is not a valid glob: {error}"))
        })?;

        let mut matches: Vec<std::path::PathBuf> = Vec::new();
        for entry in entries.flatten() {
            if ctx.cancel.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            if !entry.is_file() {
                continue;
            }
            matches.push(entry);
        }

        // Newest first: a file just touched is usually the one being asked about.
        matches.sort_by_key(|path| {
            std::cmp::Reverse(
                std::fs::metadata(path)
                    .and_then(|meta| meta.modified())
                    .ok(),
            )
        });

        let total = matches.len();
        let shown = matches.len().min(MAX_RESULTS);
        let body = matches
            .iter()
            .take(MAX_RESULTS)
            .map(|path| {
                path.strip_prefix(&ctx.cwd)
                    .unwrap_or(path)
                    .display()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");

        if total == 0 {
            return Ok(ToolResult::ok(format!("No files match `{trimmed}`")));
        }
        let note = if total > shown {
            format!("\n\n… {} more matches not shown", total - shown)
        } else {
            String::new()
        };
        Ok(ToolResult::ok(format!(
            "{total} file(s) match `{trimmed}`:\n{body}{note}"
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
    async fn finds_files_by_extension() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "").unwrap();
        std::fs::write(dir.path().join("b.txt"), "").unwrap();

        let result = GlobTool
            .call(json!({ "pattern": "*.rs" }), &context(dir.path()))
            .await
            .expect("globs");

        assert!(result.output.contains("a.rs"));
        assert!(!result.output.contains("b.txt"));
    }

    #[tokio::test]
    async fn finds_files_recursively() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("deep/nested")).unwrap();
        std::fs::write(dir.path().join("deep/nested/a.rs"), "").unwrap();

        let result = GlobTool
            .call(json!({ "pattern": "**/*.rs" }), &context(dir.path()))
            .await
            .expect("globs");

        assert!(result.output.contains("a.rs"));
    }

    #[tokio::test]
    async fn a_whole_tree_pattern_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for pattern in ["*", "**/*", "**", "*/*"] {
            let error = GlobTool
                .call(json!({ "pattern": pattern }), &context(dir.path()))
                .await
                .expect_err("refused");
            assert!(
                error.to_string().contains("entire tree"),
                "pattern {pattern} should be refused"
            );
        }
    }

    #[tokio::test]
    async fn no_matches_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = GlobTool
            .call(json!({ "pattern": "*.nope" }), &context(dir.path()))
            .await
            .expect("globs");
        assert!(!result.is_error);
        assert!(result.output.contains("No files match"));
    }
}
