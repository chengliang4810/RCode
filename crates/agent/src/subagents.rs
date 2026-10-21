//! Subagents: named agent definitions the model can delegate a task to.
//!
//! A definition is a Markdown file whose YAML frontmatter names the agent and
//! describes when to use it, and whose body is the system prompt the child
//! agent runs under:
//!
//! ```text
//! ---
//! name: code-reviewer
//! description: Review a diff for correctness. Use after writing code.
//! tools: Read, Grep, Glob
//! disallowedTools: Bash
//! model: inherit
//! ---
//! You are a code reviewer. ...
//! ```
//!
//! Two roots, project first so a project can shadow a personal definition —
//! the same precedence the skills scan applies:
//!
//! - `{cwd}/.rcode/agents`
//! - `{home}/.rcode/agents`
//!
//! The scan is the security boundary for what the model can delegate to, so it
//! is deliberately stricter than the skills walk: definitions are *flat* — one
//! directory of `.md` files, no recursion — symlinks are refused, and both the
//! file size and the file count are bounded. A stray `node_modules` under an
//! agents directory can therefore never turn the scan into a walk of the
//! machine.

use std::path::{Path, PathBuf};

use crate::skills::{SkillError, SkillSource, home_dir, split_frontmatter, validate_name};

/// Definition files per root. Bounds a root that turns out to be a dump.
const MAX_FILES_PER_ROOT: usize = 200;
/// A definition larger than this is not a system prompt, and reading it would
/// cost more than it can repay.
pub const MAX_DEFINITION_BYTES: u64 = 512 * 1024;

/// Where a subagent definition came from.
///
/// The same two managed roots the skills scan uses, and the same wire
/// vocabulary ([`SkillSource`] converts to and from
/// `rcode_proto::invocation::SkillScope`), because "project's copy vs the
/// person's copy" is one question whichever kind of file answers it.
pub type DefinitionSource = SkillSource;

/// The project's agents directory, relative to its root.
pub const PROJECT_AGENTS_DIR: &str = ".rcode/agents";

/// The model a child agent runs on, as the definition states it.
///
/// `inherit` follows the conversation that
/// spawned the child; explicit provider/model pairs can select another model.
/// Resolving a selection to a configured model is the host's
/// job — this crate owns no provider file — so the alias travels to the
/// [`crate::tools::SubagentRuntime`] as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ModelAlias {
    /// Whatever model the parent conversation is talking to.
    #[default]
    Inherit,
    /// A provider/model pair encoded as `model:[providerId,modelId]`.
    Custom(String),
}

impl ModelAlias {
    /// The alias's wire/frontmatter spelling.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Inherit => "inherit",
            Self::Custom(value) => value,
        }
    }

    /// Parse a frontmatter `model:` value, case-insensitively.
    ///
    /// `None` for anything outside the vocabulary — including a raw model id.
    /// A definition naming an unknown model is skipped rather than silently
    /// run on another model: an alias that meant "the cheap one" running on the
    /// flagship is a cost surprise, and the log line says why the definition
    /// was left out.
    pub fn parse(value: &str) -> Option<Self> {
        if let Some(raw) = value.trim().strip_prefix("model:") {
            let pair: [String; 2] = if let Ok((provider, model, _)) = serde_json::from_str::<(String, String, rcode_proto::ReasoningLevel)>(raw) {
                [provider, model]
            } else {
                serde_json::from_str(raw).ok()?
            };
            if pair.iter().any(|part| part.trim().is_empty()) {
                return None;
            }
            return Some(Self::Custom(value.trim().to_owned()));
        }
        match value.trim().to_ascii_lowercase().as_str() {
            // Compatibility for definitions written before virtual models were removed.
            "inherit" | "haiku" | "sonnet" | "opus" | "fable" => Some(Self::Inherit),
            _ => None,
        }
    }
}

