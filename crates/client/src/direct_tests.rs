//! Direct client contract tests over actual authenticated WebSockets.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use rcode_doc::{
    MessagePart, MessageRole, MessageStatus, SessionCommandEntry, SessionCommandPayload,
    SessionCommandStatus, SessionDoc, SessionMessageEntry,
};
use rcode_proto::{Chat, HarnessId};
use rcode_rpc::{RpcError, RpcReply, RpcService, methods};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::{Client, ClientConfig, Credentials, NewSession, SendRequest, SessionTarget};

#[derive(Clone)]
struct Host {
    chats: Arc<Mutex<Vec<Chat>>>,
    commands: Arc<Mutex<Vec<String>>>,
    doc: Arc<SessionDoc>,
    changed: watch::Sender<u64>,
}

impl Host {
    fn new() -> Self {
        let (changed, _) = watch::channel(0);
        Self {
            chats: Arc::default(),
            commands: Arc::default(),
            doc: Arc::new(SessionDoc::init("fixture-chat").unwrap()),
            changed,
        }
    }

    fn changed(&self) {
        self.changed.send_modify(|value| *value += 1);
    }

    fn reply(&self, text: &str) {
        self.doc
            .push_message(&message("answer", MessageRole::Assistant, text))
            .unwrap();
        self.changed();
    }

    fn rows(&self, method: &str) -> Value {
        match method {
            methods::WATCH_DEVICES => json!([{"id":"host","name":"Fixture host","platform":"linux",
                "lastSeenAt":chrono::Utc::now(),"capabilities":[rcode_proto::capabilities::PROJECTLESS_SCRATCH_V1]}]),
            methods::WATCH_CHATS => json!(*self.chats.lock().unwrap()),
            methods::WATCH_SPACES | methods::WATCH_SESSIONS => json!([]),
            methods::WATCH_SIDEBAR_PREFERENCES => {
                json!({"initialized":true,"synced":false,"pinnedSessionIds":[],"sections":[]})
            }
            "WatchSessionReplica" => {
                json!({"snapshot":crate::live::b64::encode(&self.doc.export_snapshot().unwrap())})
            }
            _ => Value::Null,
        }
    }
}

fn message(id: &str, role: MessageRole, text: &str) -> SessionMessageEntry {
    serde_json::from_value(json!({"id":id,"role":role,"parts":[MessagePart::Text{id:format!("text-{id}"),text:text.into()}],
        "createdAt":crate::now_ms(),"deviceId":"host","status":MessageStatus::Complete})).unwrap()
}

#[async_trait]
impl RpcService for Host {
    async fn handle(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
        match method {
            methods::LOCAL_DEVICE => RpcReply::value(&json!({"deviceId":"host"})),
            methods::WATCH_DEVICES
            | methods::WATCH_CHATS
            | methods::WATCH_SPACES
            | methods::WATCH_SESSIONS
            | methods::WATCH_SIDEBAR_PREFERENCES
            | "WatchSessionReplica" => {
                let host = self.clone();
                let method = method.to_owned();
                Ok(RpcReply::Stream(
                    futures::stream::unfold(
                        (self.changed.subscribe(), true),
                        move |(mut changed, first)| {
                            let host = host.clone();
                            let method = method.clone();
                            async move {
                                if !first {
                                    changed.changed().await.ok()?;
                                }
                                changed.borrow_and_update();
                                Some((host.rows(&method), (changed, false)))
                            }
                        },
                    )
                    .boxed(),
                ))
            }
            methods::MUTATE => {
                match params["op"].as_str() {
                    Some("createChat") => {
                        let chat: Chat = serde_json::from_value(json!({
                            "id":params["chatId"],"deviceId":"host","archived":false,
                            "cwd":params["cwd"],"config":params["config"],"createdAt":chrono::Utc::now(),"roomGen":2
                        })).unwrap();
                        self.chats.lock().unwrap().push(chat);
                    }
                    Some("renameChat") => {
                        if let Some(chat) = self
                            .chats
                            .lock()
                            .unwrap()
                            .iter_mut()
                            .find(|row| row.id == params["chatId"])
                        {
                            chat.title = params["title"].as_str().map(str::to_owned);
                        }
                    }
                    _ => {}
                }
                self.changed();
                RpcReply::value(&json!({"ok":true}))
            }
            methods::RELAY_COMMAND => {
                let entry: SessionCommandEntry =
                    serde_json::from_value(params["entry"].clone()).unwrap();
                self.commands.lock().unwrap().push(entry.id.clone());
                self.doc.queue_command(&entry).unwrap();
                if let SessionCommandPayload::Run {
                    request,
                    message_id,
                } = &entry.payload
                {
                    self.doc
                        .push_message(&message(message_id, MessageRole::User, &request.prompt))
                        .unwrap();
                }
                self.doc
                    .set_command_status(&entry.id, SessionCommandStatus::Applied, None)
                    .unwrap();
                self.changed();
                RpcReply::value(&json!({"outcome":"applied"}))
            }
            _ => Err(RpcError::UnknownMethod(method.into())),
        }
    }
}

