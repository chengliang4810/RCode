use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FrozenPrompt {
    pub version: u32,
    pub cwd: PathBuf,
    pub identity: String,
    pub static_sections: Vec<String>,
    pub environment: String,
    pub date: String,
    pub instructions: String,
}

pub(super) fn path(data_dir: &Path, session_id: &str) -> PathBuf {
    let key = format!("{:x}", Sha256::digest(session_id.as_bytes()));
    data_dir
        .join("prompt-snapshots")
        .join(format!("{key}.json"))
}

fn read(path: &Path) -> Result<Option<FrozenPrompt>, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "cannot read prompt snapshot {}: {error}",
                path.display()
            ));
        }
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read prompt snapshot {}: {error}", path.display()))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("prompt snapshot is too large: {}", path.display()));
    }
    let snapshot: FrozenPrompt = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid prompt snapshot {}: {error}", path.display()))?;
    if snapshot.version != 1
        || snapshot.static_sections.is_empty()
        || snapshot.identity.is_empty()
        || snapshot.environment.trim().is_empty()
        || snapshot.date.trim().is_empty()
        || !snapshot.cwd.is_absolute()
    {
        return Err(format!(
            "unsupported or incomplete prompt snapshot: {}",
            path.display()
        ));
    }
    Ok(Some(snapshot))
}

pub(super) async fn load(
    data_dir: &Path,
    session_id: &str,
) -> Result<Option<FrozenPrompt>, String> {
    let path = path(data_dir, session_id);
    tokio::task::spawn_blocking(move || read(&path))
        .await
        .map_err(|error| format!("prompt snapshot reader failed: {error}"))?
}

pub(super) async fn commit(
    data_dir: &Path,
    session_id: &str,
    snapshot: FrozenPrompt,
) -> Result<FrozenPrompt, String> {
    let path = path(data_dir, session_id);
    tokio::task::spawn_blocking(move || {
        let parent = path.parent().expect("snapshot has a parent");
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create prompt snapshot directory: {error}"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|error| format!("cannot stage prompt snapshot: {error}"))?;
        let bytes = serde_json::to_vec(&snapshot).map_err(|error| error.to_string())?;
        temporary
            .write_all(&bytes)
            .and_then(|_| temporary.as_file().sync_all())
            .map_err(|error| format!("cannot write prompt snapshot: {error}"))?;
        match temporary.persist_noclobber(&path) {
            Ok(_) => Ok(snapshot),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                read(&path)?.ok_or_else(|| "prompt snapshot disappeared during creation".into())
            }
            Err(error) => Err(format!(
                "cannot save prompt snapshot {}: {}",
                path.display(),
                error.error
            )),
        }
    })
    .await
    .map_err(|error| format!("prompt snapshot writer failed: {error}"))?
}
