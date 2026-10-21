//! `DiscoverSkillsTool` and `SkillTool` — finding and loading skills.
//!
//! Two tools rather than one so the model pays for the catalogue only when it
//! wants it: a skill's body is an instruction, and putting every body in every
//! request would spend the context window on instructions the turn does not
//! need. Discovery returns names and descriptions; loading returns the body.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::truncate;
use crate::skills::{self, Skill};
use crate::tool::{Tool, ToolContext, ToolError, ToolResult};

/// Skill bodies are instructions; a very long one is a red flag, and past this
/// it costs more context than it can repay.
const MAX_BODY_CHARS: usize = 100_000;

/// The skills the user switched off, keyed by skill directory — the same keys
/// the settings file stores and the harness listing flags.
///
/// Shared by both tools behind an `Arc` so one per-run read of the settings
/// feeds both. An empty set (the default) means every skill is available,
/// which is what a host that does not track settings should do.
pub type DisabledSkills = std::sync::Arc<std::collections::BTreeSet<String>>;

/// Whether `skill` may reach the model.
///
/// Keyed by directory, not name, to match how a disable is stored: two roots
/// can hold a skill of the same name (one shadowing the other), and switching
/// one off must not hide the other.
fn is_available(disabled: &DisabledSkills, skill: &Skill) -> bool {
    !disabled.contains(skill.dir.to_string_lossy().as_ref())
}

/// Drop the skills the user switched off.
///
/// Filtering happens here rather than at registration so the *live* settings
/// are honoured: the tools are built once per run with the set read from the
/// same `AgentSettings` load the rest of the run's capability gates use.
fn available_skills(disabled: &DisabledSkills, cwd: &std::path::Path, extra: &[std::path::PathBuf]) -> Vec<Skill> {
    skills::discover_with(cwd, extra)
        .into_iter()
        .filter(|skill| is_available(disabled, skill))
        .collect()
}

pub struct DiscoverSkillsTool {
    disabled: DisabledSkills,
    /// Directories installed plugins contribute. The scan reads them after the
    /// project's and the user's roots, the same precedence the listings use.
    plugin_roots: Arc<[std::path::PathBuf]>,
}

impl DiscoverSkillsTool {
    pub fn new(disabled: DisabledSkills) -> Self {
        Self {
            disabled,
            plugin_roots: Arc::from([]),
        }
    }

    /// The same tool, also reading the plugin roots a run knows about.
    pub fn with_plugin_roots(
        disabled: DisabledSkills,
        plugin_roots: Vec<std::path::PathBuf>,
    ) -> Self {
        Self {
            disabled,
            plugin_roots: Arc::from(plugin_roots),
        }
    }
}

impl Default for DiscoverSkillsTool {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

#[async_trait]
impl Tool for DiscoverSkillsTool {
    fn name(&self) -> &str {
        "DiscoverSkillsTool"
    }

    fn description(&self) -> &str {
        "List the skills available in this project and for this user. Use it \
         when the task matches the kind of work a project might have written \
         down a procedure for — a release, a review, a schema migration. Then \
         load the one you need with SkillTool."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn call(&self, _input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let found = available_skills(&self.disabled, &ctx.cwd, &self.plugin_roots);
        if found.is_empty() {
            // Not an error: most projects have no skills, and saying so lets
            // the model stop looking.
            return Ok(ToolResult::ok(
                "No skills are available here. Look for a `skills/` directory in the \
                 project or ask the user, rather than assuming a procedure exists.",
            ));
        }
        Ok(ToolResult::ok(render_catalogue(&found)))
    }
}

pub struct SkillTool {
    disabled: DisabledSkills,
    plugin_roots: Arc<[std::path::PathBuf]>,
}

impl SkillTool {
    pub fn new(disabled: DisabledSkills) -> Self {
        Self {
            disabled,
            plugin_roots: Arc::from([]),
        }
    }

