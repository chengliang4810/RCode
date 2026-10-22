//! 多轮历史的回归：证明第二轮真的把第一轮带给了模型。
//!
//! 这是原生 agent 的核心契约——loop 是无状态的，每轮靠宿主重放历史。之前
//! 这个重放从未实现，`RunRequest.resume` 只是个 TODO，所以第二轮开始模型就
//! 失忆了。E2E 只测了单轮，因此没暴露。
//!
//! 本测试用假 harness 记录 `RunControls.history`，断言：
//! 1. 第一轮没有历史；
//! 2. 第二轮带着第一轮的两条消息（用户 + 助手）；
//! 3. 当前正在派发的消息不会重复出现在历史里。
use super::*;
use crate::DataRoots;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 记录每次 run 收到的历史，并按脚本回一句助手消息。
struct HistoryHarness {
    prompts: Mutex<Vec<String>>,
    seen: Arc<Mutex<Vec<Option<Vec<rcode_proto::ConversationTurn>>>>>,
    calls: Arc<AtomicUsize>,
    /// Every (transcript, focus) pair `/compact` asked for.
    summarized: Arc<Mutex<Vec<(String, String)>>>,
    prompt_resets: Mutex<Vec<String>>,
    /// The window this fixture reports, in tokens. `None` means the harness
    /// does not know it, which disables compaction.
    window: Option<u32>,
}

#[async_trait::async_trait]
impl Harness for HistoryHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Native
    }
    fn display_name(&self) -> &str {
        "History fixture"
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
        request: RunRequest,
        controls: RunControls,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<AgentEvent, rcode_harness::HarnessError>>,
        rcode_harness::HarnessError,
    > {
        self.prompts.lock().unwrap().push(request.prompt);
        self.seen.lock().unwrap().push(controls.history.clone());
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(futures::stream::iter(vec![
            Ok(AgentEvent::TextDelta {
                text: format!("reply {n}"),
            }),
            Ok(AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: Some(format!("session-{n}")),
            }),
        ])
        .boxed())
    }

    async fn expand_custom_command(&self, cwd: &std::path::Path, input: &str) -> Result<Option<String>, rcode_harness::HarnessError> {
        let input = rcode_proto::invocation::harness_prompt(input, HarnessId::Native);
        rcode_agent::custom_commands::expand(&rcode_agent::custom_commands::list(Some(cwd), None, &[]), &input)
            .map_err(rcode_harness::HarnessError::Protocol)
    }

    async fn summarize(
        &self,
        transcript: String,
        focus: String,
    ) -> Result<String, rcode_harness::HarnessError> {
        self.summarized.lock().unwrap().push((transcript, focus));
        Ok("SUMMARY-OF-EARLIER-TURNS".to_owned())
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        self.window
    }

    async fn reset_prompt_cache(&self, session_id: &str) {
        self.prompt_resets.lock().unwrap().push(session_id.to_owned());
    }

    /// Mirrors what `NativeHarness` does: measure the transcript against the
    /// window and compare to the full-ratio threshold.
    ///
    /// Reimplemented here rather than delegating, because the engine crate does
    /// not depend on `rcode-agent`. What this test checks is that the engine
    /// *acts* on a harness that says "compact" — the estimator itself has its
    /// own suite.
    fn needs_compaction(&self, _model: Option<&str>, transcript: &str) -> bool {
        let Some(window) = self.window else {
            return false;
        };
        // Same shape as the real estimator: ASCII runs about four characters
        // per token. Close enough for a threshold test.
        let estimate = transcript.chars().count() as f64 / 4.0;
        estimate >= window as f64 * 0.95
    }

    fn needs_compaction_with_options(
        &self,
        _model: Option<&str>,
        options: &serde_json::Map<String, serde_json::Value>,
        transcript: &str,
    ) -> bool {
        let Some(window) = rcode_harness::context_window::effective_window(self.window, options)
        else {
            return false;
        };
        transcript.chars().count() as f64 / 4.0 >= window as f64 * 0.95
    }
}

fn request(prompt: &str) -> RunRequest {
    RunRequest {
        prompt: prompt.into(),
        harness: Some(HarnessId::Native),
        model: Some("test-model".into()),
        reasoning: None,
        model_options: Default::default(),
        cwd: String::new(),
        resume: None,
        attachments: Vec::new(),
        worktree: None,
    }
}

