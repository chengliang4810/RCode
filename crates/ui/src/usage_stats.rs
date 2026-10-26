//! Compact session and turn statistics. Unknown provider counters remain absent.

use gpui::{Context, Entity, IntoElement, Render, SharedString, Task, Window, div, prelude::*, px};
use rcode_doc::statistics::{ConversationStatistics, UsageStatistics};
use rcode_presentation::TextKey;

use crate::{popover, state::AppState, theme::Theme, typography::UiTextRole};

pub(crate) mod placement;
use placement::PanelBounds;
use std::{cell::Cell, rc::Rc};

const STAT_PADDING: f32 = 6.0;

/// The same remembered labels used by the model picker, refreshed by catalog loads.
#[derive(Default, Clone)]
struct ModelLabels(std::collections::HashMap<String, String>);
impl gpui::Global for ModelLabels {}

pub(crate) fn remember_model_labels<'a>(
    labels: impl Iterator<Item = (&'a str, &'a str)>,
    cx: &mut gpui::App,
) {
    let mut cache = cx.try_global::<ModelLabels>().cloned().unwrap_or_default();
    let mut changed = false;
    for (id, label) in labels {
        if !label.trim().is_empty() && cache.0.get(id).map(String::as_str) != Some(label) {
            cache.0.insert(id.to_owned(), label.to_owned());
            changed = true;
        }
    }
    if changed {
        cx.set_global(cache);
    }
}

impl ModelLabels {
    fn resolve(&self, ids: &[String]) -> String {
        ids.iter()
            .map(|id| self.0.get(id).unwrap_or(id).as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Clone, PartialEq)]
pub struct StatData {
    align: placement::Align,
    title: String,
    label: String,
    icon: &'static str,
    total: Option<String>,
    rows: Vec<(String, String)>,
    models: Vec<String>,
}

fn text(key: TextKey) -> String {
    crate::l10n::localizer().text(key)
}

fn compact(value: u64) -> String {
    rcode_presentation::statistics::tokens(crate::l10n::localizer(), value)
}

fn duration(ms: u64) -> String {
    rcode_presentation::statistics::duration(crate::l10n::localizer(), ms)
}

fn token_rate(value: f64) -> String {
    rcode_presentation::statistics::token_rate(crate::l10n::localizer(), value)
}

fn percent(value: f64) -> String {
    for precision in 0..=6 {
        let formatted = format!("{value:.precision$}");
        if value >= 100.0 || formatted.parse::<f64>().unwrap_or(100.0) < 100.0 {
            return formatted;
        }
    }
    format!("{value:.6}")
}

impl StatData {
    fn activity(stats: &UsageStatistics) -> Option<Self> {
        if stats.calls == 0 {
            return None;
        }
        let mut label = crate::l10n::localizer().format_with(
            if stats.partial_history {
                TextKey::ConversationStatsRecordedCounts
            } else {
                TextKey::ConversationStatsCounts
            },
            &[
                ("turns", &stats.turns.to_string()),
                ("steps", &stats.calls.to_string()),
            ],
        );
        let speed = stats.tokens_per_second().map(token_rate);
        if let Some(speed) = &speed {
            label.push_str(&format!(" · {speed}"));
        }
        let mut rows = vec![(
            text(TextKey::ConversationStatsModelTime),
            duration(stats.duration_ms),
        )];
        if stats.timed_calls > 0 {
            rows.push((
                text(TextKey::ConversationStatsTtft),
                duration(stats.first_token_ms / stats.timed_calls),
            ));
        }
        if let Some(speed) = speed {
            rows.push((text(TextKey::ConversationStatsSpeed), speed));
        }
        Some(Self {
            title: text(TextKey::ConversationStatsTitle),
            align: placement::Align::End,
            label,
            icon: crate::icons::USAGE_GAUGE,
            total: None,
            rows,
            models: Vec::new(),
        })
    }

