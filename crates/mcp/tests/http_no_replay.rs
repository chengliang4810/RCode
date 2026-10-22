//! Exercise the public MCP client over streamable HTTP, including the SDK's
//! session-expiration path. All effects are counters in a loopback fixture.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rcode_mcp::{McpClient, McpServer, McpSource, McpToolInfo, McpTransport};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

struct Fixture {
    url: String,
    effects: Arc<AtomicUsize>,
    initializations: Arc<AtomicUsize>,
    listener: JoinHandle<()>,
}

impl Fixture {
    async fn start(expire_after_first_commit: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let effects = Arc::new(AtomicUsize::new(0));
        let initializations = Arc::new(AtomicUsize::new(0));
        let effects_for_server = effects.clone();
        let initializations_for_server = initializations.clone();
        let listener = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        let effects = effects_for_server.clone();
                        let initializations = initializations_for_server.clone();
                        connections.spawn(async move {
                            handle(socket, effects, initializations, expire_after_first_commit).await;
                        });
                    }
                    Some(result) = connections.join_next(), if !connections.is_empty() => {
                        result.expect("loopback request handler completed");
                    }
                }
            }
        });
        Self {
            url,
            effects,
            initializations,
            listener,
        }
    }

    async fn connect(&self) -> McpClient {
        let client = tokio::time::timeout(
            Duration::from_secs(5),
            McpClient::connect(vec![McpServer {
                name: "http-fixture".to_owned(),
                transport: McpTransport::Http {
                    url: self.url.clone(),
                    headers: BTreeMap::new(),
                },
                enabled: true,
                source: McpSource::User,
            }]),
        )
        .await
        .expect("loopback MCP initialize completes");
        assert_eq!(client.tools().len(), 1, "{:?}", client.servers());
        client
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Aborting the accept task also drops its JoinSet and all open sockets.
        self.listener.abort();
    }
}

struct Request {
    method: String,
    session: Option<String>,
    message: Option<Value>,
}

async fn request(socket: &mut TcpStream) -> Request {
    let mut received = Vec::new();
    let end = loop {
        let mut chunk = [0u8; 2048];
        let count = socket.read(&mut chunk).await.unwrap();
        assert!(count > 0, "request closed before HTTP headers");
        received.extend_from_slice(&chunk[..count]);
        assert!(
            received.len() < 65_536,
            "unexpectedly large fixture request"
        );
        if let Some(index) = received.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let header = std::str::from_utf8(&received[..end]).unwrap();
    let method = header.split_whitespace().next().unwrap().to_owned();
    let headers: BTreeMap<_, _> = header
        .lines()
        .skip(1)
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect();
    let length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>().unwrap())
        .unwrap_or(0);
    assert!(length < 65_536);
    while received.len() < end + length {
        let mut chunk = [0u8; 2048];
        let count = socket.read(&mut chunk).await.unwrap();
        assert!(count > 0, "request closed before its JSON body");
        received.extend_from_slice(&chunk[..count]);
    }
    Request {
        method,
        session: headers.get("mcp-session-id").cloned(),
        message: (length > 0)
            .then(|| serde_json::from_slice(&received[end..end + length]).unwrap()),
    }
}

async fn respond(socket: &mut TcpStream, status: &str, body: Option<Value>, session: Option<&str>) {
    let body = body.map(|value| value.to_string()).unwrap_or_default();
    let session = session
        .map(|session| format!("Mcp-Session-Id: {session}\r\n"))
        .unwrap_or_default();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{session}\r\n{body}",
        body.len()
    );
    socket.write_all(response.as_bytes()).await.unwrap();
    socket.shutdown().await.unwrap();
}

async fn handle(
    mut socket: TcpStream,
    effects: Arc<AtomicUsize>,
    initializations: Arc<AtomicUsize>,
    expire_after_first_commit: bool,
) {
    let request = request(&mut socket).await;
    match request.method.as_str() {
        // The fixture intentionally uses JSON responses, not a common SSE stream.
        "GET" => return respond(&mut socket, "405 Method Not Allowed", None, None).await,
        "DELETE" => return respond(&mut socket, "200 OK", None, None).await,
        "POST" => {}
        other => panic!("unexpected HTTP method {other}"),
    }
    let message = request.message.expect("MCP POST body");
    let identity = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message["method"].as_str().unwrap();
    let result = match method {
        "initialize" => {
            assert!(request.session.is_none(), "initialize starts a new session");
            let generation = initializations.fetch_add(1, Ordering::SeqCst) + 1;
            let session = format!("fixture-session-{generation}");
            let body = json!({"jsonrpc":"2.0", "id":identity, "result": {
                "protocolVersion":"2025-06-18", "capabilities":{"tools":{}},
                "serverInfo":{"name":"http-no-replay", "version":"1"}
            }});
            return respond(&mut socket, "200 OK", Some(body), Some(&session)).await;
        }
        "tools/list" => json!({"tools":[{
            "name":"record", "description":"Commit one fixture counter increment",
            "inputSchema":{"type":"object", "properties":{}},
            "annotations":{"readOnlyHint":false,"idempotentHint":false}
        }]}),
        "tools/call" => {
            assert!(
                request.session.is_some(),
                "exercise the SDK's session-aware POST path"
            );
            let count = effects.fetch_add(1, Ordering::SeqCst) + 1;
            if expire_after_first_commit && count == 1 {
                return respond(
                    &mut socket,
                    "404 Not Found",
                    Some(json!({"error":"effect committed before session response was lost"})),
                    None,
                )
                .await;
            }
            json!({"content":[{"type":"text", "text":json!({"count":count}).to_string()}], "isError":false})
        }
        _ if identity.is_null() => {
            return respond(&mut socket, "202 Accepted", None, None).await;
        }
        // Auto-discover clients must fall back to a legacy stateful session;
        // otherwise a 404 cannot exercise SessionExpired and its hidden retry.
        _ => {
            let error = json!({"jsonrpc":"2.0", "id":identity, "error": {
                "code":-32601, "message":"Method not found"
            }});
            return respond(&mut socket, "200 OK", Some(error), None).await;
        }
    };
    let body = json!({"jsonrpc":"2.0", "id":identity, "result":result});
    respond(&mut socket, "200 OK", Some(body), None).await;
}

async fn call(client: &McpClient) -> Result<Value, String> {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        client.call(
            &McpToolInfo::wire_name("http-fixture", "record"),
            Some(json!({})),
        ),
    )
    .await
    .expect("loopback HTTP tool call settles")?;
    serde_json::from_str(&rcode_mcp::result_text(&result)).map_err(|error| error.to_string())
}

#[tokio::test]
async fn ordinary_http_calls_reuse_the_session_and_execute_once_each() {
    let fixture = Fixture::start(false).await;
    let client = fixture.connect().await;
    assert_eq!(call(&client).await.unwrap()["count"], 1);
    assert_eq!(call(&client).await.unwrap()["count"], 2);
    assert_eq!(fixture.effects.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.initializations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn http_404_after_commit_is_not_replayed_but_a_later_call_can_reconnect() {
    let fixture = Fixture::start(true).await;
    let client = fixture.connect().await;
    let error = call(&client)
        .await
        .expect_err("the uncertain call must return an error");
    assert!(error.contains("not retried"), "{error}");
    assert_eq!(fixture.effects.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.initializations.load(Ordering::SeqCst),
        1,
        "the failed public call must not initialize a replacement session"
    );

    assert_eq!(call(&client).await.unwrap()["count"], 2);
    assert_eq!(fixture.effects.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.initializations.load(Ordering::SeqCst), 2);
}
