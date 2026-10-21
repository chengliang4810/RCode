//! File-backed prompt commands, shared by settings and the native harness.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomCommand {
    pub name: String,
    pub description: String,
    pub input_hint: Option<String>,
    pub source: CommandSource,
    pub enabled: bool,
    /// Complete Markdown document, including frontmatter, for the editor.
    pub content: String,
    pub revision: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommandSource {
    Project,
    User,
    Plugin,
}

/// Settings writes only the user's managed directory. Project and plugin
/// files remain owned by the project and plugin respectively.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CommandMutation {
    Save {
        name: String,
        content: String,
        expected_revision: Option<String>,
    },
    Delete {
        name: String,
        expected_revision: String,
    },
    SetEnabled {
        name: String,
        enabled: bool,
        expected_revision: String,
    },
}
