//! Installed plugins: what they contribute, and where they live.
//!
//! A plugin is a directory under `{config root}/plugins/` with a manifest and
//! the contribution directories beside it:
//!
//! ```text
//! plugins/review-kit/
//!   .claude-plugin/plugin.json   ← manifest (Claude Code's location)
//!   commands/review.md          → prompt commands
//!   skills/review/SKILL.md       → skills
//!   agents/reviewer.md           → subagent definitions
//!   hooks/hooks.json             → hooks (the settings-file shape)
//!   .mcp.json                    → MCP servers
//! ```
//!
//! The manifest's own location is Claude Code's, and the contribution
//! directories are found by convention, so a plugin written for that ecosystem
//! installs here unchanged. A manifest may also name its contributions
//! explicitly (`skills`, `agents`, `hooks`, `mcpServers`) — those paths are
//! taken as given, because a plugin that spells them out has said where to
//! look.
//!
//! This crate hands out **paths**. Each consumer — the skill scan, the subagent
//! scan, the hook executor, the MCP loader — reads them with its own parser,
//! which is what keeps a plugin's file from being a second definition of any
//! of those formats.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub mod install;
pub mod marketplace;

pub use install::{InstallError, set_enabled, uninstall};

/// Where installed plugins live, under the config root.
pub const PLUGINS_DIR: &str = "plugins";
/// The manifest inside a plugin, at Claude Code's path.
pub const MANIFEST_DIR: &str = ".claude-plugin";
/// The manifest file name.
pub const MANIFEST_FILE: &str = "plugin.json";
/// The enable state, beside the plugin directories.
pub const STATE_FILE: &str = "state.json";

/// One installed plugin, as its manifest describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    /// The manifest's `name`, which is also the directory name.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The author's name, flattened from the manifest's object form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Where the plugin is installed. Not serialized: a listing is about the
    /// plugin, and a caller that needs the path has it from [`discover`].
    #[serde(skip)]
    pub root: PathBuf,
    /// Whether it is switched on. A switched-off plugin contributes nothing.
    pub enabled: bool,
    /// What it contributes, as paths.
    #[serde(skip)]
    pub contributions: Contributions,
}

/// The directories and files a plugin contributes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contributions {
    /// Markdown prompt-command roots contributed by this plugin.
    pub commands: Vec<PathBuf>,
    /// Roots to scan for skills, in addition to the project's and the user's.
    pub skills: Vec<PathBuf>,
    /// Roots to scan for subagent definitions.
    pub agents: Vec<PathBuf>,
    /// Hook files, in the settings-file shape.
    pub hooks: Vec<PathBuf>,
    /// MCP configuration files, in `mcp.json`'s shape.
    pub mcp: Vec<PathBuf>,
}

impl Contributions {
    /// Whether this plugin contributes anything at all.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty() && self.skills.is_empty()
            && self.agents.is_empty()
            && self.hooks.is_empty()
            && self.mcp.is_empty()
    }

    /// A one-line summary for a settings row: `2 skills · 1 agent`.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        for (count, singular, plural) in [
            (self.commands.len(), "command", "commands"),
            (self.skills.len(), "skill", "skills"),
            (self.agents.len(), "agent", "agents"),
            (self.hooks.len(), "hook", "hook"),
            (self.mcp.len(), "MCP server", "MCP servers"),
        ] {
            if count == 0 {
                continue;
            }
            // A hook file may hold several hooks, so it is counted as a file.
            parts.push(format!("{count} {plural}"));
            let _ = singular;
        }
        parts.join(" · ")
    }
}

/// Why a plugin directory could not be read.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("{path} is not valid JSON: {error}")]
    Malformed { path: PathBuf, error: String },
    #[error("{path}: {problem}")]
    Invalid { path: PathBuf, problem: String },
    #[error("cannot read {path}: {error}")]
    Unreadable { path: PathBuf, error: String },
}

