//! The shipping thin client talks to the real engine over its public socket.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use futures::{StreamExt, stream::BoxStream};
use rcode_client::{
    Client, ClientConfig, Credentials, NewSession, SendOutcome, SendRequest, SessionTarget,
};
use rcode_doc::{
    MessagePart, MessageRole, SessionCommandPayload, SessionCommandStatus, SessionDoc,
};
use rcode_engine::{EngineCore, HarnessRegistry};
use rcode_harness::{Harness, HarnessError, RunControls};
use rcode_proto::{
    AgentEvent, ChatConfig, DoneStatus, HarnessId, InteractionRequest, InteractionResponse, Model,
    ReasoningLevel, RunRequest, SessionStatus, SteeringMode, UserInputAnswer, UserInputQuestion,
};
use serde_json::{Value, json};
use tokio::sync::{Semaphore, watch};

struct AskingHarness {
    runs: Arc<AtomicUsize>,
    answered: Arc<AtomicUsize>,
    finish: Arc<Semaphore>,
}

#[async_trait]
impl Harness for AskingHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Mobile direct fixture"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[]
    }
    fn deterministic_turn_end(&self) -> bool {
        true
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }

    async fn run(
        &self,
        request: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if request.prompt == "queued from the phone" {
            return Ok(futures::stream::iter([
                Ok(AgentEvent::TextDelta {
                    text: "queued turn completed".into(),
                }),
                Ok(AgentEvent::Done {
                    status: DoneStatus::Completed,
                    result: None,
                    error: None,
                    session_id: None,
                }),
            ])
            .boxed());
        }
        assert_eq!(request.prompt, "ask from the phone");
        let answered = self.answered.clone();
        let finish = self.finish.clone();
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tokio::spawn(async move {
            let response = (controls.request_input)(InteractionRequest::Questions {
                questions: vec![UserInputQuestion {
                    id: "choice".into(),
                    header: "Pick".into(),
                    question: "Which one?".into(),
                    options: vec!["a".into(), "b".into()],
                    prefill: None,
                    multiline: false,
                    multi_select: false,
                }],
            })
            .await
            .unwrap();
            let InteractionResponse::Questions { answers } = response else {
                panic!("typed answer expected")
            };
            assert_eq!(answers[0].selected_option_ids, ["b"]);
            answered.fetch_add(1, Ordering::SeqCst);
            // The test closes every viewer while the real engine still owns
            // the running turn, then lets the harness finish offline.
            let _permit = finish.acquire().await.unwrap();
            tx.send(Ok(AgentEvent::TextDelta {
                text: "finished while the phone was disconnected".into(),
            }))
            .await
            .unwrap();
            tx.send(Ok(AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: None,
            }))
            .await
            .unwrap();
        });
        Ok(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|event| (event, rx))
        })
        .boxed())
    }
}

async fn until(label: &str, mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {label}"));
}

fn import_frame(replica: &SessionDoc, frame: &Value) {
    let encoded = frame
        .get("snapshot")
        .or_else(|| frame.get("update"))
        .and_then(Value::as_str)
        .expect("replica frame");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    replica.doc().import(&bytes).unwrap();
}

