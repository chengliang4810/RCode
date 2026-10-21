//! Application storage paths.
//!
//! Two directories, resolved in this order:
//!
//! 1. The **config root** is `$RCODE_DATA_DIR` when set, otherwise `~/.rcode`
//!    (the same name on every platform).
//! 2. The **data root** is whatever the config root's `location` pointer names,
//!    or the config root itself when there is no pointer.
//!
//! The config root holds what identifies this installation — device id, engine
//! lock, provider credentials (`providers.json` holds API keys), UI settings,
//! memory, logs — and the pointer file. The data root holds what grows with
//! use: the profile stores, cloned repositories, scratch folders, caches and
//! worktrees. Keeping them separate is what lets the bulk live on another
//! drive while the keys stay on the system one, and it is the default too:
//! with no pointer the two roots are one directory.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use rcode_engine::DataRoots;

/// The data directory's name under the user's home — the same string on every
/// platform, so the paths a document stores (`~/.rcode/scratch/<name>`, the
/// user's `~/.rcode/skills`) resolve to the directory the app actually uses.
const DATA_DIR_NAME: &str = ".rcode";

/// The pointer file in the config root: one line, an absolute directory.
const LOCATION_FILE: &str = "location";

pub fn data_roots() -> Result<DataRoots, RootsError> {
    resolve_roots(|name| std::env::var_os(name), read_location)
}

/// The config root alone — identity, credentials, settings, logs, the pointer.
///
/// For callers that have no store to open: `rcode update` staging a bundle,
/// the log directory. They must not fail because a pointer elsewhere is
/// broken — the config root is by definition where the pointer lives, so
/// resolving it never depends on one.
pub fn config_dir() -> PathBuf {
    let mut env = |name: &str| std::env::var_os(name);
    resolve_config_root(&mut env)
}

/// Why the pointer could not be honoured.
///
/// Every variant names what was written and what is wrong with it. Nothing
/// here falls back to the default: starting on an empty workspace because the
/// pointer was wrong is indistinguishable from having lost every conversation.
#[derive(Debug, PartialEq, Eq)]
pub enum RootsError {
    /// The pointer file exists but could not be read.
    Unreadable { pointer: PathBuf, error: String },
    /// The pointer file exists and names nothing.
    Empty { pointer: PathBuf },
    /// The pointer names a relative path, which would resolve differently
    /// depending on the working directory the engine happened to start in.
    Relative { pointer: PathBuf, value: String },
    /// The pointer names a directory that is not there (unplugged drive, a
    /// typo, a share that is not mounted).
    Missing { pointer: PathBuf, value: PathBuf },
}

impl fmt::Display for RootsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { pointer, error } => write!(
                f,
                "cannot read the data location in {}: {error}",
                pointer.display()
            ),
            Self::Empty { pointer } => write!(
                f,
                "{} names no directory; write the path of an existing directory there, or delete \
                 the file to use the default",
                pointer.display()
            ),
            Self::Relative { pointer, value } => write!(
                f,
                "{} names `{value}`, which is not an absolute path",
                pointer.display()
            ),
            Self::Missing { pointer, value } => write!(
                f,
                "{} names {}, which is not a directory — create it, point the file somewhere that \
                 exists, or delete it to use the default",
                pointer.display(),
                value.display()
            ),
        }
    }
}

impl std::error::Error for RootsError {}

