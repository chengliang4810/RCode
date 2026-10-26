//! This module is compiled only for the isolated QA example. All text here is
//! test material; no engine, account, message send or user-setting load exists.
use std::{any::Any, collections::HashMap, time::Instant};

use gpui::{
    AppContext, Context, Entity, Focusable, FontWeight, IntoElement, MouseButton, Render,
    ScrollHandle, Subscription, Window, div, prelude::*, px,
};

use crate::{
    composer::{ComposerInput, ComposerInputEvent},
    markdown::{self, render::RenderOptions},
    motion::{self, Presence, PropertySpring, SpringSpec},
    pull_request_skeleton::{self, SkeletonMode},
    settings::widgets::{self, SelectOption, SelectState},
    theme::Theme,
    typography::{self, UiTextRole},
};

const SAMPLE: &str = "中文与 English 2026 混排：API、Rust、100%；全角（测试）【示例】，半角 (test) [42]。Emoji 🙂 👩‍💻 1️⃣";
const BODY: &str = "中文与 English 2026 混排：API、Rust、100%；全角（测试）【示例】，半角 (test) [42]。Emoji 🙂 👩‍💻 1️⃣\n\n**粗体中文与 Bold English 123**，普通文字与 `let 中文 = 42;` 行内代码。\n\n- 中文标点：，。！？；：“”‘’（）【】\n- ASCII punctuation: , . ! ? ; : \" ' ( ) [ ] { }\n\n```rust\n// 中文注释：字符宽度与标点\nlet count = 42;\nprintln!(\"中文 English {} 🙂\", count);\n```";

#[derive(Clone, Copy)]
pub struct FixtureOptions {
    pub dark: bool,
    pub font_size: f32,
    pub scale: f32,
    pub reduced_motion: bool,
    pub business: bool,
}

impl Default for FixtureOptions {
    fn default() -> Self {
        Self {
            dark: false,
            font_size: 16.0,
            scale: 1.0,
            reduced_motion: false,
            business: false,
        }
    }
}

struct Chip {
    id: u64,
    presence: Presence,
    retiring: bool,
}

pub struct VisualFeedbackFixture {
    business: Option<Entity<BusinessFeedbackFixture>>,
    options: FixtureOptions,
    input: Entity<ComposerInput>,
    _input_events: Subscription,
    menu: SelectState,
    selected: usize,
    nav_selected: usize,
    short_list: bool,
    scroll: ScrollHandle,
    chips: Vec<Chip>,
    next_chip: u64,
    press: PropertySpring,
    _lifetime: Box<dyn Any>,
}

impl VisualFeedbackFixture {
    pub fn new(
        options: FixtureOptions,
        lifetime: Box<dyn Any>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let now = Instant::now();
        let input = cx.new(|cx| {
            let mut input = ComposerInput::new("输入中文、English 或 Emoji", cx)
                .with_viewport_height(110.0)
                .with_tab_stop();
            input.enable_mentions();
            input.set_attachment_chips("visual-feedback-fixture", HashMap::from([(1, None)]), cx);
            input.set_text("可编辑输入：中文 English 2026，光标、选择与 IME。", cx);
            input
        });
        let input_events = cx.subscribe(&input, |_, _, event, cx| {
            // Submit intentionally has no subscriber: this is a text editor,
            // not a connection to a model or a simulated successful send.
            if matches!(event, ComposerInputEvent::Edited) {
                cx.notify();
            }
        });
        Self {
            business: options
                .business
                .then(|| cx.new(|cx| BusinessFeedbackFixture::new(options, window, cx))),
            options,
            input,
            _input_events: input_events,
            menu: SelectState::default(),
            selected: 1,
            nav_selected: 1,
            short_list: false,
            scroll: ScrollHandle::new(),
            chips: (0..3)
                .map(|id| Chip {
                    id,
                    presence: Presence::present(now),
                    retiring: false,
                })
                .collect(),
            next_chip: 3,
            press: PropertySpring::new(
                1.0,
                SpringSpec::new(600.0, 30.0, 1.0).with_thresholds(0.0001, 0.002),
                now,
            ),
            _lifetime: lifetime,
        }
    }

