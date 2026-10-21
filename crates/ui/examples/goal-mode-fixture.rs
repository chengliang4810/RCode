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
    let todo_items: Vec<_> = [
        "检查项目结构与主要执行路径",
        "检查异常处理和取消行为",
        "验证目标与待办同时显示时的布局",
        "检查会话恢复",
        "运行回归测试",
        "检查窄窗口下的长文本换行",
        "整理检查结果",
    ]
    .into_iter()
    .enumerate()
    .map(|(ix, text)| {
        rcode_proto::TodoItem::new(
            text,
            match ix {
                0 => rcode_proto::TodoStatus::Completed,
                1 => rcode_proto::TodoStatus::InProgress,
                _ => rcode_proto::TodoStatus::Pending,
            },
        )
    })
    .collect();
    let todo_call = rcode_proto::ToolCall::Todo { items: todo_items };
    let entries: Vec<rcode_doc::SessionMessageEntry> = serde_json::from_value(serde_json::json!([
        {"id":"u1","role":"user","parts":[{"id":"u1t","kind":"text","text":"实现登录流程，并验证异常处理和测试。"}],"createdAt":1792468800000_i64,"deviceId":core.device_id},
        {"id":"a1","role":"assistant","parts":[{"id":"a1t","kind":"text","text":"登录流程已实现，正在收集子 Agent 的验证结果。"},{"kind":"tool","id":"fixture-todo","call":todo_call,"resolved":true}],"createdAt":1792468801000_i64,"deviceId":core.device_id,"status":"complete"}
    ]))?;
    let doc = core.doc_host.open("goal-fixture")?;
    for entry in &entries {
        doc.doc().push_message(entry)?;
    }
    let mut goal = rcode_proto::goal::SessionGoal::new(
        "fixture-goal".into(),
        "实现登录流程，并验证异常处理和测试".into(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as i64 - 44_000,
    )
    .unwrap();
    goal.iteration = 3;
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
                let geometry = window.update(cx, |shell, _, cx| shell.fixture_appshots_composer().read(cx).fixture_goal_surface_bounds())?;
                std::fs::write(output.join("layout.txt"), format!("input={geometry:?}; viewport={:?}", window.update(cx, |_, window, _| window.viewport_size())?))?;
                anyhow::ensure!(geometry.is_some_and(|bounds| f32::from(bounds.bottom()) <= 760.0), "Input surface clipped: {geometry:?}");
                capture(window.into(), cx, output.clone(), "goal-active").await?;
                window.update(cx, |shell, _, cx| shell.fixture_appshots_composer().update(cx, |composer, cx| composer.fixture_goal_action("pause", cx)))?;
                pause(cx, 700).await;
                let observed = state.read_with(cx, |state, _| state.goal.as_ref().map(|goal| goal.status));
                anyhow::ensure!(observed == Some(rcode_proto::goal::GoalStatus::Paused), "Pause command did not reach the document watch: {observed:?}");
                capture(window.into(), cx, output.clone(), "goal-paused").await?;
                goal = doc.doc().goal().unwrap();
                let frozen_time = goal.elapsed_ms(i64::MAX);
                pause(cx, 1100).await;
                anyhow::ensure!(doc.doc().goal().unwrap().elapsed_ms(i64::MAX) == frozen_time, "Paused clock kept running");
                let mut finished = entries[1].clone(); finished.id = "todos-finished".into(); finished.parts.clear();
                rcode_doc::fold_event_into_parts(&mut finished.parts, &rcode_proto::AgentEvent::ToolCall { id: "finished-todos".into(), call: rcode_proto::ToolCall::Todo { items: vec![rcode_proto::TodoItem::new("全部检查完成", rcode_proto::TodoStatus::Completed)] } });
                doc.doc().push_message(&finished)?;
                pause(cx, 700).await;
                let visible = window.update(cx, |shell, window, cx| shell.fixture_appshots_composer().update(cx, |composer, cx| composer.fixture_todo_visible(window, cx)))?;
                anyhow::ensure!(!visible, "Completed todos still visible");
                capture(window.into(), cx, output.clone(), "todos-finished-hidden").await?;
                let mut next = entries[1].clone(); next.id = "next-todos".into(); doc.doc().push_message(&next)?;
                pause(cx, 700).await;
                let visible = window.update(cx, |shell, window, cx| shell.fixture_appshots_composer().update(cx, |composer, cx| composer.fixture_todo_visible(window, cx)))?;
                anyhow::ensure!(visible, "New pending todos did not reappear");
                capture(window.into(), cx, output.clone(), "new-todos-visible").await?;
                goal.status = rcode_proto::goal::GoalStatus::Complete;
                goal.reason = Some("实现和验证均已完成".into());
                doc.doc().update_goal(Some(&goal))?;
                pause(cx, 700).await;
                capture(window.into(), cx, output.clone(), "goal-complete").await?;
                window.update(cx, |_, window, _| window.resize(size(px(390.0), px(850.0))))?;
                goal.status = rcode_proto::goal::GoalStatus::Paused;
                doc.doc().update_goal(Some(&goal))?;
                pause(cx, 700).await;
                capture(window.into(), cx, output.clone(), "goal-narrow-390").await?;
                window.update(cx, |_, window, _| window.resize(size(px(760.0), px(760.0))))?;
                let mut empty = entries[1].clone();
                empty.id = "todo-cleared".into();
                empty.parts.clear();
                rcode_doc::fold_event_into_parts(&mut empty.parts, &rcode_proto::AgentEvent::ToolCall { id: "clear-todo".into(), call: rcode_proto::ToolCall::Todo { items: vec![] } });
                doc.doc().push_message(&empty)?;
                pause(cx, 700).await;
                goal.objective = "临时测试 codex 的目标ui".into();
                doc.doc().update_goal(Some(&goal))?;
                pause(cx, 300).await;
                capture(window.into(), cx, output.clone(), "goal-only-paused").await?;
                goal.status = rcode_proto::goal::GoalStatus::Active;
                goal.active_since = Some(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as i64);
                doc.doc().update_goal(Some(&goal))?;
                pause(cx, 500).await;
                window.update(cx, |shell, window, cx| { window.resize(size(px(1240.0), px(760.0))); shell.fixture_open_goal_editor(cx); })?;
                pause(cx, 1000).await;
                capture(window.into(), cx, output.clone(), "goal-editor-open").await?;
                anyhow::ensure!(window.update(cx, |shell, window, cx| shell.fixture_goal_draft_survives_close(window, cx))?, "Closing and reopening lost the goal draft or duplicated the tab");
                window.update(cx, |shell, _, cx| shell.fixture_save_goal_editor("实现登录流程，补充失败重试与回归测试。", cx))?;
                pause(cx, 800).await;
                let edited = doc.doc().goal().unwrap();
                anyhow::ensure!(edited.objective == "实现登录流程，补充失败重试与回归测试。" && edited.status == rcode_proto::goal::GoalStatus::Active && edited.id == goal.id && edited.revision == goal.revision + 1 && edited.active_since == goal.active_since, "Editor did not save safely: {edited:?}");
                anyhow::ensure!(state.read_with(cx, |state, _| state.goal.as_ref().map(|g| &g.objective) == Some(&edited.objective)), "Saved goal did not reach the document watch");
                capture(window.into(), cx, output.clone(), "goal-editor-saved").await?;
                let mut restored = entries[1].clone(); restored.id = "todo-restored".into();
                doc.doc().push_message(&restored)?;
                cx.update(|cx| appearance::set_mode(appearance::AppearanceMode::Dark, cx));
                pause(cx, 700).await;
                capture(window.into(), cx, output.clone(), "goal-todo-dark").await?;
                window.update(cx, |shell, _, cx| shell.fixture_appshots_composer().update(cx, |composer, cx| composer.fixture_goal_action("clear", cx)))?;
                pause(cx, 700).await;
                anyhow::ensure!(state.read_with(cx, |state, _| state.goal.is_none()), "Clear command did not reach the document watch");
                capture(window.into(), cx, output.clone(), "goal-cleared").await?;
                std::fs::write(output.join("result.txt"), "Production Shell and Composer; document goal changes flow through the native IPC watch. Synthetic isolated records; no provider requests. Windows client captures.\n")?;
                Ok(())
            }.await;
            if let Err(error) = run { eprintln!("Goal fixture failed: {error:#}"); *result.lock().unwrap() = Some(format!("{error:#}")); }
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
