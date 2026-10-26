//! Native inline-link visual QA without an engine or network requests.
use gpui::{
    AppContext, Bounds, Context, IntoElement, ParentElement, Render, Styled, Window, WindowBounds,
    WindowOptions, div, px, size,
};
use rcode_ui::{
    markdown::{
        self,
        render::{LinkUi, RenderOptions},
    },
    settings,
    theme::Theme,
};

struct Fixture {
    _temp: tempfile::TempDir,
}
impl Render for Fixture {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let mut opts = RenderOptions::settled("link-style-fixture".into());
        opts.workspace_root = Some("D:/projects/zeron".into());
        opts.link = Some(LinkUi::fixture("D:/projects/zeron"));
        let tree = markdown::parse_full(
            "主要改动在 [transcript.rs (line 709)](D:/projects/zeron/crates/ui/src/transcript.rs:709)。本次完成了源码和隔离开发窗口验证。\n\n\
             网页链接：[项目文档](https://example.com/docs/reference?language=zh-CN#inline-links)，以及 [`https://example.org/reference`](https://example.org/reference)。\n\n\
             多个文件：[`registry.rs`](D:/projects/zeron/crates/engine/src/registry.rs:830)、[render.rs](D:/projects/zeron/crates/ui/src/markdown/render.rs:1600)。\n\n\
             [Cargo.toml](D:/projects/zeron/crates/ui/Cargo.toml)\n\n\
             长目标：[查看完整路径](D:/projects/zeron/crates/ui/src/markdown/a-very-long-directory-name/another-long-directory-name/a-long-file-name.rs:120)，[查看完整网址](https://example.com/a-very-long-directory-name/another-long-directory-name/resource?language=zh-CN&view=full#reference)。\n\n\
             普通代码 `cargo test -p rcode-ui` 保留原有样式。",
        );
        div()
            .size_full()
            .bg(theme.bg)
            .text_color(theme.text)
            .px(px(36.))
            .pt(px(120.))
            .child(markdown::render::selection_frame_reset())
            .child(markdown::render::render_tree(
                &tree,
                &opts,
                &theme,
                window,
                &|_| None,
            ))
    }
}
fn main() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let dark = std::env::args().any(|arg| arg == "--dark");
    gpui_platform::application()
        .with_assets(rcode_ui::icons::Assets)
        .run(move |cx| {
            gpui_base::init(cx);
            rcode_ui::typography::register_fonts(cx);
            cx.set_global(if dark { Theme::dark() } else { Theme::light() });
            settings::init(settings::UiSettings::default(), temp.path(), cx);
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
                        size(px(920.), px(580.)),
                        cx,
                    ))),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("RCode 链接样式验证".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| Fixture { _temp: temp }),
            )
            .expect("open link fixture");
            cx.activate(true);
        });
    Ok(())
}
