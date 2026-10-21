//! 自动标题的并发和持久化回归，不使用网络或真实凭证。
use super::*;
use crate::DataRoots;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

struct TitleHarness {
    calls: Arc<AtomicUsize>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    status: DoneStatus,
    observed_model: Arc<Mutex<Option<String>>>,
}

#[async_trait::async_trait]
impl Harness for TitleHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Native
    }
    fn display_name(&self) -> &str {
        "Title fixture"
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
        Ok(Vec::new())
    }
    async fn run(
        &self,
        _: RunRequest,
        _: RunControls,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<AgentEvent, rcode_harness::HarnessError>>,
        rcode_harness::HarnessError,
    > {
        unreachable!("标题必须走独立入口")
    }
    async fn run_title(
        &self,
        request: RunRequest,
        controls: RunControls,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<AgentEvent, rcode_harness::HarnessError>>,
        rcode_harness::HarnessError,
    > {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *lock(&self.observed_model) = request.model;
        self.entered.notify_one();
        tokio::select! {
            _ = controls.interrupt.cancelled() => return Err(rcode_harness::HarnessError::Protocol("cancelled".into())),
            _ = self.release.notified() => {},
        }
        Ok(futures::stream::iter(vec![
            Ok(AgentEvent::TextDelta {
                text: "修复登录错误".into(),
            }),
            Ok(AgentEvent::Done {
                status: self.status,
                result: None,
                error: None,
                session_id: None,
            }),
        ])
        .boxed())
    }
}

fn request() -> RunRequest {
    RunRequest {
        prompt: "修复登录错误".into(),
        harness: Some(HarnessId::Native),
        model: Some("selected-model".into()),
        reasoning: None,
        model_options: Default::default(),
        cwd: String::new(),
        resume: None,
        attachments: Vec::new(),
        worktree: None,
    }
}

async fn fixture(status: DoneStatus) -> (tempfile::TempDir, crate::EngineCore, Arc<TitleHarness>) {
    let dir = tempfile::tempdir().unwrap();
    let harness = Arc::new(TitleHarness {
        calls: Arc::new(AtomicUsize::new(0)),
        entered: Arc::new(Notify::new()),
        release: Arc::new(Notify::new()),
        status,
        observed_model: Arc::new(Mutex::new(None)),
    });
    let registry = HarnessRegistry::new();
    registry.register(harness.clone());
    let core = crate::EngineCore::assemble_with_profile(
        crate::EngineProfile::new(&DataRoots::same(dir.path()), "test-org", "test-user"),
        Arc::new(registry),
        HarnessId::Native,
        None,
    )
    .unwrap();
    core.workspace
        .create_chat("title-test", None, Some(&core.device_id), None, None)
        .unwrap();
    (dir, core, harness)
}

async fn wait_for_finished(core: &crate::EngineCore) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if lock(&core.sessions.inner.title_jobs).is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("标题任务未结束");
}

#[tokio::test]
async fn automatic_title_is_single_flight_and_persisted() {
    let (dir, core, harness) = fixture(DoneStatus::Completed).await;
    core.sessions
        .inner
        .spawn_auto_title("title-test".into(), HarnessId::Native, request());
    harness.entered.notified().await;
    core.sessions
        .inner
        .spawn_auto_title("title-test".into(), HarnessId::Native, request());
    assert_eq!(harness.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        lock(&harness.observed_model).as_deref(),
        Some("selected-model")
    );
    harness.release.notify_one();
    wait_for_finished(&core).await;
    assert_eq!(
        core.workspace
            .chat("title-test")
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("修复登录错误")
    );
    core.shutdown().await;
    drop(core);
    let reopened = crate::EngineCore::assemble_with_profile(
        crate::EngineProfile::new(&DataRoots::same(dir.path()), "test-org", "test-user"),
        Arc::new(HarnessRegistry::new()),
        HarnessId::Native,
        None,
    )
    .unwrap();
    assert_eq!(
        reopened
            .workspace
            .chat("title-test")
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("修复登录错误")
    );
    reopened.shutdown().await;
}

#[tokio::test]
async fn slow_title_cannot_overwrite_manual_name() {
    let (_, core, harness) = fixture(DoneStatus::Completed).await;
    core.sessions
        .inner
        .spawn_auto_title("title-test".into(), HarnessId::Native, request());
    harness.entered.notified().await;
    core.workspace
        .rename_chat("title-test", "我的手动标题")
        .unwrap();
    harness.release.notify_one();
    wait_for_finished(&core).await;
    assert_eq!(
        core.workspace
            .chat("title-test")
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("我的手动标题")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn partial_output_from_failed_title_is_not_committed() {
    let (_, core, harness) = fixture(DoneStatus::Errored).await;
    core.sessions
        .inner
        .spawn_auto_title("title-test".into(), HarnessId::Native, request());
    harness.entered.notified().await;
    harness.release.notify_one();
    wait_for_finished(&core).await;
    // 必须证明请求确实发出并产出了分片文本：否则"什么都没发生"也会让下面的断言通过，
    // 测试就无法区分"拒绝了部分输出"与"根本没跑"。
    assert_eq!(harness.calls.load(Ordering::SeqCst), 1);
    assert!(
        core.workspace
            .chat("title-test")
            .unwrap()
            .unwrap()
            .title
            .is_none()
    );
    core.shutdown().await;
}

#[tokio::test]
async fn shutdown_cancels_pending_titles() {
    let (_, core, harness) = fixture(DoneStatus::Completed).await;
    core.sessions
        .inner
        .spawn_auto_title("title-test".into(), HarnessId::Native, request());
    harness.entered.notified().await;
    core.shutdown().await;
    wait_for_finished(&core).await;
    assert!(
        core.workspace
            .chat("title-test")
            .unwrap()
            .unwrap()
            .title
            .is_none()
    );
}

#[test]
fn title_validation_rejects_paragraphs_and_accepts_unicode() {
    assert_eq!(
        clean_title("  Title: \"修复登录错误\"  "),
        Some("修复登录错误".into())
    );
    for bad in ["", "第一行\n解释", "```title```", &"字".repeat(61)] {
        assert!(clean_title(bad).is_none());
    }
}
