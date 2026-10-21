//! The device's global AGENTS.md, shared by settings and the native prompt loader.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_BYTES: usize = 32 * 1024;
static WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InstructionsDocument {
    pub content: String,
    pub revision: Option<String>,
}

pub(crate) fn read(root: &Path) -> Result<InstructionsDocument, String> {
    let path = root.join("AGENTS.md");
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(InstructionsDocument {
                content: String::new(),
                revision: None,
            });
        }
        Err(error) => return Err(error.to_string()),
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err("AGENTS.md must be a regular file".into());
        }
        Ok(_) => {}
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("AGENTS.md exceeds 32 KiB".into());
    }
    let revision = Some(format!("{:x}", Sha256::digest(&bytes)));
    let content = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    Ok(InstructionsDocument { content, revision })
}

pub(crate) fn save(
    root: &Path,
    content: String,
    expected: Option<String>,
) -> Result<InstructionsDocument, String> {
    if content.len() > MAX_BYTES {
        return Err("AGENTS.md exceeds 32 KiB".into());
    }
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let current = read(root)?;
    if current.revision != expected {
        return Err("AGENTS.md changed outside this editor; reload before saving".into());
    }
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(root).map_err(|e| e.to_string())?;
    temp.write_all(content.as_bytes())
        .map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(root.join("AGENTS.md"))
        .map_err(|e| e.error.to_string())?;
    Ok(InstructionsDocument {
        revision: Some(format!("{:x}", Sha256::digest(content.as_bytes()))),
        content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_instructions_create_replace_clear_and_preserve_text() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(dir.path()).unwrap().revision.is_none());
        let text = "# 自定义指令\r\n保留格式。\r\n";
        let saved = save(dir.path(), text.into(), None).unwrap();
        assert_eq!(saved.content, text);
        let replaced = save(dir.path(), "新指令\n".into(), saved.revision).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap(),
            replaced.content
        );
        let cleared = save(dir.path(), String::new(), replaced.revision).unwrap();
        assert_eq!(cleared.content, "");
    }

    #[test]
    fn custom_instructions_preserve_external_edits_and_reject_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let saved = save(dir.path(), "initial".into(), None).unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "external").unwrap();
        assert!(save(dir.path(), "draft".into(), saved.revision).is_err());
        assert_eq!(read(dir.path()).unwrap().content, "external");
        assert!(save(dir.path(), "x".repeat(MAX_BYTES + 1), None).is_err());
        std::fs::write(dir.path().join("AGENTS.md"), "x".repeat(MAX_BYTES + 1)).unwrap();
        assert!(read(dir.path()).is_err());
    }
}
