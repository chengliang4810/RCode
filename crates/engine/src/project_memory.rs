use std::path::Path;

use rcode_proto::{ProjectMemoryFile, ProjectMemoryListing};

const MAX_FILES: usize = 1000;
const MAX_DEPTH: usize = 16;

pub(crate) fn list(cwd: &Path, config_root: &Path) -> std::io::Result<ProjectMemoryListing> {
    let enabled = rcode_harness::settings::AgentSettings::load(config_root).memory;
    let mut listing = ProjectMemoryListing {
        enabled,
        directory: None,
        files: Vec::new(),
        truncated: false,
    };
    if !enabled {
        return Ok(listing);
    }
    let root = rcode_agent::memory::directory(cwd, config_root);
    listing.directory = Some(root.to_string_lossy().replace('\\', "/"));
    if !root.exists() {
        return Ok(listing);
    }
    scan(&root, &root, 0, &mut listing)?;
    listing
        .files
        .sort_by(|a, b| (a.path != "MEMORY.md", &a.path).cmp(&(b.path != "MEMORY.md", &b.path)));
    Ok(listing)
}

fn scan(
    root: &Path,
    directory: &Path,
    depth: usize,
    listing: &mut ProjectMemoryListing,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(directory)? {
        if listing.files.len() >= MAX_FILES || depth > MAX_DEPTH {
            listing.truncated = true;
            break;
        }
        let entry = entry?;
        let kind = entry.file_type()?;
        let path = entry.path();
        if kind.is_dir() {
            scan(root, &path, depth + 1, listing)?;
        } else if kind.is_file() || (kind.is_symlink() && path.is_file()) {
            listing.files.push(ProjectMemoryFile {
                path: path
                    .strip_prefix(root)
                    .expect("entry is below memory root")
                    .to_string_lossy()
                    .replace('\\', "/"),
                absolute_path: path.to_string_lossy().replace('\\', "/"),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_uses_the_agent_directory_and_obeys_the_memory_switch() {
        let project = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        assert!(
            list(project.path(), config.path())
                .unwrap()
                .files
                .is_empty()
        );
        let root = rcode_agent::memory::directory(project.path(), config.path());
        assert!(!root.exists());
        std::fs::create_dir_all(root.join("topics")).unwrap();
        std::fs::write(root.join("MEMORY.md"), "- [Shell](topics/shell.md)").unwrap();
        std::fs::write(root.join("topics/shell.md"), "Prefer PowerShell").unwrap();
        let listing = list(project.path(), config.path()).unwrap();
        assert!(listing.enabled);
        assert_eq!(listing.files[0].path, "MEMORY.md");
        assert_eq!(listing.files[1].path, "topics/shell.md");
        assert_eq!(
            std::fs::read_to_string(&listing.files[1].absolute_path).unwrap(),
            "Prefer PowerShell"
        );
        assert!(list(other.path(), config.path()).unwrap().files.is_empty());
        std::fs::write(
            config.path().join("agent-settings.json"),
            r#"{"memory":false}"#,
        )
        .unwrap();
        let disabled = list(project.path(), config.path()).unwrap();
        assert!(!disabled.enabled);
        assert!(disabled.files.is_empty());
        assert!(root.join("topics/shell.md").exists());
    }
}
