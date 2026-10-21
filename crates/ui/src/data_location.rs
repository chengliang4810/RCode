//! The `location` pointer: the file that says where the data root is.
//!
//! `apps/rcode` resolves it at startup (see its `paths` module) and refuses to
//! start when the file names something unusable. This module is the writer:
//! one line, an absolute path, checked before it is written so the settings
//! page cannot leave behind a pointer the next launch would reject.
//!
//! It is deliberately a plain file in the config root, not a settings field:
//! the location cannot live inside the thing it relocates.

use std::io;
use std::path::{Path, PathBuf};

/// Must match `apps/rcode`'s resolver and `DataRoots`' documentation.
const FILE_NAME: &str = "location";

pub fn pointer_path(config_root: &Path) -> PathBuf {
    config_root.join(FILE_NAME)
}

/// The pointer's value, trimmed, or `None` when there is no usable one.
///
/// Tolerant on purpose: a hand-edited file is the supported way to set this
/// without the UI, so a strange file must render as "no custom location"
/// rather than as a failure here. The next launch is what refuses.
pub fn read(config_root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(pointer_path(config_root)).ok()?;
    let value = text.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Why a chosen directory cannot be used.
#[derive(Debug, PartialEq, Eq)]
pub enum LocationError {
    Absolute,
    Directory,
}

impl LocationError {
    /// The sentence the settings row shows after the fixed prefix.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Absolute => "the path is not absolute.",
            Self::Directory => "the path is not a directory.",
        }
    }
}

/// Point the data root at `target`.
///
/// Validated here rather than at the next launch: a typo would otherwise stop
/// the app on a message about a file the user has never heard of.
pub fn point_at(config_root: &Path, target: &Path) -> Result<(), LocationError> {
    if !target.is_absolute() {
        return Err(LocationError::Absolute);
    }
    if !target.is_dir() {
        return Err(LocationError::Directory);
    }
    std::fs::create_dir_all(config_root).map_err(|_| LocationError::Directory)?;
    // Trailing newline: the file is prose, and the resolver trims.
    std::fs::write(pointer_path(config_root), format!("{}\n", target.display()))
        .map_err(|_| LocationError::Directory)
}

/// Go back to one directory. Missing is success — that is the default.
pub fn clear(config_root: &Path) -> io::Result<()> {
    match std::fs::remove_file(pointer_path(config_root)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_pointer_reads_as_the_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()), None);
        assert_eq!(clear(dir.path()).is_ok(), true, "clearing nothing is fine");
    }

    #[test]
    fn pointing_round_trips_through_the_file() {
        let config = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        point_at(config.path(), target.path()).expect("point");
        assert_eq!(
            read(config.path()).as_deref(),
            Some(target.path().to_string_lossy().as_ref())
        );
        let raw = std::fs::read_to_string(pointer_path(config.path())).expect("read");
        assert!(raw.ends_with('\n'), "the file stays prose: {raw:?}");
        clear(config.path()).expect("clear");
        assert_eq!(read(config.path()), None);
    }

    #[test]
    fn a_relative_or_missing_target_is_refused() {
        let config = tempfile::tempdir().unwrap();
        assert_eq!(
            point_at(config.path(), Path::new("relative/data")),
            Err(LocationError::Absolute)
        );
        let missing = config.path().join("not-there");
        assert_eq!(
            point_at(config.path(), &missing),
            Err(LocationError::Directory)
        );
        assert_eq!(read(config.path()), None, "a refused pick writes nothing");
    }

    #[test]
    fn a_hand_edited_file_still_reads() {
        let config = tempfile::tempdir().unwrap();
        std::fs::write(pointer_path(config.path()), "  /mnt/big/rcode  \n").unwrap();
        assert_eq!(read(config.path()).as_deref(), Some("/mnt/big/rcode"));
        std::fs::write(pointer_path(config.path()), "\n").unwrap();
        assert_eq!(read(config.path()), None, "an empty file is no pointer");
    }
}
