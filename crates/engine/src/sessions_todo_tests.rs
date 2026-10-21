use super::*;
use rcode_proto::{TodoItem, TodoStatus, ToolCall};

#[tokio::test]
async fn legacy_blank_list_recovers_from_its_own_journal_result_and_roundtrips() {
    let root = tempfile::tempdir().unwrap();
    let core = crate::EngineCore::assemble(
        root.path(),
        Arc::new(crate::HarnessRegistry::new()),
        HarnessId::Native,
        None,
    )
    .unwrap();
    core.workspace
        .create_chat("todo-recovery", None, Some(&core.device_id), None, None)
        .unwrap();
    let initial = core.doc_host.open("todo-recovery").unwrap();
    let doc = initial.doc();
    let mut parts = vec![];
    fold_event_into_parts(
        &mut parts,
        &AgentEvent::ToolCall {
            id: "todo-call".into(),
            call: ToolCall::Todo {
                items: vec![
                    TodoItem::new("", TodoStatus::Completed),
                    TodoItem::new("", TodoStatus::InProgress),
                    TodoItem::new("保留的文字", TodoStatus::Pending),
                ],
            },
        },
    );
    doc.push_message(&SessionMessageEntry {
        id: "assistant".into(),
        role: MessageRole::Assistant,
        parts,
        created_at: 0,
        device_id: core.device_id.clone(),
        status: Some(MessageStatus::Complete),
        continuation_of: None,
        duration_ms: None,
        excluded: false,
    })
    .unwrap();
    core.sessions
        .inner
        .journal
        .append(
            "todo-recovery",
            &AgentEvent::ToolResult {
                id: "todo-call".into(),
                is_error: false,
                output: Some(
                    "1 done, 1 in progress, 1 pending:\n[x] 检查代码\n[~] 验证异常处理\n[ ] 旧文字"
                        .into(),
                ),
                diff: None,
            },
        )
        .unwrap();
    core.sessions
        .inner
        .journal
        .append(
            "todo-recovery",
            &AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: None,
            },
        )
        .unwrap();
    core.doc_host.flush_all();
    core.shutdown().await;
    drop(initial);
    drop(core);
    let core = crate::EngineCore::assemble(
        root.path(),
        Arc::new(crate::HarnessRegistry::new()),
        HarnessId::Native,
        None,
    )
    .unwrap();
    let reopened = core.doc_host.open("todo-recovery").unwrap();
    let doc = reopened.doc();
    let entries = doc.read_entries().unwrap();
    let MessagePart::Tool {
        call: ToolCall::Todo { items },
        ..
    } = &entries[0].parts[0]
    else {
        panic!("missing todo");
    };
    assert_eq!(
        items
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>(),
        ["检查代码", "验证异常处理", "保留的文字"]
    );
    assert_eq!(items[0].status(), TodoStatus::Completed);
    assert_eq!(items[1].status(), TodoStatus::InProgress);
    let replica = SessionDoc::init("replica").unwrap();
    replica
        .doc()
        .import(&doc.export_snapshot().unwrap())
        .unwrap();
    assert_eq!(replica.read_entries().unwrap(), entries);
    let before = doc.export_snapshot().unwrap();
    core.sessions
        .repair_todo_labels_from_journal(&doc, "todo-recovery")
        .unwrap();
    assert_eq!(doc.export_snapshot().unwrap(), before);
}
