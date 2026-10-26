//! Real native Diff / terminal QA; uses temporary settings and no engine.
//! Change Windows display scaling to test DPI; --scale only changes UI rems.
use gpui::{AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use rcode_ui::{
    appearance, l10n, motion, settings,
    text_surfaces_fixture::{FixtureOptions, TextSurfacesFixture},
    theme_library, typography,
};

fn main() -> anyhow::Result<()> {
    let mut options = FixtureOptions::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dark" => options.dark = true,
            "--split" => options.split = true,
            "--wrap" => options.wrap = true,
            "--wide-cursor" => options.wide_cursor = true,
            "--ui-font-size" | "--code-font-size" | "--terminal-font-size" | "--scale" => {
                let value: f32 = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("missing {arg} value"))?
                    .parse()?;
                match arg.as_str() {
                    "--ui-font-size" => options.ui_font_size = value,
                    "--code-font-size" => options.code_font_size = value,
                    "--terminal-font-size" => options.terminal_font_size = value,
                    _ => options.scale = value,
                }
            }
            "--code-font" | "--terminal-font" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("missing {arg} value"))?;
                let family = match value.as_str() {
                    "geist" => typography::UiFontFamily::Geist,
                    "geistMono" => typography::UiFontFamily::GeistMono,
                    "sarasaUiSC" => typography::UiFontFamily::SarasaUiSC,
                    "system" => typography::UiFontFamily::System,
                    _ => typography::UiFontFamily::Installed(value),
                };
                if arg == "--code-font" {
                    options.code_font = family;
                } else {
                    options.terminal_font = family;
                }
            }
            _ => anyhow::bail!("unknown argument: {arg}"),
        }
    }
    anyhow::ensure!(
        typography::UiFontSize::ALL
            .iter()
            .any(|s| s.pixels() == options.ui_font_size),
        "--ui-font-size must be 12, 13, 14, 15, 16, 18 or 20"
    );
    for value in [options.code_font_size, options.terminal_font_size] {
        anyhow::ensure!(
            value.is_finite() && (8.0..=32.0).contains(&value),
            "code/terminal size must be 8..32"
        );
    }
    anyhow::ensure!(
        options.scale.is_finite() && (0.5..=2.0).contains(&options.scale),
        "--scale must be 0.5..2"
    );
    let temp = tempfile::tempdir()?;
    gpui_platform::application()
        .with_assets(rcode_ui::icons::Assets)
        .run(move |cx| {
            gpui_base::init(cx);
            let mut config = settings::UiSettings::default();
            config.ui_font_size = typography::UiFontSize::ALL
                .into_iter()
                .find(|s| s.pixels() == options.ui_font_size)
                .unwrap();
            config.code_font_family = options.code_font.clone();
            config.code_font_size = options.code_font_size;
            config.terminal_font_family = options.terminal_font.clone();
            config.terminal_font_size = options.terminal_font_size;
            l10n::init(config.language);
            settings::init(config.clone(), temp.path(), cx);
            let fonts = typography::register_fonts(cx);
            typography::init(
                config.ui_font_family.clone(),
                config.ui_font_size,
                config.terminal_font_family.clone(),
                config.terminal_font_size,
                config.code_font_family.clone(),
                config.code_font_size,
                fonts,
                cx,
            );
            theme_library::init(temp.path(), cx);
            appearance::init(
                if options.dark {
                    appearance::AppearanceMode::Dark
                } else {
                    appearance::AppearanceMode::Light
                },
                config.theme_selection,
                config.accent,
                config.surface,
                cx,
            );
            motion::init(motion::ReduceMotion::Off, true, cx);
            rcode_ui::terminal::panel::init(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1360.0), px(960.0)),
                        cx,
                    ))),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("RCode Diff / Terminal 字体隔离验证".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                move |window, cx| {
                    window.set_rem_size(px(options.ui_font_size * options.scale));
                    cx.new(|cx| TextSurfacesFixture::new(options, Box::new(temp), cx))
                },
            )
            .expect("open text surfaces fixture");
            cx.activate(true);
        });
    Ok(())
}
