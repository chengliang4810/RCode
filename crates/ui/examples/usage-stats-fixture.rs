//! Native visual QA with deterministic, isolated records. Sends no model requests.
use gpui::{AppContext, AsyncApp, Bounds, WindowBounds, WindowOptions, px, size};
use rcode_ui::*;
use std::{path::PathBuf, sync::Arc, time::Duration};

async fn pause(cx: &mut AsyncApp, ms: u64) {
    cx.background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

async fn capture(cx: &mut AsyncApp, output: PathBuf, name: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let helper = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/test-windows-capture-client.ps1");
        let quote = |path: &std::path::Path| path.to_string_lossy().replace('\'', "''");
        let pid = std::process::id();
        let script = format!(
            "$fixtureProcess = Get-Process -Id {pid}; & '{}' -WindowHandle ($fixtureProcess.MainWindowHandle.ToInt64()) -ExpectedProcessId {pid} -OutputFile '{}' -MetadataFile '{}'",
            quote(&helper),
            quote(&output.join(format!("{name}.png"))),
            quote(&output.join(format!("{name}.json")))
        );
        cx.background_executor()
            .spawn(async move {
                let mut child = std::process::Command::new("powershell.exe")
                    .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                    .creation_flags(0x08000000)
                    .spawn()?;
                let started = std::time::Instant::now();
                loop {
                    if let Some(status) = child.try_wait()? {
                        anyhow::ensure!(status.success(), "fixture capture failed: {status}");
                        return Ok(());
                    }
                    if started.elapsed() > Duration::from_secs(15) {
                        let _ = child.kill();
                        let _ = child.wait();
                        anyhow::bail!("fixture capture timed out");
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            })
            .await
    }
    #[cfg(not(windows))]
    {
        let _ = (cx, output, name);
        anyhow::bail!("this fixture captures its Windows client window");
    }
}

fn port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("warn").init();
    let output = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("output directory");
    std::fs::create_dir_all(&output)?;
    let output = std::path::absolute(output)?;
    let language = if std::env::args().nth(2).as_deref() == Some("en-US") {
        rcode_presentation::LanguagePreference::EnUs
    } else {
        rcode_presentation::LanguagePreference::ZhCn
    };
    let temp = tempfile::tempdir()?;
    let runtime = tokio::runtime::Runtime::new()?;
    let _runtime_guard = runtime.enter();
    let core = runtime.block_on(async {
        rcode_engine::EngineCore::assemble(
            &temp.path().join("engine"),
            Arc::new(rcode_engine::default_registry()),
            rcode_proto::HarnessId::Native,
            None,
        )
    })?;
    core.workspace
        .create_chat("usage-fixture", None, Some(&core.device_id), None, None)?;
    core.workspace.rename_chat("usage-fixture", "会话统计")?;
    let entries: Vec<rcode_doc::SessionMessageEntry> = serde_json::from_value(serde_json::json!([
        {"id":"u1","role":"user","parts":[{"id":"u1t","kind":"text","text":"检查项目"}],"createdAt":1792468800000_i64,"deviceId":core.device_id},
        {"id":"a1","role":"assistant","parts":[{"id":"a1t","kind":"text","text":"已完成检查。"},
            {"id":"call1","kind":"usage","statistics":{"model":"m_usage_fixture","inputTokens":10080407,"outputTokens":338989,"cacheReadTokens":10000,"cacheWriteTokens":null,"durationMs":2358500,"firstTokenMs":4400}}],"createdAt":1792468801000_i64,"deviceId":core.device_id,"status":"complete"},
        {"id":"u2","role":"user","parts":[{"id":"u2t","kind":"text","text":"再检查一下统计数据"}],"createdAt":1792468860000_i64,"deviceId":core.device_id},
        {"id":"a2","role":"assistant","parts":[{"id":"a2t","kind":"text","text":"已核对输入、输出和缓存用量。"},
            {"id":"call2","kind":"usage","statistics":{"model":"m_usage_fixture","inputTokens":10589,"outputTokens":297,"cacheReadTokens":0,"cacheWriteTokens":null,"durationMs":20000,"firstTokenMs":4400}}],"createdAt":1792468861000_i64,"deviceId":core.device_id,"status":"complete"}
    ]))?;
    let doc = core.doc_host.open("usage-fixture")?;
    for entry in &entries {
        doc.doc().push_message(entry)?;
    }
    doc.doc().update_context_usage(Some(10589), Some(200000))?;
    let ipc_port = port();
    let _ipc = runtime.block_on(rcode_engine::serve_ipc(ipc_port, core.rpc_service()))?;
    let data = temp.path().join("ui");
    std::fs::create_dir(&data)?;
    settings::composer::ComposerDefaults {
        model_labels: std::collections::HashMap::from([(
            "m_usage_fixture".into(),
            "Fixture provider · DeepSeek V4.1 Flash".into(),
        )]),
        ..Default::default()
    }.save(&data)?;
    let boot = EngineBootConfig {
        roots: DataRoots::same(data.clone()),
        ipc_port,
        edge_url: String::new(),
        edge_token: None,
        org_id: None,
        default_harness: rcode_proto::HarnessId::Native,
    };
    let handle = runtime.block_on(state::EngineHandle::bootstrap(boot.clone()))?;
    let chats = core.workspace.read_chats()?;
    let device = core.device_id.clone();
    let failure = Arc::new(std::sync::Mutex::new(None));
    let result = failure.clone();
    gpui_platform::application().with_assets(icons::Assets).run(move |cx| {
        gpui_tokio::init(cx); gpui_base::init(cx);
        l10n::init(language);
        let settings = settings::UiSettings::default();
        settings::init(settings.clone(), data.clone(), cx);
        let fonts = typography::register_fonts(cx);
        typography::init(settings.ui_font_family.clone(), settings.ui_font_size, settings.terminal_font_family.clone(), settings.terminal_font_size, settings.code_font_family.clone(), settings.code_font_size, fonts, cx);
        theme_library::init(data.clone(), cx);
        appearance::init(appearance::AppearanceMode::Dark, settings.theme_selection, settings.accent, settings.surface, cx);
        history::init(settings.git_history_columns, settings.git_history_column_widths, settings.git_history_column_order, settings.git_history_author_display, cx);
        composer::init(cx, settings.composer_send_behavior); terminal::panel::init(cx); app_menus::init(cx);
        let state = cx.new(|_| {
            let mut s = state::AppState::new(); s.fixture_attachment_engine(handle);
            s.data_dir = Some(data.clone());
            s.no_project = true;
            s.connection = rcode_proto::view::ConnectionStatus::Ready; s.local_device_id = Some(device.clone());
            s.devices = vec![serde_json::from_value(serde_json::json!({"id":device,"name":"This device","platform":std::env::consts::OS,"lastSeenAt":null})).unwrap()];
            s.chats = chats; s.selected_chat = Some("usage-fixture".into()); s.auto_selected = true; s.chats_synced = true; s.spaces_synced = true; s
        });
        let window = cx.open_window(WindowOptions { window_background: theme::Theme::of(cx).window_background_appearance(), window_bounds: Some(WindowBounds::Windowed(Bounds::new(gpui::point(px(20.0), px(40.0)), size(px(1100.0), px(850.0))))), ..Default::default() }, |_, cx| cx.new(|cx| shell::Shell::new(state.clone(), boot, cx))).unwrap();
        cx.activate(true);
        cx.spawn(async move |cx| {
            let run: anyhow::Result<()> = async {
                pause(cx, 1000).await;
                state.update(cx, |state, cx| {
                    state.receive_transcript_frame(rcode_doc::TranscriptFrame::Reset { reset: entries }, cx).unwrap();
                    state.context_usage = Some(rcode_proto::ContextUsage { tokens: Some(10589), window: Some(200000) }); cx.notify();
                });
                window.update(cx, |shell, _, cx| shell.fixture_appshots_sidebar(true, cx))?;
                pause(cx, 750).await;
                capture(cx, output.clone(), "session-statistics").await?;
                window.update(cx, |shell, window, cx| shell.fixture_appshots_composer().update(cx, |c, cx| c.fixture_usage_panel(Some(0), window, cx)))?;
                pause(cx, 700).await;
                capture(cx, output.clone(), "session-performance-card").await?;
                let opened = window.update(cx, |shell, _, cx| shell.fixture_appshots_composer().read(cx).fixture_usage_open_count(cx))?;
                anyhow::ensure!(opened == 1, "performance panel was not open");
                window.update(cx, |shell, window, cx| shell.fixture_appshots_composer().update(cx, |c, cx| c.fixture_usage_panel(None, window, cx)))?;
                pause(cx, 700).await;
                let opened = window.update(cx, |shell, _, cx| shell.fixture_appshots_composer().read(cx).fixture_usage_open_count(cx))?;
                anyhow::ensure!(opened == 0, "Statistics panel did not close");
                window.update(cx, |shell, window, cx| shell.fixture_appshots_composer().update(cx, |c, cx| c.fixture_usage_panel(Some(1), window, cx)))?;
                pause(cx, 700).await;
                capture(cx, output.clone(), "session-token-card").await?;
                window.update(cx, |shell, window, cx| {
                    shell.fixture_appshots_composer().update(cx, |c, cx| c.fixture_usage_panel(None, window, cx));
                    shell.fixture_transcript().update(cx, |t, cx| t.fixture_usage_panel("a2", window, cx));
                })?;
                pause(cx, 700).await;
                capture(cx, output.clone(), "turn-token-card").await?;
                window.update(cx, |_, window, _| window.resize(size(px(390.0), px(850.0))))?;
                pause(cx, 600).await;
                capture(cx, output.clone(), "statistics-narrow-390").await?;
                window.update(cx, |_, window, _| window.resize(size(px(1100.0), px(850.0))))?;
                state.update(cx, |state, cx| state.select_chat(None, cx));
                pause(cx, 700).await;
                capture(cx, output.clone(), "new-chat-no-statistics").await?;
                std::fs::write(output.join("result.txt"), "Production Shell, Composer and Transcript; synthetic persisted statistics; no provider requests or account data. Windows client capture only.\n")?;
                Ok(())
            }.await;
            if let Err(error) = run { eprintln!("Statistics fixture failed: {error:#}"); *result.lock().unwrap() = Some(format!("{error:#}")); }
            drop(state);
            let _ = window.update(cx, |_, window, _| window.remove_window());
            pause(cx, 100).await; cx.update(|cx| cx.quit());
        }).detach();
    });
    runtime.block_on(core.shutdown());
    if let Some(error) = failure.lock().unwrap().take() {
        anyhow::bail!(error);
    }
    Ok(())
}