/// The manifest as it appears on disk.
///
/// Every field but `name` is optional: a manifest is a declaration of what a
/// plugin is, and the parts a consumer does not need must not be required to
/// parse it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    #[serde(default)]
    commands: Option<PathsField>,
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    /// `"Ada"` or `{"name": "Ada"}` — both appear in the wild.
    author: Option<AuthorField>,
    #[serde(default)]
    skills: Option<PathsField>,
    #[serde(default)]
    agents: Option<PathsField>,
    #[serde(default)]
    hooks: Option<PathsField>,
    #[serde(default, rename = "mcpServers", alias = "mcp")]
    mcp: Option<PathsField>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum AuthorField {
    Name(String),
    Object { name: Option<String> },
}

impl AuthorField {
    fn name(&self) -> Option<String> {
        match self {
            Self::Name(name) => Some(name.clone()),
            Self::Object { name } => name.clone(),
        }
    }
}

/// A contribution path: one string, or a list of them.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum PathsField {
    One(String),
    Many(Vec<String>),
}

impl PathsField {
    fn paths(&self) -> Vec<&str> {
        match self {
            Self::One(path) => vec![path.as_str()],
            Self::Many(paths) => paths.iter().map(String::as_str).collect(),
        }
    }
}

/// The enable state file.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct StateFile {
    /// Switched-off plugins. Absent means on: the switches exist to turn
    /// things off, and a fresh install runs what it was given.
    disabled: Vec<String>,
}

/// Every plugin installed under `config_root`, switched-on or not.
///
/// A directory without a readable manifest is not a plugin and is skipped with
/// a log line: the plugins directory is a plain folder a user may keep notes
/// in, and refusing to list anything because one entry is not a plugin would be
/// the wrong failure.
pub fn discover(config_root: &Path) -> Vec<Plugin> {
    let dir = config_root.join(PLUGINS_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let state = read_state(&dir);

    let mut plugins = Vec::new();
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();

    for root in dirs {
        let manifest_path = manifest_path(&root);
        if !manifest_path.is_file() {
            tracing::debug!(
                path = %root.display(),
                "no plugin manifest here; not a plugin"
            );
            continue;
        }
        match read_plugin(&root, &manifest_path, &state) {
            Ok(plugin) => plugins.push(plugin),
            Err(error) => tracing::warn!(%error, "a plugin could not be read; skipping it"),
        }
    }
    plugins
}

/// Only the switched-on plugins, for a consumer that wants contributions.
pub fn enabled(config_root: &Path) -> Vec<Plugin> {
    discover(config_root)
        .into_iter()
        .filter(|plugin| plugin.enabled)
        .collect()
}

/// Every skill root the enabled plugins contribute.
pub fn skill_roots(config_root: &Path) -> Vec<PathBuf> {
    enabled(config_root)
        .into_iter()
        .flat_map(|plugin| plugin.contributions.skills)
        .collect()
}

/// Every subagent-definition root the enabled plugins contribute.
pub fn agent_roots(config_root: &Path) -> Vec<PathBuf> {
    enabled(config_root)
        .into_iter()
        .flat_map(|plugin| plugin.contributions.agents)
        .collect()
}

/// Every hook file the enabled plugins contribute.
pub fn hook_files(config_root: &Path) -> Vec<PathBuf> {
    enabled(config_root)
        .into_iter()
        .flat_map(|plugin| plugin.contributions.hooks)
        .collect()
}

/// Every MCP configuration file the enabled plugins contribute.
pub fn mcp_files(config_root: &Path) -> Vec<PathBuf> {
    enabled(config_root)
        .into_iter()
        .flat_map(|plugin| plugin.contributions.mcp)
        .collect()
}

/// The manifest path for a plugin root, at Claude Code's location.
pub fn manifest_path(root: &Path) -> PathBuf {
    root.join(MANIFEST_DIR).join(MANIFEST_FILE)
}

fn read_state(plugins_dir: &Path) -> StateFile {
    let path = plugins_dir.join(STATE_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
            tracing::warn!(
                path = %path.display(),
                %error,
                "the plugin state file is not valid JSON; treating every plugin as on"
            );
            StateFile::default()
        }),
        Err(_) => StateFile::default(),
    }
}

