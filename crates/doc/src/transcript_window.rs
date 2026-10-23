//! Cursor-based reads of message parts without expanding the complete transcript.
use crate::{DocError, SessionDoc, SessionMessageEntry, join_continuation_entries};
use loro::{Container, ToJson, ValueOrContainer};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptCursor {
    pub entry: String,
    pub part: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptWindow {
    pub start: Option<TranscriptCursor>,
    pub has_older: bool,
    pub parts: usize,
}

impl SessionDoc {
    pub fn read_transcript_window(
        &self,
        start: Option<&TranscriptCursor>,
        older: bool,
        page_size: usize,
    ) -> Result<(Vec<SessionMessageEntry>, TranscriptWindow), DocError> {
        let messages = self.doc().get_list("messages");
        // Resolve the boundary using scalar ids before allocating slots. A
        // stale cursor used to retain a slot for every part just to truncate
        // them at the end. Do not cap the search itself: a valid cursor may be
        // arbitrarily deep in the history.
        let start = start.filter(|cursor| {
            (0..messages.len()).rev().any(|index| {
                let Some(ValueOrContainer::Container(Container::Map(map))) = messages.get(index)
                else {
                    return false;
                };
                if !scalar_id_matches(&map, &cursor.entry) {
                    return false;
                }
                match (map.get("parts"), cursor.part.as_deref()) {
                    (Some(ValueOrContainer::Container(Container::List(parts))), Some(id)) => {
                        (0..parts.len()).any(|offset| {
                            matches!(parts.get(offset),
                                Some(ValueOrContainer::Container(Container::Map(part)))
                                    if scalar_id_matches(&part, id))
                        })
                    }
                    (Some(ValueOrContainer::Container(Container::List(parts))), None) => {
                        parts.is_empty()
                    }
                    (_, None) => true,
                    _ => false,
                }
            })
        });
        // Only scalar ids are inspected until the desired boundary is located.
        // Deleted/stale cursors fall back to a bounded tail, never a full-history decode.
        let mut slots = Vec::new();
        let mut found = start.is_none();
        let mut budget = page_size.max(1);
        let mut has_older = false;
        'entries: for index in (0..messages.len()).rev() {
            let Some(ValueOrContainer::Container(Container::Map(map))) = messages.get(index) else {
                continue;
            };
            let id = map
                .get("id")
                .map(|v| v.get_deep_value().to_json_value())
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            let parts = match map.get("parts") {
                Some(ValueOrContainer::Container(Container::List(parts))) => Some(parts),
                _ => None,
            };
            let count = parts.as_ref().map(|p| p.len()).unwrap_or(0);
            for offset in (0..count.max(1)).rev() {
                let part = parts.as_ref().and_then(|p| p.get(offset));
                let part_id = match part {
                    Some(ValueOrContainer::Container(Container::Map(ref map))) => map
                        .get("id")
                        .map(|v| v.get_deep_value().to_json_value())
                        .and_then(|v| v.as_str().map(str::to_owned)),
                    _ => None,
                };
                let cursor = TranscriptCursor {
                    entry: id.clone(),
                    part: part_id,
                };
                slots.push((
                    index,
                    if count == 0 { None } else { Some(offset) },
                    cursor.clone(),
                ));
                let at_start = start.is_some_and(|start| *start == cursor);
                if at_start && !found {
                    found = true;
                    if !older {
                        budget = 1;
                    } else {
                        budget = page_size.saturating_add(1);
                    }
                }
                if found {
                    budget = budget.saturating_sub(1);
                    if budget == 0 {
                        has_older = index > 0 || offset > 0;
                        break 'entries;
                    }
                }
            }
        }
        if !found {
            has_older = slots.len() > page_size;
            slots.truncate(page_size.max(1));
        }
        let cursor = slots.last().map(|(_, _, cursor)| cursor.clone());
        let count = slots.len();
        slots.reverse();
        let mut entries = Vec::new();
        let mut at = 0;
        while at < slots.len() {
            let index = slots[at].0;
            let Some(ValueOrContainer::Container(Container::Map(map))) = messages.get(index) else {
                at += 1;
                continue;
            };
            let mut value = map.get_value().to_json_value();
            let mut tail = Vec::new();
            let parts = match map.get("parts") {
                Some(ValueOrContainer::Container(Container::List(parts))) => Some(parts),
                _ => None,
            };
            while at < slots.len() && slots[at].0 == index {
                if let Some(part) = slots[at]
                    .1
                    .and_then(|i| parts.as_ref().and_then(|parts| parts.get(i)))
                {
                    tail.push(part.get_deep_value().to_json_value());
                }
                at += 1;
            }
            value["parts"] = serde_json::Value::Array(tail);
            if let Ok(entry) = crate::schema::entry_from_json(value) {
                entries.push(entry);
            }
        }
        if let Some(first) = entries.first_mut() {
            if let Some(root) = first.continuation_of.take() {
                first.id = root;
            }
        }
        Ok((
            join_continuation_entries(entries),
            TranscriptWindow {
                start: cursor,
                has_older,
                parts: count,
            },
        ))
    }
}

