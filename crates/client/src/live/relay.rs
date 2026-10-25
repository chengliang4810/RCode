//! Host RPCs over the device-room relay (`rcode_rpc::LinkCache`: one cached,
//! self-evicting link per target device, dark-peer fast-fail, dial cooldown),
//! plus the multi-call transfers built on it and the PR-status watches.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use rcode_proto::CheckoutChangeRequestStatus;
use rcode_rpc::{LinkCache, LinkCacheConfig, PeerLiveness, RpcError, methods};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{Bearer, b64};
use crate::client::ClientInner;
use crate::error::{ClientError, Result};
use crate::rpc::ProgressFn;
use crate::{lock, now_ms};

/// Default unary deadline (engine forward DEFAULT); long ones below.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Raw bytes per UploadChunk slice (about 680 KB base64), leaving envelope
/// headroom within a 1 MiB frame budget. Multiples of three encode independently.
const UPLOAD_SLICE_BYTES: usize = 510_000;
const UPLOAD_PARALLEL: usize = 3;
/// A device with presence older than this reads Dark (no dial) once the
/// registry has been live long enough to have heard its beats.
const DARK_AFTER_MS: i64 = 5 * 60_000;
const LIVENESS_WARMUP_MS: i64 = 60_000;

pub(crate) fn deadline(method: &str) -> Duration {
    match method {
        methods::CREATE_WORKTREE => Duration::from_secs(120),
        methods::LIST_MODELS => Duration::from_secs(100),
        methods::UPLOAD_COMMIT => Duration::from_secs(150),
        _ => CALL_TIMEOUT,
    }
}

fn map_rpc(device_id: &str, method: &str, err: RpcError) -> ClientError {
    match err {
        RpcError::UnknownMethod(m) => ClientError::Unsupported(format!("{m} on {device_id}")),
        RpcError::BadParams(m) | RpcError::Failed(m) | RpcError::Capability(m) => {
            ClientError::HostError(format!("{method}: {m}"))
        }
        RpcError::Transport(m) => ClientError::HostUnavailable(format!("{device_id}: {m}")),
        RpcError::Closed => ClientError::HostUnavailable(format!("{device_id}: connection closed")),
    }
}

pub(crate) struct Relay {
    links: Arc<LinkCache>,
    watches: Mutex<HashMap<WatchKey, CancellationToken>>,
    /// Devices whose engine lacks WatchCheckoutChangeRequest (cleared on
    /// registry reconnect).
    unsupported: Mutex<HashSet<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct WatchKey {
    pub device_id: String,
    pub cwd: String,
    pub branch: String,
}

impl Relay {
    pub(crate) fn new(inner: &Arc<ClientInner>, edge: &str, bearer: Bearer) -> Self {
        let weak = Arc::downgrade(inner);
        let started = now_ms();
        let mut config = LinkCacheConfig::new(edge, Arc::new(bearer));
        config.liveness = Some(Arc::new(move |device_id: &str| {
            let Some(inner) = weak.upgrade() else {
                return PeerLiveness::Unknown;
            };
            let now = now_ms();
            match inner.workspace.presence().get(device_id) {
                Some(at) if now - at < crate::connectivity::PRESENCE_FRESH_MS => PeerLiveness::Live,
                seen => {
                    let registry_live = inner.live().is_some_and(|l| {
                        let posture = l.registry_posture();
                        posture.connected && posture.synced
                    });
                    let stale = seen.is_none_or(|at| now - at >= DARK_AFTER_MS);
                    if registry_live && stale && now - started >= LIVENESS_WARMUP_MS {
                        PeerLiveness::Dark
                    } else {
                        PeerLiveness::Unknown
                    }
                }
            }
        }));
        let links = {
            let _runtime = crate::runtime::handle().enter();
            LinkCache::new(config)
        };
        Self {
            links,
            watches: Mutex::new(HashMap::new()),
            unsupported: Mutex::new(HashSet::new()),
        }
    }

    pub(crate) fn shutdown(&self) {
        self.links.disconnect_all();
        for (_, token) in lock(&self.watches).drain() {
            token.cancel();
        }
    }

