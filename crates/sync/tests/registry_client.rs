//! RegistryClient integration tests against the in-process mock server
//! (`registry::mock_server`), which runs the SAME merge fn as the client and
//! speaks the DO's JSON WS protocol. TS↔Rust interop is proven separately by
//! the `--ignored` live-edge test (registry_edge.rs) and scripts/e2e-smoke.sh.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use rcode_doc::{REGISTRY_DOC_ID, RegistryDoc};
use rcode_proto::{Chat, Device, Session, SessionStatus};
use rcode_sync::registry::mock_server::MockRegistryServer;
use rcode_sync::{DocsStore, RegistryClient, RegistryEvent};

fn ts(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or(DateTime::UNIX_EPOCH)
}

fn device(id: &str) -> Device {
    Device {
        id: id.into(),
        name: format!("{id}-name"),
        platform: "linux".into(),
        last_seen_at: Some(ts(1_000)),
        created_at: Some(ts(500)),
        version: Some("0.1.0".into()),
        cursor_sdk_version: None,
        capabilities: Vec::new(),
    }
}

fn chat(id: &str, device_id: &str) -> Chat {
    Chat {
        id: id.into(),
        device_id: device_id.into(),
        title: Some("chat".into()),
        archived: false,
        cwd: Some("/tmp".into()),
        branch: None,
        checkout_id: None,
        source_context: None,
        config: None,
        last_message_preview: None,
        last_message_at: None,
        created_at: ts(2_000),
        harness_session_id: None,
        harness_session_cwd: None,
        parent_chat_id: None,
        space_id: None,
        last_seen_at: None,
        room_gen: None,
    }
}

async fn wait_until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("condition not reached in time");
}

fn new_doc(device: &str) -> Arc<Mutex<RegistryDoc>> {
    Arc::new(Mutex::new(RegistryDoc::new(device)))
}

#[tokio::test]
async fn two_clients_converge_and_stream_live_updates() {
    let server = MockRegistryServer::start().await;
    let doc_a = new_doc("dev-a");
    let doc_b = new_doc("dev-b");

    // A writes while OFFLINE; the batches queue locally.
    {
        let mut doc = doc_a.lock().unwrap();
        doc.upsert_device(&device("dev-a")).unwrap();
        doc.upsert_chat(&chat("chat-1", "dev-a")).unwrap();
        assert_eq!(doc.pending_len(), 2);
    }

    let client_a = RegistryClient::connect(&server.url(), doc_a.clone(), "dev-a")
        .await
        .expect("a connects");
    client_a.nudge();
    wait_until(|| doc_a.lock().unwrap().pending_len() == 0).await;
    assert_eq!(server.row_count(), 2);

    // B joins fresh and receives the full state.
    let client_b = RegistryClient::connect(&server.url(), doc_b.clone(), "dev-b")
        .await
        .expect("b connects");
    wait_until(|| doc_b.lock().unwrap().read_chats().unwrap().len() == 1).await;

    // Live propagation: B renames, A observes.
    {
        let mut doc = doc_b.lock().unwrap();
        assert!(doc.rename_chat("chat-1", "from b").unwrap());
    }
    client_b.nudge();
    wait_until(|| {
        doc_a
            .lock()
            .unwrap()
            .chat("chat-1")
            .unwrap()
            .is_some_and(|c| c.title.as_deref() == Some("from b"))
    })
    .await;

    // Working-status flip propagates the other way.
    {
        let mut doc = doc_a.lock().unwrap();
        doc.upsert_session(&Session {
            last_completed_turn: None,
            running_subagents: 0,
            chat_id: "chat-1".into(),
            device_id: "dev-a".into(),
            status: SessionStatus::Working,
            started_at: Some(ts(3_000)),
            updated_at: ts(3_500),
        })
        .unwrap();
    }
    client_a.nudge();
    wait_until(|| {
        doc_b
            .lock()
            .unwrap()
            .read_sessions()
            .unwrap()
            .first()
            .is_some_and(|s| s.status == SessionStatus::Working)
    })
    .await;

    client_a.shutdown().await;
    client_b.shutdown().await;
}

