//! Server side: dispatch loop over string frames + the WebSocket acceptor.

use std::collections::HashMap;
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::handshake::server::{
    ErrorResponse, Request as HandshakeRequest, Response as HandshakeResponse,
};
use tokio_tungstenite::tungstenite::http::StatusCode;

use crate::{ClientFrame, RpcError, RpcReply, RpcService, ServerFrame};

#[derive(Default)]
struct RequestTasks(HashMap<u64, tokio::task::AbortHandle>);

impl Drop for RequestTasks {
    fn drop(&mut self) {
        for task in self.0.values() {
            task.abort();
        }
    }
}

/// Serve one connection: read client frames from `inbound`, write server frames to `out`.
/// Returns when `inbound` closes; all in-flight request tasks are aborted on exit.
pub async fn serve_connection(
    service: Arc<dyn RpcService>,
    out: mpsc::Sender<String>,
    mut inbound: mpsc::Receiver<String>,
) {
    let mut running = RequestTasks::default();
    while let Some(payload) = inbound.recv().await {
        // ndjson: a transport may batch several frames per message.
        for line in payload.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let frame: ClientFrame = match serde_json::from_str(line) {
                Ok(frame) => frame,
                Err(err) => {
                    tracing::warn!(error = %err, "rpc: dropping malformed client frame");
                    continue;
                }
            };
            running.0.retain(|_, task| !task.is_finished());
            if frame.cancel {
                if let Some(task) = running.0.remove(&frame.id) {
                    task.abort();
                }
                continue;
            }
            let Some(method) = frame.method else {
                tracing::warn!(id = frame.id, "rpc: frame has neither method nor cancel");
                continue;
            };
            let task = tokio::spawn(handle_request(
                service.clone(),
                out.clone(),
                frame.id,
                method,
                frame.params,
            ));
            if let Some(previous) = running.0.insert(frame.id, task.abort_handle()) {
                previous.abort();
            }
        }
    }
}

async fn handle_request(
    service: Arc<dyn RpcService>,
    out: mpsc::Sender<String>,
    id: u64,
    method: String,
    params: serde_json::Value,
) {
    let send = |frame: ServerFrame| {
        let out = out.clone();
        async move {
            match serde_json::to_string(&frame) {
                Ok(json) => out.send(json).await.map_err(|_| RpcError::Closed),
                Err(err) => {
                    tracing::error!(error = %err, "rpc: failed to serialize server frame");
                    Err(RpcError::Closed)
                }
            }
        }
    };
    match service.handle(&method, params).await {
        Ok(RpcReply::Value(value)) => {
            let _ = send(ServerFrame {
                id,
                ok: Some(value),
                ..Default::default()
            })
            .await;
        }
        Ok(RpcReply::Stream(mut stream)) => {
            // Only the versioned checkout-PR stream uses an explicit readiness
            // frame. Sending it for legacy streams would make older clients remove
            // their pending stream as if it were a unary response.
            if method == crate::methods::WATCH_CHECKOUT_CHANGE_REQUEST
                && send(ServerFrame {
                    id,
                    ok: Some(serde_json::json!({ "stream": true })),
                    ..Default::default()
                })
                .await
                .is_err()
            {
                return;
            }
            while let Some(item) = stream.next().await {
                if send(ServerFrame {
                    id,
                    item: Some(item),
                    ..Default::default()
                })
                .await
                .is_err()
                {
                    return; // connection gone
                }
            }
            let _ = send(ServerFrame {
                id,
                done: true,
                ..Default::default()
            })
            .await;
        }
        Err(err) => {
            let _ = send(ServerFrame {
                id,
                err: Some(err.to_string()),
                ..Default::default()
            })
            .await;
        }
    }
}

/// Accept WebSocket connections forever, serving each with `service`.
pub async fn serve_ws_listener(listener: TcpListener, service: Arc<dyn RpcService>) {
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                tracing::debug!(%peer, "rpc: connection accepted");
                tokio::spawn(serve_ws_socket(stream, service.clone()));
            }
            Err(err) => {
                tracing::warn!(error = %err, "rpc: accept failed");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

/// Serve external connections authenticated by a Bearer header. Disabling or
/// changing the key revokes all established sockets as well as future dials.
/// The service and engine itself remain alive when a client is disconnected.
pub async fn serve_authenticated_listener(
    listener: TcpListener,
    service: Arc<dyn RpcService>,
    mut api_key: watch::Receiver<Option<String>>,
) {
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    tokio::spawn(serve_authenticated_socket(stream, service.clone(), api_key.clone()));
                }
                Err(error) => {
                    tracing::warn!(%error, "rpc: external accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            },
            changed = api_key.changed() => {
                if changed.is_err() { return; }
            }
        }
    }
}

