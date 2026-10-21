//! Tool-card text: a tool's chip label and one-line detail, and the ToolGroup
//! summary line.
//!
//! Moved here from `rcode_proto::view` (§2.5 "展示逻辑迁移") because these
//! functions mint words a person reads — a tool's display *name*, the sentence
//! "ran 3 commands" — and words must change with the display language. `proto`
//! is deliberately language-free, so it can hold the tool data and the sort
//! orders but not the sentence around them.
//!
//! The old `tool_chip_content` returned `(&'static str, String)`: the label was
//! a hardcoded literal, so a caller that wanted a translated one had no home
//! for it short of `Box::leak`. Here every produced string is owned and built
//! from the [`Localizer`], so a translation is *constructed*, not looked up.
//!
//! # What is not translated
//!
//! A tool's argument content — a path, a command, a URL, a search pattern — is
//! the user's or the model's own bytes and must reach the screen verbatim.
//! Only the labels and the connective wording around them are localized; the
//! split runs right through `tool_chip_content`, which now returns
//! `(translated label, untouched detail)`.

use crate::{Localizer, TextKey};
use rcode_proto::ToolCall;

/// A tool's chip label and its one-line detail, both ready to draw.
///
/// The label is the tool's display name (translated); the detail is the call's
/// own content (a command, a path, a URL), left exactly as the model produced
/// it, except that newlines and runs of whitespace are collapsed so the chip
/// stays one line. See [`rcode_proto::view::single_line`] for why that collapse
/// is shared rather than reimplemented per viewport.
pub fn tool_chip_content(localizer: &Localizer, call: &ToolCall) -> (String, String) {
    let (label, detail) = tool_chip_content_raw(localizer, call);
    (label, rcode_proto::view::single_line(&detail))
}

/// The label/detail before the single-line collapse.
///
/// Split out so the collapse happens once, at the one point every arm returns
/// through, instead of at every `return` — the original code did the same to
/// keep the two concerns from being tangled into each match arm.
fn tool_chip_content_raw(localizer: &Localizer, call: &ToolCall) -> (String, String) {
    match call {
        ToolCall::Exec { command } => (localizer.text(TextKey::ToolRun), command.clone()),
        ToolCall::ReadFile { path } => (localizer.text(TextKey::ToolRead), path.clone()),
        ToolCall::WriteFile { path, .. } => (localizer.text(TextKey::ToolWrite), path.clone()),
        ToolCall::EditFile { path, .. } => (localizer.text(TextKey::ToolEdit), path.clone()),
        ToolCall::ApplyPatch { path } => (
            localizer.text(TextKey::ToolPatch),
            // No path means the patch touched the workspace as a whole; the
            // word for that is translatable, so it comes from the resources
            // rather than being a literal "workspace".
            path.clone()
                .unwrap_or_else(|| localizer.text(TextKey::ToolPatchWorkspace)),
        ),
        ToolCall::Search { pattern, path } => (
            localizer.text(TextKey::ToolSearch),
            match path {
                // The connector between pattern and path is a sentence, so it
                // is a resource; both operands stay verbatim.
                Some(path) => localizer.format_with(
                    TextKey::ToolSearchIn,
                    &[("pattern", pattern), ("path", path)],
                ),
                None => pattern.clone(),
            },
        ),
        ToolCall::Glob { pattern } => (localizer.text(TextKey::ToolGlob), pattern.clone()),
        ToolCall::WebFetch { url, .. } => (localizer.text(TextKey::ToolFetch), url.clone()),
        ToolCall::WebSearch { query } => (localizer.text(TextKey::ToolWeb), query.clone()),
        ToolCall::Todo { items } => {
            let done = items.iter().filter(|item| item.done).count().to_string();
            let total = items.len().to_string();
            (
                localizer.text(TextKey::ToolTodo),
                localizer.format_with(
                    TextKey::ToolTodoProgress,
                    &[("done", &done), ("total", &total)],
                ),
            )
        }
        ToolCall::Mcp { server, tool, .. } => (
            localizer.text(TextKey::ToolMcp),
            // `server · tool` is two identifiers and a separator — no words to
            // translate, so it is assembled here rather than in a resource.
            format!("{server} · {tool}"),
        ),
        // A subagent spawn decodes as Unknown named "Agent[: <description>]"
        // (every native driver's convention): label it with the Agent name and
        // put the description in the detail — "Tool · Agent: scan repo" read as
        // two labels fighting.
        ToolCall::Unknown { name, .. } => match name.strip_prefix("Agent: ") {
            Some(description) => (localizer.text(TextKey::ToolAgent), description.to_owned()),
            None if name == "Agent" => (localizer.text(TextKey::ToolAgent), String::new()),
            // Anything else is a tool this build does not know. It still gets a
            // chip — naming it generically and showing its raw name — because
            // dropping the card would hide a real tool call (§2.2: an
            // unidentified tool must have a generic presentation, never vanish).
            None => (localizer.text(TextKey::ToolUnknown), name.clone()),
        },
    }
}