    /// Presence says `device_id` is alive again: dial immediately next time.
    pub(crate) fn peer_alive(&self, device_id: &str) {
        self.links.reset_cooldown(device_id);
    }

    /// Reads and explicitly idempotent writes may retry a stale cached link.
    /// A lost mutation acknowledgement never authorizes a second mutation.
    pub(crate) async fn call(&self, device_id: &str, method: &str, params: Value) -> Result<Value> {
        call_with_retry(
            device_id,
            method,
            params,
            || self.links.client(device_id),
            || self.links.invalidate(device_id),
        )
        .await
    }

    /// Chunked upload (`UploadChunk` ×N with `seq`, then `UploadCommit`).
    /// Returns the durable host path. Idempotent per `upload_id`.
    pub(crate) async fn upload(
        &self,
        device_id: &str,
        upload_id: &str,
        file_name: &str,
        data: &[u8],
        progress: Option<ProgressFn>,
    ) -> Result<String> {
        let slices: Vec<&[u8]> = if data.is_empty() {
            vec![&[][..]]
        } else {
            data.chunks(UPLOAD_SLICE_BYTES).collect()
        };
        let total = slices.len();
        let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let report = |done: usize| {
            if let Some(progress) = &progress {
                progress((done as f64 / total as f64).min(0.99));
            }
        };
        report(0);
        let mut next = 0usize;
        while next < total {
            let batch: Vec<(usize, &[u8])> = slices
                .iter()
                .enumerate()
                .skip(next)
                .take(UPLOAD_PARALLEL)
                .map(|(i, s)| (i, *s))
                .collect();
            next += batch.len();
            let calls = batch.into_iter().map(|(seq, slice)| {
                let done = done.clone();
                let report = &report;
                async move {
                    let params =
                        json!({ "uploadId": upload_id, "seq": seq, "data": b64::encode(slice) });
                    let mut last = None;
                    for attempt in 0..3u64 {
                        if attempt > 0 {
                            tokio::time::sleep(Duration::from_millis(
                                50 * attempt * (seq as u64 + 1),
                            ))
                            .await;
                        }
                        match self
                            .call(device_id, methods::UPLOAD_CHUNK, params.clone())
                            .await
                        {
                            Ok(_) => {
                                let now =
                                    done.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
                                report(now);
                                return Ok(());
                            }
                            Err(
                                err @ (ClientError::Unsupported(_) | ClientError::HostError(_)),
                            ) => return Err(err),
                            Err(err) => last = Some(err),
                        }
                    }
                    Err(last.unwrap_or(ClientError::HostUnavailable(device_id.to_owned())))
                }
            });
            for result in futures::future::join_all(calls).await {
                result?;
            }
        }
        let reply = self
            .call(
                device_id,
                methods::UPLOAD_COMMIT,
                json!({ "uploadId": upload_id, "fileName": file_name }),
            )
            .await?;
        if let Some(progress) = &progress {
            progress(1.0);
        }
        reply
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| ClientError::HostError("UploadCommit returned no path".into()))
    }

    /// `ReadAttachmentChunk` until done.
    pub(crate) async fn read_attachment(&self, device_id: &str, path: &str) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut offset: u64 = 0;
        for _ in 0..1000 {
            let reply = self
                .call(
                    device_id,
                    methods::READ_ATTACHMENT_CHUNK,
                    json!({ "path": path, "offset": offset }),
                )
                .await?;
            let data = reply
                .get("data")
                .and_then(Value::as_str)
                .unwrap_or_default();
            out.extend(
                b64::decode(data)
                    .ok_or_else(|| ClientError::HostError("bad attachment chunk".into()))?,
            );
            if reply.get("done").and_then(Value::as_bool).unwrap_or(true) {
                return Ok(out);
            }
            let next = reply
                .get("nextOffset")
                .and_then(Value::as_u64)
                .unwrap_or(offset);
            if next <= offset {
                return Err(ClientError::HostError(
                    "attachment read made no progress".into(),
                ));
            }
            offset = next;
        }
        Err(ClientError::HostError("attachment too large".into()))
    }

    pub(crate) fn clear_unsupported(&self) {
        lock(&self.unsupported).clear();
    }

    /// Converge the running PR-status streams onto `targets`.
    pub(crate) fn reconcile_watches(&self, inner: &Arc<ClientInner>, targets: HashSet<WatchKey>) {
        let unsupported = lock(&self.unsupported).clone();
        let mut watches = lock(&self.watches);
        watches.retain(|key, token| {
            let keep = targets.contains(key) && !unsupported.contains(&key.device_id);
            if !keep {
                token.cancel();
            }
            keep
        });
        for key in targets {
            if watches.contains_key(&key) || unsupported.contains(&key.device_id) {
                continue;
            }
            let token = CancellationToken::new();
            watches.insert(key.clone(), token.clone());
            spawn_watch(Arc::downgrade(inner), key, token);
        }
    }

    pub(crate) fn stop_watches(&self) {
        for (_, token) in lock(&self.watches).drain() {
            token.cancel();
        }
    }
}

