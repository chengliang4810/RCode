use super::*;

fn topic(root: &Path, name: &str, kind: &str) {
    std::fs::write(
        root.join(name),
        format!("---\nname: example\ndescription: context\ntype: {kind}\n---\nPRIVATE_TOPIC_BODY"),
    )
    .unwrap();
}

#[test]
fn only_declared_fact_pointers_move_and_body_is_never_injected() {
    let root = tempfile::tempdir().unwrap();
    for (name, kind) in [
        ("fact.md", "project"),
        ("ref.md", "reference"),
        ("user.md", "user"),
        ("feedback.md", "feedback"),
    ] {
        topic(root.path(), name, kind);
    }
    std::fs::write(root.path().join("unknown.md"), "no metadata").unwrap();
    std::fs::write(
        root.path().join("MEMORY.md"),
        [
            "# Index",
            "- [Fact](fact.md) — current fact",
            "- [Reference](ref.md) — pointer",
            "- [User](user.md) — preference",
            "- [Feedback](feedback.md) — behavior",
            "- [Unknown](unknown.md)",
            "Unclassified instruction",
        ]
        .join("\n"),
    )
    .unwrap();
    let index = load(root.path());
    assert_eq!(index.facts.entries.len(), 2);
    assert!(!index.protected.contains("current fact"));
    for retained in [
        "preference",
        "behavior",
        "unknown.md",
        "Unclassified instruction",
    ] {
        assert!(index.protected.contains(retained));
    }
    assert!(!format!("{index:?}").contains("PRIVATE_TOPIC_BODY"));
}

#[test]
fn duplicate_or_malformed_pointers_remain_in_system_instead_of_losing_content() {
    let root = tempfile::tempdir().unwrap();
    topic(root.path(), "a.md", "project");
    let text = "- [A](a.md) — first\n- [Alias](./a.md) — second\n- [Missing](missing.md)\n- [Nested](a(b).md)";
    std::fs::write(root.path().join("MEMORY.md"), text).unwrap();
    let index = load(root.path());
    assert!(index.facts.entries.is_empty());
    assert_eq!(index.protected, text);
}

#[test]
fn external_paths_keep_existing_trust_all_behavior_and_header_changes_are_seen() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    topic(outside.path(), "a.md", "reference");
    let link = format!(
        "- [Outside]({}) — reference",
        outside.path().join("a.md").display()
    );
    std::fs::write(root.path().join("MEMORY.md"), &link).unwrap();
    assert_eq!(load(root.path()).facts.entries.len(), 1);
    topic(outside.path(), "a.md", "feedback");
    let index = load(root.path());
    assert!(index.facts.entries.is_empty());
    assert_eq!(index.protected, link);
}

#[test]
fn errors_are_distinct_from_empty_and_unbounded_headers_stay_protected() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(load(root.path()).facts.status, Status::Ready);
    std::fs::create_dir(root.path().join("MEMORY.md")).unwrap();
    assert_eq!(load(root.path()).facts.status, Status::Unavailable);
    std::fs::remove_dir(root.path().join("MEMORY.md")).unwrap();
    std::fs::write(
        root.path().join("a.md"),
        format!("---\n{}\ntype: project\n---", "name: x\n".repeat(1000)),
    )
    .unwrap();
    std::fs::write(root.path().join("MEMORY.md"), "- [A](a.md)").unwrap();
    assert!(load(root.path()).facts.entries.is_empty());
}

#[test]
fn unavailable_directory_does_not_claim_memory_is_ready_or_already_exists() {
    let project = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    std::fs::write(config.path().join("projects"), "not a directory").unwrap();
    let system = crate::memory::system_prompt_section(project.path(), Some(config.path())).unwrap();
    assert!(system.contains("Automatic memory is unavailable"));
    assert!(!system.contains("directory already exists"));
    let root = crate::memory::directory(project.path(), config.path());
    assert_eq!(load(&root).facts.status, Status::Unavailable);
}