    pub fn usage(stats: &UsageStatistics, turn: bool) -> Option<Self> {
        let total = stats.total_tokens()?;
        let title = text(if turn {
            TextKey::ConversationStatsTurnUsage
        } else if stats.partial_history {
            TextKey::ConversationStatsRecordedUsage
        } else {
            TextKey::ConversationStatsSessionUsage
        });
        let mut label = compact(total);
        if turn {
            label = crate::l10n::localizer()
                .format_with(TextKey::ConversationStatsTurnSummary, &[("tokens", &label)]);
        }
        let hit = stats.cache_hit_percent().map(percent);
        if !turn {
            if let Some(hit) = &hit {
                let cached = crate::l10n::localizer()
                    .format_with(TextKey::ConversationStatsCacheSummary, &[("percent", hit)]);
                label.push_str(&format!(" · {cached}"));
            }
        }
        let mut rows = Vec::new();
        if !stats.models.is_empty() {
            rows.push((
                text(TextKey::ConversationStatsModel),
                stats.models.join(", "),
            ));
        }
        let input_label = if stats.cache_read_tokens.is_some() {
            TextKey::ConversationStatsUncachedInput
        } else {
            TextKey::ConversationStatsInput
        };
        let uncached = if stats.cache_read_tokens.is_some() {
            stats
                .input_tokens
                .saturating_sub(stats.cache_read_tokens.unwrap_or(0))
                .saturating_sub(stats.cache_write_tokens.unwrap_or(0))
        } else {
            stats.input_tokens
        };
        rows.push((text(input_label), compact(uncached)));
        if let Some(read) = stats.cache_read_tokens {
            rows.push((text(TextKey::ConversationStatsCacheRead), compact(read)));
        }
        if let Some(write) = stats.cache_write_tokens.filter(|write| *write > 0) {
            rows.push((text(TextKey::ConversationStatsCacheWrite), compact(write)));
        }
        rows.push((
            text(TextKey::ConversationStatsOutput),
            compact(stats.output_tokens),
        ));
        if let Some(hit) = hit {
            rows.push((text(TextKey::ConversationStatsCacheHit), format!("{hit}%")));
        }
        Some(Self {
            title,
            align: if turn {
                placement::Align::Start
            } else {
                placement::Align::End
            },
            label,
            icon: crate::icons::USAGE_TOKENS,
            total: Some(compact(total)),
            rows,
            models: stats.models.clone(),
        })
    }
}

fn card(data: &StatData, theme: &Theme, cx: &gpui::App) -> gpui::Div {
    popover::popover_card(theme)
        .w(px(300.0))
        .p(px(12.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .text_size(UiTextRole::Primary.size())
                .line_height(UiTextRole::Primary.line_height())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            crate::icons::icon(data.icon)
                                .size(px(14.0))
                                .text_color(theme.text),
                        )
                        .child(data.title.clone()),
                )
                .children(data.total.clone()),
        )
        .child(popover::picker_divider())
        .children(data.rows.iter().enumerate().map(|(index, (key, value))| {
            let value = if index == 0 && !data.models.is_empty() {
                cx.try_global::<ModelLabels>()
                    .map(|labels| labels.resolve(&data.models))
                    .unwrap_or_else(|| value.clone())
            } else {
                value.clone()
            };
            div()
                .flex()
                .items_start()
                .justify_between()
                .gap(px(16.0))
                .text_size(UiTextRole::Secondary.size())
                .line_height(UiTextRole::Secondary.line_height())
                .child(
                    div()
                        .flex_none()
                        .text_color(theme.text_muted)
                        .child(key.clone()),
                )
                .child(
                    div()
                        .min_w_0()
                        .text_right()
                        .text_size(UiTextRole::Primary.size())
                        .line_height(UiTextRole::Primary.line_height())
                        .child(value),
                )
        }))
}

pub struct StatControl {
    data: StatData,
    open: bool,
    hovered: bool,
    show_task: Option<Task<()>>,
    hide_task: Option<Task<()>>,
    #[cfg(feature = "appshots-fixture")]
    fixture_pinned: bool,
    bounds: PanelBounds,
    panel_bounds: PanelBounds,
}

impl StatControl {
    pub fn new(data: StatData, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<ModelLabels>(|_, cx| cx.notify())
            .detach();
        Self {
            data,
            open: false,
            hovered: false,
            show_task: None,
            hide_task: None,
            #[cfg(feature = "appshots-fixture")]
            fixture_pinned: false,
            bounds: Rc::new(Cell::new(None)),
            panel_bounds: Rc::new(Cell::new(None)),
        }
    }