fn contains_text(doc: &SessionDoc, expected: &str) -> bool {
    doc.read_entries().unwrap().iter().any(|entry| {
        entry
            .parts
            .iter()
            .any(|part| matches!(part, MessagePart::Text { text, .. } if text == expected))
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn real_engine_mobile_direct_preserves_questions_commands_and_offline_completion() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rcode_engine=debug,rcode_client=debug")
        .with_test_writer()
        .try_init();
    let data = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let answered = Arc::new(AtomicUsize::new(0));
    let finish = Arc::new(Semaphore::new(0));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(AskingHarness {
        runs: runs.clone(),
        answered: answered.clone(),
        finish: finish.clone(),
    }));
    let core =
        EngineCore::assemble(data.path(), Arc::new(registry), HarnessId::Mock, None).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (key, rx) = watch::channel(Some("FixtureKey12".to_owned()));
    let server = tokio::spawn(rcode_rpc::serve_authenticated_listener(
        listener,
        Arc::new(core.rpc_service().as_ref().clone().without_remote_access()),
        rx,
    ));
    let client = Client::new(
        ClientConfig::new(&url, cache.path()),
        Credentials::Direct {
            api_key: "FixtureKey12".into(),
        },
        Arc::new(rcode_client::events::NullListener),
    )
    .unwrap();
    until("the real host catalog", || {
        client.workspace().device(&core.device_id).is_some()
    })
    .await;
    let space_id = client
        .create_project(&core.device_id, &project.path().to_string_lossy(), false)
        .await
        .unwrap();
    until("the real project", || {
        client.workspace().project(&space_id).is_some()
            && core.workspace.space(&space_id).unwrap().is_some()
    })
    .await;
    let chat_id = client
        .create_session(NewSession {
            target: SessionTarget::Project { space_id },
            config: Some(ChatConfig {
                harness: HarnessId::Mock,
                model: None,
                reasoning: None,
                model_options: Default::default(),
            }),
            branch: None,
            cwd: None,
            title: Some("Phone fixture".into()),
        })
        .unwrap();
    until("the real chat", || {
        core.workspace.chat(&chat_id).unwrap().is_some()
    })
    .await;
    let session = client.open_session(&chat_id).unwrap();
    until("initial mobile replica", || session.snapshot().hydrated).await;
    let probe = rcode_rpc::connect_ws_authenticated(&url, "FixtureKey12")
        .await
        .unwrap();
    let mut frames = probe
        .subscribe_scoped("WatchSessionReplica", json!({"chatId":chat_id}))
        .await
        .unwrap();
    let replica = Arc::new(SessionDoc::from_doc(loro::LoroDoc::new()));
    let first = frames.recv().await.unwrap();
    assert!(first.get("snapshot").is_some());
    import_frame(&replica, &first);
    let witness = replica.clone();
    let watching = tokio::spawn(async move {
        while let Some(frame) = frames.recv().await {
            import_frame(&witness, &frame);
        }
    });
    let SendOutcome::Started { message_id } = session
        .send(SendRequest::text("ask from the phone"))
        .unwrap()
    else {
        panic!("idle turn should start")
    };
    until("harness execution", || {
        let commands = core
            .doc_host
            .open(&chat_id)
            .unwrap()
            .doc()
            .read_commands()
            .unwrap();
        if commands
            .iter()
            .any(|command| command.status == SessionCommandStatus::Rejected)
        {
            panic!("the engine rejected the run: {commands:?}");
        }
        runs.load(Ordering::SeqCst) == 1
    })
    .await;
    until("mobile question and applied run command", || session.composer().open_input.is_some()
        && replica.read_commands().unwrap().iter().any(|command| matches!(&command.payload, SessionCommandPayload::Run { message_id: id, .. } if id == &message_id) && command.status == SessionCommandStatus::Applied)).await;
    assert!(
        session.snapshot().pending.is_empty(),
        "the engine adopted the mobile send"
    );
    let input = session.composer().open_input.clone().unwrap();
    assert!(
        matches!(&input.request, InteractionRequest::Questions { questions } if questions[0].id == "choice")
    );
    assert!(
        replica
            .read_entries()
            .unwrap()
            .iter()
            .any(|entry| entry.parts.iter().any(|part| matches!(
                part,
                MessagePart::Input {
                    resolved: false,
                    ..
                }
            )))
    );
    let queued_id = session
        .enqueue("remove before execution", vec![], true)
        .unwrap();
    until("the phone's exact queue id and issuer", || {
        session
            .composer()
            .queue
            .iter()
            .any(|row| row.id == queued_id && row.from_this_device)
            && replica
                .read_queue()
                .unwrap()
                .iter()
                .any(|row| row.id == queued_id)
    })
    .await;
    let repeated = probe.call(rcode_rpc::methods::QUEUE_MESSAGE,
        json!({"chatId":chat_id,"id":queued_id,"issuedBy":client.device_id(),"text":"remove before execution","holdForTurnEnd":true})).await.unwrap();
    assert_eq!(repeated["id"], queued_id);
    assert_eq!(
        core.doc_host
            .open(&chat_id)
            .unwrap()
            .doc()
            .read_queue()
            .unwrap()
            .iter()
            .filter(|row| row.id == queued_id)
            .count(),
        1
    );
    // Older clients omit both new fields and still receive a host-minted id.
    let legacy = probe
        .call(
            rcode_rpc::methods::QUEUE_MESSAGE,
            json!({"chatId":chat_id,"text":"legacy queued row","holdForTurnEnd":true}),
        )
        .await
        .unwrap();
    let legacy_id = legacy["id"].as_str().unwrap();
    assert_ne!(legacy_id, queued_id);
    until("the legacy queue row on the phone", || session.composer().queue.iter().any(|row| row.id == legacy_id)).await;
    assert!(session.remove_queued(&queued_id).await.unwrap());
    assert!(session.remove_queued(legacy_id).await.unwrap());
    until("removed queue rows", || session.composer().queue.is_empty()).await;
    let answer = InteractionResponse::Questions {
        answers: vec![UserInputAnswer {
            question_id: "choice".into(),
            selected_option_ids: vec!["b".into()],
            text: None,
        }],
    };
    session
        .respond_input("invalid-request-id", answer.clone())
        .unwrap();
    until("a rejected command in the replica", || {
        replica
            .read_commands()
            .unwrap()
            .iter()
            .any(|command| command.status == SessionCommandStatus::Rejected)
    })
    .await;
    assert!(
        session.composer().open_input.is_some(),
        "a rejected answer keeps the question open"
    );
    session.respond_input(&input.request_id, answer).unwrap();
    until("an applied answer in the replica", || answered.load(Ordering::SeqCst) == 1 && replica.read_commands().unwrap().iter().any(|command|
        matches!(&command.payload, SessionCommandPayload::RespondInput { request_id, .. } if request_id == &input.request_id) && command.status == SessionCommandStatus::Applied)).await;
    until("the resolved mobile question", || {
        session.composer().open_input.is_none()
    })
    .await;
    key.send_replace(None);
    until("viewer disconnection", || {
        watching.is_finished() && !session.composer().room.connected
    })
    .await;
    assert!(
        session
            .send(SendRequest::text("must not enqueue offline"))
            .is_err()
    );
    finish.add_permits(1);
    let handle = core.doc_host.open(&chat_id).unwrap();
    until("server completion with no viewer", || {
        contains_text(handle.doc(), "finished while the phone was disconnected")
            && core
                .sessions
                .session_status(&chat_id)
                .is_some_and(|state| state.status == SessionStatus::Idle)
    })
    .await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    key.send_replace(Some("FixtureKey12".into()));
    client.on_foreground();
    until("recovered mobile result", || session.snapshot().entries.iter().any(|entry| entry.parts().iter().any(|part|
        matches!(part, MessagePart::Text { text, .. } if text == "finished while the phone was disconnected")))).await;
    assert_eq!(
        session
            .snapshot()
            .entries
            .iter()
            .filter(|entry| entry.role() == MessageRole::User)
            .count(),
        1
    );
    until("probe reconnection", || probe.is_connected()).await;
    let mut resumed = probe
        .subscribe_scoped("WatchSessionReplica", json!({"chatId":chat_id}))
        .await
        .unwrap();
    let frame = resumed.recv().await.unwrap();
    assert!(
        frame.get("snapshot").is_some(),
        "a new subscriber receives the settled snapshot"
    );
    let settled = SessionDoc::from_doc(loro::LoroDoc::new());
    import_frame(&settled, &frame);
    let commands = settled.read_commands().unwrap();
    assert_eq!(
        commands.len(),
        3,
        "run, rejected answer and accepted answer are never replayed"
    );
    assert!(
        commands
            .iter()
            .all(|command| command.status != SessionCommandStatus::Pending)
    );
    assert!(settled.read_entries().unwrap().iter().any(|entry| {
        entry
            .parts
            .iter()
            .any(|part| matches!(part, MessagePart::Input { resolved: true, .. }))
    }));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    let mut original = commands
        .iter()
        .find(|command| matches!(command.payload, SessionCommandPayload::Run { .. }))
        .unwrap()
        .clone();
    original.status = SessionCommandStatus::Pending;
    original.resolution = None;
    let duplicate = probe
        .call(
            rcode_rpc::methods::RELAY_COMMAND,
            json!({"chatId":chat_id,"entry":original}),
        )
        .await
        .unwrap();
    assert_eq!(duplicate["outcome"], "duplicate");
    assert_eq!(
        handle.doc().read_commands().unwrap().len(),
        3,
        "a repeated relay does not append another command"
    );
    // A legacy chat2 sender may later synchronize its original pending row.
    // That row must inherit the settled result, never overwrite Applied with
    // a false crashed-execution rejection or run the prompt a second time.
    handle.doc().queue_command(&original).unwrap();
    until("the settled outcome on a late chat2 duplicate", || {
        let commands = handle.doc().read_commands().unwrap();
        commands
            .iter()
            .filter(|command| command.id == original.id)
            .count()
            == 2
            && commands
                .iter()
                .filter(|command| command.id == original.id)
                .all(|command| command.status == SessionCommandStatus::Applied)
    })
    .await;
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    let promoted = session
        .enqueue("queued from the phone", vec![], false)
        .unwrap();
    until("promotion under the phone's queue id", || {
        session
            .snapshot()
            .entries
            .iter()
            .any(|entry| entry.id == promoted && entry.role() == MessageRole::User)
            && contains_text(handle.doc(), "queued turn completed")
    })
    .await;
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    client.shutdown();
    probe.close();
    server.abort();
    core.shutdown().await;
}
