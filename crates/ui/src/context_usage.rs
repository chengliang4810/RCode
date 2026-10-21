//! Context occupancy is read from the replicated chat snapshot, never local CLI state.
use crate::{
    theme::Theme,
    usage_stats::placement::{self, PanelBounds},
};
use gpui::{
    Context, Entity, IntoElement, PathBuilder, Render, SharedString, Subscription, Task, Window,
    canvas, div, point, prelude::*, px,
};
use rcode_proto::ContextUsage;
use std::{cell::Cell, rc::Rc, time::Duration};

/// Context occupancy with an anchored, hover-only details panel.
pub struct Indicator {
    chat: Option<String>,
    usage: Option<ContextUsage>,
    open: bool,
    hovered: bool,
    show_task: Option<Task<()>>,
    hide_task: Option<Task<()>>,
    bounds: PanelBounds,
    panel_bounds: PanelBounds,
    statistics_bounds: Option<PanelBounds>,
    _statistics: Option<Subscription>,
}

impl Indicator {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            chat: None,
            usage: None,
            open: false,
            hovered: false,
            show_task: None,
            hide_task: None,
            bounds: Rc::new(Cell::new(None)),
            panel_bounds: Rc::new(Cell::new(None)),
            statistics_bounds: None,
            _statistics: None,
        }
    }

    pub(crate) fn for_footer(
        statistics: Entity<crate::usage_stats::FooterStatistics>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut indicator = Self::new(cx);
        indicator.statistics_bounds = Some(statistics.read(cx).icon_bounds());
        indicator._statistics = Some(cx.observe(&statistics, |_, _, cx| cx.notify()));
        indicator
    }

    pub fn set_usage(
        &mut self,
        chat: Option<String>,
        usage: Option<ContextUsage>,
        cx: &mut Context<Self>,
    ) {
        if self.chat != chat || self.usage != usage {
            if self.chat != chat {
                self.open = false;
                self.hovered = false;
                self.show_task = None;
                self.hide_task = None;
                self.panel_bounds.set(None);
            }
            self.chat = chat;
            self.usage = usage;
            cx.notify();
        }
    }

    fn hover_trigger(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.hovered = hovered;
        if hovered {
            self.hide_task = None;
            if !self.open && self.show_task.is_none() {
                self.show_task = Some(cx.spawn(async |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(500))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.show_task = None;
                        if this.hovered {
                            this.open = true;
                            cx.notify();
                        }
                    });
                }));
            }
        } else {
            self.show_task = None;
        }
    }

    fn defer_hide(&mut self, cx: &mut Context<Self>) {
        if !self.open || self.hide_task.is_some() {
            return;
        }
        self.hide_task = Some(cx.spawn(async |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.hide_task = None;
                this.open = false;
                this.panel_bounds.set(None);
                cx.notify();
            });
        }));
    }
}