fn scalar_id_matches(map: &loro::LoroMap, expected: &str) -> bool {
    matches!(map.get("id"), Some(ValueOrContainer::Value(loro::LoroValue::String(id)))
        if id.as_ref() == expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MessagePart, MessageRole, MessageStatus};
    fn entry(id: &str, count: usize) -> SessionMessageEntry {
        SessionMessageEntry {
            id: id.into(),
            role: MessageRole::Assistant,
            parts: (0..count)
                .map(|i| MessagePart::Text {
                    id: format!("p{i}"),
                    text: format!("paragraph {i}"),
                })
                .collect(),
            created_at: 0,
            device_id: "host".into(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
            duration_ms: Some(10),
            excluded: false,
        }
    }
    #[test]
    fn stale_cursor_is_bounded_without_discarding_a_deep_valid_cursor() {
        let doc = SessionDoc::init("deep-window").unwrap();
        doc.push_message(&entry("old", 1)).unwrap();
        doc.push_message(&entry("new", 2048)).unwrap();
        let deep = TranscriptCursor {
            entry: "old".into(),
            part: Some("p0".into()),
        };
        let (entries, window) = doc.read_transcript_window(Some(&deep), false, 8).unwrap();
        assert_eq!(window.parts, 2049);
        assert_eq!(entries.first().unwrap().id, "old");
        let stale = TranscriptCursor {
            entry: "new".into(),
            part: Some("deleted".into()),
        };
        let (entries, window) = doc.read_transcript_window(Some(&stale), false, 8).unwrap();
        assert_eq!(window.parts, 8);
        assert!(window.has_older);
        assert_eq!(entries[0].parts[0].id(), "p2040");
    }

    #[test]
    fn window_expands_by_parts_and_retains_its_boundary_as_new_messages_arrive() {
        let doc = SessionDoc::init("window").unwrap();
        doc.push_message(&entry("old", 400)).unwrap();
        let (tail, window) = doc.read_transcript_window(None, false, 128).unwrap();
        assert_eq!(tail[0].parts.len(), 128);
        assert_eq!(tail[0].parts[0].id(), "p272");
        assert!(window.has_older);
        let (page, window) = doc
            .read_transcript_window(window.start.as_ref(), true, 128)
            .unwrap();
        assert_eq!(page[0].parts.len(), 256);
        assert_eq!(page[0].parts[0].id(), "p144");
        doc.push_message(&entry("new", 1)).unwrap();
        let (live, next) = doc
            .read_transcript_window(window.start.as_ref(), false, 128)
            .unwrap();
        assert_eq!(live[0].parts[0].id(), "p144");
        assert_eq!(live.last().unwrap().id, "new");
        assert_eq!(next.parts, 257);
        let (all, window) = doc
            .read_transcript_window(next.start.as_ref(), true, 200)
            .unwrap();
        assert!(!window.has_older);
        assert_eq!(all[0].parts.len(), 400);
        let stale = TranscriptCursor {
            entry: "deleted".into(),
            part: Some("gone".into()),
        };
        let (fallback, window) = doc
            .read_transcript_window(Some(&stale), false, 128)
            .unwrap();
        assert_eq!(window.parts, 128);
        assert_eq!(fallback.len(), 2);
        assert!(window.has_older);
        assert_eq!(
            doc.read_entries().unwrap()[0].parts.len(),
            400,
            "view windows never truncate durable history"
        );
    }
    #[test]
    fn page_across_legacy_continuations_keeps_part_and_joined_message_identity() {
        let doc = SessionDoc::init("continuation").unwrap();
        doc.push_message(&entry("root", 2)).unwrap();
        let mut segment = entry("segment", 3);
        segment.continuation_of = Some("root".into());
        for (i, part) in segment.parts.iter_mut().enumerate() {
            if let MessagePart::Text { id, .. } = part {
                *id = format!("s{i}");
            }
        }
        doc.push_message(&segment).unwrap();
        let (tail, window) = doc.read_transcript_window(None, false, 2).unwrap();
        assert_eq!(tail[0].id, "root");
        assert_eq!(tail[0].parts[0].id(), "s1");
        let (page, window) = doc
            .read_transcript_window(window.start.as_ref(), true, 4)
            .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].id, "root");
        assert_eq!(page[0].parts.len(), 5);
        assert!(!window.has_older);
    }
}
