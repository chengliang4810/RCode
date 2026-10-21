//! Recover identity metadata left by native warm turns that reused a user id.
use loro::{Container, ContainerTrait, LoroMap, LoroValue, ValueOrContainer};
use rcode_doc::{DocError, SessionDoc};
use std::collections::{HashMap, HashSet};

fn string(map: &LoroMap, key: &str) -> Option<String> {
    match map.get(key)? {
        ValueOrContainer::Value(LoroValue::String(value)) => Some(value.to_string()),
        _ => None,
    }
}

fn maps(doc: &SessionDoc) -> Vec<LoroMap> {
    let messages = doc.doc().get_list("messages");
    (0..messages.len())
        .filter_map(|index| match messages.get(index) {
            Some(ValueOrContainer::Container(Container::Map(map))) => Some(map),
            _ => None,
        })
        .collect()
}

pub(crate) fn exists(doc: &SessionDoc, id: &str) -> bool {
    maps(doc)
        .iter()
        .any(|map| string(map, "id").as_deref() == Some(id))
}

pub(crate) struct Repair {
    identities: Vec<(LoroMap, String)>,
    continuations: Vec<(LoroMap, String)>,
}

impl Repair {
    pub(crate) fn scan(doc: &SessionDoc) -> Self {
        let maps = maps(doc);
        let mut groups = HashMap::<String, Vec<usize>>::new();
        for (index, map) in maps.iter().enumerate() {
            if let Some(id) = string(map, "id") {
                groups.entry(id).or_default().push(index);
            }
        }
        let mut reserved: HashSet<String> = groups.keys().cloned().collect();
        let mut identities = Vec::new();
        let mut roots = HashMap::new();
        // Document order makes repair deterministic even if two peers recover it.
        for (index, map) in maps.iter().enumerate() {
            let Some(id) = string(map, "id") else {
                continue;
            };
            let indices = &groups[&id];
            if indices.len() < 2 {
                continue;
            }
            // Queue delivery and optimistic echoes depend on the original user id.
            let keeper = indices
                .iter()
                .copied()
                .find(|&i| string(&maps[i], "role").as_deref() == Some("user"))
                .unwrap_or(indices[0]);
            if keeper == index {
                continue;
            }
            let base = format!("{id}~recovered-{}", map.id());
            let mut unique = base.clone();
            let mut suffix = 0;
            while !reserved.insert(unique.clone()) {
                suffix += 1;
                unique = format!("{base}-{suffix}");
            }
            if string(map, "role").as_deref() == Some("assistant")
                && string(map, "continuationOf").is_none()
                && (string(&maps[keeper], "role").as_deref() != Some("assistant")
                    || string(&maps[keeper], "continuationOf").is_some())
            {
                roots.entry(id).or_insert_with(|| unique.clone());
            }
            identities.push((map.clone(), unique));
        }
        let continuations = maps
            .iter()
            .filter_map(|map| {
                if string(map, "role").as_deref() != Some("assistant") {
                    return None;
                }
                let root = string(map, "continuationOf")?;
                Some((map.clone(), roots.get(&root)?.clone()))
            })
            .collect();
        Self {
            identities,
            continuations,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.identities.is_empty()
    }

    pub(crate) fn apply(self, doc: &SessionDoc) -> Result<usize, DocError> {
        let count = self.identities.len();
        for (map, id) in self.identities {
            map.insert("id", id)?;
        }
        for (map, root) in self.continuations {
            map.insert("continuationOf", root)?;
        }
        if count > 0 {
            doc.doc().commit();
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcode_doc::{MessagePart, MessageRole, MessageStatus, SessionMessageEntry};

    fn entry(id: &str, role: MessageRole, part: &str) -> SessionMessageEntry {
        SessionMessageEntry {
            id: id.into(),
            role,
            parts: vec![MessagePart::Text {
                id: part.into(),
                text: part.into(),
            }],
            created_at: 10,
            device_id: "host".into(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
            duration_ms: Some(200),
            excluded: false,
        }
    }

    #[test]
    fn duplicate_identity_repair_preserves_user_ids_bodies_and_continuations() {
        let doc = SessionDoc::init("recovery").unwrap();
        // User can follow the assistant in merged CRDT order: it must still keep its id.
        doc.push_message(&entry("shared", MessageRole::Assistant, "answer"))
            .unwrap();
        doc.push_message(&entry("shared", MessageRole::User, "prompt"))
            .unwrap();
        let mut continuation = entry("segment", MessageRole::Assistant, "continued");
        continuation.continuation_of = Some("shared".into());
        doc.push_message(&continuation).unwrap();
        let original = doc.read_entries().unwrap();
        let snapshot = doc.export_snapshot().unwrap();
        assert_eq!(Repair::scan(&doc).apply(&doc).unwrap(), 1);
        let repaired = doc.read_entries().unwrap();
        assert_eq!(repaired[1].id, "shared");
        assert_ne!(repaired[0].id, "shared");
        assert_eq!(
            repaired[2].continuation_of.as_deref(),
            Some(repaired[0].id.as_str())
        );
        for (before, after) in original.iter().zip(&repaired) {
            assert_eq!(before.parts, after.parts);
            assert_eq!(before.duration_ms, after.duration_ms);
            assert_eq!(before.status, after.status);
        }
        assert!(Repair::scan(&doc).is_empty());
        let replica = SessionDoc::init("replica").unwrap();
        replica.doc().import(&snapshot).unwrap();
        Repair::scan(&replica).apply(&replica).unwrap();
        assert_eq!(
            replica.read_entries().unwrap(),
            repaired,
            "container identities make recovery deterministic across replicas"
        );
        let (window, _) = doc.read_transcript_window(None, false, 128).unwrap();
        assert_eq!(window.len(), 2);
        assert_eq!(window[0].parts.len(), 2);
        let mut live = repaired.clone();
        if let MessagePart::Text { text, .. } = &mut live[0].parts[0] {
            text.push_str(" live");
        }
        let mut view = repaired.clone();
        rcode_doc::apply_transcript_frame(&mut view, rcode_doc::diff_transcript(&repaired, &live))
            .unwrap();
        assert_eq!(view, live);
    }

    #[tokio::test]
    async fn opening_a_legacy_doc_keeps_a_backup_and_persists_the_repaired_ids() {
        use crate::doc_host::{DocHost, DocHostConfig};
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(rcode_sync::DocsStore::open(dir.path()).unwrap());
        let legacy = SessionDoc::init("legacy").unwrap();
        legacy
            .push_message(&entry("user-id", MessageRole::User, "prompt"))
            .unwrap();
        legacy
            .push_message(&entry("user-id", MessageRole::Assistant, "answer"))
            .unwrap();
        let original = legacy.export_snapshot().unwrap();
        store.save_snapshot("legacy", &original).unwrap();
        let config = || DocHostConfig {
            device_id: "host".into(),
            default_harness: rcode_proto::HarnessId::Mock,
            edge: None,
        };
        let host = DocHost::new(store.clone(), config());
        let handle = host.open("legacy").unwrap();
        let repaired = handle.doc().read_entries().unwrap();
        assert_eq!(repaired[0].id, "user-id");
        assert_ne!(repaired[1].id, "user-id");
        let backup = store
            .load_snapshot("legacy.pre-message-id-repair")
            .unwrap()
            .unwrap();
        let backup_doc = SessionDoc::from_doc(loro::LoroDoc::new());
        backup_doc.doc().import(&backup).unwrap();
        assert_eq!(
            backup_doc.read_entries().unwrap(),
            legacy.read_entries().unwrap()
        );
        host.flush_all();
        host.shutdown_workers().await;
        let host2 = DocHost::new(store.clone(), config());
        assert_eq!(
            host2.open("legacy").unwrap().doc().read_entries().unwrap(),
            repaired
        );
        assert_eq!(
            store
                .load_snapshot("legacy.pre-message-id-repair")
                .unwrap()
                .unwrap(),
            backup
        );
        host2.shutdown_workers().await;
    }
}
