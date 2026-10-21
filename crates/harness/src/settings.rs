//! Which capabilities the agent is allowed to use.
//!
//! Separate from `providers.json` because the two answer different questions:
//! that file says *which model to talk to*, and this one says *what the agent
//! may do*. They also have different lifetimes — a provider must not change
//! under a live conversation, whereas a capability switch is meant to take
//! effect the next time the agent runs.
//!
//! Read fresh at the start of every run rather than held on the harness for its
//! lifetime. That is deliberate: the settings page writes the file, and a user
//! who turns memory off expects the next message to behave differently, not the
//! next restart. The read is one small file per run.
//!
//! A missing file means every capability is on, which is what a fresh install
//! should do — the switches exist to turn things off, not to make the agent
//! inert until they are visited.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What the agent may use. Every field defaults to `true`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentSettings {
    /// Whether the agent reads and writes memory across sessions.
    ///
    /// Off omits automatic-memory instructions and its index. Ordinary file
    /// tools remain available under the application's trust-all policy.
    pub memory: bool,
    /// Skills the user switched off, keyed by their directory.
    ///
    /// Kept here with the other capability switches because it is the same
    /// question — what may the agent use — and read on the same schedule: fresh
    /// each run, so a skill switched off takes effect on the next message.
    ///
    /// `BTreeSet` rather than a list so the file is stable: a settings file
    /// that reorders itself on every save is one the user cannot read a diff
    /// of, and paths are the sort of value that wanders.
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub disabled_skills: std::collections::BTreeSet<String>,
    /// Whether the agent may reach the MCP servers in `mcp.json`.
    ///
    /// Off removes every `mcp__*` tool from the next run without touching the
    /// servers' configuration, which is the switch a user wants when a server
    /// is misbehaving but they do not want to delete what they wrote.
    pub mcp: bool,
    /// Subagent definitions the user switched off, keyed by their **file
    /// path** — the same path the listing reports, so a toggle names the copy
    /// it is looking at. Off-by-path rather than off-by-name because a project
    /// definition and the person's of the same name are different files, and
    /// switching one off must not switch off the other.
    ///
    /// A definition defaults to *enabled*: the switches here record the
    /// exceptions, so a fresh install delegates to whatever its projects and
    /// config define, and an explicit off is what hides one.
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub disabled_subagents: std::collections::BTreeSet<String>,
    /// Per-definition model-alias overrides, keyed by the same file path.
    ///
    /// A definition's own `model:` stays as the author wrote it; this map is
    /// the user's say-so on top ("run *this* one on haiku"). The value is one
    /// of the aliases the agent crate parses — `inherit` resets to whatever
    /// the file says, so it is also how an override is removed in place.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub subagent_models: std::collections::BTreeMap<String, String>,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            memory: true,
            disabled_skills: Default::default(),
            mcp: true,
            disabled_subagents: Default::default(),
            subagent_models: Default::default(),
        }
    }
}

const FILE_NAME: &str = "agent-settings.json";

impl AgentSettings {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }

    /// Read the settings, falling back to all-on.
    ///
    /// A missing file is the default. A *malformed* file is also the default,
    /// with a log line: this gate decides whether the agent can work, and
    /// refusing to start an agent because its own settings file is corrupt
    /// would be a worse failure than running with the capability on.
    pub fn load(data_dir: &Path) -> Self {
        let path = Self::path(data_dir);
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(settings) => settings,
                Err(error) => {
                    tracing::warn!(
                        path = %path.display(),
                        %error,
                        "agent-settings.json is not valid; using defaults"
                    );
                    Self::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    %error,
                    "agent-settings.json could not be read; using defaults"
                );
                Self::default()
            }
        }
    }

    /// Write the settings, creating the directory if needed.
    pub fn save(&self, data_dir: &Path) -> Result<(), std::io::Error> {
        std::fs::create_dir_all(data_dir)?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        // Write beside the final name, then rename, so an interrupted save
        // cannot leave a truncated file that reads as "everything off".
        let path = Self::path(data_dir);
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, json)?;
        std::fs::rename(&temporary, &path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_install_has_every_capability_on() {
        let dir = tempfile::tempdir().expect("temp dir");
        let settings = AgentSettings::load(dir.path());
        assert!(settings.memory);
        assert!(settings.mcp);
    }

    #[test]
    fn a_missing_field_defaults_to_on() {
        // An older file written before a capability existed must not read as
        // "that capability is off".
        let settings: AgentSettings = serde_json::from_str("{}").expect("an empty object is valid");
        assert!(settings.memory);
    }

    #[test]
    fn turning_memory_off_round_trips() {
        let dir = tempfile::tempdir().expect("temp dir");
        AgentSettings {
            memory: false,
            ..AgentSettings::default()
        }
        .save(dir.path())
        .expect("save");
        assert!(!AgentSettings::load(dir.path()).memory);
    }

    #[test]
    fn a_malformed_file_falls_back_to_defaults_rather_than_failing() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(AgentSettings::path(dir.path()), "{ not json").expect("write");
        assert!(
            AgentSettings::load(dir.path()).memory,
            "a corrupt settings file must not silently disable the agent"
        );
    }

    #[test]
    fn saving_leaves_no_temporary_file_behind() {
        let dir = tempfile::tempdir().expect("temp dir");
        AgentSettings::default().save(dir.path()).expect("save");

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("reads")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }

    #[test]
    fn the_wire_shape_is_camel_case() {
        let value = serde_json::to_value(AgentSettings::default()).expect("serialize");
        // The empty exception maps serialize away, so a fresh file is the
        // three switches and nothing else.
        assert_eq!(value, serde_json::json!({ "memory": true, "mcp": true }));
    }

    #[test]
    fn a_definition_defaults_to_enabled_and_an_override_defaults_to_absent() {
        // A file written before subagents existed must not read as "the
        // runtime is off".
        let settings: AgentSettings = serde_json::from_str(
            r#"{"memory":true,"disabledSubagents":["/cfg/agents/x.md"],
                "subagentModels":{"/cfg/agents/y.md":"haiku"}}"#,
        )
        .expect("parses");
        assert!(settings.disabled_subagents.contains("/cfg/agents/x.md"));
        assert_eq!(
            settings.subagent_models.get("/cfg/agents/y.md"),
            Some(&"haiku".to_owned())
        );

        let settings: AgentSettings =
            serde_json::from_str(r#"{"subagents":false}"#).expect("parses");
        assert!(
            serde_json::to_value(&settings)
                .unwrap()
                .get("subagents")
                .is_none()
        );
        assert!(settings.disabled_subagents.is_empty());
    }
}