fn read_location(pointer: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(pointer) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn resolve_roots(
    mut env: impl FnMut(&str) -> Option<OsString>,
    read: impl FnOnce(&Path) -> std::io::Result<Option<String>>,
) -> Result<DataRoots, RootsError> {
    let config = resolve_config_root(&mut env);

    let pointer = config.join(LOCATION_FILE);
    let Some(text) = read(&pointer).map_err(|error| RootsError::Unreadable {
        pointer: pointer.clone(),
        error: error.to_string(),
    })?
    else {
        return Ok(DataRoots::same(config));
    };
    let value = text.trim();
    if value.is_empty() {
        return Err(RootsError::Empty { pointer });
    }
    let target = PathBuf::from(value);
    if !target.is_absolute() {
        return Err(RootsError::Relative {
            pointer,
            value: value.to_string(),
        });
    }
    if !target.is_dir() {
        return Err(RootsError::Missing {
            pointer,
            value: target,
        });
    }
    Ok(DataRoots::split(config, target))
}

fn resolve_config_root(env: &mut impl FnMut(&str) -> Option<OsString>) -> PathBuf {
    if let Some(dir) = env("RCODE_DATA_DIR").filter(|value| !value.is_empty()) {
        // An explicit override is the user's own choice of location, and it is
        // a per-launch override: a second instance (a dev build beside an
        // installed one) gives itself its own directory without changing the
        // install's. Its pointer travels with it.
        return PathBuf::from(dir);
    }
    default_config_root(env)
}

fn default_config_root(env: &mut impl FnMut(&str) -> Option<OsString>) -> PathBuf {
    #[cfg(windows)]
    {
        // The user-profile directory, never LOCALAPPDATA and never HOME:
        // Explorer does not set HOME, so honouring it would let a shell launch
        // select a different workspace (and a different set of credentials)
        // than a desktop launch of the same build.
        let home = env("USERPROFILE")
            .filter(|value| !value.is_empty())
            .expect("USERPROFILE is not set; set RCODE_DATA_DIR to choose a data directory");
        PathBuf::from(home).join(DATA_DIR_NAME)
    }
    #[cfg(not(windows))]
    {
        let home = env("HOME").expect("HOME not set");
        PathBuf::from(home).join(DATA_DIR_NAME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(vars: &[(&str, &str)]) -> Result<DataRoots, RootsError> {
        resolve_env(vars, |_| Ok(None))
    }

    fn resolve_env(
        vars: &[(&str, &str)],
        read: impl FnOnce(&Path) -> std::io::Result<Option<String>>,
    ) -> Result<DataRoots, RootsError> {
        resolve_roots(
            |name| {
                vars.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.into())
            },
            read,
        )
    }

    fn homedir(vars: &[(&str, &str)]) -> PathBuf {
        resolve(vars).expect("default roots").config().to_path_buf()
    }

    #[test]
    fn explicit_override_needs_no_home() {
        assert_eq!(
            homedir(&[("RCODE_DATA_DIR", "custom data")]),
            PathBuf::from("custom data")
        );
    }

    #[test]
    fn no_pointer_means_one_directory() {
        let roots = resolve(&[("RCODE_DATA_DIR", "/srv/rcode")]).expect("roots");
        assert_eq!(roots.config(), Path::new("/srv/rcode"));
        assert_eq!(roots.data(), Path::new("/srv/rcode"));
        assert!(!roots.is_split());
    }

    #[test]
    fn a_pointer_moves_only_the_data_root() {
        // The target has to exist, so the fixture makes one.
        let target = std::env::temp_dir().join("rcode-paths-test-target");
        std::fs::create_dir_all(&target).expect("fixture dir");
        let roots = resolve_env(&[("RCODE_DATA_DIR", "/srv/rcode")], |pointer| {
            assert!(pointer.ends_with("location"));
            Ok(Some(format!("{}\n", target.display())))
        })
        .expect("roots");
        assert_eq!(roots.config(), Path::new("/srv/rcode"));
        assert_eq!(roots.data(), target.as_path());
        assert!(roots.is_split());
    }

    /// The target must exist: a typo that silently created a fresh workspace is
    /// how a user concludes the app deleted their conversations.
    #[test]
    fn a_pointer_to_nothing_is_an_error_not_a_default() {
        let missing = std::env::temp_dir().join("rcode-paths-test-missing-target");
        assert!(!missing.exists(), "the fixture must not exist");
        let error = resolve_env(&[("RCODE_DATA_DIR", "/srv/rcode")], |_| {
            Ok(Some(missing.display().to_string()))
        })
        .expect_err("must refuse");
        assert!(matches!(error, RootsError::Missing { .. }));
        let message = error.to_string();
        assert!(message.contains("missing-target"), "{message}");
        assert!(message.contains("location"), "{message}");
    }

    #[test]
    fn a_pointer_must_be_absolute() {
        let error = resolve_env(&[("RCODE_DATA_DIR", "/srv/rcode")], |_| {
            Ok(Some("data/rcode".into()))
        })
        .expect_err("must refuse");
        assert!(matches!(error, RootsError::Relative { .. }));
    }

    #[test]
    fn an_empty_pointer_is_an_error() {
        let error = resolve_env(&[("RCODE_DATA_DIR", "/srv/rcode")], |_| {
            Ok(Some("\n".into()))
        })
        .expect_err("must refuse");
        assert!(matches!(error, RootsError::Empty { .. }));
    }

    #[test]
    fn an_unreadable_pointer_is_an_error() {
        let error = resolve_env(&[("RCODE_DATA_DIR", "/srv/rcode")], |_| {
            Err(std::io::Error::other("permission denied"))
        })
        .expect_err("must refuse");
        assert!(matches!(error, RootsError::Unreadable { .. }));
    }

    #[cfg(windows)]
    #[test]
    fn windows_default_is_the_dotted_name_under_the_user_profile() {
        assert_eq!(
            homedir(&[("USERPROFILE", r"C:\Users\Test User")]),
            PathBuf::from(r"C:\Users\Test User\.rcode"),
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_profile_handles_unicode_and_apostrophes() {
        assert_eq!(
            homedir(&[("USERPROFILE", r"C:\Users\O'Brien 日本語")]),
            PathBuf::from(r"C:\Users\O'Brien 日本語\.rcode"),
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_default_does_not_depend_on_shell_home_or_local_app_data() {
        // A Git Bash launch sets HOME to a Unix-style path and the desktop
        // launch does not; the two must still resolve to one directory. The
        // old LOCALAPPDATA base is ignored too, so an upgrade finds the data
        // where this build puts it.
        assert_eq!(
            homedir(&[
                ("HOME", r"D:\msys-home"),
                ("LOCALAPPDATA", r"C:\Local"),
                ("USERPROFILE", r"C:\Users\dev"),
            ]),
            PathBuf::from(r"C:\Users\dev\.rcode"),
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_without_a_profile_falls_back_to_the_override() {
        assert_eq!(
            homedir(&[("RCODE_DATA_DIR", r"D:\rcode-data")]),
            PathBuf::from(r"D:\rcode-data"),
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_default_is_the_dotted_name_under_home() {
        assert_eq!(
            homedir(&[("HOME", "/home/dev")]),
            PathBuf::from("/home/dev/.rcode"),
        );
    }
}
