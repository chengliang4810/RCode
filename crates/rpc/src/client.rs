//! Client side: request/stream multiplexing over string frames + the WebSocket dialer.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use futures::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::{ClientFrame, RpcError, ServerFrame};

/// Per-stream queue depth. A lagging consumer loses its subscription and can
/// resubscribe; it cannot block the shared connection reader or grow forever.
const STREAM_QUEUE_CAP: usize = 256;

enum Pending {
    Call(oneshot::Sender<Result<serde_json::Value, RpcError>>),
    Stream(mpsc::Sender<serde_json::Value>),
    CheckedStream {
        items: mpsc::Sender<serde_json::Value>,
        ready: Option<oneshot::Sender<Result<(), RpcError>>>,
    },
}

struct Shared {
    pending: Mutex<HashMap<u64, Pending>>,
    connected: AtomicBool,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn disconnected(&self) {
        self.connected.store(false, Ordering::Release);
        let drained: Vec<_> = self.lock().drain().map(|(_, pending)| pending).collect();
        for pending in drained {
            match pending {
                Pending::Call(reply) => {
                    let _ = reply.send(Err(RpcError::Closed));
                }
                Pending::CheckedStream {
                    ready: Some(reply), ..
                } => {
                    let _ = reply.send(Err(RpcError::Closed));
                }
                Pending::Stream(_) | Pending::CheckedStream { ready: None, .. } => {}
            }
        }
    }
}

/// A multiplexing RPC client over any string-frame duplex ([`crate::memory_client`] or
/// [`connect_ws`]). Cheap to clone-by-Arc internally; use one per connection.
pub struct RpcClient {
    out: mpsc::Sender<String>,
    shared: Arc<Shared>,
    next_id: AtomicU64,
    reader: tokio::task::JoinHandle<()>,
    transport: Option<tokio::task::JoinHandle<()>>,
    reconnect: Option<mpsc::Sender<()>>,
}

/// Owned stream receiver whose drop immediately cancels the server task.
pub struct RpcSubscription {
    id: u64,
    items: mpsc::Receiver<serde_json::Value>,
    out: mpsc::Sender<String>,
    shared: Arc<Shared>,
}

impl RpcSubscription {
    pub async fn recv(&mut self) -> Option<serde_json::Value> {
        self.items.recv().await
    }
}

impl Drop for RpcSubscription {
    fn drop(&mut self) {
        if self.shared.lock().remove(&self.id).is_none() {
            return;
        }
        let Ok(frame) = serde_json::to_string(&ClientFrame {
            id: self.id,
            method: None,
            params: serde_json::Value::Null,
            cancel: true,
        }) else {
            return;
        };
        match self.out.try_send(frame) {
            Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => {}
            Err(mpsc::error::TrySendError::Full(frame)) => {
                let out = self.out.clone();
                if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                    runtime.spawn(async move {
                        let _ = out.send(frame).await;
                    });
                }
            }
        }
    }
}

