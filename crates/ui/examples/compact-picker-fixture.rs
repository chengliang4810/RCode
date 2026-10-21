//! Isolated visual and keyboard evidence for the compact model picker.
use gpui::{
    AppContext, AsyncApp, Bounds, Context, Entity, Render, Window, WindowBounds, WindowOptions,
    div, prelude::*, px, size,
};
use rcode_ui::*;
use std::{path::Path, path::PathBuf, time::Duration};

struct Fixture {
    pickers: Entity<pickers::Pickers>,
    title: &'static str,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::Theme::of(cx).clone();
        div()
            .id("compact-picker-fixture")
            .size_full()
            .bg(theme.surface)
            .text_color(theme.text)
            .font_family(theme.font_sans.clone())
            .p(px(24.0))
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .gap(px(16.0))
                    .child(settings::widgets::page_header(&theme, self.title, None))
                    .child(
                        div()
                            .relative()
                            .h(px(48.0))
                            .flex()
                            .items_center()
                            .child(self.pickers.clone()),
                    ),
            )
    }
}

async fn pause(cx: &mut AsyncApp) {
    cx.background_executor()
        .timer(Duration::from_millis(260))
        .await;
}

async fn capture(
    window: gpui::AnyWindowHandle,
    cx: &mut AsyncApp,
    output: &Path,
    name: &str,
) -> anyhow::Result<()> {
    #[cfg(not(windows))]
    {
        window.update(cx, |_, window, cx| {
            window.draw(cx).clear();
            window.render_to_image()?.save(output.join(name))?;
            Ok(())
        })?
    }
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
        let target = output.join(name);
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/test-windows-capture-client.ps1");
        let mut command = tokio::process::Command::new("powershell.exe");
        command
            .creation_flags(0x08000000)
            .kill_on_drop(true)
            .args(["-NoProfile", "-NonInteractive", "-File"])
            .arg(script)
            .arg("-WindowHandle")
            .arg(hwnd.to_string())
            .arg("-ExpectedProcessId")
            .arg(std::process::id().to_string())
            .arg("-OutputFile")
            .arg(&target)
            .arg("-MetadataFile")
            .arg(target.with_extension("json"));
        let captured = tokio::time::timeout(Duration::from_secs(15), command.output()).await??;
        anyhow::ensure!(
            captured.status.success(),
            "Window capture failed: {}",
            String::from_utf8_lossy(&captured.stderr)
        );
        Ok(())
    }
}

fn press(window: gpui::AnyWindowHandle, cx: &mut AsyncApp, key: &str) -> anyhow::Result<()> {
    window.update(cx, |_, window, cx| {
        assert!(
            window.dispatch_keystroke(gpui::Keystroke::parse(key).unwrap(), cx),
            "compact picker did not handle {key}"
        );
    })?;
    Ok(())
}