    pub fn set_data(&mut self, data: StatData, cx: &mut Context<Self>) {
        if self.data != data {
            self.data = data;
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
                        .timer(popover::DETAILS_OPEN_DELAY)
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
        #[cfg(feature = "appshots-fixture")]
        if self.fixture_pinned {
            return;
        }
        if !self.open || self.hide_task.is_some() {
            return;
        }
        // Allow the pointer to cross the gap between the trigger and card.
        self.hide_task = Some(cx.spawn(async |this, cx| {
            cx.background_executor()
                .timer(popover::DETAILS_CLOSE_DELAY)
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, TestAppContext};
    use std::time::Duration;

    #[gpui::test]
    fn usage_model_labels_refresh_when_the_catalog_arrives(cx: &mut TestAppContext) {
        let ids = vec!["m_50017e69".into(), "legacy-model".into()];
        let control = cx.new(|cx| StatControl::new(data(), cx));
        control.update(cx, |control, _| control.open = true);
        cx.update(|cx| {
            remember_model_labels([("m_50017e69", "My provider · My model")].into_iter(), cx);
            assert_eq!(
                cx.global::<ModelLabels>().resolve(&ids),
                "My provider · My model, legacy-model"
            );
            remember_model_labels(
                [("m_50017e69", "My provider · Renamed model")].into_iter(),
                cx,
            );
            assert_eq!(
                cx.global::<ModelLabels>().resolve(&ids),
                "My provider · Renamed model, legacy-model"
            );
        });
        control.read_with(cx, |control, _| assert!(control.open));
    }

    struct Fixture {
        control: Entity<StatControl>,
        focus: gpui::FocusHandle,
        top: bool,
        left: bool,
    }

    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .track_focus(&self.focus)
                .flex()
                .when(self.top, |row| row.items_start())
                .when(!self.top, |row| row.items_end())
                .p(px(10.0))
                .when(self.left, |row| row.justify_start().pl(px(220.0)))
                .when(!self.left, |row| row.justify_end())
                .child(self.control.clone())
        }
    }

    fn data() -> StatData {
        StatData::activity(&UsageStatistics {
            turns: 1,
            calls: 2,
            duration_ms: 2_000,
            first_token_ms: 200,
            timed_calls: 2,
            decode_ms: 1_800,
            decode_tokens: 180,
            ..Default::default()
        })
        .unwrap()
    }

    #[gpui::test]
    fn statistics_hover_shows_a_live_card_without_moving_focus_or_pinning_on_click(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
        });
        let (view, cx) = cx.add_window_view(|window, cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            Fixture {
                control: cx.new(|cx| StatControl::new(data(), cx)),
                focus,
                top: false,
                left: false,
            }
        });
        cx.simulate_resize(gpui::size(px(600.0), px(400.0)));
        cx.run_until_parked();
        let control = view.read_with(cx, |view, _| view.control.clone());
        let trigger = cx.debug_bounds("statistics-control").unwrap();
        cx.simulate_mouse_move(trigger.center(), None, Default::default());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        let panel = cx
            .debug_bounds("statistics-hover-card")
            .expect("hover shows statistics");
        assert!(panel.left() >= px(0.0) && panel.right() <= px(600.0));
        assert!(panel.top() >= px(0.0) && panel.bottom() <= px(400.0));
        assert!(panel.bottom() <= trigger.top() - px(8.0));
        assert!(!panel.intersects(&trigger));
        // Different mouse landing points within the same entry must not move the card.
        for position in [
            trigger.origin + gpui::point(px(3.0), px(3.0)),
            trigger.bottom_right() - gpui::point(px(3.0), px(3.0)),
        ] {
            cx.simulate_mouse_move(gpui::point(px(5.0), px(5.0)), None, Default::default());
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(600));
            cx.run_until_parked();
            assert!(cx.debug_bounds("statistics-hover-card").is_none());
            cx.simulate_mouse_move(position, None, Default::default());
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(600));
            cx.run_until_parked();
            assert_eq!(cx.debug_bounds("statistics-hover-card").unwrap(), panel);
        }
        cx.update(|window, cx| assert!(view.read(cx).focus.is_focused(window)));
        control.update(cx, |control, cx| {
            let mut update = data();
            update.rows.push(("live update".into(), "42".into()));
            control.set_data(update.clone(), cx);
            assert!(control.open);
            assert!(control.data == update);
        });
        cx.run_until_parked();
        // Moving into the details keeps them readable; moving away dismisses.
        let panel = cx.debug_bounds("statistics-hover-card").unwrap();
        cx.simulate_mouse_move(
            gpui::point(trigger.center().x, trigger.top() - px(4.0)),
            None,
            Default::default(),
        );
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        cx.simulate_mouse_move(panel.center(), None, Default::default());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        assert!(cx.debug_bounds("statistics-hover-card").is_some());
        cx.simulate_click(trigger.center(), Default::default());
        cx.simulate_mouse_move(gpui::point(px(5.0), px(5.0)), None, Default::default());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("statistics-hover-card").is_none(),
            "click must not pin a panel"
        );
    }

    #[gpui::test]
    fn statistics_near_the_top_open_below_the_entry(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
        });
        let (_, cx) = cx.add_window_view(|_, cx| Fixture {
            control: cx.new(|cx| StatControl::new(data(), cx)),
            focus: cx.focus_handle(),
            top: true,
            left: false,
        });
        cx.simulate_resize(gpui::size(px(600.0), px(400.0)));
        cx.run_until_parked();
        let trigger = cx.debug_bounds("statistics-control").unwrap();
        cx.simulate_mouse_move(trigger.center(), None, Default::default());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        let panel = cx.debug_bounds("statistics-hover-card").unwrap();
        assert!(panel.top() >= trigger.bottom() + px(8.0));
        assert!(!panel.intersects(&trigger));
    }

    #[gpui::test]
    fn turn_usage_opens_toward_the_reply_instead_of_over_the_sidebar(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
        });
        let usage = StatData::usage(
            &UsageStatistics {
                calls: 1,
                token_calls: 1,
                input_tokens: 8_708,
                output_tokens: 94,
                cache_read_tokens: Some(8_448),
                models: vec!["deepseek-v4.1-flash".into()],
                ..Default::default()
            },
            true,
        )
        .unwrap();
        let (_, cx) = cx.add_window_view(|_, cx| Fixture {
            control: cx.new(|cx| StatControl::new(usage, cx)),
            focus: cx.focus_handle(),
            top: false,
            left: true,
        });
        cx.simulate_resize(gpui::size(px(600.0), px(600.0)));
        cx.run_until_parked();
        let trigger = cx.debug_bounds("statistics-control").unwrap();
        cx.simulate_mouse_move(trigger.center(), None, Default::default());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        let panel = cx.debug_bounds("statistics-hover-card").unwrap();
        assert_eq!(panel.left(), trigger.left());
        assert!(
            panel.left() >= px(220.0),
            "the turn card must not expand into the sidebar"
        );
        assert!(panel.bottom() <= trigger.top() - px(8.0));
        assert!(panel.right() <= px(592.0));
    }
}