/// A subagent definition found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentDefinition {
    /// The frontmatter `name`, which is what the model passes to `Agent`.
    pub name: String,
    pub description: String,
    /// Tools the child may use. Empty means "everything the parent has" — an
    /// allowlist is opt-in, because a stale name on it silently disables a
    /// tool the author meant to keep.
    pub tools: Vec<String>,
    /// Tools the child must never see. Applied after `tools`.
    pub disallowed_tools: Vec<String>,
    /// Which model the child runs on.
    pub model: ModelAlias,
    /// The body: the child's system prompt.
    pub system_prompt: String,
    /// The definition file itself — the stable identity the enable and model
    /// overrides are keyed by.
    pub path: PathBuf,
    pub source: DefinitionSource,
}

impl SubagentDefinition {
    /// Whether `tool` may reach the child's registry.
    ///
    /// A name comparison, case-insensitively: tool names are PascalCase, and a
    /// definition that spells `bash` should still hit `Bash`. The allowlist
    /// wins nothing — it only narrows — and the denylist always wins.
    pub fn allows(&self, tool: &str) -> bool {
        let canonical = |name: &str| match name.to_ascii_lowercase().as_str() {
            "skilltool" | "skill" => "skill".to_owned(), other => other.to_owned()
        };
        let matches = |listed: &String| canonical(listed) == canonical(tool);
        if !self.tools.is_empty() && !self.tools.iter().any(|listed| listed == "*" || matches(listed)) {
            return false;
        }
        !self.disallowed_tools.iter().any(matches)
    }
}

/// The roots to scan, in precedence order: project before user.
///
/// [`agents_dir`] builds write targets from the same two shapes, so a created
/// definition is one the scan finds.
pub fn roots(cwd: &Path, home: Option<&Path>) -> Vec<(PathBuf, DefinitionSource)> {
    let mut roots = vec![(project_agents_dir(cwd), DefinitionSource::Project)];
    if let Some(home) = home {
        roots.push((user_agents_dir(home), DefinitionSource::User));
    }
    roots
}

/// The project's agents directory.
pub fn project_agents_dir(cwd: &Path) -> PathBuf {
    cwd.join(PROJECT_AGENTS_DIR)
}

/// The person's own agents directory: `{home}/.rcode/agents`.
///
/// Takes the home directory — the same argument `crate::skills::user_skills_dir`
/// takes — so the two user roots resolve identically, and `.rcode` here is the
/// config root the storage docs name.
pub fn user_agents_dir(home: &Path) -> PathBuf {
    home.join(".rcode").join("agents")
}

/// The one agents directory a scope writes to.
///
/// The write side of [`roots`], mirroring `crate::skills::scope_root`: the
/// caller names a *scope*, never a path, and the root is derived here so a
/// request cannot aim a write outside the two directories this module manages.
///
/// `None` is the user scope with no readable home — there is nowhere a user
/// definition could live, and inventing a location would be worse than
/// refusing.
pub fn scope_root(cwd: &Path, source: DefinitionSource, home: Option<&Path>) -> Option<PathBuf> {
    match source {
        DefinitionSource::Project => Some(project_agents_dir(cwd)),
        DefinitionSource::User => home.map(user_agents_dir),
        // A plugin's definitions are read-only, like its skills.
        DefinitionSource::Plugin => None,
    }
}

/// The file a definition of `name` is written to, under `root`.
///
/// The name is validated before it becomes a path segment, so the join can
/// never escape the root.
pub fn definition_file(root: &Path, name: &str) -> Result<PathBuf, SkillError> {
    validate_name(name)?;
    Ok(root.join(format!("{}.md", name.trim())))
}

/// Find every definition under `cwd`, project roots first.
///
/// A missing root is not an error — most projects have no subagents. A file
/// that cannot be parsed is skipped with a log line rather than failing the
/// scan: one malformed definition should not hide the others.
///
/// Project before user, and the first occurrence of a name wins, so a project
/// definition shadows the person's of the same name — the model then sees one
/// entry, not two to choose between.
pub fn discover(cwd: &Path) -> Vec<SubagentDefinition> {
    discover_with(cwd, home_dir().as_deref())
}

