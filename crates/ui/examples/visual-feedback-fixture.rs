//! Interactive native visual QA. No user settings, engine, network or accounts.
//! --scale is an application rem multiplier, never a Windows DPI simulation.
use gpui::{AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use rcode_ui::{
    appearance, composer, l10n, motion, settings, theme_library, typography,
    visual_feedback_fixture::{FixtureOptions, VisualFeedbackFixture},
};

fn main() -> anyhow::Result<()> {
    let mut options = FixtureOptions::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dark" => options.dark = true,
            "--reduced-motion" => options.reduced_motion = true,
            "--business" => options.business = true,
            "--font-size" => {
                options.font_size = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("missing --font-size value"))?
                    .parse()?
            }
            "--scale" => {
                options.scale = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("missing --scale value"))?
                    .parse()?
            }
            _ => anyhow::bail!("unknown argument: {arg}"),
        }
    }
    anyhow::ensure!(
        [12.0, 14.0, 16.0, 20.0].contains(&options.font_size),
        "--font-size must be 12, 14, 16 or 20"
    );
    anyhow::ensure!(
        options.scale.is_finite() && (0.5..=2.0).contains(&options.scale),
        "--scale must be between 0.5 and 2"
    );
    let temp = tempfile::tempdir()?;
    gpui_platform::application()
        .with_assets(rcode_ui::icons::Assets)
        .run(move |cx| {
            gpui_base::init(cx);
            let mut config = settings::UiSettings::default();
            config.ui_font_size = typography::UiFontSize::ALL
                .into_iter()
                .find(|size| size.pixels() == options.font_size)
                .unwrap();
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
            motion::init(
                if options.reduced_motion {
                    motion::ReduceMotion::On
                } else {
                    motion::ReduceMotion::Off
                },
                true,
                cx,
            );
            composer::init(cx, config.composer_send_behavior);
            if options.business {
                rcode_ui::app_menus::init(cx);
                rcode_ui::history::init(
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    cx,
                );
            }
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
                        size(px(1320.0), px(880.0)),
                        cx,
                    ))),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("RCode 视觉与反馈隔离验证".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                move |window, cx| {
                    window.set_rem_size(px(options.font_size * options.scale));
                    cx.new(|cx| VisualFeedbackFixture::new(options, Box::new(temp), window, cx))
                },
            )
            .expect("open visual feedback fixture");
            cx.activate(true);
        });
    Ok(())
}