/// The ToolGroup summary line — "Ran 3 commands · edited 2 files".
///
/// Takes `(call, is_error)` pairs so each viewport can keep its own row model;
/// the summary itself is one implementation for both. Fragments are joined by
/// " · " with only the first capitalized, which is the original app's style — the English
/// fragments are written lowercase in the resources ("ran 3 commands") and the
/// leading capital is applied here rather than duplicated into each one.
pub fn tool_group_summary(localizer: &Localizer, tools: &[(ToolCall, bool)]) -> String {
    let mut commands = 0usize;
    let mut edited: Vec<&str> = Vec::new();
    let mut reads = 0usize;
    let mut searches = 0usize;
    let mut fetches = 0usize;
    let mut todos = 0usize;
    let mut other = 0usize;
    let mut failed = 0usize;
    for (call, is_error) in tools {
        if *is_error {
            failed += 1;
        }
        match call {
            ToolCall::Exec { .. } => commands += 1,
            ToolCall::WriteFile { path, .. } | ToolCall::EditFile { path, .. } => {
                if !edited.contains(&path.as_str()) {
                    edited.push(path);
                }
            }
            ToolCall::ApplyPatch { path } => {
                let path = path.as_deref().unwrap_or("patch");
                if !edited.contains(&path) {
                    edited.push(path);
                }
            }
            ToolCall::ReadFile { .. } => reads += 1,
            ToolCall::Search { .. } | ToolCall::Glob { .. } | ToolCall::WebSearch { .. } => {
                searches += 1
            }
            ToolCall::WebFetch { .. } => fetches += 1,
            ToolCall::Todo { .. } => todos += 1,
            ToolCall::Mcp { .. } | ToolCall::Unknown { .. } => other += 1,
        }
    }
    let mut segments: Vec<String> = Vec::new();
    if commands > 0 {
        segments.push(localizer.format_count(TextKey::ToolSummaryRan, "count", commands as i64));
    }
    if !edited.is_empty() {
        segments.push(localizer.format_count(
            TextKey::ToolSummaryEdited,
            "count",
            edited.len() as i64,
        ));
    }
    if reads > 0 {
        segments.push(localizer.format_count(TextKey::ToolSummaryRead, "count", reads as i64));
    }
    if searches > 0 {
        segments.push(localizer.format_count(
            TextKey::ToolSummarySearched,
            "count",
            searches as i64,
        ));
    }
    if fetches > 0 {
        segments.push(localizer.format_count(TextKey::ToolSummaryFetched, "count", fetches as i64));
    }
    if todos > 0 {
        segments.push(localizer.text(TextKey::ToolSummaryTodos));
    }
    if other > 0 {
        segments.push(localizer.format_count(TextKey::ToolSummaryCalled, "count", other as i64));
    }
    if segments.is_empty() {
        segments.push(localizer.format_count(
            TextKey::ToolSummaryNone,
            "count",
            tools.len() as i64,
        ));
    }
    if failed > 0 {
        segments.push(localizer.format_count(TextKey::ToolSummaryFailed, "count", failed as i64));
    }
    capitalize_first(segments.join(" · "))
}