async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("the direct client converges");
}

#[tokio::test(flavor = "multi_thread")]
async fn direct_client_restores_host_results_after_disconnect_without_resending_commands() {
    let host = Host::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (key, rx) = watch::channel(Some("FixtureKey12".into()));
    let server = tokio::spawn(rcode_rpc::serve_authenticated_listener(
        listener,
        Arc::new(host.clone()),
        rx,
    ));
    let cache = tempfile::tempdir().unwrap();
    let client = Client::new(
        ClientConfig::new(url, cache.path()),
        Credentials::Direct {
            api_key: "FixtureKey12".into(),
        },
        Arc::new(crate::events::NullListener),
    )
    .unwrap();
    until(|| client.inner.direct().unwrap().connected() && !client.workspace().devices.is_empty())
        .await;
    let chat_id = client
        .create_session(NewSession {
            target: SessionTarget::Projectless {
                device_id: "host".into(),
            },
            config: Some(
                serde_json::from_value(json!({"harness":HarnessId::Native,"model":"fixture"}))
                    .unwrap(),
            ),
            branch: None,
            cwd: None,
            title: Some("Fixture".into()),
        })
        .unwrap();
    until(|| {
        host.chats
            .lock()
            .unwrap()
            .iter()
            .any(|row| row.id == chat_id)
    })
    .await;
    until(|| {
        client
            .inner
            .workspace
            .chat(&chat_id)
            .is_some_and(|chat| chat.title.as_deref() == Some("Fixture"))
    })
    .await;
    let session = client.open_session(&chat_id).unwrap();
    until(|| session.snapshot().hydrated).await;
    session
        .send(SendRequest::text("hello from the phone"))
        .unwrap();
    until(|| host.commands.lock().unwrap().len() == 1).await;
    key.send_replace(None);
    until(|| !client.inner.direct().unwrap().connected()).await;
    assert!(
        session
            .send(SendRequest::text("do not send this while disconnected"))
            .is_err()
    );
    host.reply("The host completed while the phone was disconnected.");
    key.send_replace(Some("FixtureKey12".into()));
    until(|| session.snapshot().entries.iter().any(|entry| entry.parts().iter()
        .any(|part| matches!(part, MessagePart::Text { text, .. } if text.contains("host completed"))))).await;
    assert_eq!(
        host.commands.lock().unwrap().len(),
        1,
        "reconnection must not replay a command"
    );
    assert_eq!(
        session
            .snapshot()
            .entries
            .iter()
            .filter(|entry| entry.role() == MessageRole::User)
            .count(),
        1
    );
    client.on_background();
    assert!(
        client
            .inner
            .direct()
            .unwrap()
            .store
            .load_snapshot(&chat_id)
            .unwrap()
            .is_some()
    );
    client.on_foreground();
    client.shutdown();
    host.reply("A client shutdown does not shut down the host.");
    assert_eq!(host.doc.read_entries().unwrap().len(), 3);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn rejected_direct_credentials_request_reconnection_without_exposing_the_key() {
    #[derive(Default)]
    struct Events(Mutex<Vec<crate::ClientEvent>>);
    impl crate::ClientListener for Events {
        fn on_event(&self, event: crate::ClientEvent) {
            self.0.lock().unwrap().push(event);
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (_key, rx) = watch::channel(Some("FixtureKey12".into()));
    let server = tokio::spawn(rcode_rpc::serve_authenticated_listener(
        listener,
        Arc::new(Host::new()),
        rx,
    ));
    let cache = tempfile::tempdir().unwrap();
    let events = Arc::new(Events::default());
    let client = Client::new(
        ClientConfig::new(url, cache.path()),
        Credentials::Direct {
            api_key: "Incorrect123".into(),
        },
        events.clone(),
    )
    .unwrap();
    until(|| {
        events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, crate::ClientEvent::AuthExpired { .. }))
    })
    .await;
    assert!(!format!("{:?}", events.0.lock().unwrap()).contains("Incorrect123"));
    assert!(client.workspace().devices.is_empty());
    client.shutdown();
    server.abort();
}
