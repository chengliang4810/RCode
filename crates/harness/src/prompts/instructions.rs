use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: usize = 32 * 1024;

/// User instructions precede the bound workspace instructions, without wrappers.
pub fn load(workspace: &Path, user_home: Option<&Path>) -> String {
    let user_config = user_home.map(|home| home.join(".rcode"));
    let mut seen = HashSet::new();
    user_config
        .as_deref()
        .into_iter()
        .chain(std::iter::once(workspace))
        .filter_map(read)
        .filter_map(|(path, text)| seen.insert(path).then_some(text))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn read(workspace: &Path) -> Option<(PathBuf, String)> {
    let path = workspace.join("AGENTS.md");
    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
        return None;
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return None;
    }
    let (Ok(path), Ok(scope)) = (path.canonicalize(), workspace.canonicalize()) else {
        return None;
    };
    if !path.starts_with(&scope) {
        return None;
    }
    let Ok(file) = std::fs::File::open(&path) else {
        return None;
    };
    let mut bytes = Vec::new();
    if file
        .take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return None;
    }
    let truncated = bytes.len() > MAX_FILE_BYTES;
    bytes.truncate(MAX_FILE_BYTES);
    let text = String::from_utf8_lossy(&bytes);
    if text.trim().is_empty() {
        return None;
    }
    let mut text = text.into_owned();
    if truncated {
        text.push_str("\n[Instruction file truncated]");
    }
    Some((path, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_and_project_bodies_are_joined_verbatim_and_duplicate_files_are_skipped() {
        let user = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir(user.path().join(".rcode")).unwrap();
        let user_body = "  USER RULES\n\n";
        let project_body = "\nPROJECT RULES  \n";
        std::fs::write(user.path().join(".rcode/AGENTS.md"), user_body).unwrap();
        std::fs::write(project.path().join("AGENTS.md"), project_body).unwrap();
        assert_eq!(
            load(project.path(), Some(user.path())),
            format!("{user_body}\n\n{project_body}")
        );
        assert_eq!(
            load(&user.path().join(".rcode"), Some(user.path())),
            user_body
        );
        std::fs::write(user.path().join(".rcode/AGENTS.md"), " \n").unwrap();
        assert_eq!(load(project.path(), Some(user.path())), project_body);
    }

    #[test]
    fn only_the_bound_root_file_is_loaded() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::create_dir_all(project.join(".claude")).unwrap();
        std::fs::write(root.path().join("AGENTS.md"), "PARENT POLICY").unwrap();
        std::fs::write(project.join("src/AGENTS.md"), "CHILD POLICY").unwrap();
        std::fs::write(project.join("CLAUDE.md"), "CLAUDE POLICY").unwrap();
        std::fs::write(project.join(".claude/AGENTS.md"), "HIDDEN POLICY").unwrap();
        assert!(load(&project, None).is_empty());
        std::fs::write(project.join("AGENTS.md"), "ROOT POLICY\n@shared.md").unwrap();
        std::fs::write(project.join("shared.md"), "IMPORTED POLICY").unwrap();
        let text = load(&project, None);
        assert!(text.contains("ROOT POLICY"));
        for ignored in [
            "PARENT POLICY",
            "CHILD POLICY",
            "CLAUDE POLICY",
            "HIDDEN POLICY",
            "IMPORTED POLICY",
        ] {
            assert!(!text.contains(ignored));
        }
        let nested = project.join("src/deep");
        std::fs::create_dir_all(&nested).unwrap();
        assert!(
            load(&nested, None).is_empty(),
            "must not search above the bound directory"
        );
    }

    #[test]
    fn missing_empty_and_oversized_files_have_explicit_results() {
        let project = tempfile::tempdir().unwrap();
        assert!(load(project.path(), None).is_empty());
        std::fs::write(project.path().join("AGENTS.md"), " \n\t").unwrap();
        assert!(load(project.path(), None).is_empty());
        std::fs::write(
            project.path().join("AGENTS.md"),
            "x".repeat(MAX_FILE_BYTES + 10),
        )
        .unwrap();
        let text = load(project.path(), None);
        assert!(text.ends_with("[Instruction file truncated]"));
        assert_eq!(
            text,
            format!(
                "{}\n[Instruction file truncated]",
                "x".repeat(MAX_FILE_BYTES)
            )
        );
    }
}