impl Render for StatControl {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let panel = self.open.then(|| {
            // Capping at half the available height guarantees the card fits on
            // at least one side of the trigger, even in a short window.
            let max_height = ((f32::from(window.viewport_size().height)
                - crate::surface_chrome::CONTROL_SIZE
                - 32.0)
                / 2.0)
                .max(1.0);
            let content = card(&self.data, &theme.for_popup(), cx)
                .id("statistics-hover-scroll")
                .debug_selector(|| "statistics-hover-card".into())
                .max_w((window.viewport_size().width - px(16.0)).max(px(1.0)))
                .max_h(px(max_height))
                .overflow_y_scroll()
                .occlude();
            let content = crate::motion::fade_quick(
                "statistics-details-in",
                div().child(crate::frost::frosted(
                    popover::CARD_RADIUS,
                    crate::frost::MENU_BLUR,
                    content,
                )),
            );
            div().absolute().inset_0().size_0().child(
                gpui::deferred(placement::Panel::new(
                    content.into_any_element(),
                    self.bounds.clone(),
                    self.panel_bounds.clone(),
                    self.data.align,
                ))
                .priority(1),
            )
        });
        let bounds = self.bounds.clone();
        let open = self.open;
        let owner = cx.weak_entity();
        div()
            .id("statistics-control")
            .debug_selector(|| "statistics-control".into())
            .relative()
            .aria_label(format!("{} {}", self.data.title, self.data.label))
            .h(px(crate::surface_chrome::CONTROL_SIZE))
            .px(px(STAT_PADDING))
            .rounded(px(crate::surface_chrome::CONTROL_RADIUS))
            .flex()
            .items_center()
            .gap(px(5.0))
            .text_size(UiTextRole::Caption.size())
            .line_height(UiTextRole::Caption.line_height())
            .text_color(theme.text_muted)
            .hover(|s| s.bg(theme.element_hover))
            .on_hover(cx.listener(|this, hovered, _, cx| this.hover_trigger(*hovered, cx)))
            .child(
                crate::icons::icon(self.data.icon)
                    .size(px(12.0))
                    .text_color(theme.text_muted),
            )
            .child(SharedString::from(self.data.label.clone()))
            .child(
                gpui::canvas(
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
            .children(panel)
    }
}

pub struct FooterStatistics {
    state: Entity<AppState>,
    revision: Option<(Option<String>, u64, bool, bool)>,
    controls: [Option<Entity<StatControl>>; 2],
    icon_bounds: PanelBounds,
}

impl FooterStatistics {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self {
            state,
            revision: None,
            controls: [None, None],
            icon_bounds: Rc::new(Cell::new(None)),
        }
    }