/// [`discover`] with the home directory named, for a caller that already has
/// one (or a test that wants a temporary one).
pub fn discover_with(cwd: &Path, home: Option<&Path>) -> Vec<SubagentDefinition> {
    scan(roots(cwd, home))
}

/// [`discover`] plus the directories installed plugins contribute.
///
/// Plugin roots come last, so a project's own definition still wins a name
/// collision — the same precedence skills use.
pub fn discover_with_plugins(cwd: &Path, plugin_roots: &[PathBuf]) -> Vec<SubagentDefinition> {
    let mut all = roots(cwd, home_dir().as_deref());
    all.extend(
        plugin_roots
            .iter()
            .map(|root| (root.clone(), DefinitionSource::Plugin)),
    );
    scan(all)
}

/// Personal settings omit project definitions before resolving name collisions.
pub fn discover_personal(home: Option<&Path>, plugin_roots: &[PathBuf]) -> Vec<SubagentDefinition> {
    let mut all = home
        .map(|home| vec![(user_agents_dir(home), DefinitionSource::User)])
        .unwrap_or_default();
    all.extend(
        plugin_roots
            .iter()
            .map(|root| (root.clone(), DefinitionSource::Plugin)),
    );
    scan(all)
}

fn scan(roots: Vec<(PathBuf, DefinitionSource)>) -> Vec<SubagentDefinition> {
    let mut found: Vec<SubagentDefinition> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (root, source) in roots {
        let namespace = if source == DefinitionSource::Plugin { crate::skills::plugin_namespace(&root) } else { None };
        for mut definition in discover_in(&root, source) {
            if let Some(namespace) = &namespace { definition.name = format!("{namespace}:{}", definition.name); }
            if seen.insert(definition.name.clone()) {
                found.push(definition);
            } else {
                tracing::debug!(
                    agent = %definition.name,
                    root = %root.display(),
                    "a subagent definition with this name was already found; ignoring this one"
                );
            }
        }
    }
    found
}

/// Find every definition in one root, tagged `source`.
///
/// The scope-addressed half of [`discover`]: a caller that already knows which
/// scope it wants (delete the user's copy, update the project's) scans only
/// that root, so the shadowed copy in the other root stays out of the result.
pub fn discover_in(root: &Path, source: DefinitionSource) -> Vec<SubagentDefinition> {
    let mut found = Vec::new();
    if root.is_file() && root.extension().is_some_and(|extension| extension == "md") {
        if std::fs::symlink_metadata(root).is_ok_and(|metadata| !metadata.file_type().is_symlink()) {
            if let Some(definition) = read_definition(root, source) { found.push(definition); }
        }
        return found;
    }

    let Ok(entries) = std::fs::read_dir(root) else {
        return found;
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        // Symlinks are refused outright: a link can point outside the root,
        // and a scan that follows them reads whatever it points at.
        .filter(|entry| {
            !entry
                .file_type()
                .map(|kind| kind.is_symlink())
                .unwrap_or(true)
        })
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && path.extension().is_some_and(|extension| extension == "md")
        })
        .collect();
    // Sorted so a listing is stable: the file count is bounded, but a
    // directory order that changes between runs would still churn every
    // caller that shows the list.
    files.sort();
    files.truncate(MAX_FILES_PER_ROOT);
    for path in files {
        if found.len() >= MAX_FILES_PER_ROOT {
            break;
        }
        if let Some(definition) = read_definition(&path, source) {
            found.push(definition);
        }
    }
    found
}