async fn capture_matrix_scene(
    window: gpui::WindowHandle<Fixture>,
    cx: &mut AsyncApp,
    output: &Path,
    appearance: appearance::AppearanceMode,
    width: f32,
    label: &str,
) -> anyhow::Result<()> {
    window.update(cx, |view, window, cx| {
        appearance::set_mode(appearance, cx);
        window.resize(size(px(width), px(560.0)));
        view.title = if appearance == appearance::AppearanceMode::Light {
            "Compact picker · light"
        } else {
            "Compact picker · dark"
        };
        cx.notify();
    })?;
    pause(cx).await;
    capture(
        window.into(),
        cx,
        output,
        &format!("compact-panel-{label}.png"),
    )
    .await?;
    press(window.into(), cx, "enter")?;
    pause(cx).await;
    window.update(cx, |view, _, cx| {
        assert!(view.pickers.read(cx).fixture_compact_state(cx).model_list);
    })?;
    capture(
        window.into(),
        cx,
        output,
        &format!("compact-model-list-{label}.png"),
    )
    .await?;
    press(window.into(), cx, "escape")?;
    pause(cx).await;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let _guard = runtime.enter();
    let output = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    let providers = std::env::args().any(|arg| arg == "--providers");
    std::fs::create_dir_all(&output)?;
    let temp = tempfile::tempdir()?;
    let data = temp.path().to_path_buf();
    gpui_platform::application()
        .with_assets(icons::Assets)
        .run(move |cx| {
            gpui_tokio::init(cx);
            gpui_base::init(cx);
            let mut prefs = settings::UiSettings::default();
            prefs.compact_model_picker = true;
            if providers { prefs.language = rcode_presentation::LanguagePreference::ZhCn; }
            settings::init(prefs.clone(), data.clone(), cx);
            let fonts = typography::register_fonts(cx);
            typography::init(
                prefs.ui_font_family.clone(),
                prefs.ui_font_size,
                prefs.terminal_font_family.clone(),
                prefs.terminal_font_size,
                prefs.code_font_family.clone(),
                prefs.code_font_size,
                fonts,
                cx,
            );
            theme_library::init(data, cx);
            appearance::init(
                appearance::AppearanceMode::Dark,
                prefs.theme_selection,
                prefs.accent,
                prefs.surface,
                cx,
            );
            composer::init(cx, prefs.composer_send_behavior);

            let state = cx.new(|_| state::AppState::new());
            let pickers = cx.new(|cx| {
                let mut pickers = pickers::Pickers::new(state, cx);
                pickers.fixture_compact_catalog(cx);
                pickers
            });
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            gpui::point(px(40.0), px(40.0)),
                            size(px(840.0), px(560.0)),
                        ))),
                        ..Default::default()
                    },
                    |window, cx| {
                        pickers.update(cx, |pickers, cx| {
                            pickers.open_model_menu(window, cx)
                        });
                        cx.new(|_| Fixture {
                            pickers,
                            title: "Compact picker · dark",
                        })
                    },
                )
                .unwrap();
            cx.activate(true);

            cx.spawn(async move |cx| {
                let result: anyhow::Result<()> = async {
                    if providers {
                        for (mode, label) in [(appearance::AppearanceMode::Light, "light"), (appearance::AppearanceMode::Dark, "dark")] {
                            for route in [false, true] {
                                window.update(cx, |view, window, cx| {
                                    appearance::set_mode(mode, cx);
                                    view.title = "模型";
                                    view.pickers.update(cx, |pickers, cx| pickers.fixture_provider_scene(route, window, cx));
                                    cx.notify();
                                })?;
                                pause(cx).await;
                                capture(window.into(), cx, &output, &format!("providers-{label}-{}.png", if route { "route" } else { "custom" })).await?;
                            }
                        }
                        std::fs::write(output.join("result.txt"), "PASS: grouped provider picker rendered with selected routes and models in light and dark themes.\n")?;
                        return Ok(());
                    }
                    pause(cx).await;
                    for (appearance, width, label) in [
                        (appearance::AppearanceMode::Dark, 840.0, "dark-wide"),
                        (appearance::AppearanceMode::Dark, 360.0, "dark-narrow"),
                        (appearance::AppearanceMode::Light, 840.0, "light-wide"),
                        (appearance::AppearanceMode::Light, 360.0, "light-narrow"),
                    ] {
                        capture_matrix_scene(window, cx, &output, appearance, width, label).await?;
                    }

                    // Every visual theme animates while the provider's Fast option stays off.
                    for (mode, label) in [
                        (appearance::AppearanceMode::Dark, "dark"),
                        (appearance::AppearanceMode::Light, "light"),
                    ] {
                        for preset in effort_theme::EffortTheme::ALL {
                            window.update(cx, |view, window, cx| {
                                appearance::set_mode(mode, cx);
                                window.resize(size(px(840.0), px(560.0)));
                                settings::update(settings::SavePolicy::Immediate, cx, |settings| {
                                    settings.effort_theme = preset;
                                });
                                assert!(!view.pickers.read(cx).fixture_compact_state(cx).fast);
                                cx.notify();
                            })?;
                            pause(cx).await;
                            capture(window.into(), cx, &output,
                                &format!("effort-theme-{label}-{preset:?}.png")).await?;
                        }
                    }
                    window.update(cx, |_, _, cx| {
                        settings::update(settings::SavePolicy::Immediate, cx, |settings| {
                            settings.effort_theme = effort_theme::EffortTheme::Purple;
                        });
                        motion::set_reduced_motion(cx, false);
                    })?;
                    pause(cx).await;
                    capture(window.into(), cx, &output, "effort-motion-first.png").await?;
                    pause(cx).await;
                    capture(window.into(), cx, &output, "effort-motion-second.png").await?;

                    window.update(cx, |view, window, cx| {
                        appearance::set_mode(appearance::AppearanceMode::Dark, cx);
                        window.resize(size(px(840.0), px(560.0)));
                        view.title = "Compact picker · keyboard controls";
                        cx.notify();
                    })?;
                    pause(cx).await;

                    // The panel's direct effort shortcut selects its maximum.
                    for key in ["end"] {
                        press(window.into(), cx, key)?;
                    }
                    pause(cx).await;
                    window.update(cx, |view, _, cx| {
                        let state = view.pickers.read(cx).fixture_compact_state(cx);
                        assert_eq!(state.reasoning, Some(rcode_proto::ReasoningLevel::XHigh));
                    })?;
                    capture(
                        window.into(),
                        cx,
                        &output,
                        "compact-effort-keyboard-xhigh.png",
                    ).await?;

                    // F toggles the exact advertised service-tier option.
                    for key in ["f"] {
                        press(window.into(), cx, key)?;
                    }
                    window.update(cx, |view, _, cx| {
                        assert!(view.pickers.read(cx).fixture_compact_state(cx).fast);
                    })?;
                    pause(cx).await;
                    capture(
                        window.into(),
                        cx,
                        &output,
                        "compact-fast-keyboard.png",
                    ).await?;

                    // Enter the model list and move focus without changing selection.
                    for key in ["enter", "down"] {
                        press(window.into(), cx, key)?;
                    }
                    pause(cx).await;
                    window.update(cx, |view, _, cx| {
                        let state = view.pickers.read(cx).fixture_compact_state(cx);
                        assert!(state.model_list);
                        assert_eq!(state.focused_model.as_deref(), Some("gpt-5.3-codex"));
                    })?;
                    capture(window.into(), cx, &output, "compact-model-keyboard.png").await?;

                    // Repeat the keyboard paths at the opposite visual and
                    // geometry extreme so the dense controls are exercised,
                    // rather than only photographed, in the narrow layout.
                    press(window.into(), cx, "escape")?;
                    window.update(cx, |view, window, cx| {
                        appearance::set_mode(appearance::AppearanceMode::Light, cx);
                        window.resize(size(px(360.0), px(560.0)));
                        view.title = "Compact picker · light narrow keyboard";
                        cx.notify();
                    })?;
                    // Fast turns back off, then effort moves to its lowest level.
                    for key in ["f", "home"] {
                        press(window.into(), cx, key)?;
                    }
                    pause(cx).await;
                    window.update(cx, |view, _, cx| {
                        let state = view.pickers.read(cx).fixture_compact_state(cx);
                        assert!(!state.fast);
                        assert_eq!(state.reasoning, Some(rcode_proto::ReasoningLevel::Low));
                    })?;
                    capture(
                        window.into(),
                        cx,
                        &output,
                        "compact-effort-keyboard-light-narrow.png",
                    ).await?;

                    for key in ["enter", "down"] {
                        press(window.into(), cx, key)?;
                    }
                    pause(cx).await;
                    window.update(cx, |view, _, cx| {
                        let state = view.pickers.read(cx).fixture_compact_state(cx);
                        assert!(state.model_list);
                        assert_eq!(state.focused_model.as_deref(), Some("gpt-5.3-codex"));
                    })?;
                    capture(
                        window.into(),
                        cx,
                        &output,
                        "compact-model-keyboard-light-narrow.png",
                    ).await?;

                    std::fs::write(
                        output.join("result.txt"),
                        "PASS: compact picker panel/list layout at dark/light and narrow/wide sizes; keyboard effort, fast tier, and model navigation behavior.\n",
                    )?;
                    Ok(())
                }
                .await;
                if let Err(error) = result {
                    eprintln!("compact picker fixture failed: {error:#}");
                }
                // Release the view and deferred focus handles before test-support
                // checks the entity map at application shutdown.
                let _ = window.update(cx, |_, window, _| window.remove_window());
                pause(cx).await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    Ok(())
}