/// Explicit allowlist: an unfamiliar method is a mutation until its contract
/// proves otherwise. In particular, same-value setters, queue leases, terminal
/// input and worktree creation may have observable side effects on each call.
fn replay_safe(method: &str, params: &Value) -> bool {
    match method {
        methods::LIST_HARNESSES
        | methods::LIST_MODELS
        | methods::LIST_SKILLS
        | methods::LIST_CUSTOM_COMMANDS
        | methods::LIST_COMMANDS
        | methods::LIST_SUBAGENTS
        | methods::LIST_MCP_SERVERS
        | methods::LIST_PLUGINS
        | methods::LIST_PROJECT_MEMORY
        | methods::LIST_REPOS
        | methods::LIST_BRANCHES
        | methods::LIST_REFS
        | methods::LIST_GIT_HISTORY
        | methods::SEARCH_GIT_HISTORY
        | methods::RESOLVE_GIT_AVATARS
        | methods::LIST_FOLDERS
        | methods::LIST_DRIVES
        | methods::SEARCH_HOME_FOLDERS
        | methods::SEARCH_FILES
        | methods::LIST_WORKSPACE_DIRECTORY
        | methods::SEARCH_WORKSPACE_FILES
        | methods::SEARCH_WORKSPACE_CONTENT
        | methods::READ_WORKSPACE_IMAGE
        | methods::READ_WORKSPACE_FILE
        | methods::READ_ATTACHMENT_CHUNK
        | methods::READ_COMMAND_OUTPUT
        | methods::GET_TITLE_SETTINGS
        | methods::GET_PROVIDER_SETTINGS
        | methods::GET_PROVIDER_API_KEY
        | methods::GET_AGENT_SETTINGS
        | methods::GET_CUSTOM_INSTRUCTIONS
        | methods::GET_WORKTREE_SETTINGS
        | methods::GET_CHANGE_REQUEST_REPOSITORY
        | methods::LIST_CHANGE_REQUEST_PAGE => true,
        // Positional chunks overwrite the same slot; append-only legacy chunks
        // do not. UploadCommit is deliberately absent: older peers remove its
        // staging before replying and do not support idempotent completion.
        methods::UPLOAD_CHUNK => params.get("seq").and_then(Value::as_u64).is_some(),
        // RelayCommand carries a client-minted id, claimed in the host ledger.
        methods::RELAY_COMMAND => params
            .get("entry")
            .and_then(|entry| entry.get("id"))
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty()),
        _ => false,
    }
}

async fn call_with_retry<Dial, DialFuture, Invalidate>(
    device_id: &str,
    method: &str,
    params: Value,
    mut dial: Dial,
    invalidate: Invalidate,
) -> Result<Value>
where
    Dial: FnMut() -> DialFuture,
    DialFuture:
        std::future::Future<Output = std::result::Result<Arc<rcode_rpc::RpcClient>, RpcError>>,
    Invalidate: Fn(),
{
    let safe = replay_safe(method, &params);
    for attempt in 0..2 {
        let client = dial()
            .await
            .map_err(|err| map_rpc(device_id, method, err))?;
        match tokio::time::timeout(deadline(method), client.call(method, params.clone())).await {
            Ok(Ok(value)) => return Ok(value),
            Ok(Err(err @ (RpcError::Closed | RpcError::Transport(_)))) => {
                invalidate();
                if !safe {
                    return Err(uncertain_outcome(device_id, method, "connection lost"));
                }
                if attempt == 1 {
                    return Err(map_rpc(device_id, method, err));
                }
            }
            Ok(Err(err)) => return Err(map_rpc(device_id, method, err)),
            Err(_) => {
                invalidate();
                return Err(if safe {
                    ClientError::HostUnavailable(format!("{method} on {device_id} timed out"))
                } else {
                    uncertain_outcome(device_id, method, "request timed out")
                });
            }
        }
    }
    unreachable!("the second attempt always returns")
}

