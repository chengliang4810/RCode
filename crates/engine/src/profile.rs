//! Workspace profile identity and storage boundaries.
//!
//! Two directories decide where everything lands. The **config root** holds
//! what identifies this installation and what a reader needs to find it: the
//! device id, the engine lock, provider credentials, UI settings, logs and the
//! `location` pointer. The **data root** holds what grows with use: the profile
//! stores (documents, journals, uploads), cloned repositories, scratch folders,
//! caches and worktrees. They are the same directory until the user points the
//! data root elsewhere.
//!
//! Stores, journals, and uploads are profile-scoped, under the data root.
//!
//! One profile shape: an identity (`org_id`/`user_id`) and the store root
//! derived from it. The `Local`/`Development`/`Synced` distinction this type
//! also carried went with the cloud — `Synced` first, then the rest, because a
//! runtime that reports one shape does not need an enum to say so.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::EngineError;

const LEGACY_UPLOADS_DIR: &str = "uploads";
const LEGACY_UPLOADS_OWNER_FILE: &str = "legacy-uploads-owner.json";

/// The two directories a runtime reads.
///
/// `RCODE_DATA_DIR` overrides the config root for one launch (which is how a
/// dev build runs beside an installed one, and the pointer file travels with
/// it); the data root comes from that root's `location` pointer, or is the
/// same directory when there is no pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRoots {
    /// Identity, credentials, settings, logs — and the pointer itself.
    config: PathBuf,
    /// The growth: profile stores, clones, scratch, caches, worktrees.
    data: PathBuf,
}

impl DataRoots {
    /// One directory for both. Tests, fixtures, and any caller that has a
    /// single path and no pointer to honour.
    pub fn same(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        Self {
            data: dir.clone(),
            config: dir,
        }
    }

    /// Both roots named explicitly (the pointer case).
    pub fn split(config: impl Into<PathBuf>, data: impl Into<PathBuf>) -> Self {
        Self {
            config: config.into(),
            data: data.into(),
        }
    }

    pub fn config(&self) -> &Path {
        &self.config
    }

    pub fn data(&self) -> &Path {
        &self.data
    }

    /// Whether the pointer actually moved anything. Diagnostics read better
    /// with one path than two identical ones.
    pub fn is_split(&self) -> bool {
        self.config != self.data
    }
}

/// A resolved, immutable identity and storage boundary for one engine runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineProfile {
    config_root: PathBuf,
    data_root: PathBuf,
    store_root: PathBuf,
    uploads_root: PathBuf,
    org_id: String,
    user_id: String,
}

impl EngineProfile {
    /// Resolve the profile for an identity, under its store root.
    pub fn new(roots: &DataRoots, org_id: &str, user_id: &str) -> Self {
        let store_root = roots
            .data()
            .join("orgs")
            .join(sanitize_path_id(org_id))
            .join(sanitize_path_id(user_id));
        Self {
            config_root: roots.config().to_path_buf(),
            data_root: roots.data().to_path_buf(),
            uploads_root: store_root.join("uploads"),
            store_root,
            org_id: org_id.to_string(),
            user_id: user_id.to_string(),
        }
    }

    /// Identity, credentials, settings, and the lock live here.
    pub fn config_root(&self) -> &Path {
        &self.config_root
    }

    /// The store roots, clones, scratch folders and worktrees live here.
    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub fn store_root(&self) -> &Path {
        &self.store_root
    }

    pub fn uploads_root(&self) -> &Path {
        &self.uploads_root
    }

