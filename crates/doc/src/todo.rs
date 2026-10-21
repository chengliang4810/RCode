//! Recover labels erased by the legacy content-only TodoWrite decoder.
use rcode_proto::TodoItem;

pub fn recover_blank_todo_labels(items: &[TodoItem], output: &str) -> Option<Vec<TodoItem>> {
    if items.is_empty() || !items.iter().any(|item| item.text.trim().is_empty()) {
        return None;
    }
    let mut labels: Vec<String> = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if let Some(text) = ["[x] ", "[~] ", "[ ] "]
            .into_iter()
            .find_map(|prefix| line.strip_prefix(prefix))
        {
            labels.push(text.to_owned());
        } else if !labels.is_empty() && !line.starts_with("(note:") && !line.is_empty() {
            labels.last_mut()?.push_str(&format!("\n{line}"));
        }
    }
    // Counts must match exactly. Never invent labels or overwrite known text.
    if labels.len() != items.len() || labels.iter().any(|label| label.trim().is_empty()) {
        return None;
    }
    let mut repaired = items.to_vec();
    for (item, label) in repaired.iter_mut().zip(labels) {
        if item.text.trim().is_empty() {
            item.text = label;
        }
    }
    Some(repaired)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcode_proto::TodoStatus;
    #[test]
    fn recovers_labels_without_changing_statuses_or_existing_text() {
        let items = vec![
            TodoItem::new("", TodoStatus::Completed),
            TodoItem::new("已知任务", TodoStatus::InProgress),
            TodoItem::new("", TodoStatus::Pending),
        ];
        let result = recover_blank_todo_labels(
            &items,
            "1 done, 1 in progress, 1 pending:\n[x] 检查代码\n[~] 旧文本\n[ ] 验证测试",
        )
        .unwrap();
        assert_eq!(
            result
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            ["检查代码", "已知任务", "验证测试"]
        );
        assert_eq!(
            result.iter().map(TodoItem::status).collect::<Vec<_>>(),
            items.iter().map(TodoItem::status).collect::<Vec<_>>()
        );
    }
    #[test]
    fn truncated_or_unrelated_output_does_not_guess_labels() {
        let items = vec![
            TodoItem::new("", TodoStatus::Pending),
            TodoItem::new("", TodoStatus::Pending),
        ];
        assert!(recover_blank_todo_labels(&items, "[ ] only one").is_none());
        assert!(recover_blank_todo_labels(&items, "1 done, 1 pending").is_none());
        assert!(
            recover_blank_todo_labels(
                &[TodoItem::new("known", TodoStatus::Pending)],
                "[ ] changed"
            )
            .is_none()
        );
    }
}