fn read_plugin(
    root: &Path,
    manifest_path: &Path,
    state: &StateFile,
) -> Result<Plugin, PluginError> {
    let text = std::fs::read_to_string(manifest_path).map_err(|error| PluginError::Unreadable {
        path: manifest_path.to_path_buf(),
        error: error.to_string(),
    })?;
    let manifest: Manifest =
        serde_json::from_str(&text).map_err(|error| PluginError::Malformed {
            path: manifest_path.to_path_buf(),
            error: error.to_string(),
        })?;

    let name = manifest
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| PluginError::Invalid {
            path: manifest_path.to_path_buf(),
            problem: "a plugin needs a `name`".to_owned(),
        })?;

    let contributions = Contributions {
        commands: contributions(root, manifest.commands.as_ref(), "commands"),
        skills: contributions(root, manifest.skills.as_ref(), "skills"),
        agents: contributions(root, manifest.agents.as_ref(), "agents"),
        hooks: contribution_files(root, manifest.hooks.as_ref(), "hooks/hooks.json"),
        mcp: contribution_files(root, manifest.mcp.as_ref(), ".mcp.json")
            .into_iter()
            .chain(
                // Both spellings are in use; a plugin that ships one should not
                // have to know which this build prefers.
                contribution_files(root, None, "mcp.json"),
            )
            .collect(),
    };

    Ok(Plugin {
        enabled: !state.disabled.iter().any(|disabled| disabled == &name),
        name,
        version: manifest.version,
        description: manifest.description,
        author: manifest.author.as_ref().and_then(AuthorField::name),
        root: root.to_path_buf(),
        contributions,
    })
}

/// The directories a contribution field names, or the conventional one.
fn contributions(root: &Path, field: Option<&PathsField>, conventional: &str) -> Vec<PathBuf> {
    let mut paths = vec![root.join(conventional)];
    if let Some(field) = field { paths.extend(field.paths().into_iter().map(|path| root.join(path))); }
    let mut found = Vec::new();
    for path in paths {
        let path = if conventional == "skills" && path.is_file() && path.file_name().is_some_and(|name| name == "SKILL.md") {
            path.parent().unwrap().to_owned()
        } else { path };
        if (path.is_dir() || matches!(conventional, "commands" | "agents") && path.is_file()) && !found.contains(&path) { found.push(path); }
    }
    found

}