    fn add_chip(&mut self, cx: &mut Context<Self>) {
        let index = self.chips.iter().filter(|chip| !chip.retiring).count();
        self.chips.push(Chip {
            id: self.next_chip,
            presence: Presence::enter(index, Instant::now(), motion::reduced_motion(cx)),
            retiring: false,
        });
        self.next_chip += 1;
        cx.notify();
    }

    fn retire_chip(&mut self, cx: &mut Context<Self>) {
        if let Some(chip) = self.chips.iter_mut().rev().find(|chip| !chip.retiring) {
            chip.retiring = true;
            chip.presence
                .exit(Instant::now(), motion::reduced_motion(cx));
            cx.notify();
        }
    }

    fn press(&mut self, pressed: bool, cx: &mut Context<Self>) {
        self.press.retarget(
            if pressed { 0.975 } else { 1.0 },
            motion::reduced_motion(cx),
            Instant::now(),
        );
        cx.notify();
    }

    fn button(
        &self,
        id: &'static str,
        text: &'static str,
        theme: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        widgets::action_button(theme, widgets::ActionTone::Outlined)
            .id(id)
            .tab_index(0)
            .role(gpui::Role::Button)
            .child(text)
    }
}

impl Render for VisualFeedbackFixture {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(business) = &self.business {
            return business.clone().into_any_element();
        }
        let theme = Theme::of(cx).clone();
        let now = Instant::now();
        let reduced = motion::reduced_motion(cx);
        self.chips
            .retain(|chip| !chip.presence.sample(now, reduced).finished);
        let spring = self.press.sample(now, reduced);
        let chip_active = self
            .chips
            .iter()
            .any(|chip| chip.presence.sample(now, reduced).active);
        // Each bookkeeping hook must run exactly once per root frame, even
        // while a presence/spring is also active.
        let hover_active = motion::hover_fades_active();
        let morph_active = motion::state_morphs_active();
        if spring.active || chip_active || hover_active || morph_active {
            window.request_animation_frame();
        }
        let mut options = RenderOptions::settled("visual-feedback-body".into());
        options.workspace_root = None;
        let body = markdown::render::render_tree(
            &markdown::parse_full(BODY),
            &options,
            &theme,
            window,
            &|_| None,
        );
        let select = widgets::select("fixture-menu", "公共菜单", &theme, |view: &mut Self| {
            &mut view.menu
        })
        .options(
            [
                SelectOption::new("较低 Low").detail("快速响应"),
                SelectOption::new("默认 Default").detail("当前选择"),
                SelectOption::new("较高 High").detail("更多计算"),
            ],
            self.selected,
        )
        .width(238.0)
        .open_rightward()
        .on_select(|view, index, _, cx| {
            view.selected = index;
            cx.notify();
        })
        .render(&self.menu, cx);
        let nav_rows = (0..if self.short_list { 4 } else { 32 })
            .map(|index| {
                let selected = self.nav_selected == index;
                div()
                    .id(("fixture-nav", index))
                    .tab_index(0)
                    .role(gpui::Role::Button)
                    .aria_selected(selected)
                    .px(px(10.0))
                    .py(px(7.0))
                    .rounded(px(6.0))
                    .text_size(UiTextRole::Primary.size())
                    .line_height(UiTextRole::Primary.line_height())
                    .text_color(theme.text)
                    .bg(if selected {
                        theme.wash(0.10)
                    } else {
                        gpui::transparent_black()
                    })
                    .hover(|style| style.bg(theme.wash(0.06)))
                    .focus_visible(|style| style.border_1().border_color(theme.accent))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.nav_selected = index;
                        cx.notify();
                    }))
                    .child(format!("项目审查 {index:02} · Rust API"))
            })
            .collect::<Vec<_>>();
        let navigation = div()
            .w(px(244.0))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .bg(theme.surface)
            .p(px(12.0))
            .child(label("导航主文字", &theme))
            .child(
                self.button(
                    "fixture-short-list",
                    if self.short_list {
                        "长列表"
                    } else {
                        "短列表"
                    },
                    &theme,
                )
                .on_click(cx.listener(|view, _, _, cx| {
                    view.short_list = !view.short_list;
                    cx.notify();
                })),
            )
            .child(
                crate::edge_fade::edge_faded(
                    24.0,
                    true,
                    true,
                    div()
                        .id("fixture-nav-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .flex()
                        .flex_col()
                        .children(nav_rows),
                )
                .fade_overflow_y(&self.scroll),
            );

        let chip_views = self
            .chips
            .iter()
            .map(|chip| {
                let sample = chip.presence.sample(now, reduced);
                div()
                    .id(("fixture-presence", chip.id))
                    .relative()
                    .top(px(sample.offset_y))
                    .opacity(sample.opacity)
                    .h(px(28.0))
                    .px(px(8.0))
                    .rounded(px(6.0))
                    .bg(theme.wash(0.055))
                    .border_1()
                    .border_color(theme.border)
                    .text_size(UiTextRole::Secondary.size())
                    .flex()
                    .items_center()
                    .child(format!("附件 {}.rs", chip.id + 1))
            })
            .collect::<Vec<_>>();
        let fill = theme.accent;
        let foreground = theme.bg;
        let scale = spring.value;
        let spring_button = div()
            .id("fixture-spring")
            .size(px(48.0))
            .tab_index(0)
            .role(gpui::Role::Button)
            .aria_label("弹簧按压样例")
            .rounded_full()
            .cursor_pointer()
            .focus_visible(|style| style.border_1().border_color(fill))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.press(true, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.press(false, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.press(false, cx)),
            )
            .on_click(cx.listener(|view, event: &gpui::ClickEvent, _, cx| {
                if event.is_keyboard() {
                    view.press.snap(0.975, Instant::now());
                    view.press(false, cx);
                }
            }))
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let diameter = px(42.0 * scale);
                        let center = bounds.center();
                        let circle = gpui::Bounds {
                            origin: center - gpui::point(diameter / 2.0, diameter / 2.0),
                            size: gpui::size(diameter, diameter),
                        };
                        window.paint_quad(gpui::fill(circle, fill).corner_radii(diameter / 2.0));
                        // The two pause strokes share the same scale and center as the
                        // background; this is a real graphic transform, no font resize.
                        for offset in [-4.0, 4.0] {
                            let rect = gpui::Bounds {
                                origin: center
                                    + gpui::point(px((offset - 1.5) * scale), px(-6.0 * scale)),
                                size: gpui::size(px(3.0 * scale), px(12.0 * scale)),
                            };
                            window.paint_quad(gpui::fill(rect, foreground).corner_radii(px(scale)));
                        }
                    },
                )
                .size_full(),
            );
        let controls = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                self.button("fixture-add", "添加附件", &theme)
                    .on_click(cx.listener(|view, _, _, cx| view.add_chip(cx))),
            )
            .child(
                self.button("fixture-remove", "移除附件", &theme)
                    .on_click(cx.listener(|view, _, _, cx| view.retire_chip(cx))),
            )
            .child(
                self.button("fixture-reference", "插入引用", &theme)
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.input.update(cx, |input, cx| {
                            input.insert_reference("[Image 1](rcode-image:1)", cx);
                        });
                        window.focus(&view.input.focus_handle(cx), cx);
                    })),
            )
            .child(spring_button);
        let composer = div()
            .p(px(14.0))
            .rounded(px(14.0))
            .border_1()
            .border_color(theme.border)
            .bg(theme.input_bg)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(div().flex().flex_wrap().gap(px(6.0)).children(chip_views))
            .child(self.input.clone())
            .child(controls);
        let reading = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .p(px(24.0))
            .child(label("正文 · 真实 Markdown", &theme))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .id("fixture-body-scroll")
                    .overflow_y_scroll()
                    .child(body),
            )
            .child(composer);
        let mut loading = div().flex().flex_col().gap(px(8.0));
        for index in 0..3 {
            loading = loading.child(pull_request_skeleton::bar(
                gpui::relative(0.9 - index as f32 * 0.16),
                10.0,
                index,
                cx.entity_id(),
                &theme,
                cx,
            ));
        }
        let mut wave = div().flex().flex_col().gap(px(8.0));
        for index in 0..3 {
            wave = wave.child(pull_request_skeleton::bar_with_mode(
                gpui::relative(0.95 - index as f32 * 0.18),
                10.0,
                index,
                SkeletonMode::Wave,
                cx.entity_id(),
                &theme,
                cx,
            ));
        }
        let details = div()
            .w(px(292.0))
            .flex_none()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(18.0))
            .child(label("详情主文字", &theme))
            .child(
                div()
                    .text_size(UiTextRole::Primary.size())
                    .line_height(UiTextRole::Primary.line_height())
                    .child(SAMPLE),
            )
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(UiTextRole::Primary.size())
                    .child("粗体中文 Bold 123"),
            )
            .child(label("公共菜单 · Tab / ↑↓ / Enter", &theme))
            .child(select)
            .child(label("密集占位 · 同步呼吸", &theme))
            .child(loading)
            .child(label("文字占位 · 轻量扫光", &theme))
            .child(wave)
            .child(
                div()
                    .text_size(UiTextRole::Caption.size())
                    .line_height(UiTextRole::Caption.line_height())
                    .text_color(theme.text_muted)
                    .child("滚动左栏检查边缘羽化；输入区可检查中文 IME、光标与选择。"),
            );
        div().id("visual-feedback-fixture").size_full().bg(theme.bg).text_color(theme.text)
            .font(typography::font(theme.font_sans.clone())).flex().flex_col()
            .child(div().h(px(44.0)).flex_none().px(px(18.0)).flex().items_center().border_b_1().border_color(theme.border)
                .text_size(UiTextRole::Secondary.size()).child(format!("隔离视觉验证 · {} · 字号 {}px · 应用缩放 {:.0}% · 窗口 DPI scale_factor={:.2}（{:.0}%） · 减少动效 {}",
                    if self.options.dark { "深色" } else { "浅色" }, self.options.font_size, self.options.scale * 100.0,
                    window.scale_factor(), window.scale_factor() * 100.0, if reduced { "开" } else { "关" })))
            .child(markdown::render::selection_frame_reset())
            .child(div().flex_1().min_h_0().flex().child(navigation).child(reading).child(details)).into_any_element()
    }
}

