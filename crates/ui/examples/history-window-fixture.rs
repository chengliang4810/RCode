//! Native visual QA with deterministic, isolated records. Sends no model requests.
use gpui::{AppContext, AsyncApp, Bounds, WindowBounds, WindowOptions, px, size};
use rcode_ui::*;
use std::{path::PathBuf, sync::Arc, time::Duration};

async fn pause(cx: &mut AsyncApp, ms: u64) {
    cx.background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

async fn capture(
    window: gpui::AnyWindowHandle,
    cx: &mut AsyncApp,
    output: PathBuf,
    name: &str,
) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let hwnd = window.update(cx, |_, window, cx| -> anyhow::Result<isize> {
            window.draw(cx).clear();
            match HasWindowHandle::window_handle(window)?.as_raw() {
                RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get()),
                _ => anyhow::bail!("Expected the fixture's Win32 window"),
            }
        })??;
        let helper = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/test-windows-capture-client.ps1");
        let mut command = tokio::process::Command::new("powershell.exe");
        command
            .creation_flags(0x08000000)
            .kill_on_drop(true)
            .args(["-NoProfile", "-NonInteractive", "-File"])
            .arg(helper)
            .arg("-WindowHandle")
            .arg(hwnd.to_string())
            .arg("-ExpectedProcessId")
            .arg(std::process::id().to_string())
            .arg("-OutputFile")
            .arg(output.join(format!("{name}.png")))
            .arg("-MetadataFile")
            .arg(output.join(format!("{name}.json")));
        let result = tokio::time::timeout(Duration::from_secs(15), command.output()).await??;
        anyhow::ensure!(
            result.status.success(),
            "Window capture failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (window, cx, output, name);
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
        .create_chat("goal-fixture", None, Some(&core.device_id), None, None)?;
    core.workspace.rename_chat("goal-fixture", "目标模式")?;
    let doc = core.doc_host.open("goal-fixture")?;
    let mut entries: Vec<rcode_doc::SessionMessageEntry> = serde_json::from_value(
        serde_json::json!([
            {"id":"old","role":"assistant","parts":[],"createdAt":1792468800000_i64,"deviceId":core.device_id,"status":"complete"}
        ]),
    )?;
    entries[0].parts = (0..400)
        .map(|i| rcode_doc::MessagePart::Text {
            id: format!("part-{i}"),
            text: format!(
                "历史记录 {i}：{}",
                "这一段用于检查向前加载时的滚动位置，以及长记录的窗口读取。".repeat(5)
            ),
        })
        .collect();
    doc.doc().push_message(&entries[0])?;
    for (i, text) in [
        "第 1/3 轮：已检查项目结构，下一轮验证执行路径。",
        "第 2/3 轮：执行路径验证完成，下一轮整理结果。",
        "第 3/3 轮：回归检查通过，目标已完成。",
    ]
    .into_iter()
    .enumerate()
    {
        let mut entry = entries[0].clone();
        entry.id = format!("round-{i}");
        entry.parts = vec![rcode_doc::MessagePart::Text {
            id: format!("round-text-{i}"),
            text: text.into(),
        }];
        entry.duration_ms = Some(12_000 + i as i64 * 8000);
        doc.doc().push_message(&entry)?;
    }
    let mut goal = rcode_proto::goal::SessionGoal::new(
        "fixture-goal".into(),
        "检查历史分页与目标分轮".into(),
        1792468800000_i64,
    )
    .unwrap();
    goal.iteration = 3;
    goal.status = rcode_proto::goal::GoalStatus::Complete;
    goal.active_since = None;
    goal.time_used_ms = 60_000;
    doc.doc().update_goal(Some(&goal))?;
    let ipc_port = port();
    let _ipc = runtime.block_on(rcode_engine::serve_ipc(ipc_port, core.rpc_service()))?;
    let data = temp.path().join("ui");
    std::fs::create_dir(&data)?;
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
        l10n::init(rcode_presentation::LanguagePreference::ZhCn);
        let settings = settings::UiSettings::default();
        settings::init(settings.clone(), data.clone(), cx);
        let fonts = typography::register_fonts(cx);
        typography::init(settings.ui_font_family.clone(), settings.ui_font_size, settings.terminal_font_family.clone(), settings.terminal_font_size, settings.code_font_family.clone(), settings.code_font_size, fonts, cx);
        theme_library::init(data.clone(), cx);
        appearance::init(appearance::AppearanceMode::Light, settings.theme_selection, settings.accent, settings.surface, cx);
        history::init(settings.git_history_columns, settings.git_history_column_widths, settings.git_history_column_order, settings.git_history_author_display, cx);
        composer::init(cx, settings.composer_send_behavior); terminal::panel::init(cx); app_menus::init(cx);
        let state = cx.new(|_| {
            let mut s = state::AppState::new(); s.fixture_attachment_engine(handle);
            s.no_project = true;
            s.connection = rcode_proto::view::ConnectionStatus::Ready; s.local_device_id = Some(device.clone());
            s.devices = vec![serde_json::from_value(serde_json::json!({"id":device,"name":"This device","platform":std::env::consts::OS,"lastSeenAt":null})).unwrap()];
            s.chats = chats; s.auto_selected = true; s.chats_synced = true; s.spaces_synced = true; s
        });
        let window = cx.open_window(WindowOptions { window_background: theme::Theme::of(cx).window_background_appearance(), window_bounds: Some(WindowBounds::Windowed(Bounds::new(gpui::point(px(20.0), px(40.0)), size(px(760.0), px(760.0))))), ..Default::default() }, |_, cx| cx.new(|cx| shell::Shell::new(state.clone(), boot, cx))).unwrap();
        cx.activate(true);
        cx.spawn(async move |cx| {
            let run: anyhow::Result<()> = async {
                state.update(cx, |state, cx| state.select_chat(Some("goal-fixture".into()), cx));
                window.update(cx, |_, window, _| window.activate_window())?;
                pause(cx, 1000).await;
                window.update(cx, |shell, _, cx| shell.fixture_appshots_sidebar(true, cx))?;
                pause(cx, 750).await;
                let initial=state.read_with(cx, |state,_| state.fixture_history_window()).expect("window metadata");
                anyhow::ensure!(initial.parts==128 && initial.has_older, "Unexpected initial window: {initial:?}");
                capture(window.into(), cx, output.clone(), "independent-rounds-tail").await?;
                pause(cx, 700).await;
                anyhow::ensure!(state.read_with(cx, |state,_| state.fixture_history_window()).unwrap().parts==128, "History automatically expanded");
                let transcript=window.update(cx, |shell,_,_|shell.fixture_transcript())?;
                let before=transcript.update(cx, |this,cx|this.fixture_history_page(cx));
                pause(cx, 1000).await;
                let expanded=state.read_with(cx, |state,_|state.fixture_history_window()).unwrap();
                anyhow::ensure!(expanded.parts==256 && expanded.has_older, "Paging failed: {expanded:?}");
                let after=transcript.read_with(cx, |this,_|this.fixture_history_anchor());
                anyhow::ensure!(before.0==after.0 && (before.1-after.1).abs()<2.0, "Viewport moved: {before:?} -> {after:?}");
                capture(window.into(), cx, output.clone(), "older-page-retained-position").await?;
                std::fs::write(output.join("result.txt"), format!("Native IPC + production Shell/Transcript: initial 128/403 parts; no automatic expansion; paging 256/403; visible anchor {before:?} -> {after:?}. Synthetic isolated data; no provider requests.\n"))?;
                Ok(())
            }.await;
            if let Err(error) = run { eprintln!("History fixture failed: {error:#}"); *result.lock().unwrap() = Some(format!("{error:#}")); }
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