    pub fn org_id(&self) -> &str {
        &self.org_id
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// Claim the historical device-global uploads cache for one account.
    ///
    /// Legacy transcript entries persist absolute paths under the old
    /// `{data_dir}/uploads`,
    /// so moving the directory would break them. The first account opened after
    /// upgrade becomes its durable owner and may read it as a compatibility
    /// fallback. New writes always use [`Self::uploads_root`], and other accounts
    /// never receive the fallback root.
    pub(crate) fn claim_legacy_uploads_root(&self) -> Result<Option<PathBuf>, EngineError> {
        let legacy_root = self.config_root.join(LEGACY_UPLOADS_DIR);
        let metadata = match std::fs::metadata(&legacy_root) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        if !metadata.is_dir() {
            return Err(EngineError::Other(format!(
                "invalid legacy uploads cache {}: expected a directory",
                legacy_root.display()
            )));
        }

        let owner_path = self.config_root.join(LEGACY_UPLOADS_OWNER_FILE);
        let expected = LegacyUploadsOwner {
            org_id: self.org_id.clone(),
            user_id: self.user_id.clone(),
        };
        let owner = match read_legacy_uploads_owner(&owner_path) {
            Ok(owner) => owner,
            Err(ProfileReadError::Missing) => {
                publish_legacy_uploads_owner(&owner_path, &expected)?;
                read_legacy_uploads_owner(&owner_path).map_err(ProfileReadError::into_engine)?
            }
            Err(ProfileReadError::Engine(err)) => return Err(err),
        };

        Ok((owner == expected).then_some(legacy_root))
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyUploadsOwner {
    org_id: String,
    user_id: String,
}

enum ProfileReadError {
    Missing,
    Engine(EngineError),
}

impl ProfileReadError {
    fn into_engine(self) -> EngineError {
        match self {
            Self::Missing => EngineError::Other("local profile disappeared during creation".into()),
            Self::Engine(err) => err,
        }
    }
}

fn read_legacy_uploads_owner(path: &Path) -> Result<LegacyUploadsOwner, ProfileReadError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(ProfileReadError::Missing);
        }
        Err(err) => return Err(ProfileReadError::Engine(err.into())),
    };
    serde_json::from_slice(&bytes).map_err(|err| {
        ProfileReadError::Engine(EngineError::Other(format!(
            "invalid legacy uploads owner {}: {err}",
            path.display()
        )))
    })
}

fn publish_legacy_uploads_owner(
    path: &Path,
    owner: &LegacyUploadsOwner,
) -> Result<(), EngineError> {
    let mut bytes = serde_json::to_vec_pretty(owner)
        .map_err(|err| EngineError::Other(format!("serialize legacy uploads owner: {err}")))?;
    bytes.push(b'\n');
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(LEGACY_UPLOADS_OWNER_FILE);
    let temp_path = path.with_file_name(format!(
        ".{file_name}.tmp-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    let write_result = (|| -> Result<(), EngineError> {
        let mut temp = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        temp.write_all(&bytes)?;
        temp.sync_all()?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(err);
    }

    let publish_result = std::fs::hard_link(&temp_path, path);
    let _ = std::fs::remove_file(&temp_path);
    match publish_result {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(err.into()),
    }
}

/// Filesystem-safe form of an org/user id used by the historical synced layout.
fn sanitize_path_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_keeps_its_identity_and_derives_its_store_root() {
        let dir = tempfile::tempdir().unwrap();
        let roots = DataRoots::same(dir.path());
        let profile = EngineProfile::new(&roots, "acme", "alice");
        let store = dir.path().join("orgs/acme/alice");

        assert_eq!(profile.org_id(), "acme");
        assert_eq!(profile.user_id(), "alice");
        assert_eq!(profile.store_root(), store);
        assert_eq!(profile.uploads_root(), store.join("uploads"));
        assert_eq!(profile.config_root(), dir.path());
        assert_eq!(profile.data_root(), dir.path());
    }

    /// An id with a separator in it must not become a directory of its own:
    /// the store root is built by joining, so `../` would escape it. Every
    /// character outside the safe set becomes one underscore, so `../etc`
    /// reads as three.
    #[test]
    fn path_ids_are_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        let profile = EngineProfile::new(&DataRoots::same(dir.path()), "../etc", "a/b");
        assert_eq!(profile.store_root(), dir.path().join("orgs/___etc/a_b"));
    }

    /// The two roots are independent: the identity stays where the settings
    /// are, the stores follow the pointer.
    #[test]
    fn a_split_puts_the_store_under_the_data_root() {
        let config = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let profile = EngineProfile::new(
            &DataRoots::split(config.path(), data.path()),
            "acme",
            "alice",
        );
        assert_eq!(profile.config_root(), config.path());
        assert_eq!(profile.data_root(), data.path());
        assert_eq!(profile.store_root(), data.path().join("orgs/acme/alice"));
    }
}