    pub fn with_plugin_roots(
        disabled: DisabledSkills,
        plugin_roots: Vec<std::path::PathBuf>,
    ) -> Self {
        Self {
            disabled,
            plugin_roots: Arc::from(plugin_roots),
        }
    }
}

impl Default for SkillTool {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "SkillTool"
    }

    fn description(&self) -> &str {
        "Load a skill's instructions by name. The body replaces nothing — it is \
         guidance to follow alongside the user's request, so read it before \
         acting. Discover the names with DiscoverSkillsTool first."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The skill's name, as DiscoverSkillsTool listed it."
                }
            },
            "required": ["name"]
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let name = input
            .get("name").or_else(|| input.get("skill"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| ToolError::InvalidInput("`name` is required".to_owned()))?;

        let found = available_skills(&self.disabled, &ctx.cwd, &self.plugin_roots);
        let exact = found.iter().find(|skill| skill.name == name);
        let mut suffixes = found.iter().filter(|skill| skill.name.rsplit_once(':').is_some_and(|(_, suffix)| suffix == name));
        let unique = suffixes.next().filter(|_| suffixes.next().is_none());
        let Some(skill) = exact.or(unique) else {
            // Name what *is* available: a model that guessed wrongly should not
            // have to spend another call to find out. A switched-off skill is
            // named the same way as a missing one — to the model there is no
            // difference, which is the point of having switched it off.
            let available = if found.is_empty() {
                "no skills are available here".to_owned()
            } else {
                format!(
                    "available: {}",
                    found
                        .iter()
                        .map(|skill| skill.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            return Err(ToolError::NotFound(format!(
                "no skill named `{name}` ({available})"
            )));
        };

        let body = skills::read_body(skill).map_err(|error| {
            ToolError::Execution(format!(
                "cannot read `{}`: {error}",
                skill.manifest().display()
            ))
        })?;
        let (body, truncated) = truncate(&body, MAX_BODY_CHARS);
        let note = if truncated { "\n\n… truncated" } else { "" };
        let host_tools = if skill.source == skills::SkillSource::Plugin {
            "RCode tools: SkillTool loads skills, Task delegates work, and TodoWrite tracks tasks. Bash calls wait for completion; start persistent services with their supported background option (for example --background), with output redirected by the launcher.\n\n"
        } else { "" };
        Ok(ToolResult::ok(format!(
            "# Skill: {}\nBase directory: {}\n\n{host_tools}{body}{note}",
            skill.name, skill.dir.display()
        )))
    }
}

/// Render the catalogue the way the model reads best: name first (it is what
/// it will pass back), description second, source last so a person scanning
/// the transcript can tell where each came from.
fn render_catalogue(skills: &[Skill]) -> String {
    let mut out = format!("{} skill(s) available:\n\n", skills.len());
    for skill in skills {
        use std::fmt::Write as _;
        let _ = writeln!(
            out,
            "- **{}** ({}) — {}",
            skill.name,
            skill.source.label(),
            skill.description
        );
    }
    out.push_str("\nLoad one with SkillTool.");
    out
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

    fn write_skill(cwd: &std::path::Path, name: &str, description: &str) {
        let dir = cwd.join(".agents").join("skills").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\nFollow these steps.\n"),
        )
        .unwrap();
    }

    /// The disabled key for a project skill, spelled exactly the way the
    /// harness listing spells `Skill::path` (`project_skills_dir` joins the
    /// same constant the scan joins, so the two strings are identical).
    fn disabled(dir: &std::path::Path, name: &str) -> DisabledSkills {
        std::sync::Arc::new(
            [crate::skills::project_skills_dir(dir)
                .join(name)
                .to_string_lossy()
                .into_owned()]
            .into_iter()
            .collect(),
        )
    }

    #[tokio::test]
    async fn a_project_skill_is_listed() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "release", "Cut a release.");

        let result = DiscoverSkillsTool::default()
            .call(json!({}), &context(dir.path()))
            .await
            .expect("lists");

        assert!(result.output.contains("release"));
        assert!(result.output.contains("Cut a release."));
        assert!(result.output.contains("project"));
    }