/// Parse one definition file. `None` when it is not a definition.
fn read_definition(path: &Path, source: DefinitionSource) -> Option<SubagentDefinition> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return None;
    };
    if metadata.len() > MAX_DEFINITION_BYTES {
        tracing::warn!(
            path = %path.display(),
            limit = MAX_DEFINITION_BYTES,
            "subagent definition is larger than the limit; skipping it"
        );
        return None;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return None;
    };
    let (frontmatter, body) = split_frontmatter(&text)?;
    let parsed: serde_yaml_ng::Value = serde_yaml_ng::from_str(frontmatter).ok()?;
    let map = parsed.as_mapping()?;

    let string = |key: &str| -> Option<String> {
        map.get(serde_yaml_ng::Value::String(key.to_owned()))
            .and_then(|value| value.as_str())
            .map(str::to_owned)
    };
    let names = |key: &str| -> Vec<String> {
        map.get(serde_yaml_ng::Value::String(key.to_owned()))
            .and_then(|value| value.as_sequence())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::trim))
                    .filter(|item| !item.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };

    let name = string("name")?;
    if name.trim().is_empty() {
        return None;
    }
    if name.contains('/') || name.contains('\\') || name.chars().any(char::is_whitespace) {
        tracing::warn!(
            agent = %name,
            "a subagent name must not contain whitespace or path separators; skipping"
        );
        return None;
    }
    let description = string("description").unwrap_or_default();
    let model = match string("model") {
        Some(raw) => match ModelAlias::parse(&raw) {
            Some(alias) => alias,
            None => {
                tracing::warn!(
                    agent = %name,
                    model = %raw,
                    "unknown subagent model (want inherit or a provider/model selection); skipping"
                );
                return None;
            }
        },
        None => ModelAlias::Inherit,
    };
    Some(SubagentDefinition {
        name,
        description,
        tools: names("tools"),
        disallowed_tools: names("disallowedTools"),
        model,
        system_prompt: body.trim().to_owned(),
        path: path.to_path_buf(),
        source,
    })
}

/// Create a definition, returning the file it wrote.
///
/// `description` and `system_prompt` are both required: the description is
/// what the model reads when deciding to delegate, and the body is what the
/// child is told to be — a definition missing either is one that cannot work.
///
/// Fails rather than overwriting when the name is taken, like the skills
/// writer: silently replacing a definition the user wrote is a loss with no
/// undo.
pub fn create(
    root: &Path,
    name: &str,
    description: &str,
    system_prompt: &str,
    tools: &[String],
    disallowed_tools: &[String],
    model: ModelAlias,
) -> Result<PathBuf, SkillError> {
    validate_name(name)?;
    if description.trim().is_empty() {
        return Err(SkillError::Malformed(
            "a description is required: it is what the model reads when deciding whether to \
             delegate to this agent"
                .to_owned(),
        ));
    }
    if system_prompt.trim().is_empty() {
        return Err(SkillError::Malformed(
            "a system prompt is required: it is what the subagent is told to be".to_owned(),
        ));
    }
    let path = definition_file(root, name)?;
    if path.exists() {
        return Err(SkillError::Exists(name.trim().to_owned()));
    }
    std::fs::create_dir_all(root)?;
    std::fs::write(
        &path,
        render(
            name,
            description,
            tools,
            disallowed_tools,
            model,
            system_prompt,
        ),
    )?;
    Ok(path)
}

/// The definition file's text, frontmatter and body.
///
/// Written by hand rather than through a YAML serializer, like the skills
/// writer: the file is something the user will open and edit, and a minimal,
/// predictable shape is worth more there than generality. Lists render one
/// item per line, which reads the same in every editor.
fn render(
    name: &str,
    description: &str,
    tools: &[String],
    disallowed_tools: &[String],
    model: ModelAlias,
    system_prompt: &str,
) -> String {
    // A description with a `:` or a leading quote needs quoting to stay valid
    // YAML; the plain-true case stays bare so hand edits stay pleasant.
    let quote = |value: &str| -> String {
        let needs = value.contains(": ")
            || value.ends_with(':')
            || value.split_whitespace().any(|word| {
                word.eq_ignore_ascii_case("true") || word.eq_ignore_ascii_case("false")
            });
        if needs {
            format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
        } else {
            value.to_owned()
        }
    };
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("name: {}\n", quote(name.trim())));
    out.push_str(&format!("description: {}\n", quote(description.trim())));
    if !tools.is_empty() {
        out.push_str("tools:\n");
        for tool in tools {
            out.push_str(&format!("  - {}\n", tool.trim()));
        }
    }
    if !disallowed_tools.is_empty() {
        out.push_str("disallowedTools:\n");
        for tool in disallowed_tools {
            out.push_str(&format!("  - {}\n", tool.trim()));
        }
    }
    if model != ModelAlias::Inherit {
        out.push_str(&format!("model: {}\n", model.as_str()));
    }
    out.push_str("---\n\n");
    out.push_str(system_prompt.trim());
    out.push('\n');
    out
}

