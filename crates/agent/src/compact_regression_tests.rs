use super::*;

fn exchange(messages: &mut Vec<Message>, id: &str, text: String, failed: bool) {
    messages.push(Message::assistant(vec![ContentBlock::ToolUse {
        id: id.into(),
        name: "Read".into(),
        input: serde_json::json!({"file_path":id}),
    }]));
    messages.push(Message::tool_result(id, text, failed));
}

#[test]
fn micro_preserves_recent_results_errors_and_both_ends_of_old_output() {
    let mut messages = vec![Message::user("fix the parser")];
    let large = format!("HEAD\n{}\nTAIL", "content".repeat(500));
    exchange(&mut messages, "old", large.clone(), false);
    exchange(&mut messages, "failed", large.clone(), true);
    exchange(&mut messages, "recent", large.clone(), false);
    exchange(&mut messages, "newest", large.clone(), false);
    let original = messages.clone();
    let config = CompactConfig {
        keep_recent_turns: 2,
        max_tool_output_chars: 160,
        ..Default::default()
    };
    assert_eq!(micro_compact(&mut messages, &config), 1);
    assert_eq!(
        messages[4..],
        original[4..],
        "error and recent outputs must survive verbatim"
    );
    let MessageContent::Blocks(blocks) = &messages[2].content else {
        panic!()
    };
    let ContentBlock::ToolResult { content, .. } = &blocks[0] else {
        panic!()
    };
    assert!(content.starts_with("HEAD"));
    assert!(content.ends_with("TAIL"));
    let once = messages.clone();
    assert_eq!(micro_compact(&mut messages, &config), 0);
    assert_eq!(
        messages, once,
        "repeated pressure must not chew away retained content"
    );
}

#[tokio::test]
async fn a_single_user_task_can_compact_old_tool_cycles_without_losing_its_instruction() {
    let task = "Fix the Windows link without changing unrelated files";
    let mut messages = vec![Message::user(task)];
    for index in 0..12 {
        exchange(
            &mut messages,
            &format!("call-{index}"),
            format!("RESULT-{index}"),
            false,
        );
    }
    let config = CompactConfig {
        keep_recent_turns: 4,
        ..Default::default()
    };
    let outcome = full_compact(messages, &config, |transcript| async move {
        assert!(transcript.contains(task));
        assert!(transcript.contains("RESULT-0"));
        assert!(!transcript.contains("RESULT-11"));
        Ok("SUMMARY".into())
    })
    .await
    .unwrap();
    assert!(outcome.compacted);
    assert_eq!(outcome.messages[1].content.text_content(), task);
    let calls: std::collections::HashSet<_> = outcome
        .messages
        .iter()
        .flat_map(|m| m.content.tool_calls())
        .map(|(id, _, _)| id)
        .collect();
    assert_eq!(calls.len(), 4);
    for message in &outcome.messages {
        if let MessageContent::Blocks(blocks) = &message.content {
            for block in blocks {
                if let ContentBlock::ToolResult { tool_use_id, .. } = block {
                    assert!(calls.contains(tool_use_id));
                }
            }
        }
    }
    assert!(calls.contains("call-11"));
}

#[test]
fn a_pending_tool_batch_cannot_be_split_by_an_internal_user_message() {
    let messages = vec![
        Message::user("task"),
        Message::assistant(vec![ContentBlock::ToolUse {
            id: "one".into(),
            name: "Read".into(),
            input: serde_json::json!({}),
        }]),
        Message::user("notification"),
        Message::tool_result("one", "result", false),
        Message::assistant("done"),
    ];
    assert_eq!(exchange_boundaries(&messages), vec![0, 4]);
}
