//! Prompt templates: workspace `.agents/commands`, then `~/.rcode/commands`.
//! Roots never walk up the workspace. Nested files use colon namespaces.
use rcode_proto::custom_commands::{CommandMutation, CommandSource, CustomCommand};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_BYTES: usize = 100_000;
const MAX_FILES: usize = 512;
const MAX_DEPTH: usize = 8;
static WRITE_LOCK: Mutex<()> = Mutex::new(());

pub fn user_root(home: &Path) -> PathBuf {
    home.join(".rcode").join("commands")
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.split(':').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        })
}
fn reserved(name: &str) -> bool {
    matches!(name, "compact" | "c" | "core:compact" | "goal")
}
fn revision(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn read(path: &Path) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("Command must be a regular file".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("Command exceeds 100 KB".into());
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

fn parse_document(content: &str) -> Result<(yaml_serde::Mapping, String), String> {
    let normalized = content.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    if let Some(rest) = normalized.strip_prefix("---\n") {
        let (header, body) = rest
            .split_once("\n---\n")
            .ok_or("Unclosed command frontmatter")?;
        let header = yaml_serde::from_str(header)
            .map_err(|e| format!("Invalid command frontmatter: {e}"))?;
        Ok((header, body.to_owned()))
    } else {
        Ok((Default::default(), normalized))
    }
}
fn parse(name: String, content: String, source: CommandSource) -> Result<CustomCommand, String> {
    let (header, body) = parse_document(&content)?;
    if body.trim().is_empty() {
        return Err("Command prompt cannot be empty".into());
    }
    let get = |key: &str| header.get(yaml_serde::Value::String(key.into()));
    for key in ["description", "argument-hint"] {
        if get(key).is_some_and(|v| !v.is_string()) {
            return Err(format!("{key} must be text"));
        }
    }
    if get("enabled").is_some_and(|v| !v.is_bool()) {
        return Err("enabled must be a boolean".into());
    }
    Ok(CustomCommand {
        name,
        description: get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| body.lines().find(|l| !l.trim().is_empty()).unwrap_or(""))
            .chars()
            .take(256)
            .collect(),
        input_hint: get("argument-hint")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        enabled: get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
        source,
        revision: revision(&content),
        content,
    })
}

fn scan(root: &Path, dir: &Path, depth: usize, budget: &mut usize, files: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || *budget == 0 {
        return;
    }
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return;
    };
    if meta.is_file() && !meta.file_type().is_symlink() && dir.extension().is_some_and(|extension| extension == "md") {
        *budget -= 1;
        files.push(dir.to_owned());
        return;
    }
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .take(MAX_FILES)
        .collect();
    paths.sort();
    for path in paths {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            scan(root, &path, depth + 1, budget, files);
        } else if path.extension().is_some_and(|e| e == "md") && path.starts_with(root) {
            files.push(path);
        }
    }
}

/// All origins, including shadowed user commands, remain editable in settings.
pub fn list(
    cwd: Option<&Path>,
    home: Option<&Path>,
    plugins: &[(String, PathBuf)],
) -> Vec<CustomCommand> {
    let mut roots = Vec::new();
    if let Some(cwd) = cwd {
        roots.push((
            cwd.join(".agents/commands"),
            CommandSource::Project,
            String::new(),
        ));
    }
    if let Some(home) = home {
        roots.push((user_root(home), CommandSource::User, String::new()));
    }
    roots.extend(plugins.iter().map(|(name, path)| {
        (
            path.clone(),
            CommandSource::Plugin,
            format!("plugin:{name}:"),
        )
    }));
    let mut commands = Vec::new();
    for (root, source, prefix) in roots {
        let mut files = Vec::new();
        let mut budget = MAX_FILES;
        scan(&root, &root, 0, &mut budget, &mut files);
        for path in files {
            let base = if root.is_file() { root.parent().unwrap_or(&root) } else { &root };
            let Some(relative) = path.strip_prefix(base).ok() else {
                continue;
            };
            let name = relative
                .with_extension("")
                .iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(":")
                .to_lowercase();
            if !valid_name(&name) || reserved(&name) {
                continue;
            }
            match read(&path).and_then(|content| parse(format!("{prefix}{name}"), content, source))
            {
                Ok(command) => commands.push(command),
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "Skipping invalid command")
                }
            }
        }
    }
    commands
}