fn uncertain_outcome(device_id: &str, method: &str, reason: &str) -> ClientError {
    ClientError::HostUnavailable(format!(
        "{method} on {device_id}: {reason}; the outcome may be unknown and the request was not retried. Check the host state before retrying."
    ))
}

#[cfg(test)]
mod retry_tests {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio_tungstenite::tungstenite::Message;

    async fn next_request(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    ) -> rcode_rpc::ClientFrame {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(payload) => socket.send(Message::Pong(payload)).await.unwrap(),
                Message::Pong(_) => {}
                message => panic!("expected an RPC text frame, received {message:?}"),
            }
        }
    }

    async fn finish_close(socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) {
        socket.close(None).await.unwrap();
        // A successful reply must not be discarded by an abortive TCP drop
        // with unread peer control data. Finish transport cleanup, not an
        // application acknowledgement; the lost-ACK branch stays abortive.
        loop {
            match socket.next().await {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {},
                Some(Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed
                    | tokio_tungstenite::tungstenite::Error::AlreadyClosed
                    | tokio_tungstenite::tungstenite::Error::Protocol(
                        tokio_tungstenite::tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                    ))) => break,
                Some(Err(error)) => panic!("unexpected error draining peer close: {error}"),
            }
        }
    }

    async fn drop_first_reply(method: &'static str, params: Value, retry: bool) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let received = Arc::new(AtomicUsize::new(0));
        let observed = received.clone();
        let server = tokio::spawn(async move {
            for attempt in 0..if retry { 2 } else { 1 } {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let request = next_request(&mut socket).await;
                assert_eq!(request.method.as_deref(), Some(method));
                observed.fetch_add(1, Ordering::SeqCst);
                if attempt == 1 {
                    socket
                        .send(Message::Text(
                            json!({"id":request.id,"ok":42}).to_string().into(),
                        ))
                        .await
                        .unwrap();
                    finish_close(&mut socket).await;
                }
                // Model a completed host operation whose acknowledgement is
                // lost with the transport. No synthetic RpcError is injected.
                drop(socket);
            }
        });
        let dials = AtomicUsize::new(0);
        let clients = Mutex::new(Vec::<Arc<rcode_rpc::RpcClient>>::new());
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            call_with_retry(
                "host",
                method,
                params,
                || async {
                    dials.fetch_add(1, Ordering::SeqCst);
                    let client = Arc::new(rcode_rpc::connect_ws(&url).await?);
                    clients.lock().unwrap().push(client.clone());
                    Ok(client)
                },
                || {
                    for client in clients.lock().unwrap().drain(..) {
                        client.close();
                    }
                },
            ),
        )
        .await
        .expect("RPC retry policy must finish");
        for client in clients.lock().unwrap().drain(..) {
            client.close();
        }
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("host must observe the expected number of RPC requests")
            .unwrap();
        let expected = if retry { 2 } else { 1 };
        assert_eq!(dials.load(Ordering::SeqCst), expected);
        assert_eq!(received.load(Ordering::SeqCst), expected);
        if retry {
            assert_eq!(result.unwrap(), json!(42));
        } else {
            let message = result.unwrap_err().to_string();
            assert!(message.contains("outcome may be unknown"), "{message}");
            assert!(message.contains("not retried"), "{message}");
        }
    }

    #[tokio::test]
    async fn reply_before_graceful_close_is_delivered_before_pending_requests_fail() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let request = next_request(&mut socket).await;
            // Flush the valid reply and the normal Close in wire order. TCP
            // reset/lost bytes cannot explain discarding this received reply.
            socket
                .feed(Message::Text(
                    json!({"id":request.id,"ok":42}).to_string().into(),
                ))
                .await
                .unwrap();
            finish_close(&mut socket).await;
        });
        let client = rcode_rpc::connect_ws(&url).await.unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            client.call(methods::LIST_REFS, json!({})),
        )
        .await
        .unwrap();
        client.close();
        server.await.unwrap();
        assert_eq!(
            result.unwrap(),
            json!(42),
            "a received reply precedes connection loss"
        );
    }

    #[tokio::test]
    async fn acknowledged_side_effect_is_not_replayed_after_socket_loss() {
        for method in [
            methods::CREATE_WORKTREE,
            methods::BEGIN_QUEUED_MESSAGE_EDIT,
            methods::FINISH_QUEUED_MESSAGE_EDIT,
            methods::SEND_QUEUED_MESSAGE_NOW,
            methods::SWITCH_REF,
            methods::UPLOAD_COMMIT,
        ] {
            drop_first_reply(method, json!({}), false).await;
        }
    }

    #[tokio::test]
    async fn reads_and_positional_upload_chunks_reconnect_once() {
        drop_first_reply(methods::LIST_REFS, json!({}), true).await;
        drop_first_reply(methods::UPLOAD_CHUNK, json!({"seq":0}), true).await;
        drop_first_reply(methods::UPLOAD_CHUNK, json!({}), false).await;
    }

    #[test]
    fn retry_contract_is_explicit_and_defaults_to_no_replay() {
        for method in [
            methods::LIST_HARNESSES,
            methods::LIST_MODELS,
            methods::SEARCH_FILES,
            methods::LIST_FOLDERS,
            methods::READ_ATTACHMENT_CHUNK,
        ] {
            assert!(replay_safe(method, &json!({})), "{method}");
        }
        assert!(replay_safe(
            methods::RELAY_COMMAND,
            &json!({"entry":{"id":"stable-command"}})
        ));
        assert!(!replay_safe(methods::RELAY_COMMAND, &json!({"entry":{}})));
        assert!(!replay_safe("FutureMutation", &json!({})));
        assert!(!replay_safe(methods::WRITE_TERMINAL, &json!({})));
    }
}

