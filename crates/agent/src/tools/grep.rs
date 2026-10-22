//! `Grep` — search file contents.
//!
//! Implemented by walking the tree in Rust rather than shelling out to `rg`, so
//! the tool works on a machine with no search binary installed and its output
//! shape does not depend on one. Trees that are usually irrelevant (`.git`,
//! `target`, `node_modules`) are skipped unless the caller asks otherwise.

use std::path::Path;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{Value, json};

use super::{resolve_path, truncate};
use crate::tool::{Tool, ToolContext, ToolError, ToolResult};

/// Matches returned before the list is cut short.
const MAX_MATCHES: usize = 200;
/// Total characters of output before truncation.
const MAX_CHARS: usize = 100_000;
/// Directories skipped while walking.
const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".venv", "dist", "build"];
/// Files above this size are not searched.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn description(&self) -> &str {
        "Search file contents with a regular expression. Use Grep for content \
         search rather than `grep`/`rg` in Bash. Results are `path:line: text`, \
         capped at 200 matches."
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { true }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "A regular expression to search for."
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search, relative to the working directory. Defaults to the working directory."
                },
                "glob": {
                    "type": "string",
                    "description": "Only search files whose path matches this glob, e.g. `*.rs`."
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Match case-sensitively. Defaults to true."
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
        let case_sensitive = input
            .get("case_sensitive")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let expression = if case_sensitive {
            pattern.to_owned()
        } else {
            format!("(?i){pattern}")
        };
        let regex = Regex::new(&expression).map_err(|error| {
            ToolError::InvalidInput(format!("`{pattern}` is not a valid regex: {error}"))
        })?;

        let file_glob = input
            .get("glob")
            .and_then(Value::as_str)
            .map(|glob| {
                glob::Pattern::new(glob).map_err(|error| {
                    ToolError::InvalidInput(format!("`{glob}` is not a valid glob: {error}"))
                })
            })
            .transpose()?;

        let root = match input.get("path").and_then(Value::as_str) {
            Some(path) => resolve_path(&ctx.cwd, path)?,
            None => resolve_path(&ctx.cwd, ".")?,
        };

        let mut out = String::new();
        let mut matches = 0usize;
        let mut files = 0usize;
        walk(
            &root,
            &regex,
            file_glob.as_ref(),
            ctx,
            &mut out,
            &mut matches,
            &mut files,
        )?;

        if matches == 0 {
            return Ok(ToolResult::ok(format!("No matches for `{pattern}`")));
        }
        let (body, truncated) = truncate(&out, MAX_CHARS);
        let note = if matches >= MAX_MATCHES {
            format!("\n\n… capped at {MAX_MATCHES} matches")
        } else if truncated {
            "\n\n… output truncated".to_owned()
        } else {
            String::new()
        };
        Ok(ToolResult::ok(format!(
            "{matches} match(es) in {files} file(s):\n{body}{note}"
        )))
    }
}

#[allow(clippy::too_many_arguments)]
fn walk(
    path: &Path,
    regex: &Regex,
    file_glob: Option<&glob::Pattern>,
    ctx: &ToolContext,
    out: &mut String,
    matches: &mut usize,
    files: &mut usize,
) -> Result<(), ToolError> {
    if ctx.cancel.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    if *matches >= MAX_MATCHES {
        return Ok(());
    }

    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        // A broken symlink or a racing delete is not worth failing the search.
        Err(_) => return Ok(()),
    };

    if metadata.is_file() {
        if let Some(pattern) = file_glob {
            if !pattern.matches(&path.to_string_lossy()) {
                return Ok(());
            }
        }
    }

    if metadata.is_dir() {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if SKIP_DIRS.contains(&name) {
            return Ok(());
        }
        let entries = match std::fs::read_dir(path) {
            Ok(entries) => entries,
            Err(_) => return Ok(()),
        };
        for entry in entries.flatten() {
            walk(&entry.path(), regex, file_glob, ctx, out, matches, files)?;
            if *matches >= MAX_MATCHES {
                break;
            }
        }
        return Ok(());
    }

    if metadata.len() > MAX_FILE_BYTES {
        return Ok(());
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // Binary or non-UTF-8: not searchable as text.
        Err(_) => return Ok(()),
    };

    let display = path
        .strip_prefix(&ctx.cwd)
        .unwrap_or(path)
        .display()
        .to_string();
    let mut found_here = false;
    for (index, line) in text.lines().enumerate() {
        if *matches >= MAX_MATCHES {
            break;
        }
        if regex.is_match(line) {
            found_here = true;
            *matches += 1;
            use std::fmt::Write as _;
            let _ = writeln!(out, "{display}:{}: {}", index + 1, line.trim_end());
        }
    }
    if found_here {
        *files += 1;
    }
    Ok(())
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
    async fn finds_a_pattern_across_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn other() {}\n").unwrap();

        let result = GrepTool
            .call(json!({ "pattern": "fn \\w+" }), &context(dir.path()))
            .await
            .expect("greps");

        assert!(result.output.contains("a.rs:1"));
        assert!(result.output.contains("b.rs:1"));
    }

    #[tokio::test]
    async fn the_glob_filter_limits_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "needle\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "needle\n").unwrap();

        let result = GrepTool
            .call(
                json!({ "pattern": "needle", "glob": "*.rs" }),
                &context(dir.path()),
            )
            .await
            .expect("greps");

        assert!(result.output.contains("a.rs"));
        assert!(!result.output.contains("b.txt"));
    }

    #[tokio::test]
    async fn an_invalid_regex_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let error = GrepTool
            .call(json!({ "pattern": "([unclosed" }), &context(dir.path()))
            .await
            .expect_err("bad regex");
        assert!(error.to_string().contains("not a valid regex"));
    }

    #[tokio::test]
    async fn no_matches_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "hello\n").unwrap();

        let result = GrepTool
            .call(json!({ "pattern": "absent" }), &context(dir.path()))
            .await
            .expect("greps");

        assert!(!result.is_error);
        assert!(result.output.contains("No matches"));
    }

    #[tokio::test]
    async fn skipped_directories_are_not_searched() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target/a.rs"), "needle\n").unwrap();
        std::fs::write(dir.path().join("src.rs"), "needle\n").unwrap();

        let result = GrepTool
            .call(json!({ "pattern": "needle" }), &context(dir.path()))
            .await
            .expect("greps");

        assert!(result.output.contains("src.rs"));
        assert!(!result.output.contains("target/"));
    }
}