impl RpcClient {
    /// Wrap an existing duplex: `out` carries client frames, `inbound` server frames.
    pub fn new(out: mpsc::Sender<String>, mut inbound: mpsc::Receiver<String>) -> Self {
        let shared = Arc::new(Shared {
            pending: Mutex::new(HashMap::new()),
            connected: AtomicBool::new(true),
        });
        let reader_shared = shared.clone();
        let reader_out = out.clone();
        let reader = tokio::spawn(async move {
            while let Some(payload) = inbound.recv().await {
                for line in payload.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let frame: ServerFrame = match serde_json::from_str(line) {
                        Ok(frame) => frame,
                        Err(err) => {
                            tracing::warn!(error = %err, "rpc: dropping malformed server frame");
                            continue;
                        }
                    };
                    route_frame(&reader_shared, &reader_out, frame).await;
                }
            }
            reader_shared.disconnected();
        });
        Self {
            out,
            shared,
            next_id: AtomicU64::new(1),
            reader,
            transport: None,
            reconnect: None,
        }
    }

    /// Whether the transport is currently connected. A reconnecting client
    /// keeps its identity while each connection's subscriptions end on loss.
    pub fn is_connected(&self) -> bool {
        self.shared.connected.load(Ordering::Acquire)
    }

    /// Permanently close this client, including when outstanding operations
    /// still retain an `Arc`. Reconfiguration/removal must stop reconnecting.
    pub fn close(&self) {
        if let Some(transport) = &self.transport {
            transport.abort();
        }
        self.reader.abort();
        self.shared.disconnected();
    }

    /// Replace the current socket immediately, for example after a mobile
    /// app resumes. Existing operations fail; no mutation is replayed.
    /// In-memory clients have no external transport to reconnect.
    pub fn reconnect_now(&self) {
        if let Some(reconnect) = &self.reconnect {
            self.shared.disconnected();
            let _ = reconnect.try_send(());
        }
    }

    /// Unary request.
    pub async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.shared.lock().insert(id, Pending::Call(tx));
        self.send(ClientFrame {
            id,
            method: Some(method.into()),
            params,
            cancel: false,
        })
        .await
        .inspect_err(|_| {
            self.shared.lock().remove(&id);
        })?;
        rx.await.map_err(|_| RpcError::Closed)?
    }

    /// Typed unary request.
    pub async fn call_as<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T, RpcError> {
        let value = self.call(method, params).await?;
        serde_json::from_value(value).map_err(|e| RpcError::BadParams(e.to_string()))
    }

    /// Legacy streaming request. Cancellation on receiver drop is only detected
    /// when another item arrives. Prefer `subscribe_scoped` for owned streams.
    pub async fn subscribe(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<mpsc::Receiver<serde_json::Value>, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(STREAM_QUEUE_CAP);
        self.shared.lock().insert(id, Pending::Stream(tx));
        self.send(ClientFrame {
            id,
            method: Some(method.into()),
            params,
            cancel: false,
        })
        .await
        .inspect_err(|_| {
            self.shared.lock().remove(&id);
        })?;
        Ok(rx)
    }

    /// A drop-cancelled stream without waiting for its first item. Unlike a
    /// checked subscription this also works for legitimately silent streams.
    pub async fn subscribe_scoped(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<RpcSubscription, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(STREAM_QUEUE_CAP);
        self.shared.lock().insert(id, Pending::Stream(tx));
        // Own cancellation before the send, including cancellation while the
        // outbound channel is backpressured.
        let subscription = RpcSubscription {
            id,
            items: rx,
            out: self.out.clone(),
            shared: self.shared.clone(),
        };
        self.send(ClientFrame {
            id,
            method: Some(method.into()),
            params,
            cancel: false,
        })
        .await?;
        Ok(subscription)
    }

    /// Streaming request with a server acknowledgement before returning.
    ///
    /// Use this for optional/versioned stream methods: an older server's
    /// `unknown method` is returned distinctly instead of looking like an empty stream.
    pub async fn subscribe_checked(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<RpcSubscription, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (items_tx, items_rx) = mpsc::channel(STREAM_QUEUE_CAP);
        let (ready_tx, ready_rx) = oneshot::channel();
        self.shared.lock().insert(
            id,
            Pending::CheckedStream {
                items: items_tx,
                ready: Some(ready_tx),
            },
        );
        let subscription = RpcSubscription {
            id,
            items: items_rx,
            out: self.out.clone(),
            shared: self.shared.clone(),
        };
        self.send(ClientFrame {
            id,
            method: Some(method.into()),
            params,
            cancel: false,
        })
        .await
        .inspect_err(|_| {
            self.shared.lock().remove(&id);
        })?;
        ready_rx.await.map_err(|_| RpcError::Closed)??;
        Ok(subscription)
    }

    async fn send(&self, frame: ClientFrame) -> Result<(), RpcError> {
        if !self.is_connected() {
            return Err(RpcError::Closed);
        }
        let json = serde_json::to_string(&frame)
            .map_err(|e| RpcError::Transport(format!("serialize frame: {e}")))?;
        self.out.send(json).await.map_err(|_| RpcError::Closed)
    }
}

impl Drop for RpcClient {
    fn drop(&mut self) {
        self.close();
    }
}

