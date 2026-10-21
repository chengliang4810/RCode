//! A paged live transcript. Older clients keep the complete-mirror subscription.
use crate::doc_host::DocHost;
use futures::{StreamExt, stream::BoxStream};
use rcode_doc::{
    TranscriptBaseline, TranscriptCursor, TranscriptFrame, TranscriptUpdate, diff_transcript,
};
use rcode_rpc::RpcError;
use std::sync::{Arc, Mutex};

pub(crate) async fn watch(
    host: DocHost,
    chat: String,
    start: Option<TranscriptCursor>,
    older: bool,
) -> Result<BoxStream<'static, serde_json::Value>, RpcError> {
    let handle = tokio::task::spawn_blocking(move || host.open(&chat))
        .await
        .map_err(|e| RpcError::Failed(e.to_string()))?
        .map_err(|e| RpcError::Failed(e.to_string()))?;
    let lease = handle.retain_window_view();
    let doc = handle.doc_arc();
    let history = Arc::new(Mutex::new(
        crate::transcript_history::TranscriptHistory::default(),
    ));
    let (changed, rx) = tokio::sync::watch::channel(0u64);
    let history_observer = history.clone();
    let weak = Arc::downgrade(&doc);
    // Subscribe before reading: a commit during the initial read remains pending.
    let sub = doc.doc().subscribe_root(Arc::new(move |event| {
        if let Some(doc) = weak.upgrade() {
            history_observer.lock().unwrap().observe(doc.doc(), &event);
        }
        changed.send_modify(|revision| *revision = revision.wrapping_add(1));
    }));
    let stream = futures::stream::unfold(
        (
            rx,
            doc,
            start,
            older,
            None::<Vec<rcode_doc::SessionMessageEntry>>,
            history,
            TranscriptBaseline::default(),
            None,
            lease,
            sub,
        ),
        |(
            mut rx,
            doc,
            mut cursor,
            mut older,
            mut previous,
            history,
            mut baseline,
            mut metadata,
            lease,
            sub,
        )| async move {
            loop {
                if previous.is_some() {
                    rx.changed().await.ok()?;
                }
                rx.borrow_and_update();
                let read_doc = doc.clone();
                let read_cursor = cursor.clone();
                let read_history = history.clone();
                let (entries, window, replay) = tokio::task::spawn_blocking(move || {
                    let (entries, window) =
                        read_doc.read_transcript_window(read_cursor.as_ref(), older, 128)?;
                    let replay = read_history
                        .lock()
                        .unwrap()
                        .snapshot(read_doc.doc(), &entries);
                    Ok::<_, rcode_doc::DocError>((entries, window, replay))
                })
                .await
                .ok()?
                .ok()?;
                older = false;
                cursor = window.start.clone();
                let frame = previous
                    .as_ref()
                    .map(|previous| diff_transcript(previous, &entries))
                    .unwrap_or_else(|| TranscriptFrame::reset(&entries));
                let first = previous.is_none();
                if first {
                    baseline = TranscriptBaseline::capture(&entries);
                }
                let mut replay_baseline = (*replay).clone();
                for (id, parts) in &baseline.entries {
                    let merged = replay_baseline.entries.entry(id.clone()).or_default();
                    for (id, len) in parts {
                        merged
                            .entry(id.clone())
                            .and_modify(|old| *old = (*old).max(*len))
                            .or_insert(*len);
                    }
                }
                let state = (
                    doc.context_usage(),
                    doc.goal(),
                    window.clone(),
                    replay_baseline.clone(),
                );
                previous = Some(entries);
                if frame.is_empty_delta() && metadata.as_ref() == Some(&state) {
                    continue;
                }
                metadata = Some(state.clone());
                let mut value = serde_json::to_value(TranscriptUpdate {
                    frame,
                    context_usage: state.0,
                    goal: state.1,
                    replay_baseline: Some(replay_baseline),
                })
                .ok()?;
                value["historyWindow"] = serde_json::to_value(window).ok()?;
                return Some((
                    value,
                    (
                        rx, doc, cursor, older, previous, history, baseline, metadata, lease, sub,
                    ),
                ));
            }
        },
    );
    Ok(stream.boxed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc_host::DocHostConfig;
    use rcode_doc::{MessagePart, MessageRole, SegmentWriter, SessionMessageEntry};
    use std::time::{Duration, Instant};
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tail_stays_windowed_and_live_and_pages_without_a_full_reset() {
        let dir = tempfile::tempdir().unwrap();
        let host = DocHost::new(
            Arc::new(rcode_sync::DocsStore::open(dir.path()).unwrap()),
            DocHostConfig {
                device_id: "viewer".into(),
                default_harness: rcode_proto::HarnessId::Mock,
                edge: None,
            },
        );
        let handle = host.open("large").unwrap();
        let parts: Vec<_> = (0..2000)
            .map(|i| MessagePart::Text {
                id: format!("p{i}"),
                text: "paragraph ".repeat(200),
            })
            .collect();
        handle
            .doc()
            .push_message(&SessionMessageEntry {
                id: "work".into(),
                role: MessageRole::Assistant,
                parts: parts.clone(),
                created_at: 0,
                device_id: "host".into(),
                status: Some(rcode_doc::MessageStatus::Streaming),
                continuation_of: None,
                duration_ms: None,
                excluded: false,
            })
            .unwrap();
        let timer = Instant::now();
        let mut stream = watch(host.clone(), "large".into(), None, false)
            .await
            .unwrap();
        let first = stream.next().await.unwrap();
        let first_ms = timer.elapsed().as_millis();
        let first_bytes = serde_json::to_vec(&first).unwrap().len();
        assert_eq!(first["reset"][0]["parts"].as_array().unwrap().len(), 128);
        assert!(
            tokio::time::timeout(Duration::from_millis(80), stream.next())
                .await
                .is_err(),
            "opening must not silently hydrate full history"
        );
        let start: TranscriptCursor =
            serde_json::from_value(first["historyWindow"]["start"].clone()).unwrap();
        let mut paged = watch(host.clone(), "large".into(), Some(start), true)
            .await
            .unwrap();
        let second = paged.next().await.unwrap();
        assert_eq!(second["reset"][0]["parts"].as_array().unwrap().len(), 256);
        let mut updated = parts.clone();
        if let Some(MessagePart::Text { text, .. }) = updated.last_mut() {
            text.push_str(" live tail");
        }
        SegmentWriter::resume(handle.doc(), 0, parts)
            .sync(&updated)
            .unwrap();
        let delta = stream.next().await.unwrap();
        assert_eq!(delta["append"][0]["text"], " live tail");
        assert!(delta.get("reset").is_none());
        let delta2 = paged.next().await.unwrap();
        assert_eq!(delta2["append"][0]["text"], " live tail");
        let full_bytes = serde_json::to_vec(&handle.doc().read_entries().unwrap())
            .unwrap()
            .len();
        assert!(
            first_bytes * 10 < full_bytes,
            "first-frame payload must depend on its window, not total history"
        );
        println!(
            "window-first: {first_ms}ms, {first_bytes} bytes; full-history: {full_bytes} bytes; first parts 128 / 2000"
        );
        drop((stream, paged));
        host.shutdown_workers().await;
    }
}