/// The files a contribution field names, or the conventional one.
fn contribution_files(root: &Path, field: Option<&PathsField>, conventional: &str) -> Vec<PathBuf> {
    let mut paths = vec![root.join(conventional)];
    if let Some(field) = field { paths.extend(field.paths().into_iter().map(|path| root.join(path))); }
    let mut found = Vec::new();
    for path in paths { if path.is_file() && !found.contains(&path) { found.push(path); } }
    found

}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a plugin directory with a manifest and the given files.
    fn plugin(config: &Path, name: &str, manifest: &str, files: &[(&str, &str)]) {
        let root = config.join(PLUGINS_DIR).join(name);
        std::fs::create_dir_all(manifest_path(&root).parent().unwrap()).unwrap();
        std::fs::write(manifest_path(&root), manifest).unwrap();
        for (path, body) in files {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, body).unwrap();
        }
    }

    #[test]
    fn a_plugin_with_no_manifest_is_not_a_plugin() {
        let config = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(config.path().join(PLUGINS_DIR).join("notes")).unwrap();
        assert!(discover(config.path()).is_empty());
    }

    #[test]
    fn conventional_directories_become_contributions() {
        let config = tempfile::tempdir().unwrap();
        plugin(
            config.path(),
            "review-kit",
            r#"{"name":"review-kit","version":"1.2.0","description":"Review helpers."}"#,
            &[
                (
                    "skills/review/SKILL.md",
                    "---\nname: review\ndescription: d\n---\nbody",
                ),
                (
                    "agents/reviewer.md",
                    "---\nname: reviewer\ndescription: d\n---\nbody",
                ),
                ("hooks/hooks.json", r#"{"hooks":{}}"#),
                (".mcp.json", r#"{"mcpServers":{}}"#),
            ],
        );

        let plugins = discover(config.path());
        assert_eq!(plugins.len(), 1);
        let plugin = &plugins[0];
        assert_eq!(plugin.name, "review-kit");
        assert_eq!(plugin.version.as_deref(), Some("1.2.0"));
        assert!(plugin.enabled);
        assert_eq!(plugin.contributions.skills.len(), 1);
        assert_eq!(plugin.contributions.agents.len(), 1);
        assert_eq!(plugin.contributions.hooks.len(), 1);
        assert_eq!(plugin.contributions.mcp.len(), 1);
        assert!(plugin.contributions.summary().contains("skill"));
    }

    #[test]
    fn an_author_may_be_a_string_or_an_object() {
        let config = tempfile::tempdir().unwrap();
        plugin(config.path(), "a", r#"{"name":"a","author":"Ada"}"#, &[]);
        plugin(
            config.path(),
            "b",
            r#"{"name":"b","author":{"name":"Grace"}}"#,
            &[],
        );
        let authors: Vec<Option<String>> = discover(config.path())
            .into_iter()
            .map(|plugin| plugin.author)
            .collect();
        assert_eq!(authors, [Some("Ada".to_owned()), Some("Grace".to_owned())]);
    }

    #[test]
    fn a_manifest_may_name_its_contributions_explicitly() {
        let config = tempfile::tempdir().unwrap();
        plugin(
            config.path(),
            "explicit",
            r#"{"name":"explicit","skills":"my-skills","agents":["a1","a2"]}"#,
            &[
                ("my-skills/s/SKILL.md", "x"),
                ("a1/one.md", "x"),
                ("a2/two.md", "x"),
            ],
        );
        let plugin = &discover(config.path())[0];
        assert_eq!(plugin.contributions.skills.len(), 1);
        assert_eq!(plugin.contributions.agents.len(), 2);
    }

    #[test]
    fn a_switched_off_plugin_contributes_nothing() {
        let config = tempfile::tempdir().unwrap();
        plugin(
            config.path(),
            "quiet",
            r#"{"name":"quiet"}"#,
            &[("skills/s/SKILL.md", "x")],
        );
        set_enabled(config.path(), "quiet", false).unwrap();

        assert!(skill_roots(config.path()).is_empty());
        // It is still installed, and still listed.
        let plugins = discover(config.path());
        assert_eq!(plugins.len(), 1);
        assert!(!plugins[0].enabled);

        set_enabled(config.path(), "quiet", true).unwrap();
        assert_eq!(skill_roots(config.path()).len(), 1);
    }

    #[test]
    fn a_manifest_without_a_name_is_refused() {
        let config = tempfile::tempdir().unwrap();
        plugin(config.path(), "nameless", r#"{"version":"1"}"#, &[]);
        assert!(discover(config.path()).is_empty());
    }

    #[test]
    fn a_malformed_manifest_does_not_hide_the_others() {
        let config = tempfile::tempdir().unwrap();
        plugin(config.path(), "broken", "{ not json", &[]);
        plugin(config.path(), "fine", r#"{"name":"fine"}"#, &[]);
        let names: Vec<String> = discover(config.path())
            .into_iter()
            .map(|plugin| plugin.name)
            .collect();
        assert_eq!(names, ["fine"]);
    }

    #[test]
    fn the_state_file_decides_what_is_on() {
        let config = tempfile::tempdir().unwrap();
        plugin(config.path(), "on", r#"{"name":"on"}"#, &[]);
        plugin(config.path(), "off", r#"{"name":"off"}"#, &[]);
        std::fs::write(
            config.path().join(PLUGINS_DIR).join(STATE_FILE),
            r#"{"disabled":["off"]}"#,
        )
        .unwrap();

        let states: Vec<(String, bool)> = discover(config.path())
            .into_iter()
            .map(|plugin| (plugin.name, plugin.enabled))
            .collect();
        assert_eq!(states, [("off".to_owned(), false), ("on".to_owned(), true)]);
    }
}