async fn revoked(mut api_key: watch::Receiver<Option<String>>, accepted: Option<String>) {
    // Every policy revision revokes this handshake, even if rapid toggles
    // coalesce back to the original value before this task is scheduled.
    if *api_key.borrow() != accepted || api_key.has_changed().unwrap_or(true) {
        return;
    }
    let _ = api_key.changed().await;
}

async fn serve_authenticated_socket(
    stream: TcpStream,
    service: Arc<dyn RpcService>,
    mut api_key: watch::Receiver<Option<String>>,
) {
    let accepted_key = api_key.borrow_and_update().clone();
    let expected = accepted_key.clone();
    #[allow(clippy::result_large_err)]
    let authenticate = move |req: &HandshakeRequest, response: HandshakeResponse| {
        let matches = expected.as_deref().is_some_and(|expected| {
            let Some(credential) = req
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
            else {
                return false;
            };
            crate::access::key_matches(credential.as_bytes(), expected.as_bytes())
        });
        if matches {
            return Ok(response);
        }
        let mut response = ErrorResponse::new(Some("authentication required".into()));
        *response.status_mut() = StatusCode::UNAUTHORIZED;
        Err(response)
    };
    let socket = tokio::select! {
        handshake = tokio::time::timeout(std::time::Duration::from_secs(5), tokio_tungstenite::accept_hdr_async(stream, authenticate)) => {
            match handshake {
                Ok(Ok(socket)) => socket,
                // Handshake errors may carry request details: never log them.
                _ => return,
            }
        },
        _ = revoked(api_key.clone(), accepted_key.clone()) => return,
    };
    tokio::select! {
        _ = serve_socket(socket, service) => {},
        _ = revoked(api_key, accepted_key) => {},
    }
}

async fn serve_ws_socket(stream: TcpStream, service: Arc<dyn RpcService>) {
    // Native viewports dial this socket with a bare `connect_async` and send
    // no `Origin` header. A browser always attaches `Origin` to a WebSocket
    // handshake and cannot forge or suppress it from script, and WebSockets
    // are exempt from the Same-Origin Policy — so only rejecting any handshake
    // that carries `Origin` keeps a page the user happens to visit from
    // reaching this local socket. Keep this check.
    //
    // The large `Err` (ErrorResponse) is the shape tungstenite's Callback
    // trait requires; it can't be boxed away here.
    #[allow(clippy::result_large_err)]
    let reject_cross_origin = |req: &HandshakeRequest, resp: HandshakeResponse| {
        if let Some(origin) = req.headers().get("origin") {
            tracing::warn!(
                origin = %String::from_utf8_lossy(origin.as_bytes()),
                "rpc: rejecting handshake carrying an Origin header (cross-origin browser dial)"
            );
            let mut err = ErrorResponse::new(Some("origin not allowed on local IPC".to_string()));
            *err.status_mut() = StatusCode::FORBIDDEN;
            return Err(err);
        }
        Ok(resp)
    };
    let ws = match tokio_tungstenite::accept_hdr_async(stream, reject_cross_origin).await {
        Ok(ws) => ws,
        Err(err) => {
            tracing::warn!(error = %err, "rpc: websocket handshake failed");
            return;
        }
    };
    serve_socket(ws, service).await;
}

async fn serve_socket(
    ws: tokio_tungstenite::WebSocketStream<TcpStream>,
    service: Arc<dyn RpcService>,
) {
    let (mut sink, mut ws_stream) = ws.split();
    let (out_tx, mut out_rx) = mpsc::channel::<String>(256);
    let (in_tx, in_rx) = mpsc::channel::<String>(256);

    // Pump: socket <-> string channels. Ends when either side closes.
    let pump = async move {
        loop {
            tokio::select! {
                frame = out_rx.recv() => match frame {
                    Some(text) => {
                        if sink.send(WsMessage::Text(text)).await.is_err() {
                            break;
                        }
                    }
                    None => {
                        let _ = sink.send(WsMessage::Close(None)).await;
                        break;
                    }
                },
                message = ws_stream.next() => match message {
                    Some(Ok(WsMessage::Text(text))) => {
                        if in_tx.send(text).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(WsMessage::Ping(payload))) => {
                        if sink.send(WsMessage::Pong(payload)).await.is_err() { break; }
                    }
                    Some(Ok(_)) => {}
                },
            }
        }
    };

    // Own both futures in this scope so key revocation also drops the socket
    // pump; an unowned spawned pump would keep its authenticated socket alive.
    tokio::select! {
        _ = serve_connection(service, out_tx, in_rx) => {},
        _ = pump => {},
    }
}