    pub(crate) fn icon_bounds(&self) -> PanelBounds {
        self.icon_bounds.clone()
    }
}

impl Render for FooterStatistics {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let compact = window.viewport_size().width < px(600.0);
        let revision = (
            state.selected_chat.clone(),
            state.transcript_revision,
            state.transcript_replayed,
            compact,
        );
        if self.revision.as_ref() != Some(&revision) {
            let stats = if state.selected_chat.is_some() && state.transcript_replayed {
                ConversationStatistics::from_entries(&state.transcript).session
            } else {
                Default::default()
            };
            // Navigation drops popup state along with the previous conversation's readings.
            if self
                .revision
                .as_ref()
                .is_some_and(|old| old.0 != revision.0)
            {
                self.controls = [None, None];
            }
            let mut activity = StatData::activity(&stats);
            if compact {
                if let (Some(data), Some(speed)) = (&mut activity, stats.tokens_per_second()) {
                    data.label = token_rate(speed);
                }
            }
            for (slot, data) in self
                .controls
                .iter_mut()
                .zip([activity, StatData::usage(&stats, false)])
            {
                match data {
                    Some(data) => match slot {
                        Some(control) => {
                            control.update(cx, |control, cx| control.set_data(data, cx))
                        }
                        None => *slot = Some(cx.new(|cx| StatControl::new(data, cx))),
                    },
                    None => *slot = None,
                }
            }
            self.revision = Some(revision);
        }
        let icon_bounds = self.icon_bounds.clone();
        let has_statistics = self.controls.iter().any(Option::is_some);
        let owner = cx.weak_entity();
        div()
            .id("session-statistics")
            .relative()
            .flex()
            .items_center()
            .gap(px(4.0))
            .children(self.controls.iter().flatten().cloned())
            .child(
                gpui::canvas(
                    move |rect, _, cx| {
                        let next = has_statistics.then(|| {
                            gpui::Bounds::new(
                                gpui::point(rect.left() + px(STAT_PADDING), rect.top()),
                                gpui::size(
                                    (rect.size.width - px(STAT_PADDING)).max(px(0.0)),
                                    rect.size.height,
                                ),
                            )
                        });
                        if icon_bounds.get() != next {
                            icon_bounds.set(next);
                            let _ = owner.update(cx, |_, cx| cx.notify());
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }
}

#[cfg(feature = "appshots-fixture")]
impl FooterStatistics {
    pub(crate) fn fixture_open_count(&self, cx: &gpui::App) -> usize {
        self.controls
            .iter()
            .flatten()
            .filter(|control| control.read(cx).open)
            .count()
    }

    pub(crate) fn fixture_show(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(control) = index
            .and_then(|index| self.controls.get(index))
            .and_then(Option::as_ref)
        {
            control.update(cx, |control, cx| control.fixture_show(true, window, cx));
        } else {
            for control in self.controls.iter().flatten() {
                control.update(cx, |control, cx| control.fixture_show(false, window, cx));
            }
        }
    }
}

#[cfg(feature = "appshots-fixture")]
impl StatControl {
    pub(crate) fn fixture_show(
        &mut self,
        hover: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Snapshot fixtures pin the panel directly; hover behavior is covered
        // by the interaction tests above, without native pointer interference.
        self.show_task = None;
        self.hide_task = None;
        self.hovered = hover;
        self.open = hover;
        self.fixture_pinned = hover;
        cx.notify();
    }
}
