use super::*;

fn fixture() -> (tempfile::TempDir, Arc<DocsStore>, DocHost) {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(DocsStore::open(directory.path()).unwrap());
    let host = DocHost::new(
        store.clone(),
        DocHostConfig {
            device_id: "deletion-host".into(),
            default_harness: HarnessId::Mock,
            edge: None,
        },
    );
    (directory, store, host)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn purging_chat_stops_retired_lineages_and_orphaned_persisters() {
    let (_directory, store, host) = fixture();
    let first = host.open("lineages").unwrap();
    first
        .write_user_message("first", "before adoption", 1)
        .unwrap();
    host.flush_all();
    let orphan_doc = first.doc_arc();
    let orphan_persistence = first.persistence.clone().unwrap();
    lock(&host.inner.handles).remove("lineages");
    drop(first);
    let second = host.open("lineages").unwrap();
    second
        .write_user_message("second", "current lineage", 2)
        .unwrap();
    host.flush_all();
    assert!(store.has_snapshot("lineages").unwrap());
    host.purge_chat("lineages").unwrap();
    orphan_doc
        .doc()
        .get_map("late")
        .insert("value", true)
        .unwrap();
    orphan_doc.doc().commit();
    orphan_persistence.dirty(true);
    orphan_persistence.flush_sync();
    second
        .write_user_message("late", "must not persist", 3)
        .unwrap();
    if let Some(persistence) = &second.persistence {
        persistence.flush_sync();
    }
    host.save_snapshot(&second);
    assert!(!store.has_snapshot("lineages").unwrap());
    assert!(host.open("lineages").is_err());
    host.shutdown_workers().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn purge_refuses_an_active_writer_even_on_a_retired_handle() {
    let (_directory, store, host) = fixture();
    let first = host.open("writer").unwrap();
    first
        .write_user_message("first", "keep this snapshot on failed deletion", 1)
        .unwrap();
    host.flush_all();
    let writer = first.writer().unwrap();
    lock(&host.inner.handles).remove("writer");
    let second = host.open("writer").unwrap();
    assert!(host.purge_chat("writer").is_err());
    assert!(store.has_snapshot("writer").unwrap());
    drop(writer);
    host.purge_chat("writer").unwrap();
    host.save_snapshot(&first);
    host.save_snapshot(&second);
    assert!(!store.has_snapshot("writer").unwrap());
    host.shutdown_workers().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_transport_update_cannot_recreate_a_purged_outbox() {
    use rcode_sync::chat_client::ChatDocSink as _;
    let (_directory, store, host) = fixture();
    let handle = host.open("late-transport").unwrap();
    let sink =
        crate::chat2_host::EngineChatSink::new(&handle.doc_arc(), store.clone(), "late-transport")
            .with_handle(Arc::downgrade(&handle));
    sink.persist_update("before", b"before deletion").unwrap();
    assert!(store.has_pending_chat_updates("late-transport").unwrap());
    host.purge_chat("late-transport").unwrap();
    assert!(
        sink.persist_update("late", b"late transport callback")
            .is_err()
    );
    assert!(!store.has_pending_chat_updates("late-transport").unwrap());
    drop(handle);
    assert!(
        sink.persist_update("orphan", b"owner already gone")
            .is_err()
    );
    assert!(!store.has_pending_chat_updates("late-transport").unwrap());
    assert!(!store.has_snapshot("late-transport").unwrap());
    host.shutdown_workers().await;
}