/// One `WatchCheckoutChangeRequest` stream, retried 0.5s→5s forever (last
/// snapshot kept through gaps); an engine without the method marks the
/// device unsupported.
fn spawn_watch(weak: Weak<ClientInner>, key: WatchKey, cancel: CancellationToken) {
    crate::runtime::shared().spawn(async move {
        let mut backoff = Duration::from_millis(500);
        loop {
            if cancel.is_cancelled() {
                return;
            }
            let Some(inner) = weak.upgrade() else { return };
            let Some(live) = inner.live() else { return };
            let links = live.relay.links.clone();
            drop(inner);
            let result = async {
                let client = links.client(&key.device_id).await?;
                client
                    .subscribe_checked(
                        methods::WATCH_CHECKOUT_CHANGE_REQUEST,
                        json!({ "cwd": key.cwd, "branch": key.branch }),
                    )
                    .await
            };
            let subscription = tokio::select! {
                _ = cancel.cancelled() => return,
                result = result => result,
            };
            match subscription {
                Ok(mut stream) => loop {
                    let item = tokio::select! {
                        _ = cancel.cancelled() => return,
                        item = stream.recv() => item,
                    };
                    let Some(item) = item else { break };
                    let Ok(status) = serde_json::from_value::<CheckoutChangeRequestStatus>(item)
                    else {
                        continue;
                    };
                    if status.device_id != key.device_id || status.cwd != key.cwd {
                        continue;
                    }
                    backoff = Duration::from_millis(500);
                    let Some(inner) = weak.upgrade() else { return };
                    inner.workspace.put_change_request(status);
                    inner.recompute_workspace();
                },
                Err(RpcError::UnknownMethod(_)) => {
                    if let Some(inner) = weak.upgrade()
                        && let Some(live) = inner.live()
                    {
                        lock(&live.relay.unsupported).insert(key.device_id.clone());
                    }
                    return;
                }
                Err(RpcError::Closed | RpcError::Transport(_)) => links.invalidate(&key.device_id),
                Err(_) => {}
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(Duration::from_secs(5));
        }
    });
}