pub fn available(commands: &[CustomCommand]) -> Vec<CustomCommand> {
    let mut selected = BTreeMap::new();
    for command in commands {
        // A disabled project definition still shadows the global definition.
        selected.entry(command.name.clone()).or_insert(command);
    }
    selected
        .into_values()
        .filter(|c| c.enabled)
        .cloned()
        .collect()
}

pub fn expand(commands: &[CustomCommand], input: &str) -> Result<Option<String>, String> {
    let Some(rest) = input.trim().strip_prefix('/') else {
        return Ok(None);
    };
    let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let name = name.to_lowercase();
    if reserved(&name) {
        return Ok(None);
    }
    let Some(command) = commands.iter().find(|c| c.name == name || c.source == CommandSource::Plugin && c.name.strip_prefix("plugin:") == Some(name.as_str())) else {
        return Ok(None);
    };
    if !command.enabled {
        return Err(format!("Command /{name} is disabled"));
    }
    let (_, body) = parse_document(&command.content)?;
    if body.contains("!`") || body.contains("```!") {
        return Err("Dynamic shell expansion is not supported in commands".into());
    }
    let args = args.trim();
    let pattern = regex::Regex::new(r"\$ARGUMENTS\b|\$([1-9][0-9]*)").expect("constant pattern");
    // Free-form prose (for example "don't change tests") must not be parsed
    // as shell-like positional arguments unless the template uses $1, $2, ….
    let positional = if pattern.captures_iter(&body).any(|c| c.get(1).is_some()) {
        arguments(args)?
    } else {
        Vec::new()
    };
    let used = pattern.is_match(&body);
    // One replacement pass: argument contents are never expanded as templates.
    let expanded = pattern.replace_all(&body, |captures: &regex::Captures<'_>| {
        if let Some(index) = captures.get(1) {
            index
                .as_str()
                .parse::<usize>()
                .ok()
                .and_then(|n| positional.get(n - 1))
                .cloned()
                .unwrap_or_default()
        } else {
            args.to_owned()
        }
    });
    let tail = if !used && !args.is_empty() {
        format!("\n\nUser arguments:\n{args}")
    } else {
        String::new()
    };
    Ok(Some(format!(
        "Run custom command /{name}.\n\n{}{tail}",
        expanded.trim()
    )))
}

fn arguments(input: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut started = false;
    for ch in input.chars() {
        if Some(ch) == quote {
            quote = None;
        } else if quote.is_none() && matches!(ch, '\'' | '"') {
            quote = Some(ch);
            started = true;
        } else if quote.is_none() && ch.is_whitespace() {
            if started {
                args.push(std::mem::take(&mut current));
                started = false;
            }
        } else {
            current.push(ch);
            started = true;
        }
    }
    if quote.is_some() {
        return Err("Unclosed quote in command arguments".into());
    }
    if started {
        args.push(current);
    }
    Ok(args)
}

fn write_path(root: &Path, name: &str) -> Result<PathBuf, String> {
    if !valid_name(name) || reserved(name) {
        return Err("Invalid or reserved command name".into());
    }
    let mut path = root.to_path_buf();
    for part in name.split(':') {
        path.push(part);
    }
    path.set_extension("md");
    let mut current = root.to_path_buf();
    // Refuse linked managed directories so edits never escape the requested root.
    for part in std::iter::once(None).chain(path.strip_prefix(root).unwrap().components().map(Some))
    {
        if let Some(part) = part {
            current.push(part);
        }
        if let Ok(meta) = std::fs::symlink_metadata(&current) {
            if meta.file_type().is_symlink() {
                return Err("Command path contains a symbolic link".into());
            }
        }
    }
    Ok(path)
}