async fn route_frame(shared: &Arc<Shared>, out: &mpsc::Sender<String>, frame: ServerFrame) {
    let id = frame.id;
    if let Some(err) = frame.err {
        let error = wire_error(err);
        match shared.lock().remove(&id) {
            Some(Pending::Call(tx)) => {
                let _ = tx.send(Err(error));
            }
            Some(Pending::CheckedStream {
                ready: Some(ready), ..
            }) => {
                let _ = ready.send(Err(error));
            }
            Some(Pending::Stream(_)) | Some(Pending::CheckedStream { ready: None, .. }) | None => {
                // Stream errored: the sender drop closes the receiver.
                tracing::debug!(id, %error, "rpc: stream ended with error");
            }
        }
        return;
    }
    if let Some(value) = frame.ok {
        let mut pending = shared.lock();
        if matches!(pending.get(&id), Some(Pending::Call(_))) {
            if let Some(Pending::Call(tx)) = pending.remove(&id) {
                let _ = tx.send(Ok(value));
            }
        } else if let Some(Pending::CheckedStream { ready, .. }) = pending.get_mut(&id)
            && let Some(ready) = ready.take()
        {
            let _ = ready.send(Ok(()));
        }
        return;
    }
    if let Some(item) = frame.item {
        let ready = match shared.lock().get_mut(&id) {
            Some(Pending::CheckedStream { ready, .. }) => ready.take(),
            _ => None,
        };
        if let Some(ready) = ready {
            let _ = ready.send(Ok(()));
        }
        // Deliver without awaiting. This reader is the ONLY one for the whole
        // connection: every reply, every other subscription and the connection
        // events all come through it. An `await` here means one consumer that
        // stopped reading — a paused panel, a slow phone — stops *everything*,
        // because its full queue is the reader's problem. Measured: 257 items
        // into an unread stream and a later unary reply could not be delivered
        // at all until that stream was dropped.
        //
        // A full queue is instead treated as a lagging consumer: it is dropped
        // with an explicit gap marker, and the consumer reconnects with
        // `afterSeq` (the same recovery the transport already uses). Dropping
        // silently would show up as an unexplained hole in a transcript.
        let tx = match shared.lock().get(&id) {
            Some(Pending::Stream(tx)) => Some(tx.clone()),
            Some(Pending::CheckedStream { items, .. }) => Some(items.clone()),
            _ => None,
        };
        let dead = match tx {
            Some(tx) => match tx.try_send(item.clone()) {
                Ok(()) => false,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // The consumer is not reading. Dropping the subscription
                    // ends its `recv()` with `None`, which every consumer
                    // already treats as "stream over" — and its `Drop` cancels
                    // server-side. It can resubscribe with `afterSeq` and
                    // recover the gap, which is the same path a dropped
                    // connection takes. A queue-full marker could not be
                    // delivered anyway: the queue is full by definition.
                    tracing::warn!(
                        id,
                        "rpc: a subscription is not reading; dropping it so it cannot stall                          the connection"
                    );
                    true
                }
                Err(mpsc::error::TrySendError::Closed(_)) => true,
            },
            None => false,
        };
        if dead {
            // Receiver was dropped — cancel server-side and forget the stream.
            shared.lock().remove(&id);
            if let Ok(json) = serde_json::to_string(&ClientFrame {
                id,
                method: None,
                params: serde_json::Value::Null,
                cancel: true,
            }) {
                let _ = out.send(json).await;
            }
        }
        return;
    }
    if frame.done
        && let Some(Pending::CheckedStream {
            ready: Some(ready), ..
        }) = shared.lock().remove(&id)
    {
        let _ = ready.send(Ok(()));
    }
}

fn wire_error(error: String) -> RpcError {
    if let Some(method) = error.strip_prefix("unknown method: ") {
        RpcError::UnknownMethod(method.to_owned())
    } else if let Some(code) = error.strip_prefix("capability error: ") {
        RpcError::Capability(code.to_owned())
    } else {
        RpcError::Failed(error)
    }
}

/// How long a dial may take before we give up.
///
/// This is localhost: a real engine answers in milliseconds. Without a bound,
/// *any* other process holding the port accepts the TCP connection and then
/// never completes the WebSocket handshake, and the caller waits forever — a
/// stranger on port 27654 would hang the app at boot rather than degrade it.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Dial local IPC and reconnect after a later transport loss. Outstanding
/// requests fail and subscriptions end; callers resubscribe after reconnect.
pub async fn connect_ws(url: &str) -> Result<RpcClient, RpcError> {
    connect_reconnecting(url, None).await
}

/// Connect to the external listener with a header credential. Connection loss
/// never replays requests: writes with a lost reply have an unknown outcome.
pub async fn connect_ws_authenticated(url: &str, api_key: &str) -> Result<RpcClient, RpcError> {
    crate::validate_api_key(api_key)?;
    connect_reconnecting(url, Some(api_key.to_owned())).await
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn dial(url: &str, api_key: Option<&str>) -> Result<Socket, RpcError> {
    let mut request = url
        .into_client_request()
        .map_err(|_| RpcError::Transport("invalid WebSocket address".into()))?;
    if let Some(api_key) = api_key {
        let credential = format!("Bearer {api_key}")
            .parse()
            .map_err(|_| RpcError::Transport("invalid connection credential".into()))?;
        request.headers_mut().insert("Authorization", credential);
    }
    let result = tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(request))
        .await
        .map_err(|_| RpcError::Transport("WebSocket connection timed out".into()))?;
    match result {
        Ok((socket, _)) => Ok(socket),
        Err(tokio_tungstenite::tungstenite::Error::Http(response))
            if response.status().as_u16() == 401 || response.status().as_u16() == 403 =>
        {
            Err(RpcError::Transport(
                "WebSocket authentication rejected".into(),
            ))
        }
        Err(_) => Err(RpcError::Transport(
            "could not connect to WebSocket server".into(),
        )),
    }
}

