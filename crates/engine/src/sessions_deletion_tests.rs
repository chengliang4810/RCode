use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Clone, Copy)]
enum Mode {
    Tool { background: bool },
    StuckStart,
    UnsettledWorker,
}

struct LifecycleHarness {
    mode: Mode,
    started: Arc<tokio::sync::Notify>,
    reaped: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
    held_worker: Mutex<Option<rcode_harness::RuntimeWorkerLease>>,
    forgotten: Mutex<Vec<(String, usize)>>,
}

impl LifecycleHarness {
    fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            started: Arc::default(),
            reaped: Arc::default(),
            dropped: Arc::default(),
            held_worker: Mutex::default(),
            forgotten: Mutex::default(),
        })
    }
}

struct MarkDropped(Arc<AtomicBool>);
impl Drop for MarkDropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[async_trait::async_trait]
impl Harness for LifecycleHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Deletion lifecycle fixture"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> rcode_proto::SteeringMode {
        rcode_proto::SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[rcode_proto::ReasoningLevel] {
        &[]
    }
    async fn models(&self) -> Result<Vec<rcode_proto::Model>, rcode_harness::HarnessError> {
        Ok(vec![])
    }

    fn forget_conversation(&self, chat_id: &str) {
        lock(&self.forgotten).push((chat_id.to_owned(), self.reaped.load(Ordering::Acquire)));
    }

    async fn run(
        &self,
        request: RunRequest,
        controls: RunControls,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<AgentEvent, rcode_harness::HarnessError>>,
        rcode_harness::HarnessError,
    > {
        if matches!(self.mode, Mode::StuckStart) {
            let _dropped = MarkDropped(self.dropped.clone());
            self.started.notify_one();
            return futures::future::pending().await;
        }
        if matches!(self.mode, Mode::UnsettledWorker) {
            *lock(&self.held_worker) = Some(controls.turn.track_runtime_worker());
            self.started.notify_one();
            return Ok(futures::stream::pending().boxed());
        }
        let Mode::Tool { background } = self.mode else {
            unreachable!()
        };
        let chat = controls.conversation_id.as_deref().unwrap();
        let marker = std::path::Path::new(&request.cwd).join(format!("{chat}.tool"));
        // Use this test executable as a deterministic cross-platform tool: no
        // provider, shell profile, Python installation or external service.
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sessions::deletion_tests::child_tool_fixture",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("RCODE_ENGINE_DELETE_CHILD", marker)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut ready = false;
        while let Some(line) = output.next_line().await.unwrap() {
            if line.contains("DELETE_TEST_READY") {
                ready = true;
                break;
            }
        }
        assert!(ready, "tool child exited before its running handshake");
        let lease = controls.turn.track_runtime_worker();
        controls.turn.set_background(1);
        let reaped = self.reaped.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let _lease = lease;
            controls.interrupt.cancelled().await;
            child.kill().await.unwrap();
            let _status = child.wait().await.unwrap();
            reaped.fetch_add(1, Ordering::AcqRel);
            controls.turn.set_background(0);
            let _ = tx.send(AgentEvent::Done {
                status: DoneStatus::Interrupted,
                result: None,
                error: None,
                session_id: None,
            });
        });
        self.started.notify_one();
        if background {
            // The driver may release its routing slot before detached work
            // exits. Deletion must still find and await that worker.
            Ok(futures::stream::iter([Ok(AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: None,
            })])
            .boxed())
        } else {
            Ok(futures::stream::unfold(rx, |mut rx| async {
                rx.recv().await.map(|event| (Ok(event), rx))
            })
            .boxed())
        }
    }
}