fn label(text: &'static str, theme: &Theme) -> gpui::Div {
    div()
        .text_size(UiTextRole::Section.size())
        .line_height(UiTextRole::Section.line_height())
        .text_color(theme.text_muted)
        .child(text)
}

/// Real business views driven by explicit, local QA states. There is no
/// EngineHandle, transport, account load, message dispatch or shell runner.
struct BusinessFeedbackFixture {
    options: FixtureOptions,
    composer: Entity<crate::composer::Composer>,
    queue: Entity<crate::composer::Composer>,
    files: Entity<crate::files::FilesSurface>,
    composer_width: f32,
    queue_phase: u8,
    sequence: Option<gpui::Task<()>>,
    _observations: Vec<Subscription>,
    _workspace: tempfile::TempDir,
}

fn feedback_state(workspace: &std::path::Path) -> crate::state::AppState {
    let mut state = crate::state::AppState::new();
    state.local_device_id = Some("fixture-device".into());
    state.selected_device = Some("fixture-device".into());
    state.selected_chat = Some("fixture-chat".into());
    state.devices.push(
        serde_json::from_value(serde_json::json!({
            "id": "fixture-device", "name": "隔离测试", "platform": "windows", "capabilities": []
        }))
        .expect("fixture device"),
    );
    state.chats.push(
        serde_json::from_value(serde_json::json!({
            "id": "fixture-chat", "deviceId": "fixture-device", "archived": false,
            "cwd": workspace.to_string_lossy(), "createdAt": "2026-01-01T00:00:00Z"
        }))
        .expect("fixture chat"),
    );
    state
}

