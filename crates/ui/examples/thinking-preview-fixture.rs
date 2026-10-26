//! Isolated native thought preview and scrolling QA. No engine or model requests.
use gpui::{
    AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription,
    Window, WindowBounds, WindowOptions, div, px, size,
};
use rcode_ui::{settings, state::AppState, theme::Theme, transcript::Transcript};
use std::time::Duration;

struct Fixture {
    transcript: Entity<Transcript>,
    text: String,
    _changed: Subscription,
    _temp: tempfile::TempDir,
}

impl Fixture {
    fn new(temp: tempfile::TempDir, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|_| AppState::new());
        let transcript = cx.new(|cx| Transcript::new(state, cx));
        let changed = cx.observe(&transcript, |_, _, cx| cx.notify());
        let text = (1..=100)
            .map(|line| format!("思考内容第 {line:03} 行：检查项目结构、调用关系和验证结果。"))
            .collect::<Vec<_>>()
            .join("\n\n")
            + "\n\n";
        cx.spawn(async move |this, cx| {
            for character in
                "实时预览：文字逐字更新，思考明细保持折叠，点击后可以在固定高度内滚动查看全部内容。"
                    .chars()
            {
                cx.background_executor()
                    .timer(Duration::from_millis(180))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        this.text.push(character);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            transcript,
            text,
            _changed: changed,
            _temp: temp,
        }
    }
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        div()
            .size_full()
            .bg(theme.bg)
            .text_color(theme.text)
            .p(px(24.0))
            .child(self.transcript.update(cx, |transcript, cx| {
                transcript.fixture_thought_preview(&self.text, true, cx)
            }))
    }
}

fn main() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    gpui_platform::application()
        .with_assets(rcode_ui::icons::Assets)
        .run(move |cx| {
            gpui_base::init(cx);
            rcode_ui::typography::register_fonts(cx);
            cx.set_global(Theme::light());
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
                        size(px(900.0), px(650.0)),
                        cx,
                    ))),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("RCode 思考预览验证".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_, cx| cx.new(|cx| Fixture::new(temp, cx)),
            )
            .expect("open thought fixture");
            cx.activate(true);
        });
    Ok(())
}