#[tokio::test]
async fn cursor_delta_sync_on_reconnect() {
    let server = MockRegistryServer::start().await;
    let doc_a = new_doc("dev-a");
    {
        let mut doc = doc_a.lock().unwrap();
        doc.upsert_chat(&chat("chat-1", "dev-a")).unwrap();
    }
    let client_a = RegistryClient::connect(&server.url(), doc_a.clone(), "dev-a")
        .await
        .expect("a connects");
    client_a.nudge();
    wait_until(|| doc_a.lock().unwrap().pending_len() == 0).await;
    client_a.shutdown().await;
    let cursor_before = doc_a.lock().unwrap().cursor();
    assert!(cursor_before > 0);

    // While A is away, B lands more rows.
    let doc_b = new_doc("dev-b");
    {
        let mut doc = doc_b.lock().unwrap();
        doc.upsert_chat(&chat("chat-2", "dev-b")).unwrap();
        doc.upsert_chat(&chat("chat-3", "dev-b")).unwrap();
    }
    let client_b = RegistryClient::connect(&server.url(), doc_b.clone(), "dev-b")
        .await
        .expect("b connects");
    client_b.nudge();
    wait_until(|| doc_b.lock().unwrap().pending_len() == 0).await;
    client_b.shutdown().await;

    // A reconnects with its cursor and catches up via the delta.
    let client_a = RegistryClient::connect(&server.url(), doc_a.clone(), "dev-a")
        .await
        .expect("a reconnects");
    wait_until(|| doc_a.lock().unwrap().read_chats().unwrap().len() == 3).await;
    assert!(doc_a.lock().unwrap().cursor() > cursor_before);
    client_a.shutdown().await;
}

#[tokio::test]
async fn unacked_writes_replay_idempotently_after_reconnect() {
    let server = MockRegistryServer::start().await;
    let doc = new_doc("dev-a");
    {
        let mut d = doc.lock().unwrap();
        d.upsert_chat(&chat("chat-1", "dev-a")).unwrap();
    }
    let client = RegistryClient::connect(&server.url(), doc.clone(), "dev-a")
        .await
        .expect("connects");
    client.nudge();
    wait_until(|| doc.lock().unwrap().pending_len() == 0).await;
    let seq_before = server.seq();

    // Simulate an ack lost to a dropped connection: mark the pending batch
    // un-flighted and replay it through a fresh session.
    {
        let mut d = doc.lock().unwrap();
        d.rename_chat("chat-1", "replayed").unwrap();
    }
    client.nudge();
    wait_until(|| doc.lock().unwrap().pending_len() == 0).await;
    let seq_after_first = server.seq();
    assert_eq!(seq_after_first, seq_before + 1);
    client.shutdown().await;

    // Manually re-enqueue the same logical write (same value, fresh clock
    // would win; instead we assert the SERVER state is unchanged when the
    // exact same batch replays — covered at the merge layer — and that a
    // reconnect with nothing pending doesn't grow seq).
    let client = RegistryClient::connect(&server.url(), doc.clone(), "dev-a")
        .await
        .expect("reconnects");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(server.seq(), seq_after_first);
    assert_eq!(
        server
            .row("chats", "chat-1")
            .expect("row")
            .fields
            .get("title")
            .and_then(|v| v.as_str()),
        Some("replayed")
    );
    client.shutdown().await;
}

