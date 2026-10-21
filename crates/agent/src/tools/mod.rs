//! The file and shell tools the agent drives.
//!
//! Relative paths resolve against the run working directory. Under the
//! application's trust-all policy, absolute and parent paths may access other
//! directories with the host process permissions, just like shell commands.
//!
//! Tool names and argument names follow the original project's, so a model that has seen those
//! prompts behaves the same way here.

pub mod ask_user;
pub(crate) mod bash;
mod edit;
mod folder;
mod glob;
mod grep;
mod read;
mod skill;
mod agent;
mod todo;
mod web_fetch;
mod web_search;
mod write;

pub use ask_user::{ASK_USER_QUESTION, AskUserQuestionTool, validate as validate_questions};
pub use bash::BashTool;
pub use edit::EditTool;
pub use folder::FolderTool;
pub use glob::GlobTool;
pub use grep::GrepTool;
pub use read::ReadTool;
pub use skill::{DiscoverSkillsTool, SkillTool};
pub use agent::{
    GENERAL_PURPOSE, NestedEventSender, SubagentOutcome, SubagentRuntime, SubagentSpawn, AGENT,
    AgentTool,
};
pub use todo::TodoWriteTool;
pub use web_fetch::WebFetchTool;
pub use web_search::{WebSearchConfig, WebSearchTool};
pub use write::WriteTool;

use std::path::{Path, PathBuf};

use crate::tool::{ToolError, ToolRegistry};

/// 注册内置工具集，WebSearch 默认使用免费后端且不携带凭据。
///
/// 宿主可用 [`register_core_tools_with_search`] 注入独立搜索配置；如需明示
/// 禁用，在注册后用 [`WebSearchTool::disabled`] 覆盖同名工具即可。
pub fn register_core_tools(registry: &mut ToolRegistry) {
    registry.register(std::sync::Arc::new(ReadTool));
    registry.register(std::sync::Arc::new(WriteTool));
    registry.register(std::sync::Arc::new(EditTool));
    registry.register(std::sync::Arc::new(GlobTool));
    registry.register(std::sync::Arc::new(GrepTool));
    registry.register(std::sync::Arc::new(FolderTool));
    registry.register(std::sync::Arc::new(BashTool));
    registry.register(std::sync::Arc::new(TodoWriteTool));
    registry.register(std::sync::Arc::new(WebFetchTool));
    registry.register(std::sync::Arc::new(WebSearchTool::with_default_backend()));
    registry.register(std::sync::Arc::new(AskUserQuestionTool));
    // The defaults carry no disabled set: a host that knows which skills are
    // switched off re-registers these two with one, and the registry replaces
    // a tool by name, keeping its position in the list stable.
    registry.register(std::sync::Arc::new(DiscoverSkillsTool::default()));
    registry.register(std::sync::Arc::new(SkillTool::default()));
}

/// 用独立搜索配置覆盖免费后端，保持工具注册位置不变。
pub fn register_core_tools_with_search(registry: &mut ToolRegistry, search: WebSearchConfig) {
    register_core_tools(registry);
    registry.register(std::sync::Arc::new(WebSearchTool::new(search)));
}

/// The POSIX shell to run shell commands through, when one is available.
///
/// Shared so the hook runner and the Bash tool agree on which shell a command
/// gets: two answers would mean a hook that works when run by hand behaves
/// differently under the agent.
pub fn posix_shell() -> Option<&'static str> {
    match bash::shell() {
        bash::Shell::Posix(program) => Some(program),
        bash::Shell::Cmd => None,
    }
}

/// The path to name in a diff: workspace-relative when the file is inside
/// `cwd`, absolute otherwise.
///
/// Relative is what the diff pane and the file tree key on; an absolute
/// leftover would show the machine's layout instead of the project's.
pub(crate) fn relative_to(cwd: &Path, path: &Path) -> String {
    path.strip_prefix(cwd)
        .unwrap_or(path)
        .to_string_lossy()
        .replace(char::from(92), "/")
}