/// 造一个带假 harness 的引擎，并建一个 chat。
async fn fixture() -> (tempfile::TempDir, crate::EngineCore, Arc<HistoryHarness>) {
    fixture_with_window(None).await
}

async fn fixture_with_window(
    window: Option<u32>,
) -> (tempfile::TempDir, crate::EngineCore, Arc<HistoryHarness>) {
    let dir = tempfile::tempdir().unwrap();
    let harness = Arc::new(HistoryHarness {
        prompts: Mutex::new(Vec::new()),
        seen: Arc::new(Mutex::new(Vec::new())),
        calls: Arc::new(AtomicUsize::new(0)),
        summarized: Arc::new(Mutex::new(Vec::new())),
        prompt_resets: Mutex::new(Vec::new()),
        window,
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
        .create_chat("history-test", None, Some(&core.device_id), None, None)
        .unwrap();
    (dir, core, harness)
}

/// 等所有 run 结束。
async fn wait_idle(core: &crate::EngineCore) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if lock(&core.sessions.inner.runs).is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("run 未结束");
}

#[tokio::test]
async fn the_first_turn_carries_no_history() {
    let (_dir, core, harness) = fixture().await;
    core.sessions
        .dispatch("history-test", HarnessId::Native, request("第一轮"), None)
        .await
        .unwrap();
    wait_idle(&core).await;

    let seen = harness.seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "应当只跑了一轮");
    assert!(
        seen[0].is_none(),
        "第一轮不该有历史，实际拿到: {:?}",
        seen[0]
    );
    core.shutdown().await;
}

#[tokio::test]
async fn restored_history_hydrates_full_local_tool_output_without_expanding_the_display_doc() {
    let (_dir, core, _) = fixture().await;
    let handle = core.doc_host.open("history-test").unwrap();
    handle.write_user_message("u", "inspect the file", 1).unwrap();
    let full = format!("file.rs (lines 1-2000)\n{}\nIMPORTANT-LAST-LINE", "contents\n".repeat(4000));
    let call = AgentEvent::ToolCall { id: "read-one".into(), call: rcode_proto::ToolCall::ReadFile { path: "file.rs".into() } };
    let result = AgentEvent::ToolResult { id: "read-one".into(), output: Some(full.clone()), is_error: false, diff: None };
    let mut parts = Vec::new();
    for event in [&call, &result] {
        rcode_doc::fold_event_into_parts(&mut parts, event);
        core.sessions.inner.journal.append("history-test", event).unwrap();
    }
    finish_segment(handle.doc(), None, "assistant-root", &core.device_id, 1, &parts, MessageStatus::Complete).unwrap();
    let history = core.sessions.conversation_history("history-test", None).await.unwrap();
    assert!(history[1].text.contains(&full));
    assert_eq!(history[1].native_history_id.as_deref(), Some("assistant-root"));
    let display = handle.doc().read_entries().unwrap();
    assert!(!flatten_parts(&display[1].parts).contains("IMPORTANT-LAST-LINE"));
    handle.doc().set_excluded(&["assistant-root".into()], true).unwrap();
    let history = core.sessions.conversation_history("history-test", None).await.unwrap();
    assert_eq!(history.len(), 1, "excluded records must not reappear");
    core.shutdown().await;
}