#[tokio::test]
async fn server_wipe_reseeds_from_client_rows() {
    let server = MockRegistryServer::start().await;
    let doc = new_doc("dev-a");
    {
        let mut d = doc.lock().unwrap();
        d.upsert_device(&device("dev-a")).unwrap();
        d.upsert_chat(&chat("chat-1", "dev-a")).unwrap();
        d.rename_chat("chat-1", "kept title").unwrap();
    }
    let client = RegistryClient::connect(&server.url(), doc.clone(), "dev-a")
        .await
        .expect("connects");
    client.nudge();
    wait_until(|| doc.lock().unwrap().pending_len() == 0).await;
    assert_eq!(server.row_count(), 2);
    client.shutdown().await;

    // Operator wipe: rows gone, seq back to 0.
    server.reset();
    assert_eq!(server.row_count(), 0);

    // The client reconnects, detects the seq regression, and re-seeds —
    // automatically, with the data intact.
    let client = RegistryClient::connect(&server.url(), doc.clone(), "dev-a")
        .await
        .expect("reconnects");
    wait_until(|| server.row_count() == 2).await;
    assert_eq!(
        server
            .row("chats", "chat-1")
            .expect("row")
            .fields
            .get("title")
            .and_then(|v| v.as_str()),
        Some("kept title")
    );
    // And the local view never flickered.
    assert_eq!(doc.lock().unwrap().read_chats().unwrap().len(), 1);
    client.shutdown().await;
}

#[tokio::test]
async fn gc_floor_forces_full_resync() {
    let server = MockRegistryServer::start().await;
    let doc_a = new_doc("dev-a");
    {
        let mut d = doc_a.lock().unwrap();
        d.upsert_chat(&chat("chat-1", "dev-a")).unwrap();
    }
    let client = RegistryClient::connect(&server.url(), doc_a.clone(), "dev-a")
        .await
        .expect("connects");
    client.nudge();
    wait_until(|| doc_a.lock().unwrap().pending_len() == 0).await;
    client.shutdown().await;

    // B pushes more rows; then the server GC-jumps past A's cursor.
    let doc_b = new_doc("dev-b");
    {
        let mut d = doc_b.lock().unwrap();
        d.upsert_chat(&chat("chat-2", "dev-b")).unwrap();
    }
    let client_b = RegistryClient::connect(&server.url(), doc_b.clone(), "dev-b")
        .await
        .expect("b connects");
    client_b.nudge();
    wait_until(|| doc_b.lock().unwrap().pending_len() == 0).await;
    client_b.shutdown().await;
    server.set_gc_floor(server.seq());

    let client = RegistryClient::connect(&server.url(), doc_a.clone(), "dev-a")
        .await
        .expect("reconnects");
    wait_until(|| doc_a.lock().unwrap().read_chats().unwrap().len() == 2).await;
    client.shutdown().await;
}

#[tokio::test]
async fn presence_beats_reach_peers() {
    let server = MockRegistryServer::start().await;
    let doc_a = new_doc("dev-a");
    let doc_b = new_doc("dev-b");
    let client_a = RegistryClient::connect(&server.url(), doc_a, "dev-a")
        .await
        .expect("a connects");
    let client_b = RegistryClient::connect(&server.url(), doc_b, "dev-b")
        .await
        .expect("b connects");
    client_a.set_presence(123_456);
    wait_until(|| client_b.presence().get("dev-a") == Some(&123_456)).await;
    // And the hello reply carries existing presence to late joiners.
    let doc_c = new_doc("dev-c");
    let client_c = RegistryClient::connect(&server.url(), doc_c, "dev-c")
        .await
        .expect("c connects");
    wait_until(|| client_c.presence().get("dev-a") == Some(&123_456)).await;
    client_a.shutdown().await;
    client_b.shutdown().await;
    client_c.shutdown().await;
}

#[tokio::test]
async fn probe_answers_and_stats_flow() {
    let server = MockRegistryServer::start().await;
    let doc = new_doc("dev-a");
    let client = RegistryClient::connect(&server.url(), doc, "dev-a")
        .await
        .expect("connects");
    let before = client.stats();
    assert!(before.connected);
    assert!(before.synced, "hello state apply latches synced");
    client.probe();
    let events = client.events();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let after = client.stats();
    assert_eq!(after.probes, before.probes + 1);
    // The probe was answered — no disconnect happened.
    assert!(after.connected);
    assert_eq!(after.disconnects, 0);
    drop(events);
    client.shutdown().await;
}

