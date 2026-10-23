use super::*;

#[tokio::test]
async fn internal_context_metadata_and_rendered_content_survive_native_history() {
    let dir = tempfile::tempdir().unwrap();
    let snapshot = rcode_agent::context::Snapshot { domain: rcode_agent::context::Domain::MemoryIndex,
        scope: "memory".into(), status: rcode_agent::context::Status::Ready,
        entries: std::collections::BTreeMap::from([("fact.md".into(), "fact".into())]) };
    let context = rcode_agent::context::next_record(&[], snapshot.clone()).unwrap().message();
    let recorder = Recorder::new(dir.path(), "entry", "model");
    recorder.record(&context).await.unwrap();
    recorder.record(&Message::assistant("done")).await.unwrap();
    let restored = load(dir.path(), "entry", "model").await.unwrap().unwrap();
    assert_eq!(restored[0], context);
    assert!(!restored[0].is_user_input());
    assert!(rcode_agent::context::next_record(&restored, snapshot).is_none());
}

#[tokio::test]
async fn manual_compaction_restores_only_todos_from_excluded_groups() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = Recorder::new(dir.path(), "excluded", "model");
    let call = Message::assistant(vec![ContentBlock::tool_use("todo", "TodoWrite", serde_json::json!({
        "todos":[{"text":"Validate the change","status":"in_progress"}]
    })), ContentBlock::tool_use("read", "Read", serde_json::json!({"file_path":"large.log"}))]);
    recorder.record(&call).await.unwrap();
    recorder.record(&Message::tool_result("todo", "todo result", false)).await.unwrap();
    recorder.record(&Message::tool_result("read", "EXCLUDED-TOOL-BODY", false)).await.unwrap();
    let state = rcode_agent::todo::State::from_history(&[
        call, Message::tool_result("todo", "todo result", false),
        Message::tool_result("read", "EXCLUDED-TOOL-BODY", false),
    ]);
    let reminder = state.record("reminder");
    recorder.record(&reminder).await.unwrap();
    Recorder::new(dir.path(), "later", "model").record(&Message::assistant("More work")).await.unwrap();
    let messages = crate::native::conversation_history(vec![rcode_proto::ConversationTurn {
        role: "user".into(), text: "Visible compacted summary".into(), native_history_id: None,
        todo_history_ids: vec!["excluded".into(), "later".into()],
    }], Some(dir.path()), "model").await.unwrap();
    let restored = rcode_agent::todo::State::from_history(&messages);
    assert!(restored.unfinished());
    assert_eq!(restored.turns_since_reminder, 1);
    assert_eq!(restored.current.as_ref().unwrap().todos[0].text, "Validate the change");
    assert!(messages.iter().all(|m| m.content.tool_calls().is_empty()));
    assert!(!messages.iter().any(|m| m.content.text_content().contains("EXCLUDED-TOOL-BODY")));
    let checkpoint = messages.iter().find(|m| m.context.is_some()).unwrap();
    assert!(!checkpoint.is_user_input());
}

fn assistant() -> Message {
    Message::assistant(vec![
        ContentBlock::Reasoning {
            text: "reason".into(),
            signature: Some("signature".into()),
        },
        ContentBlock::ToolUse {
            id: "one".into(),
            name: "Read".into(),
            input: serde_json::json!({"file_path":"a", "offset":800,"limit":100}),
        },
        ContentBlock::ToolUse {
            id: "two".into(),
            name: "Read".into(),
            input: serde_json::json!({"file_path":"b"}),
        },
    ])
}

#[tokio::test]
async fn reload_keeps_full_results_arguments_and_signatures() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = Recorder::new(dir.path(), "entry", "model");
    let original = assistant();
    let full = format!(
        "file.rs (lines 1-2000)\n{}\nLAST-LINE",
        "content\n".repeat(8000)
    );
    recorder.record(&original).await.unwrap();
    // Concurrent tools can complete out of order.
    recorder
        .record(&Message::tool_result("two", "second", false))
        .await
        .unwrap();
    recorder
        .record(&Message::tool_result("one", &full, false))
        .await
        .unwrap();
    drop(recorder);
    let history = load(dir.path(), "entry", "model").await.unwrap().unwrap();
    assert_eq!(history[0], original);
    let MessageContent::Blocks(blocks) = &history[1].content else {
        panic!()
    };
    assert!(
        matches!(&blocks[0], ContentBlock::ToolResult {tool_use_id,content,is_error:false} if tool_use_id=="one" && content==&full)
    );
    assert!(
        matches!(&blocks[1], ContentBlock::ToolResult {tool_use_id,content,..} if tool_use_id=="two" && content=="second")
    );
}