impl Render for Indicator {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let fraction = self.usage.and_then(ContextUsage::fraction);
        let color = match fraction {
            Some(f) if f >= 0.9 => theme.danger,
            Some(f) if f >= 0.75 => theme.warning,
            Some(_) => theme.text_muted,
            None => theme.text_faint,
        };
        let popup = self.open.then(|| {
            let width = self
                .statistics_bounds
                .as_ref()
                .and_then(|bounds| bounds.get())
                .zip(self.bounds.get())
                .map(|(statistics, trigger)| (trigger.right() - statistics.left()).max(px(1.0)))
                .unwrap_or(px(380.0));
            let content = card(self.usage, &theme.for_popup())
                .w(width)
                .id("context-usage-popup")
                .debug_selector(|| "context-usage-popup".into())
                .max_w((window.viewport_size().width - px(16.0)).max(px(1.0)))
                .max_h(px(((f32::from(window.viewport_size().height)
                    - 24.0
                    - 32.0)
                    / 2.0)
                    .max(1.0)))
                .overflow_y_scroll()
                .occlude();
            let content = crate::frost::frosted(
                crate::popover::CARD_RADIUS,
                crate::frost::MENU_BLUR,
                content,
            );
            div().absolute().inset_0().size_0().child(
                gpui::deferred(placement::Panel::new(
                    content.into_any_element(),
                    self.bounds.clone(),
                    self.panel_bounds.clone(),
                    placement::Align::End,
                ))
                .priority(1),
            )
        });
        let bounds = self.bounds.clone();
        let open = self.open;
        let owner = cx.weak_entity();
        div()
            .id("composer-context-usage")
            .debug_selector(|| "composer-context-usage".into())
            .relative()
            .size(px(24.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .aria_label(format!(
                "{}: {}",
                crate::l10n::localizer()
                    .text(rcode_presentation::TextKey::ConversationContextWindow),
                details(self.usage)
            ))
            .hover(|el| el.bg(theme.element_hover))
            .on_hover(cx.listener(|this, hovered, _, cx| this.hover_trigger(*hovered, cx)))
            .child(ring(fill_fraction(self.usage), color, &theme))
            .child(
                canvas(
                    move |rect, _, _| bounds.set(Some(rect)),
                    move |_, _, window, _| {
                        if open {
                            let owner = owner.clone();
                            window.on_mouse_event(
                                move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                                    if phase == gpui::DispatchPhase::Capture {
                                        let _ = owner.update(cx, |this, cx| {
                                            let inside =
                                                [this.bounds.get(), this.panel_bounds.get()]
                                                    .into_iter()
                                                    .flatten()
                                                    .any(|bounds| bounds.contains(&event.position));
                                            if inside {
                                                this.hide_task = None;
                                            } else {
                                                this.defer_hide(cx);
                                            }
                                        });
                                    }
                                },
                            );
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .children(popup)
    }
}

/// The footer's 16px progress ring: a faint full track under a `fraction` arc
/// starting at twelve o'clock.
pub(crate) fn ring(fraction: f32, color: gpui::Hsla, theme: &Theme) -> impl IntoElement {
    let track = theme.text_faint.opacity(0.25);
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let center = bounds.center();
            let mut arc = |fraction: f32, color| {
                if fraction <= 0.0 {
                    return;
                }
                let steps = (64.0 * fraction).ceil().max(2.0) as usize;
                let mut path = PathBuilder::stroke(px(1.8));
                for i in 0..=steps {
                    let angle = -std::f32::consts::FRAC_PI_2
                        + std::f32::consts::TAU * fraction * i as f32 / steps as f32;
                    let p = point(
                        center.x + px(6.0 * angle.cos()),
                        center.y + px(6.0 * angle.sin()),
                    );
                    if i == 0 {
                        path.move_to(p);
                    } else {
                        path.line_to(p);
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            };
            arc(1.0, track);
            arc(fraction.clamp(0.0, 1.0), color);
        },
    )
    .size(px(16.0))
}

fn fill_fraction(usage: Option<ContextUsage>) -> f32 {
    usage
        .and_then(ContextUsage::fraction)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0) as f32
}

fn compact_count(count: u64, locale: rcode_presentation::Locale) -> String {
    use rcode_presentation::Locale;
    let (divisor, suffix) = match (locale, count) {
        (Locale::ZhCn, 10_000..) => (10_000.0, "万"),
        (_, 1_000_000..) => (1_000_000.0, "M"),
        (Locale::EnUs, 1_000..) => (1_000.0, "K"),
        _ => return count.to_string(),
    };
    let value = format!("{:.1}", count as f64 / divisor);
    format!("{}{suffix}", value.trim_end_matches(".0"))
}

fn details(usage: Option<ContextUsage>) -> String {
    details_for(usage, crate::l10n::localizer())
}

fn details_for(usage: Option<ContextUsage>, localizer: &rcode_presentation::Localizer) -> String {
    use rcode_presentation::TextKey;
    let compact = |value| compact_count(value, localizer.locale());
    let tokens =
        |key: TextKey, value: u64| localizer.format_with(key, &[("total", &compact(value))]);
    match usage.unwrap_or_default() {
        ContextUsage {
            tokens: Some(token_count),
            window: Some(window),
        } if window > 0 => localizer.format_with(
            TextKey::ConversationContextTokens,
            &[
                ("used", &compact(token_count)),
                ("total", &compact(window)),
                (
                    "percent",
                    &format!("{:.1}", token_count as f64 / window as f64 * 100.0),
                ),
            ],
        ),
        ContextUsage {
            tokens: Some(token_count),
            ..
        } => {
            let used = localizer.format_with(
                TextKey::ConversationContextUsed,
                &[("used", &compact(token_count))],
            );
            format!(
                "{used} · {}",
                localizer.text(TextKey::ConversationContextLimitUnknown)
            )
        }
        ContextUsage {
            window: Some(window),
            ..
        } if window > 0 => {
            let capacity = tokens(TextKey::ConversationContextCapacity, window);
            format!(
                "{capacity} · {}",
                localizer.text(TextKey::ConversationContextWaiting)
            )
        }
        _ => localizer.text(TextKey::ConversationContextNotReported),
    }
}

/// The context ring's popover content.
pub fn card(usage: Option<ContextUsage>, theme: &Theme) -> gpui::Div {
    let heading =
        crate::l10n::localizer().text(rcode_presentation::TextKey::ConversationContextWindow);
    crate::popover::popover_card(theme)
        .w(px(380.0))
        .p(px(12.0))
        .flex()
        .flex_col()
        .gap(px(14.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(14.0))
                        .child(SharedString::from(heading)),
                )
                .child(
                    div()
                        .min_w_0()
                        .text_right()
                        .text_size(px(12.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(details(usage))),
                ),
        )
        .child(
            div()
                .debug_selector(|| "context-usage-track".into())
                .w_full()
                .h(px(8.0))
                .rounded_full()
                .overflow_hidden()
                .bg(theme.text.opacity(0.06))
                .child(
                    div()
                        .debug_selector(|| "context-usage-fill".into())
                        .h_full()
                        .w(gpui::relative(fill_fraction(usage)))
                        .rounded_full()
                        .bg(theme.text),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
    use std::time::Duration;

    struct Fixture(Entity<Indicator>);

    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .items_end()
                .justify_end()
                .p(px(10.0))
                .child(self.0.clone())
        }
    }

    fn fixture(
        cx: &mut TestAppContext,
        usage: Option<ContextUsage>,
    ) -> (Entity<Indicator>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
        });
        let (view, cx) = cx.add_window_view(|_, cx| {
            Fixture(cx.new(|cx| {
                let mut indicator = Indicator::new(cx);
                indicator.set_usage(Some("first".into()), usage, cx);
                indicator
            }))
        });
        cx.simulate_resize(gpui::size(px(600.0), px(400.0)));
        cx.run_until_parked();
        let indicator = view.read_with(cx, |view, _| view.0.clone());
        (indicator, cx)
    }

    #[gpui::test]
    fn context_hover_opens_an_anchored_panel_and_click_does_not_pin_it(cx: &mut TestAppContext) {
        let (_indicator, cx) = fixture(
            cx,
            Some(ContextUsage {
                tokens: Some(50_000),
                window: Some(200_000),
            }),
        );
        let trigger = cx.debug_bounds("composer-context-usage").unwrap();
        cx.simulate_mouse_move(trigger.center(), None, Default::default());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        let popup = cx
            .debug_bounds("context-usage-popup")
            .expect("hover opens context details");
        assert!(popup.bottom() <= trigger.top() - px(8.0));
        assert!(!popup.intersects(&trigger));
        assert!(popup.left() >= px(8.0) && popup.right() <= px(592.0));
        let track = cx.debug_bounds("context-usage-track").unwrap();
        let fill = cx.debug_bounds("context-usage-fill").unwrap();
        assert!((f32::from(fill.size.width / track.size.width) - 0.25).abs() < 0.01);
        cx.simulate_mouse_move(popup.center(), None, Default::default());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        assert!(cx.debug_bounds("context-usage-popup").is_some());
        cx.simulate_click(trigger.center(), Default::default());
        cx.simulate_mouse_move(point(px(5.0), px(5.0)), None, Default::default());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        assert!(cx.debug_bounds("context-usage-popup").is_none());
    }

    #[gpui::test]
    fn context_hover_handles_unknown_usage_live_updates_and_navigation(cx: &mut TestAppContext) {
        let (indicator, cx) = fixture(cx, None);
        let trigger = cx.debug_bounds("composer-context-usage").unwrap();
        cx.simulate_mouse_move(trigger.center(), None, Default::default());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        assert!(cx.debug_bounds("context-usage-popup").is_some());
        indicator.update(cx, |control, cx| {
            control.set_usage(
                Some("first".into()),
                Some(ContextUsage {
                    tokens: Some(190_000),
                    window: Some(200_000),
                }),
                cx,
            );
            assert!(control.open);
            assert!(details(control.usage).contains("95.0%"));
        });
        cx.run_until_parked();
        let track = cx.debug_bounds("context-usage-track").unwrap();
        let fill = cx.debug_bounds("context-usage-fill").unwrap();
        assert!((f32::from(fill.size.width / track.size.width) - 0.95).abs() < 0.01);
        indicator.update(cx, |control, cx| {
            control.set_usage(Some("second".into()), None, cx)
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("context-usage-popup").is_none());
    }

    #[test]
    fn compact_capacity_matches_the_requested_chinese_panel() {
        use rcode_presentation::{LanguagePreference, Locale, Localizer};
        let zh = Localizer::with_system_locale(LanguagePreference::ZhCn, None);
        assert_eq!(compact_count(134_000, Locale::ZhCn), "13.4万");
        assert_eq!(compact_count(1_000_000, Locale::ZhCn), "100万");
        assert_eq!(compact_count(999, Locale::ZhCn), "999");
        assert_eq!(compact_count(134_000, Locale::EnUs), "134K");
        assert_eq!(compact_count(1_000_000, Locale::EnUs), "1M");
        assert_eq!(
            details_for(
                Some(ContextUsage {
                    tokens: Some(134_000),
                    window: Some(1_000_000),
                }),
                &zh
            ),
            "13.4万/100万 (13.4%)"
        );
    }

    #[test]
    fn unavailable_zero_and_over_capacity_are_distinct() {
        use rcode_presentation::{LanguagePreference, Localizer, TextKey};
        let zh = Localizer::with_system_locale(LanguagePreference::ZhCn, None);
        let empty = details_for(None, &zh);
        assert_eq!(empty, zh.text(TextKey::ConversationContextNotReported));
        assert!(!empty.contains("0%"));
        assert_eq!(
            details_for(
                Some(ContextUsage {
                    tokens: Some(0),
                    window: Some(200)
                }),
                &zh
            ),
            "0/200 (0.0%)"
        );
        let overflow = Some(ContextUsage {
            tokens: Some(250),
            window: Some(200),
        });
        assert_eq!(details_for(overflow, &zh), "250/200 (125.0%)");
        assert_eq!(fill_fraction(overflow), 1.0);
        assert_eq!(fill_fraction(None), 0.0);
        assert!(
            (fill_fraction(Some(ContextUsage {
                tokens: Some(134_000),
                window: Some(1_000_000)
            })) - 0.134)
                .abs()
                < 0.0001
        );
        assert!(
            details_for(
                Some(ContextUsage {
                    tokens: Some(10),
                    window: Some(0)
                }),
                &zh
            )
            .contains(&zh.text(TextKey::ConversationContextLimitUnknown))
        );
        assert!(
            details_for(
                Some(ContextUsage {
                    tokens: None,
                    window: Some(200_000)
                }),
                &zh
            )
            .contains(&zh.text(TextKey::ConversationContextWaiting))
        );
    }
}