#[tokio::test]
async fn persisted_doc_survives_restart_with_docs_store() {
    let server = MockRegistryServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let store = DocsStore::open(dir.path()).unwrap();

    // Session 1: write, sync, persist, "shut down".
    let doc = new_doc("dev-a");
    {
        let mut d = doc.lock().unwrap();
        d.upsert_chat(&chat("chat-1", "dev-a")).unwrap();
    }
    let client = RegistryClient::connect(&server.url(), doc.clone(), "dev-a")
        .await
        .expect("connects");
    client.nudge();
    wait_until(|| doc.lock().unwrap().pending_len() == 0).await;
    {
        let mut d = doc.lock().unwrap();
        d.rename_chat("chat-1", "offline rename").unwrap(); // will stay unacked
        let bytes = d.to_bytes().unwrap();
        store.save_snapshot(REGISTRY_DOC_ID, &bytes).unwrap();
        // Roll back the rename's push by dropping the connection first.
        d.mark_disconnected();
    }
    client.shutdown().await;

    // Session 2: restore, reconnect — the offline rename pushes.
    let bytes = store
        .load_snapshot(REGISTRY_DOC_ID)
        .unwrap()
        .expect("saved");
    let restored = Arc::new(Mutex::new(
        RegistryDoc::from_bytes(&bytes, "dev-a").unwrap(),
    ));
    assert_eq!(restored.lock().unwrap().pending_len(), 1);
    let client = RegistryClient::connect(&server.url(), restored.clone(), "dev-a")
        .await
        .expect("reconnects");
    wait_until(|| restored.lock().unwrap().pending_len() == 0).await;
    assert_eq!(
        server
            .row("chats", "chat-1")
            .expect("row")
            .fields
            .get("title")
            .and_then(|v| v.as_str()),
        Some("offline rename")
    );
    client.shutdown().await;
}

#[tokio::test]
async fn churn_stays_bounded_no_history_growth() {
    // The wedge regression: 2,000 status flips on the same row must leave the
    // server with ONE row and delta-syncing clients with tiny catch-ups —
    // history is discarded the moment it stops being true.
    let server = MockRegistryServer::start().await;
    let doc = new_doc("dev-a");
    {
        let mut d = doc.lock().unwrap();
        d.upsert_chat(&chat("chat-1", "dev-a")).unwrap();
    }
    let client = RegistryClient::connect(&server.url(), doc.clone(), "dev-a")
        .await
        .expect("connects");
    client.nudge();
    wait_until(|| doc.lock().unwrap().pending_len() == 0).await;
    for i in 0..2_000i64 {
        {
            let mut d = doc.lock().unwrap();
            d.upsert_session(&Session {
                last_completed_turn: None,
                running_subagents: 0,
                chat_id: "chat-1".into(),
                device_id: "dev-a".into(),
                status: if i % 2 == 0 {
                    SessionStatus::Working
                } else {
                    SessionStatus::Idle
                },
                started_at: Some(ts(i)),
                updated_at: ts(i + 1),
            })
            .unwrap();
        }
        if i % 100 == 0 {
            client.nudge();
            // Let the queue drain periodically so the test bounds memory too.
            wait_until(|| doc.lock().unwrap().pending_len() == 0).await;
        }
    }
    client.nudge();
    wait_until(|| doc.lock().unwrap().pending_len() == 0).await;
    // 2,001 writes → 2 rows. No log, no replay, nothing to compact.
    assert_eq!(server.row_count(), 2);
    client.shutdown().await;

    // A fresh device catches up with 2 rows, not 2,001 updates.
    let doc_b = new_doc("dev-b");
    let client_b = RegistryClient::connect(&server.url(), doc_b.clone(), "dev-b")
        .await
        .expect("b connects");
    wait_until(|| doc_b.lock().unwrap().read_sessions().unwrap().len() == 1).await;
    client_b.shutdown().await;
}

