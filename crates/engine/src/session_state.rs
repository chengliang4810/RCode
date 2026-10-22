//! JSON session control state for cross-platform viewers without a Loro runtime.
//!
//! Transcript tokens use WatchDocMessages; this projection only wakes for the
//! command ledger or metadata, so a long streamed answer never re-sends prompts.

use crate::ChatDocHandle;
use futures::{StreamExt, stream::BoxStream};
use loro::Index;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::watch;

pub(crate) fn stream(handle: Arc<ChatDocHandle>) -> BoxStream<'static, Value> {
    let (changed, receiver) = watch::channel(0u64);
    // Attach before the initial read: a concurrent command must never be lost.
    let subscription = handle.doc().doc().subscribe_root(Arc::new(move |event| {
        if event.events.iter().any(|change| {
            matches!(change.path.first(), Some((_, Index::Key(root))) if matches!(root.as_str(), "commands" | "meta"))
        }) {
            changed.send_modify(|revision| *revision = revision.wrapping_add(1));
        }
    }));
    futures::stream::unfold(
        (handle, receiver, subscription, false),
        |(handle, mut changed, subscription, emitted)| async move {
            if emitted {
                changed.changed().await.ok()?;
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            }
            changed.borrow_and_update();
            let doc = handle.doc();
            let value = json!({
                "version": 1,
                "commands": doc.read_commands().ok()?,
                "contextUsage": doc.context_usage(),
                "goal": doc.goal(),
            });
            Some((value, (handle, changed, subscription, true)))
        },
    )
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EngineCore, HarnessId, default_registry};
    use rcode_doc::{SessionCommandEntry, SessionCommandPayload, SessionCommandStatus};
    use rcode_rpc::{memory_client, methods};
    use std::time::Duration;

    #[tokio::test]
    async fn json_state_reports_authoritative_outcomes_and_metadata_without_transcript_tokens() {
        let root = tempfile::tempdir().unwrap();
        let core = EngineCore::assemble(
            root.path(),
            Arc::new(default_registry()),
            HarnessId::Mock,
            None,
        )
        .unwrap();
        let rpc = memory_client(core.rpc_service());
        rpc.call(methods::MUTATE, json!({"op":"createChat","chatId":"json-session","deviceId":core.device_id,"cwd":root.path()})).await.unwrap();
        let mut subscription = rpc
            .subscribe_scoped(
                methods::WATCH_SESSION_STATE,
                json!({"chatId":"json-session"}),
            )
            .await
            .unwrap();
        let initial = subscription.recv().await.unwrap();
        assert_eq!(initial["version"], 1);
        assert_eq!(initial["commands"], json!([]));
        let command = SessionCommandEntry {
            id: "expired-json-command".into(),
            payload: SessionCommandPayload::Interrupt {},
            issued_by: "flutter-viewer".into(),
            issued_at: 1,
            based_on: None,
            expires_at: Some(1),
            status: SessionCommandStatus::Pending,
            resolution: None,
        };
        rpc.call(
            methods::RELAY_COMMAND,
            json!({"chatId":"json-session","entry":command}),
        )
        .await
        .unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(2), subscription.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(outcome["commands"][0]["id"], "expired-json-command");
        assert_eq!(outcome["commands"][0]["status"], "expired");
        let handle = core.doc_host.open("json-session").unwrap();
        // Changing only the transcript root does not wake the control stream.
        handle
            .doc()
            .doc()
            .get_list("messages")
            .push("token-only")
            .unwrap();
        handle.doc().doc().commit();
        assert!(
            tokio::time::timeout(Duration::from_millis(80), subscription.recv())
                .await
                .is_err()
        );
        handle
            .doc()
            .update_context_usage(Some(123), Some(4096))
            .unwrap();
        let metadata = tokio::time::timeout(Duration::from_secs(2), subscription.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            metadata["contextUsage"],
            json!({"tokens":123,"window":4096})
        );
        drop(subscription);
        core.shutdown().await;
    }
}
