//! Stable device-local browser storage, independent of chats and repositories.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct BrowserProfile {
    pub root: PathBuf,
}

impl BrowserProfile {
    /// The profile for this installation's local workspace.
    ///
    /// There is no account or synced scope to separate stores by: this build
    /// keeps everything on the device, so the device's own id is what tells
    /// one installation's cookies and localStorage from another's.
    pub fn for_local_device(data_dir: &Path, local_device_id: Option<&str>) -> Option<Self> {
        Some(Self::new(data_dir, &["local", local_device_id?]))
    }

    fn new(data_dir: &Path, identity: &[&str]) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"rcode-browser-profile-v1");
        for part in identity {
            hash.update((part.len() as u64).to_le_bytes());
            hash.update(part.as_bytes());
        }
        // Resolve aliases so windows using the same installation share a store.
        let data_dir = data_dir.canonicalize().unwrap_or_else(|_| {
            std::path::absolute(data_dir).unwrap_or_else(|_| data_dir.to_path_buf())
        });
        Self {
            root: data_dir
                .join("browser")
                .join(format!("{:x}", hash.finalize())),
        }
    }

    #[cfg(any(target_os = "macos", test))]
    pub fn data_store_identifier(&self) -> [u8; 16] {
        // Include the installation directory: fixtures and alternative data roots
        // must never open a real user's WKWebsiteDataStore.
        let mut hash = Sha256::new();
        hash.update(b"rcode-wkwebsite-data-store-v1");
        hash.update(self.root.as_os_str().as_encoded_bytes());
        let mut id: [u8; 16] = hash.finalize()[..16].try_into().unwrap();
        id[6] = (id[6] & 0x0f) | 0x80; // UUID v8, application-defined hash.
        id[8] = (id[8] & 0x3f) | 0x80;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_stable_and_scoped_to_device_and_installation() {
        let root = tempfile::tempdir().unwrap();
        let make = |parts: &[&str]| BrowserProfile::new(root.path(), parts);
        let a = make(&["local", "device-a"]);
        assert_eq!(a, make(&["local", "device-a"]));
        let b = make(&["local", "device-b"]);
        assert_ne!(a.root, b.root);
        assert_ne!(a.data_store_identifier(), b.data_store_identifier());
        let other = tempfile::tempdir().unwrap();
        let c = BrowserProfile::new(other.path(), &["local", "device-a"]);
        assert_ne!(a.data_store_identifier(), c.data_store_identifier());
        assert_ne!(make(&["ab", "c"]), make(&["a", "bc"]));
        assert!(
            make(&["../../outside"])
                .root
                .starts_with(root.path().canonicalize().unwrap().join("browser"))
        );
    }

    #[test]
    fn unresolved_identity_does_not_select_a_shared_persistent_profile() {
        let root = Path::new("/unused");
        assert!(BrowserProfile::for_local_device(root, None).is_none());
        let local = |device| BrowserProfile::for_local_device(root, Some(device));
        assert_eq!(local("one"), local("one"));
        assert_ne!(local("one"), local("two"));
    }
}