    #[tokio::test]
    async fn no_skills_says_so_without_failing() {
        let dir = tempfile::tempdir().unwrap();
        let result = DiscoverSkillsTool::default()
            .call(json!({}), &context(dir.path()))
            .await
            .expect("lists");
        assert!(!result.is_error);
        assert!(result.output.contains("No skills are available"));
    }

    #[tokio::test]
    async fn a_skill_body_loads_without_its_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "release", "Cut a release.");

        let result = SkillTool::default()
            .call(json!({ "name": "release" }), &context(dir.path()))
            .await
            .expect("loads");

        assert!(result.output.contains("Follow these steps."));
        assert!(!result.output.contains("description:"));
    }

    #[tokio::test]
    async fn an_unknown_name_lists_what_is_available() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "release", "Cut a release.");

        let error = SkillTool::default()
            .call(json!({ "name": "nope" }), &context(dir.path()))
            .await
            .expect_err("unknown");

        assert!(matches!(error, ToolError::NotFound(_)));
        // The model should not need a second call just to learn the names.
        assert!(error.to_string().contains("release"));
    }

    #[tokio::test]
    async fn a_missing_name_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let error = SkillTool::default()
            .call(json!({}), &context(dir.path()))
            .await
            .expect_err("no name");
        assert!(matches!(error, ToolError::InvalidInput(_)));
    }

    // === Disabled skills ===

    #[tokio::test]
    async fn a_disabled_skill_is_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "release", "Cut a release.");
        write_skill(dir.path(), "review", "Review a diff.");

        let result = DiscoverSkillsTool::new(disabled(dir.path(), "release"))
            .call(json!({}), &context(dir.path()))
            .await
            .expect("lists");

        assert!(
            !result.output.contains("Cut a release."),
            "{}",
            result.output
        );
        assert!(result.output.contains("review"), "{}", result.output);
    }

    #[tokio::test]
    async fn a_disabled_skill_cannot_be_loaded() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "release", "Cut a release.");

        let error = SkillTool::new(disabled(dir.path(), "release"))
            .call(json!({ "name": "release" }), &context(dir.path()))
            .await
            .expect_err("disabled");

        // Switched off reads as absent: the refusal is the "nothing here"
        // shape, with no catalogue of names that includes the hidden skill.
        assert!(matches!(error, ToolError::NotFound(_)));
        assert!(error.to_string().contains("no skills are available"));
    }

    #[tokio::test]
    async fn all_skills_disabled_reads_as_no_skills() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "release", "Cut a release.");

        let result = DiscoverSkillsTool::new(disabled(dir.path(), "release"))
            .call(json!({}), &context(dir.path()))
            .await
            .expect("lists");

        assert!(!result.is_error);
        assert!(result.output.contains("No skills are available"));
    }

    #[tokio::test]
    async fn a_same_named_skill_in_the_other_root_stays_available() {
        // The key is the skill's directory: disabling the project's copy of a
        // name must not hide the user's copy of it.
        let project = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        write_skill(project.path(), "release", "The project's version.");
        let user_skills = user.path().join(".rcode").join("skills");
        std::fs::create_dir_all(user_skills.join("release")).unwrap();
        std::fs::write(
            user_skills.join("release").join("SKILL.md"),
            "---\nname: release\ndescription: The user's version.\n---\n\nUser steps.\n",
        )
        .unwrap();

        let result = DiscoverSkillsTool::new(disabled(project.path(), "release"))
            .call(json!({}), &context(project.path()))
            .await
            .expect("lists");
        assert!(!result.output.contains("The project's version."));

        // The user's copy is a different directory, so a different key: the
        // same disable must leave it available to the filter.
        let user_copy =
            crate::skills::discover_in(&user_skills, crate::skills::SkillSource::User).remove(0);
        assert!(is_available(&Default::default(), &user_copy));
        let user_key = user
            .path()
            .join(".rcode")
            .join("skills")
            .join("release")
            .to_string_lossy()
            .into_owned();
        assert!(!is_available(
            &std::sync::Arc::new([user_key].into_iter().collect()),
            &user_copy
        ));
    }
}