#[tokio::test]
async fn custom_command_snapshot_preserves_display_replay_retry_and_fork() {
    let (dir, core, harness) = fixture().await;
    let commands = dir.path().join(".agents/commands");
    std::fs::create_dir_all(&commands).unwrap();
    let path = commands.join("review.md");
    std::fs::write(&path, "Original review: $ARGUMENTS").unwrap();
    let invocation = format!("{} auth", rcode_proto::invocation::Invocation::Command { name: "review".into() }.link());
    let mut first = request(&invocation);
    first.cwd = dir.path().to_string_lossy().into_owned();
    core.sessions.dispatch("history-test", HarnessId::Native, first.clone(), Some("command-user".into())).await.unwrap();
    wait_idle(&core).await;
    let frozen = harness.prompts.lock().unwrap()[0].clone();
    assert!(frozen.contains("Original review: auth"));
    let handle = core.doc_host.open("history-test").unwrap();
    assert_eq!(flatten_parts(&handle.doc().read_entries().unwrap()[0].parts), invocation);
    std::fs::write(&path, "Changed review: $ARGUMENTS").unwrap();
    core.sessions.dispatch("history-test", HarnessId::Native, request("Follow up"), None).await.unwrap();
    wait_idle(&core).await;
    assert_eq!(harness.seen.lock().unwrap()[1].as_ref().unwrap()[0].text, frozen);
    core.sessions.dispatch("history-test", HarnessId::Native, first, Some("command-user".into())).await.unwrap();
    wait_idle(&core).await;
    assert_eq!(harness.prompts.lock().unwrap()[2], frozen);

    let restored = loro::LoroDoc::new();
    restored.import(&handle.doc().export_snapshot().unwrap()).unwrap();
    assert_eq!(SessionDoc::from_doc(restored).command_prompt("command-user", &invocation).as_deref(), Some(frozen.as_str()));
    let client = rcode_rpc::memory_client(core.rpc_service());
    client.call(rcode_rpc::methods::FORK_SIDE_CHAT, serde_json::json!({"sourceChatId":"history-test", "chatId":"command-fork"})).await.unwrap();
    let fork = core.doc_host.open("command-fork").unwrap();
    assert_eq!(fork.doc().command_prompt("command-user", &invocation), Some(frozen));
    core.shutdown().await;
}

#[tokio::test]
async fn the_second_turn_carries_the_first() {
    let (_dir, core, harness) = fixture().await;

    core.sessions
        .dispatch("history-test", HarnessId::Native, request("我叫小明"), None)
        .await
        .unwrap();
    wait_idle(&core).await;

    core.sessions
        .dispatch("history-test", HarnessId::Native, request("我叫什么"), None)
        .await
        .unwrap();
    wait_idle(&core).await;

    let seen = harness.seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "应当跑了两轮");

    let history = seen[1]
        .as_ref()
        .expect("第二轮必须带历史——否则模型看不到第一轮说过什么");

    // 第一轮的用户消息必须在，且是 user 角色。
    assert!(
        history
            .iter()
            .any(|turn| turn.role == "user" && turn.text.contains("我叫小明")),
        "第一轮的用户消息没有传给模型: {history:?}"
    );
    // 第一轮的助手回复也必须在，否则模型不知道自己上次答了什么。
    assert!(
        history
            .iter()
            .any(|turn| turn.role == "assistant" && turn.text.contains("reply 0")),
        "第一轮的助手回复没有传给模型: {history:?}"
    );
    // 正在派发的这一条不能重复出现在历史里。
    assert!(
        !history.iter().any(|turn| turn.text.contains("我叫什么")),
        "当前消息被重复放进了历史: {history:?}"
    );

    core.shutdown().await;
}

#[tokio::test]
async fn history_accumulates_across_three_turns() {
    let (_dir, core, harness) = fixture().await;

    for prompt in ["第一步", "第二步", "第三步"] {
        core.sessions
            .dispatch("history-test", HarnessId::Native, request(prompt), None)
            .await
            .unwrap();
        wait_idle(&core).await;
    }

    let seen = harness.seen.lock().unwrap();
    assert_eq!(seen.len(), 3);

    let third = seen[2].as_ref().expect("第三轮必须带历史");
    // 前两轮的问答都应在（每轮 2 条：用户 + 助手），加上第三轮之前的累计。
    for expected in ["第一步", "reply 0", "第二步", "reply 1"] {
        assert!(
            third.iter().any(|turn| turn.text.contains(expected)),
            "第三轮缺少 `{expected}`: {third:?}"
        );
    }
    assert!(
        !third.iter().any(|turn| turn.text.contains("第三步")),
        "当前消息不该出现在历史里: {third:?}"
    );

    core.shutdown().await;
}