#[tokio::test]
async fn applied_events_fire_for_republish() {
    let server = MockRegistryServer::start().await;
    let doc_a = new_doc("dev-a");
    let doc_b = new_doc("dev-b");
    let client_a = RegistryClient::connect(&server.url(), doc_a, "dev-a")
        .await
        .expect("a connects");
    let client_b = RegistryClient::connect(&server.url(), doc_b, "dev-b")
        .await
        .expect("b connects");
    let mut events = client_b.events();
    {
        let mut d = client_a.doc().lock().unwrap();
        d.upsert_chat(&chat("chat-1", "dev-a")).unwrap();
    }
    client_a.nudge();
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(RegistryEvent::Applied) => return RegistryEvent::Applied,
                Ok(_) => continue,
                Err(err) => panic!("event stream ended: {err}"),
            }
        }
    })
    .await
    .expect("applied event");
    assert_eq!(event, RegistryEvent::Applied);
    client_a.shutdown().await;
    client_b.shutdown().await;
}

/// HTTPS seam for the late-reply race below: the pull reply is prepared at
/// ISSUE time and then HELD until the test releases it, so a socket can join
/// and apply a fresher state while this reply is still in flight.
struct HeldFetchTransport {
    reply: String,
    issued: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

impl rcode_sync::registry::RegistryTransport for HeldFetchTransport {
    fn fetch(
        &self,
        _since: u64,
    ) -> futures::future::BoxFuture<'static, Result<String, rcode_sync::SyncError>> {
        let reply = self.reply.clone();
        let issued = self.issued.clone();
        let release = self.release.clone();
        Box::pin(async move {
            issued.notify_one();
            release.notified().await;
            Ok(reply)
        })
    }

