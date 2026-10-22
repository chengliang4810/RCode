//! Read-only Loro replication for a direct mobile client.

use crate::ChatDocHandle;
use base64::Engine as _;
use futures::{StreamExt, stream::BoxStream};
use std::sync::Arc;
use tokio::sync::watch;

pub(crate) fn stream(handle: Arc<ChatDocHandle>) -> BoxStream<'static, serde_json::Value> {
    let (changed, rx) = watch::channel(0u64);
    // Subscribe before exporting: overlapping updates are safe in Loro; gaps are not.
    let subscription = handle.doc().doc().subscribe_root(Arc::new(move |_| {
        changed.send_modify(|revision| *revision = revision.wrapping_add(1));
    }));
    futures::stream::unfold(
        (handle, rx, subscription, None),
        |(handle, mut rx, subscription, previous)| async move {
            if previous.is_some() {
                rx.changed().await.ok()?;
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            }
            rx.borrow_and_update();
            let frontier = handle.doc().doc().oplog_vv();
            let (kind, bytes) = match &previous {
                None => ("snapshot", handle.doc().export_snapshot().ok()?),
                Some(previous) => (
                    "update",
                    handle
                        .doc()
                        .doc()
                        .export(loro::ExportMode::updates(previous))
                        .ok()?,
                ),
            };
            let value =
                serde_json::json!({(kind):base64::engine::general_purpose::STANDARD.encode(bytes)});
            Some((value, (handle, rx, subscription, Some(frontier))))
        },
    )
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EngineCore, HarnessId, default_registry};
    use loro::ToJson;
    use rcode_doc::SessionDoc;

    #[tokio::test]
    async fn replica_includes_metadata_changes_and_a_new_subscriber_gets_full_state() {
        let root = tempfile::tempdir().unwrap();
        let core = EngineCore::assemble(
            root.path(),
            Arc::new(default_registry()),
            HarnessId::Mock,
            None,
        )
        .unwrap();
        let handle = core.doc_host.open("replica-test").unwrap();
        let mut stream = stream(handle.clone());
        let first = stream.next().await.unwrap();
        let replica = SessionDoc::from_doc(loro::LoroDoc::new());
        replica
            .doc()
            .import(
                &base64::engine::general_purpose::STANDARD
                    .decode(first["snapshot"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap();
        handle
            .doc()
            .doc()
            .get_map("meta")
            .insert("directReplicaTest", "landed")
            .unwrap();
        handle.doc().doc().commit();
        let update = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap();
        replica
            .doc()
            .import(
                &base64::engine::general_purpose::STANDARD
                    .decode(update["update"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            replica
                .doc()
                .get_map("meta")
                .get("directReplicaTest")
                .unwrap()
                .get_deep_value()
                .to_json_value(),
            serde_json::json!("landed")
        );
        drop(stream);
        core.shutdown().await;
    }
}