#[tokio::test]
async fn a_restarted_engine_still_replays_history() {
    // 历史来自持久化的 doc，不是内存状态——所以进程重启后第二轮仍然记得。
    let dir = tempfile::tempdir().unwrap();
    let harness = Arc::new(HistoryHarness {
        prompts: Mutex::new(Vec::new()),
        seen: Arc::new(Mutex::new(Vec::new())),
        calls: Arc::new(AtomicUsize::new(0)),
        summarized: Arc::new(Mutex::new(Vec::new())),
        prompt_resets: Mutex::new(Vec::new()),
        window: None,
    });
    {
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
            .create_chat("restart-test", None, Some(&core.device_id), None, None)
            .unwrap();
        core.sessions
            .dispatch(
                "restart-test",
                HarnessId::Native,
                request("重启前说的话"),
                None,
            )
            .await
            .unwrap();
        wait_idle(&core).await;
        core.shutdown().await;
    }

    // 重新装配一个引擎，模拟进程重启。
    let registry = HarnessRegistry::new();
    registry.register(harness.clone());
    let reopened = crate::EngineCore::assemble_with_profile(
        crate::EngineProfile::new(&DataRoots::same(dir.path()), "test-org", "test-user"),
        Arc::new(registry),
        HarnessId::Native,
        None,
    )
    .unwrap();
    reopened
        .sessions
        .dispatch(
            "restart-test",
            HarnessId::Native,
            request("重启后的问题"),
            None,
        )
        .await
        .unwrap();
    wait_idle(&reopened).await;

    let seen = harness.seen.lock().unwrap();
    let second = seen
        .last()
        .unwrap()
        .as_ref()
        .expect("重启后第二轮必须带历史");
    assert!(
        second.iter().any(|turn| turn.text.contains("重启前说的话")),
        "重启后历史丢失了: {second:?}"
    );
    reopened.shutdown().await;
}

// === /compact ===

/// 灌入若干轮对话，让 /compact 有东西可压。
async fn seed_turns(core: &crate::EngineCore, count: usize) {
    for index in 0..count {
        core.sessions
            .dispatch(
                "history-test",
                HarnessId::Native,
                request(&format!("第 {index} 轮的问题")),
                None,
            )
            .await
            .unwrap();
        wait_idle(core).await;
    }
}

#[tokio::test]
async fn compact_asks_the_model_to_summarize_the_older_turns() {
    let (_dir, core, harness) = fixture().await;
    seed_turns(&core, 8).await;

    core.sessions
        .dispatch("history-test", HarnessId::Native, request("/compact"), None)
        .await
        .expect("compact 应当成功");

    let summarized = harness.summarized.lock().unwrap();
    assert_eq!(summarized.len(), 1, "应当只总结一次");
    assert_eq!(*harness.prompt_resets.lock().unwrap(), vec!["session-7".to_owned()], "压缩后应失效该会话的提示词缓存");
    let (transcript, _) = &summarized[0];
    // 被总结的必须是较早的轮次，而不是最近几轮。
    assert!(
        transcript.contains("第 0 轮的问题"),
        "最早的一轮应当在被总结的文本里: {transcript}"
    );
    assert!(
        !transcript.contains("第 7 轮的问题"),
        "最近一轮不该被总结掉: {transcript}"
    );
}

#[tokio::test]
async fn compacted_turns_leave_the_models_view_but_stay_on_screen() {
    let (_dir, core, harness) = fixture().await;
    seed_turns(&core, 8).await;

    core.sessions
        .dispatch("history-test", HarnessId::Native, request("/compact"), None)
        .await
        .expect("compact 应当成功");

    // 用户仍然看得到全部消息——记录没有被改写。
    let entries = core
        .sessions
        .doc_handle("history-test")
        .unwrap()
        .doc()
        .read_entries()
        .unwrap();
    assert!(
        entries.iter().any(|entry| {
            entry.parts.iter().any(|part| {
                matches!(
                    part,
                    rcode_doc::MessagePart::Text { text, .. } if text.contains("第 0 轮的问题")
                )
            })
        }),
        "被压缩的消息不该从记录里消失"
    );

    // 但下一轮发给模型的历史里，它们不见了。
    let before = harness.seen.lock().unwrap().len();
    core.sessions
        .dispatch(
            "history-test",
            HarnessId::Native,
            request("压缩之后的新问题"),
            None,
        )
        .await
        .unwrap();
    wait_idle(&core).await;

    let seen = harness.seen.lock().unwrap();
    let after_compact = seen
        .get(before)
        .and_then(|h| h.as_ref())
        .expect("压缩后的这一轮应当带历史");
    assert!(
        !after_compact
            .iter()
            .any(|turn| turn.text.contains("第 0 轮的问题")),
        "被压缩的轮次仍出现在模型视野里: {after_compact:?}"
    );
    assert!(
        after_compact
            .iter()
            .any(|turn| turn.text.contains("SUMMARY-OF-EARLIER-TURNS")),
        "摘要没有进入模型视野: {after_compact:?}"
    );
}

