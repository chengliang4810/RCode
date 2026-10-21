use super::*;

#[tokio::test]
async fn user_agents_path_is_independent_of_the_data_directory() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let first_data = tempfile::tempdir().unwrap();
    let second_data = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".rcode")).unwrap();
    std::fs::write(home.path().join(".rcode/AGENTS.md"), "FIXED HOME POLICY").unwrap();
    std::fs::write(project.path().join("AGENTS.md"), "PROJECT POLICY").unwrap();
    for data in [first_data.path(), second_data.path()] {
        std::fs::write(data.join("AGENTS.md"), "REDIRECTED DATA POLICY").unwrap();
        let result = PromptSessions::default()
            .prepare(PromptRequest {
                cwd: project.path(),
                data_dir: Some(data),
                user_home: Some(home.path()),
                tools: &ToolRegistry::new(),
                memory: None,
                mcp_instructions: None,
                settings: &PromptSettings::default(),
                resume: Some("one"),
            })
            .await
            .unwrap();
        assert_eq!(
            result.system.frozen_sections.last().unwrap(),
            "FIXED HOME POLICY\n\nPROJECT POLICY"
        );
        assert!(!result.system.text().contains("REDIRECTED DATA POLICY"));
    }
}

async fn prepare(
    sessions: &PromptSessions,
    cwd: &Path,
    data: &Path,
    id: &str,
) -> Result<PreparedPrompt, String> {
    sessions
        .prepare(PromptRequest {
            cwd,
            data_dir: Some(data),
            user_home: None,
            tools: &ToolRegistry::new(),
            memory: None,
            mcp_instructions: None,
            settings: &PromptSettings::default(),
            resume: Some(id),
        })
        .await
}

#[tokio::test]
async fn restart_and_compaction_preserve_the_exact_snapshot() {
    let project = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("AGENTS.md"), "ORIGINAL POLICY").unwrap();
    let sessions = PromptSessions::default();
    let first = prepare(&sessions, project.path(), data.path(), "one")
        .await
        .unwrap();
    let path = snapshot::path(data.path(), "one");
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(project.path().join("AGENTS.md"), "UPDATED POLICY").unwrap();
    std::fs::write(project.path().join("CLAUDE.md"), "IGNORED POLICY").unwrap();
    sessions.sessions.lock().unwrap().remove("one");
    let reset = prepare(&sessions, project.path(), data.path(), "one")
        .await
        .unwrap();
    let restarted = prepare(
        &PromptSessions::default(),
        project.path(),
        data.path(),
        "one",
    )
    .await
    .unwrap();
    assert_eq!(first.system.text(), reset.system.text());
    assert_eq!(first.system.text(), restarted.system.text());
    assert_eq!(bytes, std::fs::read(path).unwrap());
    assert!(restarted.system.text().contains("ORIGINAL POLICY"));
    assert!(!restarted.system.text().contains("UPDATED POLICY"));
    assert!(!restarted.system.text().contains("IGNORED POLICY"));
    let new_chat = prepare(&sessions, project.path(), data.path(), "two")
        .await
        .unwrap();
    assert!(new_chat.system.text().contains("UPDATED POLICY"));
}

#[tokio::test]
async fn invalid_or_unwritable_snapshots_fail_without_replacing_them() {
    let project = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    prepare(
        &PromptSessions::default(),
        project.path(),
        data.path(),
        "one",
    )
    .await
    .unwrap();
    let path = snapshot::path(data.path(), "one");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["version"] = 999.into();
    let future = serde_json::to_vec(&value).unwrap();
    std::fs::write(&path, &future).unwrap();
    assert!(
        prepare(
            &PromptSessions::default(),
            project.path(),
            data.path(),
            "one"
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), future);
    std::fs::write(&path, "{broken").unwrap();
    assert!(
        prepare(
            &PromptSessions::default(),
            project.path(),
            data.path(),
            "one"
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
    let blocked = tempfile::tempdir().unwrap();
    std::fs::write(blocked.path().join("prompt-snapshots"), "not a directory").unwrap();
    assert!(
        prepare(
            &PromptSessions::default(),
            project.path(),
            blocked.path(),
            "one"
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn concurrent_creators_use_one_committed_snapshot() {
    let project = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let first = PromptSessions::default();
    let second = PromptSessions::default();
    let (one, two) = tokio::join!(
        prepare(&first, project.path(), data.path(), "../../one"),
        prepare(&second, project.path(), data.path(), "../../one"),
    );
    assert_eq!(one.unwrap().system.text(), two.unwrap().system.text());
    assert_eq!(
        std::fs::read_dir(data.path().join("prompt-snapshots"))
            .unwrap()
            .count(),
        1
    );
    assert!(
        snapshot::path(data.path(), "../../one").starts_with(data.path().join("prompt-snapshots"))
    );
}
