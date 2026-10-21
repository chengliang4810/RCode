//! Project-scoped automatic memory, maintained with the ordinary file tools.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub const INDEX_FILE: &str = "MEMORY.md";
const MAX_INDEX_LINES: usize = 200;
const MAX_INDEX_BYTES: usize = 25_000;

/// The main checkout and its worktrees share one key. Ordinary directories
/// use their canonical path; importing the same directory again keeps the key.
pub fn project_root(cwd: &Path) -> PathBuf {
    let absolute = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    for root in absolute.ancestors() {
        let marker = root.join(".git");
        if marker.is_dir() {
            return root.to_path_buf();
        }
        if let Ok(text) = std::fs::read_to_string(&marker) {
            if let Some(gitdir) = text.trim().strip_prefix("gitdir:") {
                let gitdir = root.join(gitdir.trim());
                if let Ok(common) = std::fs::read_to_string(gitdir.join("commondir")) {
                    if let Ok(common) = gitdir.join(common.trim()).canonicalize() {
                        if common.file_name().is_some_and(|name| name == ".git") {
                            return common.parent().unwrap_or(root).to_path_buf();
                        }
                    }
                }
                return root.to_path_buf();
            }
        }
    }
    absolute
}

pub fn directory(cwd: &Path, config_dir: &Path) -> PathBuf {
    let root = project_root(cwd);
    let identity = root.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    let identity = identity.to_lowercase();
    let digest = Sha256::digest(identity.as_bytes());
    let key: String = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    config_dir.join("projects").join(key).join("memory")
}

/// Only the index is injected. Fact files stay on disk until Read/Grep needs
/// them. Bound the read even when the index file has grown unexpectedly large.
pub fn read_index(root: &Path) -> std::io::Result<Option<String>> {
    let path = root.join(INDEX_FILE);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "memory index must be a regular file",
        ));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((MAX_INDEX_BYTES + 4) as u64)
        .read_to_end(&mut bytes)?;
    let complete = std::str::from_utf8(&bytes);
    let text = match complete {
        Ok(text) => text,
        Err(error) if metadata.len() > MAX_INDEX_BYTES as u64 && error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).expect("valid UTF-8 prefix")
        }
        Err(error) => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
    };
    let mut end = text.len().min(MAX_INDEX_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = &text[..end];
    let mut index = prefix
        .lines()
        .take(MAX_INDEX_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    if metadata.len() > MAX_INDEX_BYTES as u64 || prefix.lines().count() > MAX_INDEX_LINES {
        index.push_str("\n[MEMORY.md was truncated. Keep the index under 200 lines and 25,000 bytes; move detail into topic files.]");
    }
    Ok(Some(index))
}

pub fn system_prompt_section(cwd: &Path, config_dir: Option<&Path>) -> Option<String> {
    let root = directory(cwd, config_dir?);
    if let Err(error) = std::fs::create_dir_all(&root) {
        tracing::warn!(path = %root.display(), %error, "cannot create automatic memory directory");
        return None;
    }
    let index = match read_index(&root) {
        Ok(index) => index
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| "(The index is currently empty.)".into()),
        Err(error) => {
            tracing::warn!(path = %root.display(), %error, "cannot read automatic memory index");
            "(The index could not be read. Inspect it with the file tools before changing it.)"
                .into()
        }
    };
    Some(format!(
        "{}\n\n## Memory directory\n\n`{}`\n\n## MEMORY.md\n\n<auto-memory-index>\n{index}\n</auto-memory-index>",
        include_str!("memory_prompt.md").trim(),
        root.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_imports_and_worktrees_share_a_directory() {
        let project = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let worktree = tempfile::tempdir().unwrap();
        let metadata = project.path().join(".git/worktrees/feature");
        std::fs::create_dir_all(&metadata).unwrap();
        std::fs::create_dir_all(project.path().join("src")).unwrap();
        std::fs::write(metadata.join("commondir"), "../..\n").unwrap();
        std::fs::write(
            worktree.path().join(".git"),
            format!("gitdir: {}\n", metadata.display()),
        )
        .unwrap();
        let main = directory(project.path(), config.path());
        assert_eq!(main, directory(&project.path().join("src"), config.path()));
        assert_eq!(main, directory(worktree.path(), config.path()));
        assert!(main.starts_with(config.path().join("projects")));
        assert_eq!(main.file_name().unwrap(), "memory");
    }

    #[test]
    fn unrelated_folders_have_distinct_memory() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        assert_ne!(
            directory(first.path(), first.path()),
            directory(second.path(), first.path())
        );
        assert_eq!(
            directory(first.path(), first.path()),
            directory(first.path(), first.path())
        );
    }

    #[test]
    fn prompt_exists_before_first_fact_and_loads_only_the_index() {
        let project = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let empty = system_prompt_section(project.path(), Some(config.path())).unwrap();
        assert!(empty.contains("# Auto memory"));
        assert!(empty.contains("currently empty"));
        let root = directory(project.path(), config.path());
        std::fs::write(
            root.join(INDEX_FILE),
            "- [Preference](preference.md) — terminal preference",
        )
        .unwrap();
        std::fs::write(root.join("preference.md"), "PRIVATE_FACT_BODY").unwrap();
        let prompt = system_prompt_section(project.path(), Some(config.path())).unwrap();
        assert!(prompt.contains("terminal preference"));
        assert!(!prompt.contains("PRIVATE_FACT_BODY"));
        assert!(!prompt.contains("MemoryWrite"));
        assert!(!prompt.contains("team"));
    }

    #[test]
    fn index_limits_apply_to_lines_and_utf8_bytes() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(INDEX_FILE),
            (0..205).map(|i| format!("entry-{i}\n")).collect::<String>(),
        )
        .unwrap();
        let index = read_index(root.path()).unwrap().unwrap();
        assert!(index.contains("entry-199"));
        assert!(!index.contains("entry-200"));
        assert!(index.contains("truncated"));
        std::fs::write(root.path().join(INDEX_FILE), "中".repeat(10_000)).unwrap();
        let index = read_index(root.path()).unwrap().unwrap();
        assert!(index.starts_with('中'));
        assert!(index.len() < MAX_INDEX_BYTES + 200);
        assert!(index.contains("truncated"));
    }
}