/// Uppercase the first character, leaving the rest alone.
///
/// Character-wise, not byte-wise: the original took `summary.get(0..1)`, which
/// is correct for ASCII and would split a multi-byte character — panicking on
/// a Chinese summary, whose first character is three bytes. Chinese has no
/// case, so `to_uppercase` returns the character unchanged and the summary is
/// left intact.
fn capitalize_first(summary: String) -> String {
    let mut characters = summary.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LanguagePreference;

    fn en() -> Localizer {
        Localizer::with_system_locale(LanguagePreference::EnUs, None)
    }

    fn zh() -> Localizer {
        Localizer::with_system_locale(LanguagePreference::ZhCn, None)
    }

    fn exec(command: &str) -> ToolCall {
        ToolCall::Exec {
            command: command.to_owned(),
        }
    }

    fn read(path: &str) -> ToolCall {
        ToolCall::ReadFile {
            path: path.to_owned(),
        }
    }

    #[test]
    fn a_known_tool_uses_its_localized_name_and_keeps_its_argument() {
        // The label is translated; the argument is not. Conflating the two is
        // the failure this guards: a translated path would send the user to a
        // file that does not exist.
        let call = read("src/main.rs");
        assert_eq!(
            tool_chip_content(&en(), &call),
            ("Read".into(), "src/main.rs".into())
        );
        assert_eq!(
            tool_chip_content(&zh(), &call),
            ("读取".into(), "src/main.rs".into())
        );
    }

    #[test]
    fn an_unknown_tool_still_gets_a_readable_chip() {
        // §2.2: an unrecognized tool must have a generic presentation, not be
        // dropped. A new driver tool named after this build shipped must still
        // appear rather than silently vanishing from the transcript.
        let call = ToolCall::Unknown {
            name: "frobnicate".to_owned(),
            input: None,
        };
        assert_eq!(
            tool_chip_content(&en(), &call),
            ("Tool".into(), "frobnicate".into())
        );
        assert_eq!(
            tool_chip_content(&zh(), &call),
            ("工具".into(), "frobnicate".into())
        );
    }

    #[test]
    fn a_subagent_spawn_reads_as_agent_with_its_description() {
        // "Tool · Agent: scan repo" read as two labels fighting, so the spawn
        // convention is decoded into one label plus a description detail.
        let call = ToolCall::Unknown {
            name: "Agent: scan repo".to_owned(),
            input: None,
        };
        assert_eq!(
            tool_chip_content(&en(), &call),
            ("Agent".into(), "scan repo".into())
        );
        assert_eq!(
            tool_chip_content(&zh(), &call),
            ("智能体".into(), "scan repo".into())
        );
        // The bare spawn, with no description, is an empty detail but the
        // Agent label — not an empty chip.
        let bare = ToolCall::Unknown {
            name: "Agent".to_owned(),
            input: None,
        };
        assert_eq!(
            tool_chip_content(&en(), &bare),
            ("Agent".into(), String::new())
        );
    }

    #[test]
    fn a_multi_line_command_collapses_onto_one_line() {
        // A chip is one line: gpui breaks on a literal newline before its
        // ellipsis, so the detail must be flattened before it is drawn.
        let call = exec("cargo build\n  error: nope");
        let (_, detail) = tool_chip_content(&en(), &call);
        assert_eq!(detail, "cargo build error: nope");
    }

    #[test]
    fn a_search_with_a_path_names_both_in_a_localized_connector() {
        let call = ToolCall::Search {
            pattern: "fn main".to_owned(),
            path: Some("src".to_owned()),
        };
        assert_eq!(
            tool_chip_content(&en(), &call),
            ("Search".into(), "fn main in src".into())
        );
        assert_eq!(
            tool_chip_content(&zh(), &call),
            ("搜索".into(), "在 src 中查找 fn main".into())
        );
        // No path: the pattern alone, untouched.
        let bare = ToolCall::Search {
            pattern: "fn main".to_owned(),
            path: None,
        };
        assert_eq!(
            tool_chip_content(&en(), &bare),
            ("Search".into(), "fn main".into())
        );
    }

    #[test]
    fn a_patch_without_a_path_names_the_workspace_in_the_current_language() {
        // The "workspace" fallback was a hardcoded literal; it is a word, so it
        // is translated rather than passed through as an identifier.
        let call = ToolCall::ApplyPatch { path: None };
        assert_eq!(
            tool_chip_content(&en(), &call),
            ("Patch".into(), "workspace".into())
        );
        assert_eq!(
            tool_chip_content(&zh(), &call),
            ("补丁".into(), "工作区".into())
        );
    }

    #[test]
    fn a_todo_chip_reports_how_many_are_done() {
        let call = ToolCall::Todo {
            items: vec![
                rcode_proto::TodoItem {
                    text: "a".to_owned(),
                    done: true,
                    status: None,
                },
                rcode_proto::TodoItem {
                    text: "b".to_owned(),
                    done: false,
                    status: None,
                },
            ],
        };
        assert_eq!(
            tool_chip_content(&en(), &call),
            ("Todo".into(), "1/2 done".into())
        );
        assert_eq!(
            tool_chip_content(&zh(), &call),
            ("待办".into(), "已完成 1/2".into())
        );
    }

    #[test]
    fn the_group_summary_counts_each_kind() {
        let tools = vec![
            (exec("ls"), false),
            (read("a.rs"), false),
            (
                ToolCall::Search {
                    pattern: "x".to_owned(),
                    path: None,
                },
                false,
            ),
        ];
        assert_eq!(
            tool_group_summary(&en(), &tools),
            "Ran 1 command · read 1 file · searched 1 time"
        );
    }

    #[test]
    fn edited_files_are_counted_uniquely() {
        // Two edits to the same path are one file, so the summary must not
        // double-count — the reason `edited` is a set-like list, not a counter.
        let tools = vec![
            (
                ToolCall::EditFile {
                    path: "a.rs".to_owned(),
                    old_string: None,
                    new_string: None,
                },
                false,
            ),
            (
                ToolCall::EditFile {
                    path: "a.rs".to_owned(),
                    old_string: None,
                    new_string: None,
                },
                false,
            ),
        ];
        assert_eq!(tool_group_summary(&en(), &tools), "Edited 1 file");
    }

    #[test]
    fn an_empty_group_summary_still_names_the_tool_count() {
        // Without this the summary line would be empty, which reads as a
        // rendering bug rather than "nothing notable happened".
        let tools = vec![(
            ToolCall::Unknown {
                name: "thing".to_owned(),
                input: None,
            },
            false,
        )];
        // A tool in `other` is "called 1 tool", not the empty fallback.
        assert_eq!(tool_group_summary(&en(), &tools), "Called 1 tool");
    }

    #[test]
    fn a_failed_call_is_counted_at_the_end() {
        // Failures trail the sentence so the eye lands on them last.
        let tools = vec![(exec("boom"), true)];
        assert_eq!(
            tool_group_summary(&en(), &tools),
            "Ran 1 command · 1 failed"
        );
    }

    #[test]
    fn the_summary_is_localized_end_to_end() {
        let tools = vec![
            (exec("ls"), false),
            (read("a.rs"), false),
            (
                ToolCall::Search {
                    pattern: "x".to_owned(),
                    path: None,
                },
                false,
            ),
        ];
        // No capitalization is required by the script, and the leading capital
        // rule must not mangle (or panic on) a multi-byte first character.
        assert_eq!(
            tool_group_summary(&zh(), &tools),
            "运行了 1 个命令 · 读取了 1 个文件 · 搜索了 1 次"
        );
    }

    #[test]
    fn updating_todos_uses_a_word_not_a_count() {
        let tools = vec![(ToolCall::Todo { items: vec![] }, false)];
        assert_eq!(tool_group_summary(&en(), &tools), "Updated todos");
        assert_eq!(tool_group_summary(&zh(), &tools), "更新了待办");
    }

    #[test]
    fn capitalize_first_leaves_a_caseless_script_alone() {
        // Guards the byte-vs-character bug: a naive `get(0..1)` would split the
        // first multi-byte character and panic.
        assert_eq!(
            capitalize_first("运行了 1 个命令".to_owned()),
            "运行了 1 个命令"
        );
        assert_eq!(
            capitalize_first("ran 1 command".to_owned()),
            "Ran 1 command"
        );
        assert_eq!(capitalize_first(String::new()), "");
    }
}
