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
    let _lock = lock_state(config_root)?;
    let mut state = read_state(config_root)?;
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
    let _lock = lock_state(config_root)?;
    let mut state = read_state(config_root)?;
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

fn read_state(config_root: &Path) -> Result<StateFile, InstallError> {
    let path = state_path(config_root);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| InstallError::Io {
            action: "parse state",
            path,
            error: error.to_string(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(StateFile::default()),
        Err(error) => Err(InstallError::Io { action: "read state", path, error: error.to_string() }),
    }
}

fn lock_state(config_root: &Path) -> Result<std::fs::File, InstallError> {
    let directory = config_root.join(PLUGINS_DIR);
    std::fs::create_dir_all(&directory).map_err(|error| InstallError::Io {
        action: "create", path: directory.clone(), error: error.to_string(),
    })?;
    let path = directory.join(".state.lock");
    let file = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true)
        .open(&path).map_err(|error| InstallError::Io {
            action: "open state lock", path: path.clone(), error: error.to_string(),
        })?;
    // The lock covers the entire read/modify/publish transaction, including
    // other engine processes sharing this config root. Drop releases it.
    file.lock().map_err(|error| InstallError::Io {
        action: "lock state", path, error: error.to_string(),
    })?;
    Ok(file)
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
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap()).map_err(|error| InstallError::Io {
        action: "create state temporary", path: path.clone(), error: error.to_string(),
    })?;
    use std::io::Write;
    temporary.write_all(json.as_bytes()).and_then(|_| temporary.as_file().sync_all()).map_err(|error| InstallError::Io {
        action: "write",
        path: path.clone(),
        error: error.to_string(),
    })?;
    temporary.persist(&path).map_err(|error| InstallError::Io {
        action: "publish state",
        path: path.clone(),
        error: error.to_string(),
    })?;
    Ok(())
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
        assert!(read_state(config.path()).unwrap().disabled.is_empty());
    }
    #[test]
    fn uninstalling_an_absent_plugin_succeeds() {
        uninstall(tempfile::tempdir().unwrap().path(), "ghost").unwrap();
    }

    #[test]
    fn windows_path_aliases_cannot_remove_plugins_or_change_their_state() {
        let config = tempfile::tempdir().unwrap();
        let plugin = config.path().join(PLUGINS_DIR).join("kit");
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(plugin.join("keep"), b"plugin contents").unwrap();
        set_enabled(config.path(), "kit", false).unwrap();
        let before = std::fs::read(state_path(config.path())).unwrap();
        for name in ["...", ".. ", "kit.", "kit ", ".", ".."] {
            assert!(matches!(uninstall(config.path(), name), Err(InstallError::InvalidName)));
            assert!(matches!(set_enabled(config.path(), name, true), Err(InstallError::InvalidName)));
            assert_eq!(std::fs::read(plugin.join("keep")).unwrap(), b"plugin contents");
            assert_eq!(std::fs::read(state_path(config.path())).unwrap(), before);
        }
    }

    #[test]
    fn concurrent_state_edits_preserve_every_successful_update() {
        let config = tempfile::tempdir().unwrap();
        let count = 32;
        let start = std::sync::Arc::new(std::sync::Barrier::new(count));
        std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..count).map(|index| {
                let start = start.clone();
                let root = config.path();
                scope.spawn(move || {
                    start.wait();
                    set_enabled(root, &format!("plugin-{index}"), false).unwrap();
                })
            }).collect();
            for job in jobs { job.join().unwrap(); }
        });
        let state = read_state(config.path()).unwrap();
        assert_eq!(state.disabled.len(), count);
        for index in 0..count { assert!(state.disabled.contains(&format!("plugin-{index}"))); }
    }

    #[test]
    fn malformed_state_is_not_reset_or_used_to_enable_plugins() {
        let config = tempfile::tempdir().unwrap();
        let plugin = config.path().join(PLUGINS_DIR).join("kit");
        std::fs::create_dir_all(plugin.join(".claude-plugin")).unwrap();
        std::fs::write(plugin.join(".claude-plugin/plugin.json"), r#"{"name":"kit"}"#).unwrap();
        let corrupt = b"{broken state";
        std::fs::write(state_path(config.path()), corrupt).unwrap();
        assert!(set_enabled(config.path(), "kit", true).is_err());
        assert!(uninstall(config.path(), "kit").is_err());
        assert!(plugin.exists());
        assert_eq!(std::fs::read(state_path(config.path())).unwrap(), corrupt);
        let discovered = crate::discover(config.path());
        assert_eq!(discovered.len(), 1);
        assert!(!discovered[0].enabled);
        assert!(crate::enabled(config.path()).is_empty());
    }
}

fn validate_name(name: &str) -> Result<(), InstallError> {
    // Win32 trims trailing dots/spaces, so e.g. `...` aliases the plugins
    // directory itself and `kit.` aliases a different plugin. Reject these on
    // every platform so copied settings retain the same plugin identity.
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with(['.', ' '])
        || name.contains(['/', '\\', ':'])
        || name.chars().any(char::is_control)
    {
        return Err(InstallError::InvalidName);
    }
    Ok(())
}