pub fn mutate(home: &Path, mutation: CommandMutation) -> Result<(), String> {
    let _lock = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = user_root(home);
    let (name, expected) = match &mutation {
        CommandMutation::Save {
            name,
            expected_revision,
            ..
        } => (name.clone(), expected_revision.clone()),
        CommandMutation::Delete {
            name,
            expected_revision,
        }
        | CommandMutation::SetEnabled {
            name,
            expected_revision,
            ..
        } => (name.clone(), Some(expected_revision.clone())),
    };
    let path = write_path(&root, &name)?;
    let current = match std::fs::symlink_metadata(&path) {
        Ok(_) => Some(read(&path)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    if current.as_deref().map(revision).as_deref() != expected.as_deref() {
        return Err("Command changed outside this editor; reload before saving".into());
    }
    let content = match mutation {
        CommandMutation::Delete { .. } => {
            return std::fs::remove_file(path).map_err(|e| e.to_string());
        }
        CommandMutation::Save { content, .. } => content,
        CommandMutation::SetEnabled { enabled, .. } => {
            let (mut header, body) =
                parse_document(current.as_deref().ok_or("Command does not exist")?)?;
            header.insert(
                yaml_serde::Value::String("enabled".into()),
                yaml_serde::Value::Bool(enabled),
            );
            format!(
                "---\n{}---\n{body}",
                yaml_serde::to_string(&header).map_err(|e| e.to_string())?
            )
        }
    };
    if content.len() > MAX_BYTES {
        return Err("Command exceeds 100 KB".into());
    }
    parse(name.clone(), content.clone(), CommandSource::User)?;
    let parent = path.parent().ok_or("Invalid command path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(content.as_bytes())
        .map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    if expected.is_some() {
        temp.persist(path).map_err(|e| e.error.to_string())?;
    } else {
        temp.persist_noclobber(path)
            .map_err(|e| e.error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn put(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    #[test]
    fn discovery_roots_shadowing_plugins_and_fresh_reads() {
        let home = tempfile::tempdir().unwrap();
        let outer = tempfile::tempdir().unwrap();
        let project = outer.path().join("workspace");
        std::fs::create_dir(&project).unwrap();
        put(
            &outer.path().join(".agents/commands/parent.md"),
            "Do not load parents",
        );
        put(
            &project.join(".agents/commands/review.md"),
            "Project review",
        );
        put(
            &project.join(".agents/commands/frontend/check.md"),
            "Check frontend",
        );
        put(
            &project.join(".agents/commands/compact.md"),
            "Cannot override built-in",
        );
        put(&user_root(home.path()).join("review.md"), "User review");
        let plugin = tempfile::tempdir().unwrap();
        put(&plugin.path().join("review.md"), "Plugin review");
        let plugins = vec![("kit".into(), plugin.path().to_path_buf())];
        let rows = list(Some(&project), Some(home.path()), &plugins);
        assert_eq!(rows.len(), 4);
        assert_eq!(available(&rows).len(), 3);
        assert!(
            expand(&rows, "/REVIEW")
                .unwrap()
                .unwrap()
                .contains("Project review")
        );
        assert!(
            expand(&rows, "/plugin:kit:review")
                .unwrap()
                .unwrap()
                .contains("Plugin review")
        );
        assert!(rows.iter().any(|r| r.name == "frontend:check"));
        assert!(
            !rows
                .iter()
                .any(|r| r.name == "parent" || r.name == "compact")
        );
        put(
            &project.join(".agents/commands/review.md"),
            "---\nenabled: false\n---\nDisabled project",
        );
        let next = list(Some(&project), Some(home.path()), &plugins);
        assert!(expand(&next, "/review").unwrap_err().contains("disabled"));
        assert!(!available(&next).iter().any(|r| r.name == "review"));
        assert_eq!(
            available(&list(None, Some(home.path()), &[]))[0].source,
            CommandSource::User
        );
    }
    #[test]
    fn parameters_are_expanded_once_and_preserve_windows_paths() {
        let command = parse("test".into(), "---\ndescription: Test\nargument-hint: '[scope]'\n---\nAll: $ARGUMENTS\nFirst: $1\nSecond: $2\nMissing: $9".into(), CommandSource::User).unwrap();
        let expanded = expand(&[command], r#"/test "C:\my project\src" '$2'"#)
            .unwrap()
            .unwrap();
        assert!(expanded.contains(r"First: C:\my project\src"));
        assert!(expanded.contains("Second: $2"));
        assert!(expanded.ends_with("Missing:"));
        assert!(arguments("'unclosed").is_err());
        let plain = parse("plain".into(), "Do the task".into(), CommandSource::User).unwrap();
        assert!(expand(std::slice::from_ref(&plain), "/plain don't change tests").unwrap().unwrap().ends_with("don't change tests"));
        assert!(
            expand(&[plain], "/plain auth")
                .unwrap()
                .unwrap()
                .ends_with("User arguments:\nauth")
        );
    }
    #[test]
    fn writes_enable_conflicts_delete_and_path_validation() {
        let home = tempfile::tempdir().unwrap();
        let save = |content: &str, revision| CommandMutation::Save {
            name: "frontend:review".into(),
            content: content.into(),
            expected_revision: revision,
        };
        let initial =
            "---\ndescription: Review\nargument-hint: '[scope]'\n---\nReview $ARGUMENTS\n";
        mutate(home.path(), save(initial, None)).unwrap();
        assert!(user_root(home.path()).join("frontend/review.md").exists());
        assert!(mutate(home.path(), save("overwrite", None)).is_err());
        let command = list(None, Some(home.path()), &[]).remove(0);
        mutate(
            home.path(),
            CommandMutation::SetEnabled {
                name: command.name.clone(),
                enabled: false,
                expected_revision: command.revision.clone(),
            },
        )
        .unwrap();
        let disabled = list(None, Some(home.path()), &[]).remove(0);
        assert!(!disabled.enabled);
        assert_eq!(disabled.description, "Review");
        assert_eq!(disabled.input_hint.as_deref(), Some("[scope]"));
        assert!(disabled.content.contains("Review $ARGUMENTS"));
        assert!(mutate(home.path(), save("stale", Some(command.revision))).is_err());
        mutate(home.path(), save("New prompt", Some(disabled.revision))).unwrap();
        let updated = list(None, Some(home.path()), &[]).remove(0);
        mutate(
            home.path(),
            CommandMutation::Delete {
                name: updated.name,
                expected_revision: updated.revision,
            },
        )
        .unwrap();
        assert!(list(None, Some(home.path()), &[]).is_empty());
        for name in [
            "../escape",
            "a::b",
            "C:\\test",
            "compact",
            "c",
            "core:compact",
        ] {
            assert!(
                mutate(
                    home.path(),
                    CommandMutation::Save {
                        name: name.into(),
                        content: "x".into(),
                        expected_revision: None
                    }
                )
                .is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn enabling_preserves_yaml_metadata_types_tags_and_alias_values() {
        let home = tempfile::tempdir().unwrap();
        let initial = r#"---
description: &description !text Review
argument-hint: *description
enabled: true
metadata:
  quoted: 'true'
  number: 42
  items: [true, '012', !kind value]
  ? [x, y]
  : complex-key
---
Review $ARGUMENTS
Keep this body.
"#;
        mutate(
            home.path(),
            CommandMutation::Save {
                name: "review".into(),
                content: initial.into(),
                expected_revision: None,
            },
        )
        .unwrap();
        let command = list(None, Some(home.path()), &[]).remove(0);
        assert_eq!(command.description, "Review");
        assert_eq!(command.input_hint.as_deref(), Some("Review"));
        mutate(
            home.path(),
            CommandMutation::SetEnabled {
                name: command.name,
                enabled: false,
                expected_revision: command.revision,
            },
        )
        .unwrap();
        let disabled = list(None, Some(home.path()), &[]).remove(0);
        assert!(!disabled.enabled);
        let (mut expected_header, expected_body) = parse_document(initial).unwrap();
        expected_header.insert("enabled".into(), false.into());
        let (actual_header, actual_body) = parse_document(&disabled.content).unwrap();
        assert_eq!(actual_header, expected_header);
        assert_eq!(actual_body, expected_body);
        assert!(disabled.content.contains("!kind"));
        for content in [
            "---\nenabled: 'false'\n---\nBody",
            "---\ndescription: 123\n---\nBody",
        ] {
            assert!(parse("bad".into(), content.into(), CommandSource::User).is_err());
        }
    }
    #[test]
    fn invalid_files_and_dynamic_shell_are_not_executed() {
        assert!(
            parse(
                "bad".into(),
                "---\ndescription: [bad\n---\nbody".into(),
                CommandSource::User
            )
            .is_err()
        );
        assert!(parse("empty".into(), " \n".into(), CommandSource::User).is_err());
        let command = parse(
            "shell".into(),
            "Use !`git status`".into(),
            CommandSource::User,
        )
        .unwrap();
        assert!(expand(&[command], "/shell").unwrap_err().contains("shell"));
        let home = tempfile::tempdir().unwrap();
        assert!(
            mutate(
                home.path(),
                CommandMutation::Save {
                    name: "large".into(),
                    content: "x".repeat(MAX_BYTES + 1),
                    expected_revision: None
                }
            )
            .is_err()
        );
    }
}
