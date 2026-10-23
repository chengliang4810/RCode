use super::*;

fn enabled() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("request-diagnostics.json"),
        r#"{"enabled":true}"#,
    )
    .unwrap();
    dir
}
fn finish(id: &str) -> DiagnosticEvent {
    DiagnosticEvent {
        attempt_id: id.into(),
        data: DiagnosticData::Finished {
            outcome: "completed".into(),
            duration_ms: 10,
            usage: None,
            request_id: None,
            error_kind: None,
        },
    }
}
fn records(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("request-diagnostics/requests.jsonl"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}

#[test]
fn opt_in_is_separate_from_agent_settings_and_does_not_create_files_when_disabled() {
    let dir = tempfile::tempdir().unwrap();
    assert!(SessionTrace::open(dir.path(), "session", "assistant").is_none());
    std::fs::write(
        dir.path().join("request-diagnostics.json"),
        r#"{"enabled":false}"#,
    )
    .unwrap();
    assert!(SessionTrace::open(dir.path(), "session", "assistant").is_none());
    std::fs::write(dir.path().join("request-diagnostics.json"), "invalid").unwrap();
    assert!(SessionTrace::open(dir.path(), "session", "assistant").is_none());
    assert!(!dir.path().join("request-diagnostics").exists());
}

#[test]
fn links_completed_calls_in_order_even_when_consumers_lag_and_numbers_outer_retries() {
    let dir = enabled();
    let session = SessionTrace::open(dir.path(), "session", "assistant").unwrap();
    let first = session.begin("conversation");
    first.on_event(DiagnosticEvent {
        attempt_id: "failed".into(),
        data: DiagnosticData::Finished {
            outcome: "error".into(),
            duration_ms: 5,
            usage: None,
            request_id: None,
            error_kind: None,
        },
    });
    first.on_event(finish("success"));
    let second = session.begin("conversation");
    second.on_event(finish("next-call"));
    session.link_model_call("doc-call-1");
    session.link_model_call("doc-call-2");
    session.sink.flush();
    let rows = records(dir.path());
    assert_eq!(rows[0]["attempt"], 1);
    assert_eq!(rows[1]["attempt"], 2);
    assert_eq!(rows[0]["callId"], rows[1]["callId"]);
    assert_ne!(rows[1]["callId"], rows[2]["callId"]);
    assert_eq!(rows[3]["modelCallId"], "doc-call-1");
    assert_eq!(rows[3]["callId"], rows[1]["callId"]);
    assert_eq!(rows[4]["modelCallId"], "doc-call-2");
    assert_eq!(rows[4]["callId"], rows[2]["callId"]);
    assert!(Arc::ptr_eq(
        &session.sink,
        &SessionTrace::open(dir.path(), "other", "other")
            .unwrap()
            .sink
    ));
}

#[test]
fn rotation_preserves_two_backups_and_write_errors_do_not_escape_the_observer() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("log");
    for n in 0..6 {
        append(&root, format!("{n}\n").as_bytes(), 3).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(root.join("requests.jsonl")).unwrap(),
        "5\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("requests.1.jsonl")).unwrap(),
        "4\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("requests.2.jsonl")).unwrap(),
        "3\n"
    );
    let bad = enabled();
    std::fs::write(bad.path().join("request-diagnostics"), "ordinary file").unwrap();
    let session = SessionTrace::open(bad.path(), "session", "assistant").unwrap();
    session.begin("conversation").on_event(finish("request"));
    session.sink.flush();
}