#[tokio::test]
async fn compact_on_a_short_conversation_is_refused() {
    let (_dir, core, harness) = fixture().await;
    seed_turns(&core, 2).await;

    let error = core
        .sessions
        .dispatch("history-test", HarnessId::Native, request("/compact"), None)
        .await
        .expect_err("对话太短时应当拒绝");
    assert!(
        error.to_string().contains("short enough"),
        "拒绝理由应当说明原因: {error}"
    );
    // 拒绝时不该发生任何模型调用。
    assert!(
        harness.summarized.lock().unwrap().is_empty(),
        "被拒绝的压缩不该调用模型"
    );
}

#[tokio::test]
async fn a_compact_failure_leaves_the_conversation_untouched() {
    // 总结先于标记：模型调用失败时，历史必须原封不动——否则消息会藏在
    // 一个从未写出的摘要后面，永久消失。
    struct FailingSummarizer;
    #[async_trait::async_trait]
    impl Harness for FailingSummarizer {
        fn id(&self) -> HarnessId {
            HarnessId::Native
        }
        fn display_name(&self) -> &str {
            "failing summarizer"
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
            controls: RunControls,
        ) -> Result<
            futures::stream::BoxStream<'static, Result<AgentEvent, rcode_harness::HarnessError>>,
            rcode_harness::HarnessError,
        > {
            let _ = controls;
            Ok(futures::stream::iter(vec![Ok(AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: None,
            })])
            .boxed())
        }
        async fn summarize(
            &self,
            _transcript: String,
            _focus: String,
        ) -> Result<String, rcode_harness::HarnessError> {
            Err(rcode_harness::HarnessError::Protocol(
                "model is down".into(),
            ))
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(FailingSummarizer));
    let core = crate::EngineCore::assemble_with_profile(
        crate::EngineProfile::new(&DataRoots::same(dir.path()), "test-org", "test-user"),
        Arc::new(registry),
        HarnessId::Native,
        None,
    )
    .unwrap();
    core.workspace
        .create_chat("history-test", None, Some(&core.device_id), None, None)
        .unwrap();
    seed_turns(&core, 8).await;

    core.sessions
        .dispatch("history-test", HarnessId::Native, request("/compact"), None)
        .await
        .expect_err("总结失败时 /compact 应当报错");

    let entries = core
        .sessions
        .doc_handle("history-test")
        .unwrap()
        .doc()
        .read_entries()
        .unwrap();
    assert!(
        entries.iter().all(|entry| !entry.excluded),
        "总结失败却标记了消息——历史会永久丢失"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn an_ordinary_prompt_is_not_mistaken_for_a_command() {
    let (_dir, core, harness) = fixture().await;
    core.sessions
        .dispatch(
            "history-test",
            HarnessId::Native,
            request("read src/lib.rs and tell me about compact"),
            None,
        )
        .await
        .unwrap();
    wait_idle(&core).await;

    // 它必须作为普通 prompt 派发，而不是被当成命令。
    assert_eq!(
        harness.calls.load(Ordering::SeqCst),
        1,
        "普通 prompt 应当派发"
    );
    assert!(
        harness.summarized.lock().unwrap().is_empty(),
        "普通 prompt 不该触发压缩"
    );
}

// === 自动压缩（dispatch 前） ===

/// 造一个「窗口很小」的引擎，让对话很快超过阈值。
async fn small_window_fixture() -> (tempfile::TempDir, crate::EngineCore, Arc<HistoryHarness>) {
    let dir = tempfile::tempdir().unwrap();
    let harness = Arc::new(HistoryHarness {
        prompts: Mutex::new(Vec::new()),
        seen: Arc::new(Mutex::new(Vec::new())),
        calls: Arc::new(AtomicUsize::new(0)),
        summarized: Arc::new(Mutex::new(Vec::new())),
        prompt_resets: Mutex::new(Vec::new()),
        // 小到几轮就能超过阈值。测试估算按 chars/4 计，中文会长一些。
        window: Some(200),
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
        .create_chat("history-test", None, Some(&core.device_id), None, None)
        .unwrap();
    (dir, core, harness)
}

#[tokio::test]
async fn a_full_conversation_is_compacted_before_the_next_dispatch() {
    let (_dir, core, harness) = small_window_fixture().await;

    // 灌入足够多的轮次，让对话超过小窗口。
    for index in 0..6 {
        core.sessions
            .dispatch(
                "history-test",
                HarnessId::Native,
                request(&format!(
                    "这是一段足够长的消息用来把上下文填满 {index} {}",
                    "内容".repeat(80)
                )),
                None,
            )
            .await
            .unwrap();
        wait_idle(&core).await;
    }

    assert!(
        !harness.summarized.lock().unwrap().is_empty(),
        "对话超过窗口后，派发前必须自动压缩"
    );

    // 压缩必须写进 doc：被压缩的消息带上 excluded 标记。
    let entries = core
        .sessions
        .doc_handle("history-test")
        .unwrap()
        .doc()
        .read_entries()
        .unwrap();
    let excluded = entries.iter().filter(|entry| entry.excluded).count();
    assert!(
        excluded > 0,
        "压缩没有写进 doc——下一轮会把被压缩的消息全部重放回来"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn compaction_persists_so_the_next_turn_replays_a_shorter_conversation() {
    // 这是「压缩必须写 excluded」的全部意义：否则每一轮都要重新付一次总结
    // 调用，因为被压缩的消息下一轮又全长回来了。
    let (_dir, core, harness) = small_window_fixture().await;

    for index in 0..6 {
        core.sessions
            .dispatch(
                "history-test",
                HarnessId::Native,
                request(&format!("把上下文填满 {index} {}", "内容".repeat(80))),
                None,
            )
            .await
            .unwrap();
        wait_idle(&core).await;
    }
    let after_first_compact = harness.summarized.lock().unwrap().len();
    assert!(after_first_compact > 0, "应当已经压缩过一次");

    // 再发一轮：历史里被压缩的部分不该回来，所以不该再触发一次总结。
    core.sessions
        .dispatch(
            "history-test",
            HarnessId::Native,
            request("压缩之后的新一轮"),
            None,
        )
        .await
        .unwrap();
    wait_idle(&core).await;

    // 下一轮拿到的历史里不该包含最早那些被压缩掉的消息。
    let seen = harness.seen.lock().unwrap();
    let last = seen
        .last()
        .and_then(|h| h.as_ref())
        .expect("最后一轮应当带历史");
    assert!(
        !last
            .iter()
            .any(|turn| turn.text.contains("把上下文填满 0 ")),
        "被压缩的消息在下一轮又回来了: {last:?}"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn a_harness_without_a_window_never_auto_compacts() {
    // 默认 fixture 的 window 是 None——不知道窗口就不该猜。
    let (_dir, core, harness) = fixture().await;
    for index in 0..6 {
        core.sessions
            .dispatch(
                "history-test",
                HarnessId::Native,
                request(&format!("消息 {index} {}", "内容".repeat(200))),
                None,
            )
            .await
            .unwrap();
        wait_idle(&core).await;
    }
    assert!(
        harness.summarized.lock().unwrap().is_empty(),
        "窗口未知时不该自动压缩"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn selected_context_budget_triggers_persisted_compaction_before_dispatch() {
    let (_dir, core, harness) = fixture_with_window(Some(1_000_000)).await;
    seed_turns(&core, 8).await;
    core.sessions
        .doc_handle("history-test")
        .unwrap()
        .write_user_message("large-history", &"a".repeat(900_000), now_ms())
        .unwrap();
    core.sessions
        .dispatch(
            "history-test",
            HarnessId::Native,
            request("full window"),
            None,
        )
        .await
        .unwrap();
    wait_idle(&core).await;
    assert!(harness.summarized.lock().unwrap().is_empty());

    let mut selected = request("smaller window");
    selected
        .model_options
        .insert("contextWindow".into(), serde_json::json!("200k"));
    core.sessions
        .dispatch("history-test", HarnessId::Native, selected, None)
        .await
        .unwrap();
    wait_idle(&core).await;
    assert_eq!(harness.summarized.lock().unwrap().len(), 1);
    let entries = core
        .sessions
        .doc_handle("history-test")
        .unwrap()
        .doc()
        .read_entries()
        .unwrap();
    assert!(entries.iter().any(|entry| entry.excluded));
    core.shutdown().await;
}