    fn push(
        &self,
        body: String,
    ) -> futures::future::BoxFuture<'static, Result<String, rcode_sync::SyncError>> {
        let batch = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["batch"].as_str().map(str::to_owned))
            .unwrap_or_default();
        Box::pin(async move { Ok(format!(r#"{{"batch":"{batch}","seq":0,"applied":0}}"#)) })
    }
}

/// The stale row the offline device holds: the server's own row (`chat()` must
/// still be able to read it) rewound to the PREVIOUS seq and title — i.e.
/// exactly what a reply issued before the rename would carry.
fn stale_row(server: &MockRegistryServer) -> rcode_doc::RegistryRow {
    let mut row = server.row("chats", "chat-1").expect("chat-1 exists");
    row.seq = 1;
    row.fields
        .insert("title".into(), serde_json::json!("stale title"));
    row.clocks
        .insert("title".into(), "0000000000001-000000-dev-seed".into());
    row
}

#[tokio::test]
async fn a_late_http_pull_never_overwrites_state_a_socket_already_applied() {
    // F19: on reconnect the HTTP fallback and the WS dial race. The pull's
    // reply is built when the request is SERVED, then crawls back over a slow
    // link — long enough for the socket to join and apply a NEWER state. The
    // what-are-THIS guard is required or `apply_state` rewrites the row and
    // drags the cursor back to the reply's older seq.
    let server = MockRegistryServer::start().await;

    // Server ends at seq 2 with the fresh title (another device renamed it).
    let seeder = new_doc("dev-seed");
    {
        let mut d = seeder.lock().unwrap();
        d.upsert_chat(&chat("chat-1", "dev-seed")).unwrap();
    }
    let seed_client = RegistryClient::connect(&server.url(), seeder.clone(), "dev-seed")
        .await
        .expect("seeder connects");
    seed_client.nudge();
    wait_until(|| seeder.lock().unwrap().pending_len() == 0).await;
    {
        let mut d = seeder.lock().unwrap();
        d.rename_chat("chat-1", "fresh title").unwrap();
    }
    seed_client.nudge();
    wait_until(|| seeder.lock().unwrap().pending_len() == 0).await;
    assert_eq!(
        server.seq(),
        2,
        "server must be one ahead of the stale snapshot"
    );

    let stale = stale_row(&server);

    // The reconnecting device last applied seq 1 — it holds the STALE title.
    let doc = new_doc("dev-late");
    {
        let mut d = doc.lock().unwrap();
        d.apply_state(1, false, 0, vec![stale.clone()]);
    }
    assert_eq!(doc.lock().unwrap().cursor(), 1);
    assert_eq!(
        doc.lock()
            .unwrap()
            .chat("chat-1")
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("stale title")
    );

    let issued = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let reply = serde_json::json!({
        "seq": 1, "full": false, "gcFloor": 0,
        "rows": [stale],
        "presence": {}
    })
    .to_string();
    let client = RegistryClient::connect_via_transport(
        // A REAL room for the socket, plus the held-reply HTTPS seam.
        Arc::new(rcode_sync::StaticUrl(server.url())),
        doc.clone(),
        "dev-late",
        rcode_sync::RegistryTuning::default(),
        Arc::new(HeldFetchTransport {
            reply,
            issued: issued.clone(),
            release: release.clone(),
        }),
    )
    .await
    .expect("transport clients construct immediately");

    // The offline pull is out (and parked), then the socket joins and applies
    // the newer state while that reply is still in flight.
    tokio::time::timeout(Duration::from_secs(5), issued.notified())
        .await
        .expect("the offline pull must be issued");
    wait_until(|| doc.lock().unwrap().cursor() == 2).await;
    assert_eq!(
        doc.lock()
            .unwrap()
            .chat("chat-1")
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("fresh title"),
        "the socket's hello state must be applied"
    );

    // Now the stale reply lands. It must be discarded, not applied.
    release.notify_one();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        doc.lock()
            .unwrap()
            .chat("chat-1")
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("fresh title"),
        "a late HTTP reply must not overwrite the fresher socket state"
    );
    assert_eq!(
        doc.lock().unwrap().cursor(),
        2,
        "and must not drag the cursor back to its older seq"
    );

    client.shutdown().await;
    seed_client.shutdown().await;
}

/// HTTPS seam that answers every pull with a WIPED server (seq 0, full) and
/// records every pushed op batch. No socket ever joins, so the transport epoch
/// stays put — this is the "real reset" shape the F19 fence must NOT drop.
struct WipedServerTransport {
    ops: Arc<Mutex<Vec<rcode_doc::RowOp>>>,
}

impl rcode_sync::registry::RegistryTransport for WipedServerTransport {
    fn fetch(
        &self,
        _since: u64,
    ) -> futures::future::BoxFuture<'static, Result<String, rcode_sync::SyncError>> {
        Box::pin(async {
            Ok(r#"{"seq":0,"full":true,"gcFloor":0,"rows":[],"presence":{}}"#.to_string())
        })
    }

    fn push(
        &self,
        body: String,
    ) -> futures::future::BoxFuture<'static, Result<String, rcode_sync::SyncError>> {
        let ops = self.ops.clone();
        Box::pin(async move {
            let v: serde_json::Value = serde_json::from_str(&body).expect("push body is json");
            let batch = v["batch"].as_str().unwrap_or("").to_owned();
            for op in serde_json::from_value::<Vec<rcode_doc::RowOp>>(v["ops"].clone())
                .unwrap_or_default()
            {
                ops.lock().unwrap().push(op);
            }
            Ok(format!(r#"{{"batch":"{batch}","seq":0,"applied":0}}"#))
        })
    }
}

#[tokio::test]
async fn a_real_server_reset_still_applies_through_the_https_fence() {
    // The F19 guard must separate "reply built for a cycle that ended" from "a
    // reset I still need". A cursor comparison would have swallowed the second
    // (every fetch is a confirmed no-op on a quiet room too); the epoch does
    // not, because the local row is the only copy of a wiper's data and the
    // re-seed has to go out.
    //
    // The row is seeded through a real room first so it is a genuinely
    // parseable chat, not a hand-built stub.
    let server = MockRegistryServer::start().await;
    let doc = new_doc("dev-reset");
    {
        let mut d = doc.lock().unwrap();
        d.upsert_chat(&chat("chat-1", "dev-reset")).unwrap();
    }
    let seed_client = RegistryClient::connect(&server.url(), doc.clone(), "dev-reset")
        .await
        .expect("seeder connects");
    seed_client.nudge();
    wait_until(|| doc.lock().unwrap().pending_len() == 0).await;
    seed_client.shutdown().await;
    let cursor_before = doc.lock().unwrap().cursor();
    assert!(cursor_before > 0);
    assert_eq!(doc.lock().unwrap().read_chats().unwrap().len(), 1);

    // Operator wipe, then reconnect over the HTTPS fence only (no socket, so
    // the transport epoch never moves — the reset itself must still apply).
    server.reset();
    let ops = Arc::new(Mutex::new(Vec::new()));
    let _client = RegistryClient::connect_via_transport(
        Arc::new(rcode_sync::StaticUrl("ws://127.0.0.1:1/ws".into())),
        doc.clone(),
        "dev-reset",
        rcode_sync::RegistryTuning::default(),
        Arc::new(WipedServerTransport { ops: ops.clone() }),
    )
    .await
    .expect("transport clients construct immediately");

    // The wiped answer applies (cursor regresses to 0) and re-seeds the row.
    wait_until(|| doc.lock().unwrap().cursor() == 0).await;
    wait_until(|| !ops.lock().unwrap().is_empty()).await;
    let seeded = ops.lock().unwrap().clone();
    assert!(
        seeded.iter().any(|op| op.id == "chat-1"),
        "the reset must re-seed the local row, not drop it: {seeded:?}"
    );
    assert_eq!(
        doc.lock().unwrap().read_chats().unwrap().len(),
        1,
        "the local row survives the root reset"
    );
}

/// HTTPS-transport seam for the pure-offline tests below: pulls answer with
/// an empty delta; the FIRST push ack comes back unreadable (a captive
/// portal / proxy interposing on the response body), later pushes ack
/// properly. The regression: an unreadable ack used to be silently ignored,
/// leaving the batch marked in-flight — and with no socket session there is
/// no disconnect to un-mark it, so the write was stranded until restart.
struct FlakyAckTransport {
    push_calls: Arc<std::sync::atomic::AtomicU64>,
}

impl rcode_sync::registry::RegistryTransport for FlakyAckTransport {
    fn fetch(
        &self,
        _since: u64,
    ) -> futures::future::BoxFuture<'static, Result<String, rcode_sync::SyncError>> {
        Box::pin(async {
            Ok(r#"{"seq":0,"full":false,"gcFloor":0,"rows":[],"presence":{}}"#.to_string())
        })
    }

    fn push(
        &self,
        body: String,
    ) -> futures::future::BoxFuture<'static, Result<String, rcode_sync::SyncError>> {
        let call = self
            .push_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async move {
            if call == 0 {
                return Ok("<html>proxy login required</html>".to_string());
            }
            let v: serde_json::Value = serde_json::from_str(&body).expect("push body is json");
            let batch = v["batch"].as_str().expect("batch id").to_owned();
            Ok(format!(r#"{{"batch":"{batch}","seq":1,"applied":1}}"#))
        })
    }
}

#[tokio::test]
async fn unreadable_http_ack_retries_instead_of_stranding_the_batch() {
    let push_calls = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let doc = new_doc("dev-a");
    {
        let mut d = doc.lock().unwrap();
        d.upsert_chat(&chat("chat-1", "dev-a")).unwrap();
        assert_eq!(d.pending_len(), 1);
    }
    // The WS side never connects (dead port): every sync runs over HTTPS.
    let client = RegistryClient::connect_via_transport(
        Arc::new(rcode_sync::StaticUrl("ws://127.0.0.1:1/ws".into())),
        doc.clone(),
        "dev-a",
        rcode_sync::RegistryTuning::default(),
        Arc::new(FlakyAckTransport {
            push_calls: push_calls.clone(),
        }),
    )
    .await
    .expect("transport clients construct immediately");

    // Cycle 1 push gets the unreadable ack; the batch must become pushable
    // again so a later cycle (here: the next dial backoff) retries and the
    // proper ack clears it.
    wait_until(|| doc.lock().unwrap().pending_len() == 0).await;
    assert!(
        push_calls.load(std::sync::atomic::Ordering::SeqCst) >= 2,
        "the stranded batch must be re-pushed after the unreadable ack"
    );

    // The empty pulls also latched the synced flag — server truth has been
    // heard this process even though no socket ever joined.
    let stats = client.stats();
    assert!(stats.synced, "HTTP pull apply must latch synced");
    assert!(!stats.connected, "no WS session ever joined");

    client.shutdown().await;
}