async fn connect_reconnecting(url: &str, api_key: Option<String>) -> Result<RpcClient, RpcError> {
    let socket = dial(url, api_key.as_deref()).await?;
    let (out_tx, mut out_rx) = mpsc::channel::<String>(256);
    let (in_tx, in_rx) = mpsc::channel::<String>(256);
    let (reconnect_tx, mut reconnect_rx) = mpsc::channel(1);
    let mut client = RpcClient::new(out_tx, in_rx);
    client.reconnect = Some(reconnect_tx);
    let shared = client.shared.clone();
    let url = url.to_owned();
    client.transport = Some(tokio::spawn(async move {
        let mut next_socket = Some(socket);
        let mut retry = std::time::Duration::from_millis(250);
        loop {
            let socket = match next_socket.take() {
                Some(socket) => socket,
                None => {
                    // No queued frame may cross a connection boundary. Pending
                    // IDs were failed below; late frames are filtered too.
                    while out_rx.try_recv().is_ok() {}
                    tokio::select! {
                        _ = tokio::time::sleep(retry) => {},
                        request = reconnect_rx.recv() => { if request.is_none() { return; } },
                    }
                    match dial(&url, api_key.as_deref()).await {
                        Ok(socket) => socket,
                        Err(_) => {
                            retry = (retry * 2).min(std::time::Duration::from_secs(5));
                            continue;
                        }
                    }
                }
            };
            retry = std::time::Duration::from_millis(250);
            shared.connected.store(true, Ordering::Release);
            let (mut sink, mut stream) = socket.split();
            let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(15));
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut received = tokio::time::Instant::now();
            let mut immediate = false;
            loop {
                tokio::select! {
                    request = reconnect_rx.recv() => {
                        if request.is_none() { return; }
                        immediate = true;
                        break;
                    },
                    frame = out_rx.recv() => match frame {
                        Some(text) => {
                            // Requests may finish queueing after disconnect.
                            // Their removed IDs must never reach the new peer.
                            let live = serde_json::from_str::<ClientFrame>(&text).ok()
                                .is_some_and(|frame| frame.cancel || shared.lock().contains_key(&frame.id));
                            if !live { continue; }
                            if !matches!(tokio::time::timeout(CONNECT_TIMEOUT, sink.send(WsMessage::Text(text))).await, Ok(Ok(()))) {
                                break;
                            }
                        }
                        None => return,
                    },
                    message = stream.next() => match message {
                        Some(Ok(WsMessage::Text(text))) => {
                            received = tokio::time::Instant::now();
                            if !matches!(tokio::time::timeout(CONNECT_TIMEOUT, in_tx.send(text)).await, Ok(Ok(()))) { break; }
                        }
                        Some(Ok(WsMessage::Ping(payload))) => {
                            received = tokio::time::Instant::now();
                            if !matches!(tokio::time::timeout(CONNECT_TIMEOUT, sink.send(WsMessage::Pong(payload))).await, Ok(Ok(()))) { break; }
                        }
                        Some(Ok(WsMessage::Pong(_))) => received = tokio::time::Instant::now(),
                        Some(Ok(WsMessage::Close(_))) | Some(Err(_)) | None => break,
                        Some(Ok(_)) => {}
                    },
                    _ = heartbeat.tick() => {
                        if received.elapsed() >= std::time::Duration::from_secs(45) { break; }
                        if !matches!(tokio::time::timeout(CONNECT_TIMEOUT, sink.send(WsMessage::Ping(Vec::new()))).await, Ok(Ok(()))) { break; }
                    }
                }
            }
            shared.disconnected();
            while out_rx.try_recv().is_ok() {}
            if immediate {
                next_socket = dial(&url, api_key.as_deref()).await.ok();
            }
        }
    }));
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_peer_that_never_answers_heartbeats_is_disconnected() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (ready, socket) = oneshot::channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let _ = ready.send((socket, listener));
        });
        let client = connect_ws(&format!("ws://{address}")).await.unwrap();
        // Retain the socket but never poll it: this models a peer or path
        // which completed TCP/WS setup and then silently stopped responding.
        let _unresponsive_peer = socket.await.unwrap();
        tokio::task::yield_now().await;
        tokio::time::pause();
        tokio::time::advance(std::time::Duration::from_secs(46)).await;
        for _ in 0..20 {
            if !client.is_connected() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            !client.is_connected(),
            "silent sockets must not stay healthy forever"
        );
        assert!(matches!(
            client.call("Write", serde_json::json!({})).await,
            Err(RpcError::Closed)
        ));
    }
}