impl BusinessFeedbackFixture {
    fn new(options: FixtureOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workspace = tempfile::tempdir().expect("isolated fixture workspace");
        let state = cx.new(|_| feedback_state(workspace.path()));
        let composer =
            cx.new(|cx| crate::composer::Composer::new_isolated_fixture(state.clone(), cx));
        composer.update(cx, |composer, cx| {
            composer.fixture_reset_feedback_attachments(cx);
            composer.focus_pending = false;
        });
        let queue_state = cx.new(|_| {
            let mut state = feedback_state(workspace.path());
            state.queue = vec![
                rcode_doc::QueuedMessage::new(
                    "fixture-edit",
                    "中文 English：编辑请求等待确认",
                    "fixture-device",
                ),
                rcode_doc::QueuedMessage::new(
                    "fixture-remove",
                    "删除请求等待确认 · 123",
                    "fixture-device",
                ),
                rcode_doc::QueuedMessage::new(
                    "fixture-held",
                    "需要重新确认的队列内容",
                    "fixture-device",
                ),
            ];
            state.queue[2].delivery_gate = Some(rcode_doc::QueueDeliveryGate::ReviewRequired {
                previous_lease_id: "fixture-expired".into(),
                owner_device_id: "fixture-device".into(),
                since_ms: 0,
                base_text_hash: "fixture-base-hash".into(),
            });
            state
        });
        let queue = cx.new(|cx| crate::composer::Composer::new_isolated_fixture(queue_state, cx));
        queue.update(cx, |composer, cx| {
            composer.focus_pending = false;
            composer.fixture_feedback_queue_state(1, cx);
        });
        let files = cx.new(|cx| {
            let mut files = crate::files::FilesSurface::new_editor(
                state,
                "fixture-chat".into(),
                "src/字体反馈.rs".into(),
                false,
                900,
                crate::typography::code_font_size(cx),
                false,
                false,
                cx,
            );
            files.fixture_seed_feedback_document(window, cx);
            files
        });
        let observations = vec![
            cx.observe(&composer, |_, _, cx| cx.notify()),
            cx.observe(&queue, |_, _, cx| cx.notify()),
            cx.observe(&files, |_, _, cx| cx.notify()),
        ];
        Self {
            options,
            composer,
            queue,
            files,
            composer_width: 474.0,
            queue_phase: 1,
            sequence: None,
            _observations: observations,
            _workspace: workspace,
        }
    }

