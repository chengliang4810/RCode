//! Local comparison through GPUI's native text renderer, without user data.
use gpui::{
    AppContext, Bounds, Context, Entity, Font, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, StyledText,
    Subscription, TextRun, TitlebarOptions, Window, WindowBounds, WindowOptions, div, font, point,
    px, rgb, size,
};
use rcode_ui::{
    composer::{self, ComposerInput, ComposerInputEvent},
    settings::{ComposerSendBehavior, UiSettings},
    theme::Theme,
};
use std::{borrow::Cow, path::PathBuf};

const SAMPLE: &str = "项目代码审查与问题排查。中文字体应当清楚、匀称，长时间阅读也不费力。这里同时检查汉字笔画、标点位置，以及中英文混排：RCode、Windows、API、2026。";

#[derive(Clone)]
struct Candidate {
    label: String,
    regular: SharedString,
    semibold: SharedString,
}
impl Candidate {
    fn font(&self, weight: FontWeight) -> Font {
        let mut selected = font(if weight == FontWeight::NORMAL {
            self.regular.clone()
        } else {
            self.semibold.clone()
        });
        selected.weight = weight;
        selected
    }
}
struct FontProbe {
    candidates: Vec<Candidate>,
    size: f32,
    input: Entity<ComposerInput>,
    _edited: Subscription,
}
impl FontProbe {
    fn new(candidates: Vec<Candidate>, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut input = ComposerInput::new("输入对比文本", cx).with_single_line();
            input.set_text(SAMPLE, cx);
            input
        });
        let edited = cx.subscribe(&input, |_, _, event, cx| {
            if matches!(event, ComposerInputEvent::Edited) {
                cx.notify();
            }
        });
        Self {
            candidates,
            size: 14.0,
            input,
            _edited: edited,
        }
    }
}
fn text_sample(
    text: impl Into<SharedString>,
    candidate: &Candidate,
    size: f32,
    weight: FontWeight,
) -> gpui::Div {
    div()
        .font(candidate.font(weight))
        .text_size(px(size))
        .line_height(px(size * 1.75))
        .child(text.into())
}
fn section_label(label: &'static str) -> gpui::Div {
    div()
        .font_family("Segoe UI")
        .text_size(px(12.0))
        .text_color(rgb(0x777777))
        .child(label)
}
fn code_sample(candidate: &Candidate, size: f32) -> gpui::Div {
    let text: SharedString = "// 中文注释：加载项目配置\nlet name = \"中文字体\";".into();
    let mut runs: Vec<TextRun> = Vec::new();
    let mut previous_ascii = None;
    for ch in text.chars() {
        let ascii = ch.is_ascii();
        if previous_ascii == Some(ascii) {
            runs.last_mut().unwrap().len += ch.len_utf8();
        } else {
            runs.push(TextRun {
                len: ch.len_utf8(),
                font: if ascii {
                    font("Geist Mono")
                } else {
                    candidate.font(FontWeight::NORMAL)
                },
                color: rgb(0x252525).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            });
            previous_ascii = Some(ascii);
        }
    }
    div()
        .p(px(10.0))
        .rounded(px(6.0))
        .bg(rgb(0xf4f4f4))
        .text_size(px(size))
        .line_height(px(size * 1.75))
        .child(StyledText::new(text).with_runs(runs))
}
impl Render for FontProbe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sample: SharedString = self.input.read(cx).text().to_owned().into();
        let mut controls = div().flex().items_center().gap(px(8.0)).child("字号");
        for size in [13, 14, 15, 16, 18] {
            controls = controls.child(
                div()
                    .id(format!("font-size-{size}"))
                    .px(px(12.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .bg(if self.size == size as f32 {
                        rgb(0xe7e7eb)
                    } else {
                        rgb(0xf5f5f5)
                    })
                    .child(format!("{size} px"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.size = size as f32;
                        cx.notify();
                    })),
            );
        }
        div().size_full().flex().flex_col().gap(px(16.0)).p(px(20.0))
            .bg(rgb(0xffffff)).text_color(rgb(0x252525)).font_family("Segoe UI").text_size(px(14.0))
            .child(controls)
            .child(div().flex().flex_col().gap(px(6.0)).child("对比文本")
                .child(div().border_1().border_color(rgb(0xdddddd)).rounded(px(6.0))
                    .px(px(10.0)).py(px(8.0)).child(self.input.clone())))
            .child(div().flex().flex_1().min_h_0().gap(px(16.0))
                .children(self.candidates.iter().enumerate().map(|(ix, candidate)| {
                    div().id(format!("font-column-{ix}")).flex_1().min_w_0().h_full().overflow_y_scroll()
                        .border_1().border_color(rgb(0xe4e4e4)).rounded(px(8.0)).p(px(18.0))
                        .flex().flex_col().gap(px(14.0))
                        .child(div().text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).child(candidate.label.clone()))
                        .child(text_sample("中文标题与阅读体验", candidate, self.size + 2.0, FontWeight::SEMIBOLD))
                        .child(section_label("正文 · 400"))
                        .child(text_sample(sample.clone(), candidate, self.size, FontWeight::NORMAL))
                        .child(section_label("强调 · 600"))
                        .child(text_sample(sample.clone(), candidate, self.size, FontWeight::SEMIBOLD))
                        .child(section_label("标点与中英文混排"))
                        .child(text_sample("“中文引号”、（括号）、《书名号》；ABC abc 0123456789。\n正在检查 src/main.rs 与 config.json，进度为 87.5%。", candidate, self.size, FontWeight::NORMAL))
                        .child(section_label("代码中的中文"))
                        .child(code_sample(candidate, self.size))
                })))
    }
}
fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../output/font-comparison");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("fonts.json"))?)?;
    let mut fonts = Vec::new();
    let mut candidates = Vec::new();
    let profile = std::env::var_os("RCODE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("USERPROFILE").expect("Windows profile")).join(".rcode")
        });
    let settings = UiSettings::load(&profile);
    let current = settings.ui_font_family.family_name().to_owned();
    candidates.push(Candidate {
        label: format!("当前 · {}", settings.ui_font_family.label()),
        regular: current.clone().into(),
        semibold: current.into(),
    });
    for label in ["Noto Sans SC", "Sarasa UI SC"] {
        let faces: Vec<_> = manifest
            .as_array()
            .unwrap()
            .iter()
            .filter(|face| face["label"] == label)
            .collect();
        let regular = faces
            .iter()
            .find(|face| face["weight"] == 400)
            .expect("400 face");
        let semibold = faces
            .iter()
            .find(|face| face["weight"] == 600)
            .expect("600 face");
        candidates.push(Candidate {
            label: label.into(),
            regular: regular["family"].as_str().unwrap().to_owned().into(),
            semibold: semibold["family"].as_str().unwrap().to_owned().into(),
        });
        for face in faces {
            fonts.push(Cow::Owned(std::fs::read(face["file"].as_str().unwrap())?));
        }
    }
    gpui_platform::application()
        .with_assets(rcode_ui::icons::Assets)
        .run(move |cx| {
            gpui_base::init(cx);
            composer::init(cx, ComposerSendBehavior::Enter);
            rcode_ui::typography::register_fonts(cx);
            cx.text_system()
                .add_fonts(fonts)
                .expect("register preview fonts");
            let mut theme = Theme::light();
            theme.font_sans = candidates[0].regular.clone();
            cx.set_global(theme);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(60.0), px(40.0)),
                        size(px(1500.0), px(880.0)),
                    ))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("RCode 中文字体对比".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    window.set_rem_size(px(16.0));
                    for candidate in &candidates {
                        for weight in [FontWeight::NORMAL, FontWeight::SEMIBOLD] {
                            let requested = candidate.font(weight);
                            let requested_id = cx.text_system().resolve_font(&requested);
                            let resolved = cx.text_system().get_font_for_id(requested_id).unwrap();
                            assert_eq!(
                                resolved.family, requested.family,
                                "preview font failed to load"
                            );
                            let text: SharedString = "中文字体对比 ABC 123".into();
                            let run = TextRun {
                                len: text.len(),
                                font: requested,
                                color: rgb(0x252525).into(),
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            let line =
                                window
                                    .text_system()
                                    .shape_line(text, px(14.0), &[run], None);
                            assert!(
                                line.runs
                                    .iter()
                                    .flat_map(|run| &run.glyphs)
                                    .all(|glyph| glyph.id.0 != 0)
                            );
                            assert!(
                                line.runs.iter().all(|run| run.font_id == requested_id),
                                "comparison text unexpectedly used a fallback font"
                            );
                            println!(
                                "Verified {} weight {} as {}",
                                candidate.label, weight.0, resolved.family
                            );
                        }
                    }
                    cx.new(|cx| FontProbe::new(candidates, cx))
                },
            )
            .expect("open font comparison");
            cx.activate(true);
        });
    Ok(())
}