#[test]
fn child_tool_fixture() {
    let Some(marker) = std::env::var_os("RCODE_ENGINE_DELETE_CHILD") else {
        return;
    };
    use std::io::Write;
    std::fs::write(&marker, "started").unwrap();
    println!("DELETE_TEST_READY");
    std::io::stdout().flush().unwrap();
    for tick in 0u64.. {
        std::fs::write(&marker, tick.to_string()).unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn fixture(
    harness: Arc<LifecycleHarness>,
) -> (
    tempfile::TempDir,
    crate::EngineCore,
    Arc<rcode_sync::DocsStore>,
) {
    let directory = tempfile::tempdir().unwrap();
    let roots = crate::DataRoots::same(directory.path());
    let profile = crate::EngineProfile::new(&roots, "delete-tests", "delete-tests");
    let store = Arc::new(rcode_sync::DocsStore::open(profile.store_root()).unwrap());
    let registry = HarnessRegistry::new();
    registry.register(harness);
    let core = crate::EngineCore::assemble_with_profile(
        profile,
        Arc::new(registry),
        HarnessId::Mock,
        None,
    )
    .unwrap();
    (directory, core, store)
}

fn request(directory: &std::path::Path) -> RunRequest {
    RunRequest {
        prompt: "run isolated tool".into(),
        harness: None,
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: directory.to_string_lossy().into_owned(),
        resume: None,
        attachments: vec![],
        worktree: None,
    }
}

async fn start(
    core: &crate::EngineCore,
    directory: &std::path::Path,
    harness: &LifecycleHarness,
    chat: &str,
    space: Option<&str>,
) {
    core.workspace
        .create_chat(
            chat,
            space,
            Some(&core.device_id),
            None,
            Some(directory.to_string_lossy().into_owned()),
        )
        .unwrap();
    core.sessions
        .dispatch(chat, HarnessId::Mock, request(directory), None)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), harness.started.notified())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_running_chat_waits_for_real_tool_exit_and_rejects_late_work() {
    let harness = LifecycleHarness::new(Mode::Tool { background: false });
    let (directory, core, store) = fixture(harness.clone());
    start(&core, directory.path(), &harness, "delete-chat", None).await;
    let old = core.doc_host.open("delete-chat").unwrap();
    let late_session = core.sessions.session_status("delete-chat").unwrap();
    core.doc_host.flush_all();
    assert!(store.has_snapshot("delete-chat").unwrap());
    let client = rcode_rpc::memory_client(core.rpc_service());
    client
        .call(
            rcode_rpc::methods::MUTATE,
            serde_json::json!({"op":"deleteChat", "chatId":"delete-chat"}),
        )
        .await
        .unwrap();
    assert_eq!(
        harness.reaped.load(Ordering::Acquire),
        1,
        "delete acknowledgement requires actual process reap"
    );
    assert_eq!(*lock(&harness.forgotten), vec![("delete-chat".into(), 1)]);
    assert!(core.workspace.chat("delete-chat").unwrap().is_none());
    core.workspace.record_session(&late_session);
    assert!(
        core.workspace
            .read_sessions()
            .unwrap()
            .iter()
            .all(|session| session.chat_id != "delete-chat")
    );
    assert!(!store.has_snapshot("delete-chat").unwrap());
    old.write_user_message("late-writer", "must never persist again", now_ms())
        .unwrap();
    if let Some(persistence) = &old.persistence {
        persistence.flush_sync();
    }
    core.doc_host.flush_all();
    assert!(!store.has_snapshot("delete-chat").unwrap());
    assert!(core.doc_host.open("delete-chat").is_err());
    assert!(old.writer().is_err());
    assert!(
        core.sessions
            .dispatch(
                "delete-chat",
                HarnessId::Mock,
                request(directory.path()),
                None
            )
            .await
            .is_err()
    );
    assert!(
        core.sessions
            .steer("delete-chat", "late steer", None)
            .await
            .is_err()
    );
    drop(client);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deletion_also_joins_background_tool_after_driver_left_the_routing_map() {
    let harness = LifecycleHarness::new(Mode::Tool { background: true });
    let (directory, core, _store) = fixture(harness.clone());
    start(&core, directory.path(), &harness, "background-chat", None).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while core.sessions.has_run("background-chat") {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(harness.reaped.load(Ordering::Acquire), 0);
    let client = rcode_rpc::memory_client(core.rpc_service());
    client
        .call(
            rcode_rpc::methods::MUTATE,
            serde_json::json!({"op":"deleteChat", "chatId":"background-chat"}),
        )
        .await
        .unwrap();
    assert_eq!(harness.reaped.load(Ordering::Acquire), 1);
    drop(client);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminate_forces_a_hung_start_and_waits_until_its_future_is_dropped() {
    let harness = LifecycleHarness::new(Mode::StuckStart);
    let (directory, core, _store) = fixture(harness.clone());
    start(&core, directory.path(), &harness, "stuck-start", None).await;
    assert!(
        core.sessions
            .terminate_with_deadlines(
                "stuck-start",
                Duration::from_millis(25),
                Duration::from_secs(1)
            )
            .await
            .unwrap()
    );
    assert!(harness.dropped.load(Ordering::Acquire));
    assert!(!core.sessions.has_run("stuck-start"));
    // Ordinary stop is reusable; only deletion erects the permanent gate.
    assert!(core.sessions.ensure_chat_open("stuck-start").is_ok());
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unfinished_background_shutdown_returns_error_and_preserves_chat_and_snapshot() {
    let harness = LifecycleHarness::new(Mode::UnsettledWorker);
    let (directory, core, store) = fixture(harness.clone());
    start(&core, directory.path(), &harness, "unsettled-chat", None).await;
    core.doc_host.flush_all();
    let client = rcode_rpc::memory_client(core.rpc_service());
    let error = client
        .call(
            rcode_rpc::methods::MUTATE,
            serde_json::json!({"op":"deleteChat", "chatId":"unsettled-chat"}),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("shutdown incomplete"), "{error}");
    assert!(lock(&harness.forgotten).is_empty(), "failed deletion released resumable state");
    assert!(core.workspace.chat("unsettled-chat").unwrap().is_some());
    assert!(store.has_snapshot("unsettled-chat").unwrap());
    assert!(core.sessions.ensure_chat_open("unsettled-chat").is_err());
    lock(&harness.held_worker).take();
    client
        .call(
            rcode_rpc::methods::MUTATE,
            serde_json::json!({"op":"deleteChat", "chatId":"unsettled-chat"}),
        )
        .await
        .unwrap();
    assert!(core.workspace.chat("unsettled-chat").unwrap().is_none());
    assert!(!store.has_snapshot("unsettled-chat").unwrap());
    drop(client);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_close_blocks_internal_forks_before_stopping_all_its_tools() {
    let harness = LifecycleHarness::new(Mode::Tool { background: false });
    let (directory, core, store) = fixture(harness.clone());
    core.workspace
        .create_space(
            "project",
            &core.device_id,
            &directory.path().to_string_lossy(),
            None,
            false,
        )
        .unwrap();
    for chat in ["project-one", "project-two"] {
        start(&core, directory.path(), &harness, chat, Some("project")).await;
    }
    let admitted = core.workspace.begin_space_deletion("project").unwrap();
    assert_eq!(admitted.len(), 2);
    assert!(
        core.workspace
            .create_chat_with_parent(
                "late-fork",
                Some("project"),
                Some(&core.device_id),
                None,
                None,
                Some("project-one".into())
            )
            .is_err()
    );
    assert!(
        core.sessions
            .dispatch(
                "project-one",
                HarnessId::Mock,
                request(directory.path()),
                None
            )
            .await
            .is_err()
    );
    let client = rcode_rpc::memory_client(core.rpc_service());
    client
        .call(
            rcode_rpc::methods::MUTATE,
            serde_json::json!({"op":"deleteSpace", "spaceId":"project"}),
        )
        .await
        .unwrap();
    assert_eq!(harness.reaped.load(Ordering::Acquire), 2);
    assert!(core.workspace.read_spaces().unwrap().is_empty());
    for chat in ["project-one", "project-two"] {
        assert!(core.workspace.chat(chat).unwrap().is_none());
        assert!(!store.has_snapshot(chat).unwrap());
    }
    drop(client);
    core.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deletion_refuses_a_pinned_writer_before_removing_any_registry_rows() {
    for project in [false, true] {
        let harness = LifecycleHarness::new(Mode::StuckStart);
        let (directory, core, store) = fixture(harness);
        let space = project.then_some("writer-project");
        if let Some(space) = space {
            core.workspace
                .create_space(
                    space,
                    &core.device_id,
                    &directory.path().to_string_lossy(),
                    None,
                    false,
                )
                .unwrap();
        }
        core.workspace
            .create_chat(
                "pinned",
                space,
                Some(&core.device_id),
                None,
                Some(directory.path().to_string_lossy().into_owned()),
            )
            .unwrap();
        let old = core.doc_host.open("pinned").unwrap();
        old.write_user_message("before", "preserve this data", now_ms())
            .unwrap();
        core.doc_host.flush_all();
        let writer = old.writer().unwrap();
        let operation = if project {
            serde_json::json!({"op":"deleteSpace", "spaceId":"writer-project"})
        } else {
            serde_json::json!({"op":"deleteChat", "chatId":"pinned"})
        };
        let client = rcode_rpc::memory_client(core.rpc_service());
        let error = client
            .call(rcode_rpc::methods::MUTATE, operation.clone())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("document writers"), "{error}");
        assert!(core.workspace.chat("pinned").unwrap().is_some());
        assert!(store.has_snapshot("pinned").unwrap());
        assert!(old.writer().is_err());
        if let Some(space) = space {
            assert!(core.workspace.space(space).unwrap().is_some());
        }
        drop(writer);
        client
            .call(rcode_rpc::methods::MUTATE, operation)
            .await
            .unwrap();
        assert!(core.workspace.chat("pinned").unwrap().is_none());
        assert!(!store.has_snapshot("pinned").unwrap());
        drop(client);
        core.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_deletion_waits_for_snapshot_only_forks_to_publish_before_listing() {
    let harness = LifecycleHarness::new(Mode::StuckStart);
    let (directory, core, store) = fixture(harness);
    core.workspace
        .create_space(
            "fork-project",
            &core.device_id,
            &directory.path().to_string_lossy(),
            None,
            false,
        )
        .unwrap();
    core.workspace
        .create_chat("fork-source", Some("fork-project"), None, None, None)
        .unwrap();
    let mut child = core.workspace.chat("fork-source").unwrap().unwrap();
    child.id = "admitted-fork".into();
    child.parent_chat_id = Some("fork-source".into());
    let written = Arc::new(std::sync::Barrier::new(2));
    let publish = Arc::new(std::sync::Barrier::new(2));
    let workspace = core.workspace.clone();
    let docs = core.doc_host.clone();
    let written_child = written.clone();
    let publish_child = publish.clone();
    let creating = tokio::task::spawn_blocking(move || {
        let admission = workspace.admit_fork(&child, "fork-source").unwrap();
        let target = docs.open(&child.id).unwrap();
        target
            .write_user_message("copied", "fork history", now_ms())
            .unwrap();
        docs.persist_fork(&target).unwrap();
        written_child.wait();
        publish_child.wait();
        admission.publish(&child).unwrap();
    });
    tokio::task::spawn_blocking(move || written.wait())
        .await
        .unwrap();
    assert!(store.has_snapshot("admitted-fork").unwrap());
    assert!(core.workspace.chat("admitted-fork").unwrap().is_none());
    let workspace = core.workspace.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let deleting = tokio::task::spawn_blocking(move || {
        let _ = started_tx.send(());
        workspace.begin_space_deletion("fork-project").unwrap()
    });
    started_rx.await.unwrap();
    assert!(!deleting.is_finished());
    tokio::task::spawn_blocking(move || publish.wait())
        .await
        .unwrap();
    creating.await.unwrap();
    let listed = deleting.await.unwrap();
    assert!(listed.iter().any(|id| id == "admitted-fork"));
    let client = rcode_rpc::memory_client(core.rpc_service());
    client
        .call(
            rcode_rpc::methods::MUTATE,
            serde_json::json!({"op":"deleteSpace", "spaceId":"fork-project"}),
        )
        .await
        .unwrap();
    assert!(core.workspace.chat("admitted-fork").unwrap().is_none());
    assert!(!store.has_snapshot("admitted-fork").unwrap());
    drop(client);
    core.shutdown().await;
}