    fn action(id: &'static str, text: &'static str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
        widgets::action_button(theme, widgets::ActionTone::Outlined)
            .id(id)
            .tab_index(0)
            .role(gpui::Role::Button)
            .child(text)
    }

    fn remove_pair(&mut self, cx: &mut Context<Self>) {
        self.sequence = None;
        self.composer.update(cx, |composer, cx| {
            composer.fixture_remove_feedback_attachment('B', cx)
        });
        let composer = self.composer.downgrade();
        self.sequence = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(55))
                .await;
            let _ = composer.update(cx, |composer, cx| {
                composer.fixture_remove_feedback_attachment('C', cx)
            });
            let _ = this.update(cx, |this, cx| {
                this.sequence = None;
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for BusinessFeedbackFixture {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let hover_active = motion::hover_fades_active();
        let morph_active = motion::state_morphs_active();
        if hover_active || morph_active {
            window.request_animation_frame();
        }
        self.composer.update(cx, |composer, cx| {
            composer.set_available_width(self.composer_width, cx)
        });
        // The queue is displayed by its real panel renderer, with no synthetic
        // composer input or working transport beneath it.
        let queue_panel = self.queue.update(cx, |composer, cx| {
            composer.render_queue_panel(false, window, cx)
        });
        let live_count = self.composer.read(cx).fixture_feedback_attachment_count();
        let save_status = self.files.read(cx).fixture_feedback_save_status();
        let phase = match self.queue_phase {
            1 => "pending edit",
            2 => "pending remove",
            3 => "pending finish",
            _ => "idle",
        };
        let attachment_controls = div()
            .flex()
            .flex_wrap()
            .gap(px(6.0))
            .child(
                Self::action("business-reset", "重置附件 A–G", &theme).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.sequence = None;
                        this.composer.update(cx, |composer, cx| {
                            composer.fixture_reset_feedback_attachments(cx)
                        });
                    },
                )),
            )
            .child(
                Self::action("business-remove-pair", "连续删除 B、C", &theme)
                    .on_click(cx.listener(|this, _, _, cx| this.remove_pair(cx))),
            )
            .child(
                Self::action("business-width-one", "1 列压力测试", &theme).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.composer_width = 122.0;
                        cx.notify();
                    },
                )),
            )
            .child(
                Self::action("business-width-three", "3 列压力测试", &theme).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.composer_width = 250.0;
                        cx.notify();
                    },
                )),
            )
            .child(
                Self::action("business-width-normal", "6 列", &theme).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.composer_width = 474.0;
                        cx.notify();
                    },
                )),
            );
        let mut queue_controls = div().flex().flex_wrap().gap(px(6.0));
        for (phase, id, text) in [
            (0, "business-queue-idle", "队列空闲"),
            (1, "business-queue-edit", "待编辑确认"),
            (2, "business-queue-remove", "待移除确认"),
            (3, "business-queue-finish", "待保存确认"),
        ] {
            queue_controls = queue_controls.child(Self::action(id, text, &theme).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.queue_phase = phase;
                    this.queue.update(cx, |composer, cx| {
                        composer.fixture_feedback_queue_state(phase, cx)
                    });
                    cx.notify();
                }),
            ));
        }
        let left = div()
            .id("business-feedback-scroll")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .p(px(16.0))
            .child(label("真实 Composer · 附件退出", &theme))
            .child(attachment_controls)
            .child(
                div()
                    .text_size(UiTextRole::Caption.size())
                    .text_color(theme.text_muted)
                    .child(format!(
                        "实际 staged 集合：{live_count} · 外宽 {}px",
                        self.composer_width
                    )),
            )
            .child(
                div()
                    .id("business-composer-slot")
                    .w(px(self.composer_width))
                    .flex_none()
                    .child(self.composer.clone()),
            )
            .child(label("真实 Queue · 明确注入测试状态", &theme))
            .child(queue_controls)
            .child(
                div()
                    .text_size(UiTextRole::Caption.size())
                    .text_color(theme.text_muted)
                    .child(format!("状态：{phase} · 无 ACK / RPC")),
            )
            .children(queue_panel);
        let right = div()
            .w(px(520.0))
            .min_w_0()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .p(px(16.0))
            .border_l_1()
            .border_color(theme.border)
            .child(label("真实 FilesSurface · 保存生命周期", &theme))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.0))
                    .child(
                        Self::action("business-save-pending", "注入保存中", &theme).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.files.update(cx, |files, cx| {
                                    files.fixture_begin_feedback_save(cx);
                                });
                            }),
                        ),
                    )
                    .child(
                        Self::action("business-save-failed", "注入保存失败", &theme).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.files.update(cx, |files, cx| {
                                    files.fixture_fail_feedback_save(cx);
                                });
                            }),
                        ),
                    ),
            )
            .child(
                div()
                    .text_size(UiTextRole::Caption.size())
                    .text_color(theme.text_muted)
                    .child(save_status),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(8.0))
                    .overflow_hidden()
                    .child(self.files.clone()),
            );
        div().id("business-feedback-fixture").size_full().bg(theme.bg).text_color(theme.text)
            .font(typography::font(theme.font_sans.clone())).flex().flex_col()
            .child(div().h(px(44.0)).flex_none().px(px(16.0)).flex().items_center().border_b_1().border_color(theme.border)
                .text_size(UiTextRole::Secondary.size()).child(format!("业务反馈隔离验证 · 测试数据 · 无 engine / 账户 / 消息发送 · 字号 {}px · 应用 {:.0}% · DPI {:.0}% · 减少动效 {}",
                    self.options.font_size, self.options.scale * 100.0, window.scale_factor() * 100.0,
                    if motion::reduced_motion(cx) { "开" } else { "关" })))
            .child(div().flex_1().min_h_0().flex().child(left).child(right))
    }
}