/// Delete a definition file, if it lives directly under a directory this
/// application manages.
///
/// Containment is checked against the canonicalised root, and only a file
/// *directly* in it passes — the scan is flat, so a nested `.md` is not a
/// definition this module would have listed, and deleting one is not a
/// settings-page action.
pub fn delete(path: &Path, root: &Path) -> Result<(), SkillError> {
    let root = std::fs::canonicalize(root)
        .map_err(|_| SkillError::NotWritable(format!("`{}` no longer exists", root.display())))?;
    let actual = std::fs::canonicalize(path)
        .map_err(|_| SkillError::NotWritable(format!("`{}` no longer exists", path.display())))?;
    if actual.parent().as_deref() != Some(root.as_path()) {
        return Err(SkillError::NotWritable(format!(
            "`{}` is not a definition this application manages",
            path.display()
        )));
    }
    std::fs::remove_file(&actual)?;
    Ok(())
}

/// Rewrite a definition's frontmatter, and optionally its body, in place.
///
/// The file keeps its name on disk — a rename of the *identity* (`name:`)
/// does not move the file, exactly as a skill rename keeps its directory. A
/// `system_prompt` of `None` keeps the body the file carries, which is the
/// common edit.
pub fn update(
    path: &Path,
    name: &str,
    description: &str,
    tools: &[String],
    disallowed_tools: &[String],
    model: ModelAlias,
    system_prompt: Option<&str>,
) -> Result<(), SkillError> {
    validate_name(name)?;
    if description.trim().is_empty() {
        return Err(SkillError::Malformed(
            "a description is required".to_owned(),
        ));
    }
    let text = std::fs::read_to_string(path)?;
    let body = match system_prompt {
        Some(body) => body.trim().to_owned(),
        None => {
            let (_, current) = split_frontmatter(&text).ok_or_else(|| {
                SkillError::Malformed(format!(
                    "`{}` has no frontmatter block to rewrite",
                    path.display()
                ))
            })?;
            current.trim().to_owned()
        }
    };
    if body.is_empty() {
        return Err(SkillError::Malformed(
            "a system prompt is required".to_owned(),
        ));
    }
    std::fs::write(
        path,
        render(name, description, tools, disallowed_tools, model, &body),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_definition(root: &Path, file: &str, frontmatter: &str, body: &str) -> PathBuf {
        let path = root.join(file);
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(&path, format!("---\n{frontmatter}---\n\n{body}\n")).unwrap();
        path
    }

    fn simple(root: &Path, file: &str, name: &str) -> PathBuf {
        write_definition(
            root,
            file,
            &format!("name: {name}\ndescription: Does the thing.\n"),
            "You do the thing.",
        )
    }

    #[test]
    fn explicit_provider_model_selection_roundtrips_in_frontmatter() {
        let root = tempfile::tempdir().unwrap();
        let raw = r#"model:["provider-a","model-with/slash","high"]"#;
        let selected = ModelAlias::parse(raw).unwrap();
        create(
            root.path(),
            "selected",
            "Selected model",
            "Prompt",
            &[],
            &[],
            selected.clone(),
        )
        .unwrap();
        let definitions = discover_in(root.path(), DefinitionSource::User);
        assert_eq!(definitions[0].model, selected);
        assert_eq!(definitions[0].model.as_str(), raw);
    }

    #[test]
    fn a_definition_is_found_and_parsed() {
        let dir = tempfile::tempdir().unwrap();
        simple(
            &dir.path().join(".rcode").join("agents"),
            "reviewer.md",
            "reviewer",
        );

        let found = discover(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "reviewer");
        assert_eq!(found[0].description, "Does the thing.");
        assert_eq!(found[0].system_prompt, "You do the thing.");
        assert_eq!(found[0].model, ModelAlias::Inherit);
        assert_eq!(found[0].source, DefinitionSource::Project);
        assert!(found[0].tools.is_empty());
        assert!(found[0].disallowed_tools.is_empty());
    }

    #[test]
    fn the_frontmatter_lists_are_read() {
        let dir = tempfile::tempdir().unwrap();
        write_definition(
            &dir.path().join("agents"),
            "deploy.md",
            "name: deploy\ndescription: Ships it.\ntools:\n  - Read\n  - Bash\ndisallowedTools:\n  - Write\nmodel: Sonnet\n",
            "Ship.",
        );

        let found = discover_in(&dir.path().join("agents"), DefinitionSource::User);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].tools, ["Read".to_owned(), "Bash".to_owned()]);
        assert_eq!(found[0].disallowed_tools, ["Write".to_owned()]);
        assert_eq!(found[0].model, ModelAlias::Inherit);
    }

    #[test]
    fn a_definition_without_a_name_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write_definition(dir.path(), "bare.md", "description: No name.\n", "Body.");
        assert!(discover_in(dir.path(), DefinitionSource::Project).is_empty());
    }

    #[test]
    fn a_definition_without_frontmatter_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("plain.md"), "just text").unwrap();
        assert!(discover_in(dir.path(), DefinitionSource::Project).is_empty());
    }

    #[test]
    fn an_unknown_model_skips_the_whole_definition() {
        // A raw model id is outside the vocabulary. Running the agent on
        // whatever the parent talks to would be a silent cost surprise, so
        // the definition is left out and the log says why.
        let dir = tempfile::tempdir().unwrap();
        write_definition(
            dir.path(),
            "raw.md",
            "name: raw\ndescription: d\nmodel: gpt-5\n",
            "Body.",
        );
        assert!(discover_in(dir.path(), DefinitionSource::Project).is_empty());
    }

    #[test]
    fn model_aliases_parse_case_insensitively_and_nothing_else() {
        assert_eq!(ModelAlias::parse("inherit"), Some(ModelAlias::Inherit));
        assert_eq!(ModelAlias::parse("Opus"), Some(ModelAlias::Inherit));
        assert_eq!(ModelAlias::parse(" FABLE "), Some(ModelAlias::Inherit));
        for legacy in ["haiku", "sonnet", "opus", "fable"] {
            assert_eq!(ModelAlias::parse(legacy).unwrap().as_str(), "inherit");
        }
        let explicit = "model:[\"provider\",\"model\"]";
        assert_eq!(ModelAlias::parse(explicit).unwrap().as_str(), explicit);
        for bad in ["", "gpt-5", "claude-sonnet-4-5", "auto"] {
            assert_eq!(ModelAlias::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn non_markdown_and_symlinked_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("notes.txt"),
            "---\nname: a\ndescription: b\n---\n",
        )
        .unwrap();
        simple(dir.path(), "real.md", "real");

        let found = discover_in(dir.path(), DefinitionSource::Project);
        assert_eq!(found.len(), 1, "only the .md file counts");
        assert_eq!(found[0].name, "real");

        // A subdirectory is not scanned: definitions are flat.
        simple(&dir.path().join("nested"), "inner.md", "inner");
        assert_eq!(discover_in(dir.path(), DefinitionSource::Project).len(), 1);
    }

    #[test]
    fn an_oversized_definition_is_skipped_not_crashed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.md");
        let body = "x".repeat(600 * 1024);
        std::fs::write(
            &path,
            format!("---\nname: huge\ndescription: d\n---\n{body}"),
        )
        .unwrap();
        assert!(discover_in(dir.path(), DefinitionSource::Project).is_empty());
    }

    #[test]
    fn a_missing_root_is_not_an_error() {
        let missing = std::env::temp_dir().join("rcode-no-such-agents-root");
        assert!(discover_in(&missing, DefinitionSource::Project).is_empty());
    }

    #[test]
    fn a_project_definition_shadows_the_user_one() {
        let cwd = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        simple(
            &cwd.path().join(".rcode").join("agents"),
            "shared.md",
            "shared",
        );
        let user_dir = home.path().join(".rcode").join("agents");
        write_definition(
            &user_dir,
            "shared.md",
            "name: shared\ndescription: The person's version.\n",
            "User body.",
        );
        simple(&user_dir, "only-user.md", "only-user");

        let found = discover_with(cwd.path(), Some(home.path()));
        let shared: Vec<&SubagentDefinition> =
            found.iter().filter(|d| d.name == "shared").collect();
        assert_eq!(shared.len(), 1, "the name appears once");
        assert_eq!(shared[0].source, DefinitionSource::Project);
        assert_eq!(shared[0].description, "Does the thing.");
        // And the user-only definition is still listed.
        assert!(found.iter().any(|d| d.name == "only-user"));
    }

    #[test]
    fn roots_are_project_then_user() {
        let cwd = Path::new("/work/project");
        let home = Path::new("/home/someone");
        let roots = roots(cwd, Some(home));
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].0, project_agents_dir(cwd));
        assert!(roots[0].0.ends_with(".rcode/agents"));
        assert_eq!(roots[1].0, user_agents_dir(home));
        assert_eq!(roots[1].1, DefinitionSource::User);
    }

    #[test]
    fn scope_roots_are_the_two_managed_directories() {
        let cwd = Path::new("/work/project");
        let home = Path::new("/home/someone");
        assert_eq!(
            scope_root(cwd, DefinitionSource::Project, Some(home)),
            Some(project_agents_dir(cwd))
        );
        assert_eq!(
            scope_root(cwd, DefinitionSource::User, Some(home)),
            Some(user_agents_dir(home))
        );
        assert_eq!(scope_root(cwd, DefinitionSource::User, None), None);
    }

    #[test]
    fn a_created_definition_is_one_the_scan_finds() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".rcode").join("agents");
        let path = create(
            &root,
            "reviewer",
            "Reviews a diff.",
            "You review code.",
            &["Read".to_owned(), "Grep".to_owned()],
            &["Bash".to_owned()],
            ModelAlias::Inherit,
        )
        .expect("create");

        let found = discover_in(&root, DefinitionSource::Project);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, path);
        assert_eq!(found[0].name, "reviewer");
        assert_eq!(found[0].description, "Reviews a diff.");
        assert_eq!(found[0].system_prompt, "You review code.");
        assert_eq!(found[0].tools, ["Read".to_owned(), "Grep".to_owned()]);
        assert_eq!(found[0].disallowed_tools, ["Bash".to_owned()]);
        assert_eq!(found[0].model, ModelAlias::Inherit);
    }

    #[test]
    fn creating_twice_is_refused_rather_than_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        create(
            &dir.path(),
            "a",
            "First.",
            "One.",
            &[],
            &[],
            ModelAlias::Inherit,
        )
        .expect("first");
        let error = create(
            &dir.path(),
            "a",
            "Second.",
            "Two.",
            &[],
            &[],
            ModelAlias::Inherit,
        )
        .expect_err("refused");
        assert!(matches!(error, SkillError::Exists(name) if name == "a"));
        let found = discover_in(dir.path(), DefinitionSource::Project);
        assert_eq!(found[0].description, "First.");
    }

    #[test]
    fn creation_requires_a_description_and_a_body() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            create(
                &dir.path(),
                "a",
                "  ",
                "Body.",
                &[],
                &[],
                ModelAlias::Inherit
            )
            .expect_err("no description"),
            SkillError::Malformed(_)
        ));
        assert!(matches!(
            create(&dir.path(), "a", "d", "   ", &[], &[], ModelAlias::Inherit)
                .expect_err("no body"),
            SkillError::Malformed(_)
        ));
    }

    #[test]
    fn a_created_name_cannot_escape_the_root() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            create(
                &dir.path(),
                "../escape",
                "d",
                "b",
                &[],
                &[],
                ModelAlias::Inherit
            )
            .is_err()
        );
        assert!(definition_file(dir.path(), "../escape").is_err());
        assert!(definition_file(dir.path(), "a/b").is_err());
    }

    #[test]
    fn updating_rewrites_frontmatter_and_keeps_the_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = create(
            &dir.path(),
            "a",
            "Old.",
            "The prompt.",
            &[],
            &[],
            ModelAlias::Inherit,
        )
        .expect("create");

        update(
            &path,
            "a",
            "New.",
            &["Read".to_owned()],
            &[],
            ModelAlias::Inherit,
            None,
        )
        .expect("update");

        let found = discover_in(dir.path(), DefinitionSource::Project);
        assert_eq!(found[0].description, "New.");
        assert_eq!(found[0].tools, ["Read".to_owned()]);
        assert_eq!(found[0].model, ModelAlias::Inherit);
        assert_eq!(found[0].system_prompt, "The prompt.", "the body survived");
    }

    #[test]
    fn updating_can_replace_the_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = create(
            &dir.path(),
            "a",
            "d",
            "Old prompt.",
            &[],
            &[],
            ModelAlias::Inherit,
        )
        .expect("create");
        update(
            &path,
            "a",
            "d",
            &[],
            &[],
            ModelAlias::Inherit,
            Some("New prompt."),
        )
        .expect("update");
        let found = discover_in(dir.path(), DefinitionSource::Project);
        assert_eq!(found[0].system_prompt, "New prompt.");
    }

    #[test]
    fn updating_a_file_without_frontmatter_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.md");
        std::fs::write(&path, "no frontmatter").unwrap();
        assert!(matches!(
            update(&path, "a", "d", &[], &[], ModelAlias::Inherit, None).expect_err("refused"),
            SkillError::Malformed(_)
        ));
    }

    #[test]
    fn deleting_removes_only_a_file_directly_under_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let path = simple(dir.path(), "a.md", "a");
        delete(&path, dir.path()).expect("delete");
        assert!(!path.exists());

        // A nested file the flat scan would never list is not deletable
        // through this API.
        let nested = simple(&dir.path().join("nested"), "b.md", "b");
        assert!(matches!(
            delete(&nested, dir.path()).expect_err("refused"),
            SkillError::NotWritable(_)
        ));
        assert!(nested.exists());

        // Nor is a file in a different root.
        let elsewhere = tempfile::tempdir().unwrap();
        let other = simple(elsewhere.path(), "c.md", "c");
        assert!(delete(&other, dir.path()).is_err());
        assert!(other.exists());
    }

    #[test]
    fn the_tool_filters_read_as_the_model_would_spell_them() {
        let definition = SubagentDefinition {
            name: "a".into(),
            description: String::new(),
            tools: vec!["Read".into(), "Grep".into()],
            disallowed_tools: vec!["Bash".into()],
            model: ModelAlias::Inherit,
            system_prompt: String::new(),
            path: PathBuf::new(),
            source: DefinitionSource::Project,
        };
        assert!(definition.allows("Read"));
        assert!(definition.allows("read"), "spelling is forgiving");
        assert!(!definition.allows("Write"), "not on the allowlist");
        assert!(!definition.allows("Bash"));

        // Without an allowlist everything (denied excepted) passes.
        let open = SubagentDefinition {
            tools: Vec::new(),
            ..definition.clone()
        };
        assert!(open.allows("Write"));
        assert!(!open.allows("Bash"), "the denylist always wins");

        // An allowlist entry and a denylist entry for the same tool: the
        // denylist wins.
        let conflicted = SubagentDefinition {
            tools: vec!["Bash".into()],
            ..definition
        };
        assert!(!conflicted.allows("Bash"));
    }
}
