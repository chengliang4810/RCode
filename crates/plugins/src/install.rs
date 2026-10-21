//! Removing and switching installed marketplace plugins.

use std::path::{Path, PathBuf};

use crate::{PLUGINS_DIR, STATE_FILE};

/// Why an install could not happen.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("invalid plugin name")]
    InvalidName,
    #[error("cannot {action} {path}: {error}")]
    Io {
        action: &'static str,
        path: PathBuf,
        error: String,
    },
}

/// Remove an installed plugin, and forget any enable state for it.
pub fn uninstall(config_root: &Path, name: &str) -> Result<(), InstallError> {
    validate_name(name)?;
    let destination = config_root.join(PLUGINS_DIR).join(name);
    if destination.exists() {
        std::fs::remove_dir_all(&destination).map_err(|error| InstallError::Io {
            action: "remove",
            path: destination.clone(),
            error: error.to_string(),
        })?;
    }
    // A removed plugin must not leave a stale "disabled" entry behind: a
    // re-install of the same name would come back silently switched off.
    let mut state = read_state(config_root);
    let before = state.disabled.len();
    state.disabled.retain(|disabled| disabled != name);
    if state.disabled.len() != before {
        write_state(config_root, &state)?;
    }
    Ok(())
}

/// Switch a plugin on or off. Its files stay where they are.
pub fn set_enabled(config_root: &Path, name: &str, enabled: bool) -> Result<(), InstallError> {
    validate_name(name)?;
    let mut state = read_state(config_root);
    state.disabled.retain(|disabled| disabled != name);
    if !enabled {
        state.disabled.push(name.to_owned());
    }
    write_state(config_root, &state)
}

/// The state file's shape. Mirrors the reader in the parent module; kept here
/// so a write cannot invent a shape the reader would not accept.
#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, rename_all = "camelCase")]
struct StateFile {
    disabled: Vec<String>,
}

fn state_path(config_root: &Path) -> PathBuf {
    config_root.join(PLUGINS_DIR).join(STATE_FILE)
}

fn read_state(config_root: &Path) -> StateFile {
    std::fs::read_to_string(state_path(config_root))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_state(config_root: &Path, state: &StateFile) -> Result<(), InstallError> {
    let path = state_path(config_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| InstallError::Io {
            action: "create",
            path: parent.to_path_buf(),
            error: error.to_string(),
        })?;
    }
    let json = serde_json::to_string_pretty(state).map_err(|error| InstallError::Io {
        action: "serialize",
        path: path.clone(),
        error: error.to_string(),
    })?;
    // Beside the name, then rename: an interrupted write must not leave a file
    // that reads as "everything is on".
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, json).map_err(|error| InstallError::Io {
        action: "write",
        path: temporary.clone(),
        error: error.to_string(),
    })?;
    std::fs::rename(&temporary, &path).map_err(|error| InstallError::Io {
        action: "rename",
        path: path.clone(),
        error: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uninstalling_removes_files_and_enable_state() {
        let config = tempfile::tempdir().unwrap();
        let root = config.path().join(PLUGINS_DIR).join("kit");
        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(root.join(".claude-plugin/plugin.json"), r#"{"name":"kit"}"#).unwrap();
        set_enabled(config.path(), "kit", false).unwrap();
        uninstall(config.path(), "kit").unwrap();
        assert!(!root.exists());
        assert!(read_state(config.path()).disabled.is_empty());
    }
    #[test]
    fn uninstalling_an_absent_plugin_succeeds() {
        uninstall(tempfile::tempdir().unwrap().path(), "ghost").unwrap();
    }
}

fn validate_name(name: &str) -> Result<(), InstallError> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':']) {
        return Err(InstallError::InvalidName);
    }
    Ok(())
}