#[tokio::test]
async fn interrupted_calls_are_closed_without_reexecuting_them() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = Recorder::new(dir.path(), "entry", "model");
    recorder.record(&assistant()).await.unwrap();
    recorder
        .record(&Message::tool_result("one", "already written", false))
        .await
        .unwrap();
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path(dir.path(), "entry"))
        .await
        .unwrap();
    file.write_all(b"{\"partial\":").await.unwrap();
    file.flush().await.unwrap();
    let history = load(dir.path(), "entry", "model").await.unwrap().unwrap();
    let MessageContent::Blocks(blocks) = &history[1].content else {
        panic!()
    };
    assert!(
        matches!(&blocks[0], ContentBlock::ToolResult {content,is_error:false,..} if content=="already written")
    );
    assert!(
        matches!(&blocks[1], ContentBlock::ToolResult {tool_use_id,is_error:true,..} if tool_use_id=="two")
    );
}

#[tokio::test]
async fn corrupt_complete_records_are_reported_instead_of_silently_losing_history() {
    let dir = tempfile::tempdir().unwrap();
    let recorder = Recorder::new(dir.path(), "entry", "model");
    recorder.record(&assistant()).await.unwrap();
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path(dir.path(), "entry"))
        .await
        .unwrap();
    file.write_all(b"{bad}\n").await.unwrap();
    file.flush().await.unwrap();
    assert!(load(dir.path(), "entry", "model").await.is_err());
}

#[tokio::test]
async fn missing_legacy_record_is_distinct_from_an_io_failure() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        load(dir.path(), "missing", "model")
            .await
            .unwrap()
            .is_none()
    );
    tokio::fs::create_dir_all(path(dir.path(), "broken"))
        .await
        .unwrap();
    assert!(load(dir.path(), "broken", "model").await.is_err());
}

#[tokio::test]
async fn restored_groups_replace_display_summaries_once_and_legacy_text_survives() {
    let dir = tempfile::tempdir().unwrap();
    Recorder::new(dir.path(), "root", "model")
        .record(&Message::assistant("full result"))
        .await
        .unwrap();
    let turn = |role: &str, text: &str, group: Option<&str>| rcode_proto::ConversationTurn {
        role: role.into(),
        text: text.into(),
        native_history_id: group.map(str::to_owned),
        todo_history_ids: Vec::new(),
    };
    let messages = crate::native::conversation_history(
        vec![
            turn("user", "original task", None),
            turn("assistant", "summary", Some("root")),
            turn("assistant", "continuation summary", Some("root")),
            turn("assistant", "legacy full output", Some("missing")),
            // A partially compacted group deliberately has no record id.
            turn("assistant", "visible compacted tail", None),
        ],
        Some(dir.path()),
        "model",
    )
    .await
    .unwrap();
    assert_eq!(
        messages
            .iter()
            .map(|m| m.content.text_content())
            .collect::<Vec<_>>(),
        [
            "original task",
            "full result",
            "legacy full output",
            "visible compacted tail"
        ]
    );
}

#[tokio::test]
async fn switching_models_keeps_work_but_does_not_replay_foreign_reasoning() {
    let dir = tempfile::tempdir().unwrap();
    Recorder::new(dir.path(), "entry", "old-model")
        .record(&assistant())
        .await
        .unwrap();
    let history = load(dir.path(), "entry", "new-model")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(history[0].content.tool_calls().len(), 2);
    let MessageContent::Blocks(blocks) = &history[0].content else {
        panic!()
    };
    assert!(
        !blocks
            .iter()
            .any(|block| matches!(block, ContentBlock::Reasoning { .. }))
    );
}
