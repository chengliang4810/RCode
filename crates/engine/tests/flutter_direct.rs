//! Explicit end-to-end gate: Flutter's shipping Dart controllers → real EngineCore.
//! Run with a Flutter SDK on PATH:
//! cargo test -p rcode-engine --test flutter_direct -- --ignored --nocapture

use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};
use rcode_engine::{EngineCore, HarnessRegistry};
use rcode_harness::{Harness, HarnessError, RunControls};
use rcode_proto::{
    AgentEvent, DoneStatus, HarnessId, InteractionRequest, InteractionResponse, Model,
    ReasoningLevel, RunRequest, SteeringMode, UserInputQuestion,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::watch;

struct FlutterHarness(Arc<AtomicUsize>);

#[async_trait]
impl Harness for FlutterHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Flutter real engine fixture"
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
        self.0.fetch_add(1, Ordering::SeqCst);
        if request.prompt == "Flutter queued once" {
            return Ok(futures::stream::iter([
                Ok(AgentEvent::TextDelta {
                    text: "Flutter queued result".into(),
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
        assert_eq!(request.prompt, "Flutter live question");
        let (send, receive) = tokio::sync::mpsc::channel(4);
        tokio::spawn(async move {
            let response = (controls.request_input)(InteractionRequest::Questions {
                questions: vec![UserInputQuestion {
                    id: "choice".into(),
                    header: "Choose".into(),
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
            tokio::time::sleep(Duration::from_millis(1200)).await;
            send.send(Ok(AgentEvent::TextDelta {
                text: "Flutter offline result".into(),
            }))
            .await
            .unwrap();
            send.send(Ok(AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: None,
            }))
            .await
            .unwrap();
        });
        Ok(futures::stream::unfold(receive, |mut receive| async move {
            receive.recv().await.map(|event| (event, receive))
        })
        .boxed())
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the Flutter SDK; runs the real Dart controllers against an engine"]
async fn flutter_controllers_keep_questions_queues_attachments_and_offline_results() {
    let profile = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(FlutterHarness(runs.clone())));
    let core =
        EngineCore::assemble(profile.path(), Arc::new(registry), HarnessId::Mock, None).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (_access, key) = watch::channel(Some("FixtureKey12".to_owned()));
    let service = Arc::new(core.rpc_service().as_ref().clone().without_remote_access());
    let server = tokio::spawn(rcode_rpc::serve_authenticated_listener(
        listener, service, key,
    ));

    #[cfg(windows)]
    let mut flutter = {
        let mut command = tokio::process::Command::new("cmd.exe");
        command.args(["/c", "flutter.bat"]);
        command
    };
    #[cfg(not(windows))]
    let mut flutter = tokio::process::Command::new("flutter");
    let mobile = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/mobile");
    let output = tokio::time::timeout(
        Duration::from_secs(120),
        flutter
            .args([
                "test",
                "test/data/live_engine_test.dart",
                "--no-pub",
                "--reporter",
                "expanded",
            ])
            .env("RCODE_FLUTTER_RPC_URL", &url)
            .env("RCODE_FLUTTER_PROJECT", project.path())
            .current_dir(mobile)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("Flutter gate timed out")
    .expect("start Flutter SDK");
    println!("{}", String::from_utf8_lossy(&output.stdout));
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "the real Flutter/engine integration failed"
    );
    assert_eq!(
        runs.load(Ordering::SeqCst),
        2,
        "one initial run plus one queued run, without replay"
    );
    server.abort();
    core.shutdown().await;
}
