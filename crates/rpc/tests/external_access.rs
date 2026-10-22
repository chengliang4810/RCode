//! Exercise actual handshakes and connection boundaries, rather than treating
//! a successfully serialized credential as proof of authentication/recovery.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use rcode_rpc::{
    RpcClient, RpcError, RpcReply, RpcService, connect_ws, connect_ws_authenticated,
    serve_authenticated_listener,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{Notify, watch};

const FIRST_KEY: &str = "FirstKey1234";
const SECOND_KEY: &str = "NextKey56789";

#[derive(Default)]
struct Fixture {
    writes: AtomicUsize,
    active_watches: Arc<AtomicUsize>,
    write_started: Notify,
}

struct WatchLease(Arc<AtomicUsize>);
impl Drop for WatchLease {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl RpcService for Fixture {
    async fn handle(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
        match method {
            "Echo" => RpcReply::value(&params),
            "Write" => {
                self.writes.fetch_add(1, Ordering::SeqCst);
                self.write_started.notify_one();
                // Deliberately lose the reply after accepting the side effect.
                futures::future::pending::<()>().await;
                unreachable!()
            }
            "Watch" => {
                self.active_watches.fetch_add(1, Ordering::SeqCst);
                let lease = WatchLease(self.active_watches.clone());
                let stream = futures::stream::once(async { json!({"ready":true}) })
                    .chain(futures::stream::pending())
                    .map(move |item| {
                        let _keep = &lease;
                        item
                    });
                Ok(RpcReply::Stream(stream.boxed()))
            }
            _ => Err(RpcError::UnknownMethod(method.into())),
        }
    }
}

struct Server {
    url: String,
    key: watch::Sender<Option<String>>,
    service: Arc<Fixture>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (key, receiver) = watch::channel(Some(FIRST_KEY.into()));
    let service = Arc::new(Fixture::default());
    let task = tokio::spawn(serve_authenticated_listener(
        listener,
        service.clone(),
        receiver,
    ));
    Server {
        url,
        key,
        service,
        task,
    }
}

async fn wait_for(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("condition should settle");
}

async fn echo(client: &RpcClient) {
    assert_eq!(
        client
            .call("Echo", json!({"connected":true}))
            .await
            .unwrap(),
        json!({"connected":true})
    );
}

#[tokio::test]
async fn external_listener_rejects_missing_wrong_and_query_credentials() {
    let server = server().await;
    assert!(connect_ws(&server.url).await.is_err());
    let wrong = "WrongKey1234";
    let error = match connect_ws_authenticated(&server.url, wrong).await {
        Ok(_) => panic!("wrong key connected"),
        Err(error) => error.to_string(),
    };
    assert!(!error.contains(wrong));
    let query_url = format!("{}?token={FIRST_KEY}", server.url);
    let error = match connect_ws(&query_url).await {
        Ok(_) => panic!("URL query key must not authenticate"),
        Err(error) => error.to_string(),
    };
    assert!(!error.contains(FIRST_KEY));
    let client = connect_ws_authenticated(&server.url, FIRST_KEY)
        .await
        .unwrap();
    echo(&client).await;
}

#[tokio::test]
async fn rotation_and_disabling_revoke_existing_connections() {
    let server = server().await;
    let first = connect_ws_authenticated(&server.url, FIRST_KEY)
        .await
        .unwrap();
    let mut watching = first.subscribe_scoped("Watch", json!({})).await.unwrap();
    watching.recv().await.unwrap();
    server.key.send_replace(Some(SECOND_KEY.into()));
    assert!(
        tokio::time::timeout(Duration::from_secs(2), watching.recv())
            .await
            .unwrap()
            .is_none()
    );
    wait_for(|| !first.is_connected()).await;
    assert!(
        connect_ws_authenticated(&server.url, FIRST_KEY)
            .await
            .is_err()
    );
    let second = connect_ws_authenticated(&server.url, SECOND_KEY)
        .await
        .unwrap();
    echo(&second).await;
    server.key.send_replace(None);
    wait_for(|| !second.is_connected()).await;
    assert!(
        connect_ws_authenticated(&server.url, SECOND_KEY)
            .await
            .is_err()
    );
    wait_for(|| server.service.active_watches.load(Ordering::SeqCst) == 0).await;
}

#[tokio::test]
async fn reconnect_restores_new_calls_without_replaying_an_accepted_write() {
    let server = server().await;
    let client = Arc::new(
        connect_ws_authenticated(&server.url, FIRST_KEY)
            .await
            .unwrap(),
    );
    let mut watching = client.subscribe_scoped("Watch", json!({})).await.unwrap();
    watching.recv().await.unwrap();
    let write_client = client.clone();
    let write = tokio::spawn(async move { write_client.call("Write", json!({"id":"once"})).await });
    server.service.write_started.notified().await;
    server.key.send_replace(None);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), write)
            .await
            .unwrap()
            .unwrap(),
        Err(RpcError::Closed)
    ));
    assert!(
        tokio::time::timeout(Duration::from_secs(2), watching.recv())
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        client.call("Write", json!({"id":"offline"})).await,
        Err(RpcError::Closed)
    ));
    server.key.send_replace(Some(FIRST_KEY.into()));
    wait_for(|| client.is_connected()).await;
    echo(&client).await;
    let mut resumed = client.subscribe_scoped("Watch", json!({})).await.unwrap();
    resumed.recv().await.unwrap();
    assert_eq!(
        server.service.writes.load(Ordering::SeqCst),
        1,
        "neither accepted nor offline write may replay"
    );
}

#[tokio::test]
async fn dropping_a_reconnecting_client_releases_server_subscriptions() {
    let server = server().await;
    let client = connect_ws_authenticated(&server.url, FIRST_KEY)
        .await
        .unwrap();
    let mut watching = client.subscribe_scoped("Watch", json!({})).await.unwrap();
    watching.recv().await.unwrap();
    assert_eq!(server.service.active_watches.load(Ordering::SeqCst), 1);
    drop(client);
    assert!(
        tokio::time::timeout(Duration::from_secs(2), watching.recv())
            .await
            .unwrap()
            .is_none()
    );
    wait_for(|| server.service.active_watches.load(Ordering::SeqCst) == 0).await;
}