/// Resolve relative paths against the run directory without imposing a sandbox.
pub(crate) fn resolve_path(cwd: &Path, candidate: &str) -> Result<PathBuf, ToolError> {
    if candidate.trim().is_empty() {
        return Err(ToolError::InvalidInput("the path is empty".to_owned()));
    }
    if candidate.contains('\0') {
        return Err(ToolError::InvalidInput(
            "the path contains a null byte".to_owned(),
        ));
    }
    Ok(cwd.join(candidate))
}

/// Trim `text` to at most `max_chars`, appending a note that says how much was
/// dropped. Character-based, so it never splits a multi-byte sequence.
pub(crate) fn truncate(text: &str, max_chars: usize) -> (String, bool) {
    if text.chars().count() <= max_chars {
        return (text.to_owned(), false);
    }
    let head: String = text.chars().take(max_chars).collect();
    let dropped = text.chars().count() - max_chars;
    (
        format!("{head}\n\n… ({dropped} more characters truncated)"),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{Tool, ToolContext};
    use serde_json::json;

    #[test]
    fn paths_follow_trust_all_policy() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_path(root.path(), "src/main.rs").unwrap(),
            root.path().join("src/main.rs")
        );
        let absolute = other.path().join("fact.md");
        assert_eq!(
            resolve_path(root.path(), absolute.to_str().unwrap()).unwrap(),
            absolute
        );
        assert_eq!(
            resolve_path(root.path(), "../fact.md").unwrap(),
            root.path().join("../fact.md")
        );
        assert!(resolve_path(root.path(), "  ").is_err());
        assert!(resolve_path(root.path(), "bad\0path").is_err());
    }

    #[tokio::test]
    async fn ordinary_file_tools_manage_memory_outside_the_working_directory() {
        let project = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let root = crate::memory::directory(project.path(), config.path());
        let ctx = ToolContext {
            cwd: project.path().to_path_buf(),
            cancel: tokio_util::sync::CancellationToken::new(),
            nested: None,
        };
        let path = root.join("feedback.md");
        WriteTool.call(json!({"file_path":path,"content":"---\nname: Shell\ndescription: shell preference\ntype: feedback\n---\nUse PowerShell"}), &ctx).await.unwrap();
        WriteTool.call(json!({"file_path":root.join("MEMORY.md"),"content":"- [Shell](feedback.md) — terminal preference"}), &ctx).await.unwrap();
        let read = ReadTool
            .call(json!({"file_path":path}), &ctx)
            .await
            .unwrap();
        assert!(read.output.contains("Use PowerShell"));
        let edit = EditTool.call(json!({"file_path":path,"old_string":"Use PowerShell","new_string":"Prefer PowerShell"}), &ctx).await.unwrap();
        assert!(!edit.is_error);
        let glob = GlobTool
            .call(json!({"path":root,"pattern":"*.md"}), &ctx)
            .await
            .unwrap();
        assert!(glob.output.contains("feedback.md"));
        let grep = GrepTool
            .call(json!({"path":root,"pattern":"Prefer PowerShell"}), &ctx)
            .await
            .unwrap();
        assert!(grep.output.contains("feedback.md"));
        let list = FolderTool.call(json!({"path":root}), &ctx).await.unwrap();
        assert!(list.output.contains("MEMORY.md"));
        let prompt =
            crate::memory::system_prompt_section(project.path(), Some(config.path())).unwrap();
        assert!(prompt.contains("terminal preference"));
        assert!(!prompt.contains("Prefer PowerShell"));
        let mut registry = ToolRegistry::new();
        register_core_tools(&mut registry);
        assert!(registry.get("MemoryWrite").is_none());
    }

    #[test]
    fn truncation_preserves_unicode_and_reports_omitted_characters() {
        let (text, truncated) = truncate("日本語テキスト", 3);
        assert!(truncated);
        assert!(text.starts_with("日本語"));
        assert!(text.contains("4 more characters"));
        assert_eq!(truncate("abc", 10), ("abc".to_owned(), false));
    }
}
