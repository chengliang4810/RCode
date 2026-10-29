//! The agent's checklist, docked above the composer.
//!
//! Every harness that plans (Claude's TodoWrite, OpenCode, ACP and Codex
//! plans) emits the *whole* list on each update, so the panel simply tracks the
//! latest `Todo` tool part of the selected chat. It attaches to the goal header
//! when present, otherwise directly to the input, using the same layered
//! surface and overlap as the goal header.
//!
//! The first half of this file is pure — which list is current, what the header
//! says, which rows a long list folds to — so it is unit-tested without a
//! window. The second half is the gpui rendering on [`Composer`].

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::{Arc, Weak};
use std::time::Duration;

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, prelude::*, px,
};

use rcode_doc::{MessagePart, MessageRole, SessionMessageEntry, TranscriptBaseline};
use rcode_proto::{TodoItem, TodoStatus, ToolCall};

use crate::composer::{
    Composer, ATTACHED_PANEL_HEIGHT, ATTACHED_PANEL_INSET, ATTACHED_PANEL_OVERLAP,
    attached_panel_surface, clipped_composer_panel,
};
use crate::icons::{self, icon};
use crate::motion::{self, AnimationExt as _};
use crate::theme::Theme;

/// Lists longer than this fold to a [`FOCUS_WINDOW`] around the current item.
pub(crate) const FOLD_ABOVE: usize = 6;
pub(crate) const FOCUS_WINDOW: usize = 3;

/// Keep the write's identity even when its list is empty or completed: these
/// are lifecycle boundaries, not permission to rediscover an older checklist.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TodoWrite {
    entry: String,
    part: String,
    created_at: i64,
    items: Vec<TodoItem>,
}

impl TodoWrite {
    /// Render memoization only; dismissal itself uses the write identity.
    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.entry.hash(&mut hasher);
        self.part.hash(&mut hasher);
        self.created_at.hash(&mut hasher);
        for item in &self.items {
            item.text.hash(&mut hasher);
            item.status().hash(&mut hasher);
        }
        hasher.finish()
    }

    fn same_write(&self, other: &Self) -> bool {
        self.entry == other.entry && self.part == other.part
    }

    fn historical(&self, baseline: &TranscriptBaseline) -> bool {
        baseline
            .entries
            .get(&self.entry)
            .is_some_and(|parts| parts.contains_key(&self.part))
    }

    fn present_in(&self, entries: &[SessionMessageEntry]) -> bool {
        entries.iter().any(|entry| {
            entry.id == self.entry && entry.parts.iter().any(|part| part.id() == self.part)
        })
    }
}

/// The checklist the agent most recently wrote, including an explicit clear.
///
/// `entries` is the selected chat's own transcript; subagent traffic lives in
/// separate docs and never reaches it. ACP plans reuse one tool id for every
/// update, so "latest part" — not "first part with this id" — is the contract.
fn latest_todo_write(entries: &[SessionMessageEntry]) -> Option<TodoWrite> {
    entries
        .iter()
        .rev()
        .filter(|entry| entry.role == MessageRole::Assistant)
        .flat_map(|entry| entry.parts.iter().rev().map(move |part| (entry, part)))
        .find_map(|(entry, part)| match part {
            MessagePart::Tool {
                id,
                call: ToolCall::Todo { items },
                ..
            } => Some(TodoWrite {
                entry: entry.id.clone(),
                part: id.clone(),
                created_at: entry.created_at,
                items: items.clone(),
            }),
            _ => None,
        })
}

/// What the collapsed header reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TodoSummary {
    pub total: usize,
    pub done: usize,
    /// First in-progress item.
    pub active: Option<usize>,
    /// First item not yet completed.
    pub next: Option<usize>,
}

impl TodoSummary {
    pub fn of(items: &[TodoItem]) -> Self {
        let mut summary = Self {
            total: items.len(),
            done: 0,
            active: None,
            next: None,
        };
        for (ix, item) in items.iter().enumerate() {
            match item.status() {
                TodoStatus::Completed => summary.done += 1,
                TodoStatus::InProgress => {
                    summary.active.get_or_insert(ix);
                    summary.next.get_or_insert(ix);
                }
                TodoStatus::Pending => {
                    summary.next.get_or_insert(ix);
                }
            }
        }
        summary
    }

    pub fn finished(&self) -> bool {
        self.total > 0 && self.done == self.total
    }

    /// The item the header names: what is being worked on, else what is next.
    pub fn headline(&self) -> Option<usize> {
        self.active.or(self.next)
    }
}

/// The slice of a long list that stays visible when folded: three items with
/// the current one in the middle (one of context before it, one after),
/// clamped to the list ends. A finished list shows its last three. Lists up to
/// [`FOLD_ABOVE`] are never folded.
pub(crate) fn focus_window(items: &[TodoItem]) -> Range<usize> {
    let total = items.len();
    if total <= FOLD_ABOVE {
        return 0..total;
    }
    let summary = TodoSummary::of(items);
    let focus = summary.headline().unwrap_or(total - 1);
    let start = focus.saturating_sub(1).min(total - FOCUS_WINDOW);
    start..start + FOCUS_WINDOW
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FoldSide {
    Earlier,
    Later,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TodoRow {
    /// An item, by its index in the agent's list.
    Item(usize),
    /// A fold toggle standing in for `count` hidden items (`open`: they are
    /// currently revealed, and the row hides them again).
    Fold {
        side: FoldSide,
        count: usize,
        open: bool,
    },
}

/// The rows of the expanded list, top to bottom. Items always keep the
/// agent's order; folds sit at the edges of the list they stand for.
pub(crate) fn rows(items: &[TodoItem], show_earlier: bool, show_later: bool) -> Vec<TodoRow> {
    let window = focus_window(items);
    let earlier = window.start;
    let later = items.len() - window.end;
    let mut out = Vec::with_capacity(items.len() + 2);
    if earlier > 0 {
        out.push(TodoRow::Fold {
            side: FoldSide::Earlier,
            count: earlier,
            open: show_earlier,
        });
        if show_earlier {
            out.extend((0..earlier).map(TodoRow::Item));
        }
    }
    out.extend(window.clone().map(TodoRow::Item));
    if later > 0 {
        if show_later {
            out.extend((window.end..items.len()).map(TodoRow::Item));
        }
        out.push(TodoRow::Fold {
            side: FoldSide::Later,
            count: later,
            open: show_later,
        });
    }
    out
}

/// Per-chat presentation state. In memory only — like the right-pane flags in
/// `shell::SessionPanels` it lasts for the app run, not across restarts.
#[derive(Debug, Default, Clone)]
pub(crate) struct TodoPanelState {
    /// New panels start compact; only a user action expands them.
    expanded: bool,
    show_earlier: bool,
    show_later: bool,
    dismissed: bool,
    /// The last accepted write, independent of the currently loaded history.
    current: Option<TodoWrite>,
    initialized: bool,
    last_observed: Option<(Option<u64>, bool)>,
    /// A weak watermark detects history refreshes without retaining a chat's
    /// potentially large replay baseline after navigation.
    baseline: Option<Weak<TranscriptBaseline>>,
    /// Bumped on every toggle so the list's fade-in replays.
    epoch: u32,
}

impl TodoPanelState {
    /// Backfill must not create a panel, rewind its lifecycle, or undo a
    /// dismissal. A later write may advance the panel, even with identical text.
    pub fn observe(
        &mut self,
        candidate: Option<&TodoWrite>,
        entries: &[SessionMessageEntry],
        baseline: Option<&Arc<TranscriptBaseline>>,
        has_older: bool,
    ) {
        let replay_changed = match (&self.baseline, baseline) {
            (Some(old), Some(new)) => old.as_ptr() != Arc::as_ptr(new),
            (None, Some(_)) => true,
            _ => false,
        };
        self.baseline = baseline.map(Arc::downgrade);
        // Window completeness only affects an absent candidate. Changing it
        // must not reapply a historical write that was already rejected.
        let observation = (
            candidate.map(TodoWrite::fingerprint),
            candidate.is_none() && has_older,
        );
        if self.last_observed == Some(observation) && !replay_changed {
            return;
        }
        self.last_observed = Some(observation);
        let Some(candidate) = candidate else {
            self.initialized = true;
            if !has_older && let Some(current) = &mut self.current {
                current.items.clear();
                self.expanded = false;
            }
            return;
        };
        let historical = baseline.is_some_and(|baseline| candidate.historical(baseline));
        let mut fresh_panel = self.dismissed
            || self.current.as_ref().is_none_or(|current| {
                current.items.is_empty() || TodoSummary::of(&current.items).finished()
            });
        if self.initialized {
            match &self.current {
                None if historical => return,
                Some(current) => {
                    let retired =
                        current.items.is_empty() || TodoSummary::of(&current.items).finished();
                    let same_write = current.same_write(candidate);
                    if same_write {
                        if retired
                            && replay_changed
                            && historical
                            && !TodoSummary::of(&candidate.items).finished()
                        {
                            return;
                        }
                        fresh_panel = retired;
                    } else {
                        let previous_present = current.present_in(entries);
                        if !previous_present
                            && (candidate.created_at < current.created_at
                                || (historical && candidate.created_at <= current.created_at))
                        {
                            return;
                        }
                    }
                    // Replaying another version of this same tool call cannot
                    // undo its close button. A new write starts a new panel.
                    if same_write && !retired {
                        fresh_panel = false;
                    }
                }
                _ => {}
            }
        }
        let different_write = self
            .current
            .as_ref()
            .is_none_or(|current| !current.same_write(candidate));
        if different_write || fresh_panel {
            self.dismissed = false;
        }
        if fresh_panel {
            self.expanded = false;
            self.show_earlier = false;
            self.show_later = false;
        }
        if self.current.as_ref() != Some(candidate) {
            self.current = Some(candidate.clone());
        }
        self.initialized = true;
        if candidate.items.is_empty() || TodoSummary::of(&candidate.items).finished() {
            self.expanded = false;
        }
    }

    pub fn visible_items(&self) -> Option<&[TodoItem]> {
        let items = &self.current.as_ref()?.items;
        (!self.dismissed && !items.is_empty() && !TodoSummary::of(items).finished())
            .then_some(items.as_slice())
    }

    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    pub fn toggle(&mut self) {
        self.expanded = !self.expanded;
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub fn toggle_fold(&mut self, side: FoldSide) {
        match side {
            FoldSide::Earlier => self.show_earlier = !self.show_earlier,
            FoldSide::Later => self.show_later = !self.show_later,
        }
    }

    pub fn dismiss(&mut self) {
        self.dismissed = true;
        self.expanded = false;
    }
}

/// The latest list for the selected chat, recomputed only when its transcript
/// changes (the scan walks back from the end, but a chat with no todo walks all
/// of it, and the composer renders far more often than the transcript changes).
#[derive(Debug, Default)]
pub(crate) struct TodoCache {
    key: Option<(String, u64, usize)>,
    items: Option<TodoWrite>,
}

impl TodoCache {
    pub fn latest(
        &mut self,
        chat: &str,
        revision: u64,
        entries: &[SessionMessageEntry],
    ) -> Option<&TodoWrite> {
        let key = (chat.to_owned(), revision, entries.len());
        if self.key.as_ref() != Some(&key) {
            self.items = latest_todo_write(entries);
            self.key = Some(key);
        }
        self.items.as_ref()
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

const HEADER_HEIGHT: f32 = ATTACHED_PANEL_HEIGHT - ATTACHED_PANEL_OVERLAP - 2.0;
const ROW_PAD_X: f32 = Theme::SPACE_SM;
/// Rows sit in the queue tray's surface, so they share its concentric radius.
const ROW_RADIUS: f32 = crate::queue::ROW_RADIUS;
const GLYPH_SLOT: f32 = 14.0;
const ACTION_SIZE: f32 = 24.0;
impl Composer {
    /// The checklist tray, hidden when absent, fully completed or dismissed.
    pub(crate) fn render_todo_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let live = self.run_live(cx);
        let has_goal = self.state.read(cx).goal.is_some();
        let (chat_id, items, expanded, show_earlier, show_later, epoch) = {
            let state = self.state.read(cx);
            let chat_id = state.selected_chat.clone()?;
            if !state.transcript_replayed {
                return None;
            }
            let candidate =
                self.todo_cache
                    .latest(&chat_id, state.transcript_revision, &state.transcript);
            let panel = self.todo_panels.entry(chat_id.clone()).or_default();
            panel.observe(
                candidate,
                &state.transcript,
                state.transcript_baseline(&chat_id),
                state
                    .transcript_windows
                    .get(&chat_id)
                    .is_some_and(|window| window.has_older),
            );
            let items = panel.visible_items()?.to_vec();
            (
                chat_id,
                items,
                panel.is_expanded(),
                panel.show_earlier,
                panel.show_later,
                panel.epoch,
            )
        };
        let summary = TodoSummary::of(&items);

        let theme = Theme::of(cx).clone();
        let view = cx.entity_id();
        // Dismissable whenever the turn is idle, finished or not.
        let header = self.todo_header(&chat_id, &items, summary, expanded, !live, &theme, cx);

        let surface = attached_panel_surface(&theme)
            .when(!cx.has_active_drag(), |surface| surface.occlude())
            .px(px(2.0))
            .pb(px(ATTACHED_PANEL_OVERLAP))
            .flex()
            .flex_col()
            .child(header);
        let surface = if expanded {
            let list = div().flex().flex_col().children(
                rows(&items, show_earlier, show_later)
                    .into_iter()
                    .map(|row| match row {
                        TodoRow::Item(ix) => todo_item_row(ix, &items[ix], live, &theme, view, cx),
                        TodoRow::Fold { side, count, open } => {
                            self.todo_fold_row(&chat_id, side, count, open, &theme, cx)
                        }
                    }),
            );
            // Same fade-and-scroll treatment as the queue's rows; a long
            // expanded fold scrolls instead of pushing the transcript up.
            let rows = crate::edge_fade::edge_faded(
                Theme::TRANSCRIPT_FADE_BAND,
                true,
                true,
                div()
                    .id("todo-panel-rows")
                    .max_h(window.viewport_size().height * 0.3)
                    .overflow_y_scroll()
                    .track_scroll(&self.todo_scroll)
                    .child(list),
            )
            // The ramp ends at the clip edge; glyphs fade per pixel.
            .fade_overflow_y(&self.todo_scroll);
            surface.child(
                div()
                    .mt(px(2.0))
                    // The tray's lowest strip hides behind the composer; keep
                    // the last row clear of that edge.
                    .pb(px(6.0))
                    .child(rows)
                    .with_animation(
                        SharedString::from(format!("todo-rows-{epoch}")),
                        motion::FADE_QUICK.animation(),
                        |el, t| el.opacity(t),
                    ),
            )
        } else {
            surface
        };

        let inset = ATTACHED_PANEL_INSET * if has_goal { 2.0 } else { 1.0 };
        Some(
            motion::fade_quick(
                "composer-todo",
                clipped_composer_panel(surface, ATTACHED_PANEL_OVERLAP)
                    .debug_selector(|| "todo-panel".into())
                    .mx(px(inset)),
            )
            .into_any_element(),
        )
    }

    /// `Todo · 2/5` and what is being worked on; one button that toggles the
    /// list, plus a dismiss while the turn is idle.
    #[allow(clippy::too_many_arguments)]
    fn todo_header(
        &self,
        chat_id: &str,
        items: &[TodoItem],
        summary: TodoSummary,
        expanded: bool,
        dismissable: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = theme.accent;
        let localizer = crate::l10n::localizer();
        let label = localizer.text(if expanded {
            rcode_presentation::TextKey::ConversationTodoCollapse
        } else {
            rcode_presentation::TextKey::ConversationTodoExpand
        });
        let headline: Option<(SharedString, gpui::Hsla)> = if summary.finished() {
            Some((
                localizer
                    .text(rcode_presentation::TextKey::ConversationTodoAllDone)
                    .into(),
                theme.text_faint,
            ))
        } else {
            summary
                .headline()
                .map(|ix| (items[ix].text.clone().into(), theme.text_muted))
        };

        let toggle_chat = chat_id.to_owned();
        let toggle =
            div()
                .id("todo-panel-toggle")
                .debug_selector(|| "todo-panel-toggle".into())
                .role(gpui::Role::Button)
                .aria_label(label.clone())
                .flex_1()
                .min_w_0()
                .h(px(HEADER_HEIGHT))
                .px(px(ROW_PAD_X))
                .rounded(px(ROW_RADIUS))
                .flex()
                .items_center()
                .gap(px(8.0))
                .cursor_pointer()
                .hover(|s| s.bg(theme.element_hover))
                .tab_index(0)
                .focus_visible(move |s| s.bg(accent.opacity(0.14)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.todo_panels
                        .entry(toggle_chat.clone())
                        .or_default()
                        .toggle();
                    cx.notify();
                }))
                .tooltip(crate::settings::widgets::text_tooltip(label))
                .tooltip_show_delay(Duration::from_millis(350))
                .child(
                    icon(if summary.finished() {
                        icons::CHECK
                    } else {
                        icons::CHECKLIST
                    })
                    .size(px(14.0))
                    .text_color(if summary.finished() {
                        theme.success
                    } else {
                        theme.text_muted
                    }),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_baseline()
                        .gap(px(6.0))
                        .text_size(crate::typography::UiTextRole::Section.size())
                        .line_height(crate::typography::UiTextRole::Section.line_height())
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(
                                    crate::l10n::localizer()
                                        .text(rcode_presentation::TextKey::ToolTodo),
                                ),
                        )
                        .child(div().text_color(theme.text_faint).child(SharedString::from(
                            format!("{}/{}", summary.done, summary.total),
                        ))),
                )
                // The expanded list already names every item; the headline is the
                // collapsed state's whole point.
                .when(!expanded, |el| {
                    el.when_some(headline, |el, (text, color)| {
                        el.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(crate::typography::UiTextRole::Primary.size())
                                .line_height(crate::typography::UiTextRole::Primary.line_height())
                                .text_color(color)
                                .child(text),
                        )
                    })
                })
                .when(expanded, |el| el.child(div().flex_1()))
                .child(
                    icon(if expanded {
                        icons::ALT_ARROW_DOWN
                    } else {
                        icons::ALT_ARROW_UP
                    })
                    .size(px(13.0))
                    .text_color(theme.text_muted.opacity(0.7)),
                );

        let dismiss_chat = chat_id.to_owned();
        div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .child(toggle)
            .when(dismissable, |row| {
                row.child(
                    div()
                        .id("todo-panel-dismiss")
                        .role(gpui::Role::Button)
                        .aria_label(
                            crate::l10n::localizer()
                                .text(rcode_presentation::TextKey::ConversationTodoDismiss),
                        )
                        .size(px(ACTION_SIZE))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.element_hover))
                        .tab_index(0)
                        .focus_visible(move |s| s.bg(accent.opacity(0.18)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.todo_panels
                                .entry(dismiss_chat.clone())
                                .or_default()
                                .dismiss();
                            cx.notify();
                        }))
                        .tooltip(crate::settings::widgets::text_tooltip(
                            crate::l10n::localizer()
                                .text(rcode_presentation::TextKey::ActionDismiss),
                        ))
                        .tooltip_show_delay(Duration::from_millis(350))
                        .child(
                            icon(icons::CLOSE)
                                .size(px(11.0))
                                .text_color(theme.text_muted.opacity(0.8)),
                        ),
                )
            })
            .into_any_element()
    }

    /// "N earlier" / "N later": stands in for the items folded off one end of a
    /// long list; clicking reveals them in place, clicking again hides them.
    fn todo_fold_row(
        &self,
        chat_id: &str,
        side: FoldSide,
        count: usize,
        open: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = theme.accent;
        let localizer = crate::l10n::localizer();
        // The noun is a word, so it comes from the resources; `noun` stays the
        // stable slug the element id and the fold key are built from.
        let noun = match side {
            FoldSide::Earlier => "earlier",
            FoldSide::Later => "later",
        };
        let text: SharedString = localizer
            .format_with(
                match side {
                    FoldSide::Earlier => rcode_presentation::TextKey::ConversationTodoEarlier,
                    FoldSide::Later => rcode_presentation::TextKey::ConversationTodoLater,
                },
                &[("count", &count.to_string())],
            )
            .into();
        let aria = localizer.format_with(
            match (side, open) {
                (FoldSide::Earlier, false) => {
                    rcode_presentation::TextKey::ConversationTodoShowEarlier
                }
                (FoldSide::Earlier, true) => {
                    rcode_presentation::TextKey::ConversationTodoHideEarlier
                }
                (FoldSide::Later, false) => rcode_presentation::TextKey::ConversationTodoShowLater,
                (FoldSide::Later, true) => rcode_presentation::TextKey::ConversationTodoHideLater,
            },
            &[("count", &count.to_string())],
        );
        // The arrow points to where the hidden items are (or, once revealed,
        // back to where they fold away).
        let glyph = match (side, open) {
            (FoldSide::Earlier, false) | (FoldSide::Later, true) => icons::ALT_ARROW_UP,
            _ => icons::ALT_ARROW_DOWN,
        };
        let chat_id = chat_id.to_owned();
        div()
            .id(SharedString::from(format!("todo-fold-{noun}")))
            .role(gpui::Role::Button)
            .aria_label(aria)
            .h(px(24.0))
            .px(px(ROW_PAD_X))
            .rounded(px(6.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .cursor_pointer()
            .hover(|s| s.bg(theme.element_hover))
            .tab_index(0)
            .focus_visible(move |s| s.bg(accent.opacity(0.14)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.todo_panels
                    .entry(chat_id.clone())
                    .or_default()
                    .toggle_fold(side);
                cx.notify();
            }))
            .child(
                div()
                    .w(px(GLYPH_SLOT))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .child(icon(glyph).size(px(12.0)).text_color(theme.text_faint)),
            )
            .child(
                div()
                    .text_size(crate::typography::UiTextRole::Secondary.size())
                    .line_height(crate::typography::UiTextRole::Secondary.line_height())
                    .text_color(theme.text_faint)
                    .child(text),
            )
            .into_any_element()
    }
}

/// One checklist row: status glyph + text. Completed work recedes, the
/// in-progress item is the brightest thing in the list.
fn todo_item_row(
    ix: usize,
    item: &TodoItem,
    live: bool,
    theme: &Theme,
    view: gpui::EntityId,
    cx: &mut gpui::App,
) -> AnyElement {
    let status = item.status();
    let (color, weight) = match status {
        TodoStatus::Completed => (theme.text_faint, gpui::FontWeight::NORMAL),
        TodoStatus::InProgress => (theme.text, gpui::FontWeight::MEDIUM),
        TodoStatus::Pending => (theme.text_muted, gpui::FontWeight::NORMAL),
    };
    div()
        .id(SharedString::from(format!("todo-item-{ix}")))
        .min_h(crate::typography::ui_rems(Theme::CONTROL_HEIGHT))
        .px(px(ROW_PAD_X))
        .py(px(4.0))
        .flex()
        .items_start()
        .gap(px(8.0))
        .child(
            div()
                .w(px(GLYPH_SLOT))
                .h(crate::typography::UiTextRole::Primary.line_height())
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(status_glyph(ix, status, live, theme, view, cx)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(crate::typography::UiTextRole::Primary.size())
                .line_height(crate::typography::UiTextRole::Primary.line_height())
                .font_weight(weight)
                .text_color(color)
                .child(SharedString::from(item.text.clone())),
        )
        .into_any_element()
}

/// check / active / pending. An in-progress item only animates while the turn
/// is live; on an idle chat it is a still ring (the agent is not working on
/// it), so a stopped run does not look busy forever.
fn status_glyph(
    ix: usize,
    status: TodoStatus,
    live: bool,
    theme: &Theme,
    view: gpui::EntityId,
    cx: &mut gpui::App,
) -> AnyElement {
    match status {
        TodoStatus::Completed => icon(icons::CHECK)
            .size(px(12.0))
            .text_color(theme.success)
            .into_any_element(),
        TodoStatus::InProgress if live => crate::loaders::mini_glyph_spinner(
            format!("todo-active-{ix}"),
            2.5,
            theme.glyph,
            view,
            cx,
        )
        .into_any_element(),
        TodoStatus::InProgress => div()
            .size(px(10.0))
            .rounded_full()
            .border_1()
            .border_color(theme.accent)
            .flex()
            .items_center()
            .justify_center()
            .child(div().size(px(4.0)).rounded_full().bg(theme.accent))
            .into_any_element(),
        TodoStatus::Pending => div()
            .size(px(10.0))
            .rounded_full()
            .border_1()
            .border_color(theme.text_faint.opacity(0.7))
            .into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(spec: &str) -> Vec<TodoItem> {
        // x = completed, > = in progress, . = pending
        spec.chars()
            .enumerate()
            .map(|(i, c)| {
                TodoItem::new(
                    format!("item {i}"),
                    match c {
                        'x' => TodoStatus::Completed,
                        '>' => TodoStatus::InProgress,
                        _ => TodoStatus::Pending,
                    },
                )
            })
            .collect()
    }

    fn entry(role: MessageRole, parts: Vec<MessagePart>) -> SessionMessageEntry {
        SessionMessageEntry {
            id: "m".into(),
            role,
            parts,
            created_at: 0,
            device_id: "d".into(),
            status: None,
            continuation_of: None,
            duration_ms: None,
            excluded: false,
        }
    }

    fn todo_part(id: &str, list: Vec<TodoItem>) -> MessagePart {
        MessagePart::Tool {
            id: id.into(),
            call: ToolCall::Todo { items: list },
            is_error: false,
            resolved: true,
            output: None,
            diff: None,
            output_ref: None,
            output_bytes: None,
            diff_ref: None,
            diff_stats: None,
            subagent_ref: None,
            subagent_status: None,
            subagent_tail: None,
        }
    }

    fn latest_todo(entries: &[SessionMessageEntry]) -> Option<Vec<TodoItem>> {
        latest_todo_write(entries)
            .map(|write| write.items)
            .filter(|items| !items.is_empty())
    }

    fn write(id: &str, spec: &str, created_at: i64) -> SessionMessageEntry {
        let mut entry = entry(MessageRole::Assistant, vec![todo_part(id, items(spec))]);
        entry.id = format!("message-{id}");
        entry.created_at = created_at;
        entry
    }

    fn observe(
        state: &mut TodoPanelState,
        entries: &[SessionMessageEntry],
        baseline: Option<&Arc<TranscriptBaseline>>,
        has_older: bool,
    ) {
        let candidate = latest_todo_write(entries);
        state.observe(candidate.as_ref(), entries, baseline, has_older);
    }

    #[test]
    fn latest_todo_is_the_last_write_across_entries() {
        let first = entry(MessageRole::Assistant, vec![todo_part("a", items("x.."))]);
        let second = entry(
            MessageRole::Assistant,
            vec![
                todo_part("b", items("xx.")),
                MessagePart::Text {
                    id: "t".into(),
                    text: "working".into(),
                },
                todo_part("c", items("xx>")),
            ],
        );
        assert_eq!(latest_todo(&[first, second]), Some(items("xx>")));
    }

    #[test]
    fn acp_plan_reusing_one_id_resolves_to_the_newest_segment() {
        let plan = rcode_proto::LIVE_PLAN_TOOL_ID;
        let old = entry(MessageRole::Assistant, vec![todo_part(plan, items(">.."))]);
        let new = entry(MessageRole::Assistant, vec![todo_part(plan, items("x>."))]);
        assert_eq!(latest_todo(&[old, new]), Some(items("x>.")));
    }

    #[test]
    fn an_empty_write_clears_and_user_rows_are_ignored() {
        let list = entry(MessageRole::Assistant, vec![todo_part("a", items("x."))]);
        let cleared = entry(MessageRole::Assistant, vec![todo_part("b", Vec::new())]);
        assert_eq!(latest_todo(&[list.clone(), cleared]), None);
        let user = entry(MessageRole::User, vec![todo_part("u", items("."))]);
        assert_eq!(latest_todo(&[user]), None);
        assert_eq!(latest_todo(&[list]), Some(items("x.")));
        assert_eq!(latest_todo(&[]), None);
    }

    #[test]
    fn summary_counts_and_names_the_current_item() {
        let s = TodoSummary::of(&items("xx>.."));
        assert_eq!(
            (s.total, s.done, s.active, s.next),
            (5, 2, Some(2), Some(2))
        );
        assert_eq!(s.headline(), Some(2));
        assert!(!s.finished());
        // Nothing started: the first unfinished item is the headline.
        let s = TodoSummary::of(&items("x..."));
        assert_eq!((s.active, s.headline()), (None, Some(1)));
        // An in-progress item later in the list still wins over an earlier
        // pending one: the agent is working on it.
        assert_eq!(TodoSummary::of(&items(".>.")).headline(), Some(1));
        let s = TodoSummary::of(&items("xxx"));
        assert!(s.finished());
        assert_eq!(s.headline(), None);
        assert!(!TodoSummary::of(&[]).finished());
    }

    #[test]
    fn short_lists_never_fold() {
        for n in 0..=FOLD_ABOVE {
            let list = items(&".".repeat(n));
            assert_eq!(focus_window(&list), 0..n);
            assert!(
                rows(&list, false, false)
                    .iter()
                    .all(|r| matches!(r, TodoRow::Item(_)))
            );
        }
    }

    #[test]
    fn long_lists_fold_to_three_around_the_current_item() {
        // Current item in the middle: one before, it, one after.
        assert_eq!(focus_window(&items("xxxx>.....")), 3..6);
        // At the start / end the window clamps instead of shrinking.
        assert_eq!(focus_window(&items(">.......")), 0..3);
        assert_eq!(focus_window(&items("xxxxxxx>")), 5..8);
        // No in-progress item: the first unfinished one is the focus.
        assert_eq!(focus_window(&items("xxxx.....")), 3..6);
        // Everything done: the tail.
        assert_eq!(focus_window(&items("xxxxxxxx")), 5..8);
        // Always exactly three when folded.
        for spec in ["xxxxxxx>", ">.......", "xx>.....", "xxxxxxxx", "........"] {
            assert_eq!(focus_window(&items(spec)).len(), FOCUS_WINDOW, "{spec}");
        }
    }

    #[test]
    fn rows_put_folds_at_the_edges_with_counts() {
        let list = items("xxxx>.....");
        assert_eq!(
            rows(&list, false, false),
            vec![
                TodoRow::Fold {
                    side: FoldSide::Earlier,
                    count: 3,
                    open: false
                },
                TodoRow::Item(3),
                TodoRow::Item(4),
                TodoRow::Item(5),
                TodoRow::Fold {
                    side: FoldSide::Later,
                    count: 4,
                    open: false
                },
            ]
        );
        // No fold on a side with nothing hidden.
        let head = rows(&items(">......."), false, false);
        assert!(matches!(head.first(), Some(TodoRow::Item(0))));
        assert!(matches!(
            head.last(),
            Some(TodoRow::Fold {
                side: FoldSide::Later,
                count: 5,
                ..
            })
        ));
    }

    #[test]
    fn expanding_a_fold_reveals_items_without_reordering() {
        let list = items("xxxx>.....");
        let order = |rows: &[TodoRow]| -> Vec<usize> {
            rows.iter()
                .filter_map(|r| match r {
                    TodoRow::Item(ix) => Some(*ix),
                    _ => None,
                })
                .collect()
        };
        for (earlier, later) in [(false, false), (true, false), (false, true), (true, true)] {
            let shown = order(&rows(&list, earlier, later));
            assert!(shown.windows(2).all(|w| w[0] < w[1]), "{earlier} {later}");
        }
        // Both open: the whole list, in order, and each toggle still present
        // so it can be closed again.
        let all = rows(&list, true, true);
        assert_eq!(order(&all), (0..list.len()).collect::<Vec<_>>());
        assert!(matches!(
            all.first(),
            Some(TodoRow::Fold { open: true, .. })
        ));
        assert!(matches!(all.last(), Some(TodoRow::Fold { open: true, .. })));
        // Earlier items appear after their fold row, later items before theirs.
        assert_eq!(all[1], TodoRow::Item(0));
        assert_eq!(all[all.len() - 2], TodoRow::Item(9));
    }

    #[test]
    fn new_work_starts_collapsed_and_progress_keeps_the_manual_choice() {
        let mut state = TodoPanelState::default();
        observe(&mut state, &[write("a", "...", 1)], None, false);
        assert!(state.visible_items().is_some());
        assert!(!state.is_expanded());
        state.toggle();
        observe(&mut state, &[write("a", "x>.", 1)], None, false);
        assert!(state.is_expanded());
        observe(&mut state, &[write("b", "xx>", 2)], None, false);
        assert!(state.is_expanded());
        observe(&mut state, &[write("c", "xxx", 3)], None, false);
        assert!(state.visible_items().is_none());
        observe(&mut state, &[write("d", "...", 4)], None, false);
        assert!(state.visible_items().is_some());
        assert!(!state.is_expanded());
    }

    #[test]
    fn toggle_replays_the_fade_and_folds_toggle_independently() {
        let mut state = TodoPanelState::default();
        let before = state.epoch;
        state.toggle();
        assert_ne!(state.epoch, before);
        state.toggle_fold(FoldSide::Later);
        assert!(state.show_later && !state.show_earlier);
        state.toggle_fold(FoldSide::Later);
        assert!(!state.show_later);
    }

    #[test]
    fn dismissal_survives_changes_to_the_same_write_but_allows_new_writes() {
        let mut state = TodoPanelState::default();
        observe(&mut state, &[write("a", "x..", 2)], None, false);
        state.dismiss();
        let replay = vec![write("a", "...", 2)];
        let baseline = Arc::new(TranscriptBaseline::capture(&replay));
        for _ in 0..3 {
            observe(&mut state, &replay, Some(&baseline), true);
            assert!(state.visible_items().is_none());
        }
        // A later tool invocation is new work even when its text is identical.
        observe(&mut state, &[write("b", "x..", 3)], Some(&baseline), false);
        assert_eq!(state.visible_items(), Some(items("x..").as_slice()));
        assert!(!state.is_expanded());
    }

    #[test]
    fn finished_and_cleared_lists_never_rewind_during_history_replay() {
        for final_spec in ["xxx", ""] {
            let mut state = TodoPanelState::default();
            observe(&mut state, &[write("b", final_spec, 2)], None, false);
            assert!(state.visible_items().is_none());
            // A refresh can replay an older version of the same tool call.
            let replay = vec![write("b", "x..", 2)];
            let baseline = Arc::new(TranscriptBaseline::capture(&replay));
            for _ in 0..3 {
                observe(&mut state, &replay, Some(&baseline), true);
                assert!(state.visible_items().is_none(), "{final_spec}");
            }
            observe(&mut state, &replay, Some(&baseline), false);
            assert!(state.visible_items().is_none(), "{final_spec}");
            // Or backfill can expose a different, older tool invocation.
            let older = vec![write("a", "...", 1)];
            let baseline = Arc::new(TranscriptBaseline::capture(&older));
            for _ in 0..3 {
                observe(&mut state, &older, Some(&baseline), true);
                assert!(state.visible_items().is_none(), "{final_spec}");
            }
            observe(&mut state, &[write("c", "...", 3)], Some(&baseline), false);
            assert!(state.visible_items().is_some());
            assert!(!state.is_expanded());
        }
    }

    #[test]
    fn backfill_does_not_create_a_panel_when_the_initial_tail_had_none() {
        let mut state = TodoPanelState::default();
        observe(&mut state, &[], None, true);
        let older = vec![write("a", "...", 1)];
        let baseline = Arc::new(TranscriptBaseline::capture(&older));
        for _ in 0..3 {
            observe(&mut state, &older, Some(&baseline), true);
            assert!(state.visible_items().is_none());
        }
        observe(&mut state, &[write("b", "...", 2)], Some(&baseline), true);
        assert!(state.visible_items().is_some());
    }

    #[test]
    fn loading_older_messages_keeps_current_work_and_its_expansion() {
        let mut state = TodoPanelState::default();
        let current = write("b", "x..", 2);
        observe(&mut state, &[current.clone()], None, true);
        state.toggle();
        let replay = vec![write("a", "...", 1), current];
        let baseline = Arc::new(TranscriptBaseline::capture(&replay));
        observe(&mut state, &replay, Some(&baseline), false);
        assert_eq!(state.visible_items(), Some(items("x..").as_slice()));
        assert!(state.is_expanded());
        // A partial window with no todo does not erase current work.
        observe(&mut state, &[], Some(&baseline), true);
        assert!(state.visible_items().is_some());
        // A complete, empty transcript really does clear it.
        observe(&mut state, &[], Some(&baseline), false);
        assert!(state.visible_items().is_none());
    }

    #[test]
    fn live_plan_can_start_new_work_after_finishing_with_the_same_tool_id() {
        let mut state = TodoPanelState::default();
        let finished = vec![write("plan", "xxx", 1)];
        let baseline = Arc::new(TranscriptBaseline::capture(&finished));
        observe(&mut state, &finished, Some(&baseline), false);
        observe(
            &mut state,
            &[write("plan", "x..", 1)],
            Some(&baseline),
            false,
        );
        assert!(state.visible_items().is_some());
        assert!(!state.is_expanded());
    }

    #[test]
    fn cache_rescans_only_when_the_transcript_changes() {
        let mut cache = TodoCache::default();
        let one = vec![entry(
            MessageRole::Assistant,
            vec![todo_part("a", items("x."))],
        )];
        assert_eq!(
            cache
                .latest("chat", 1, &one)
                .map(|write| write.items.as_slice()),
            Some(items("x.").as_slice())
        );
        // Same chat, revision and length: served from cache, not rescanned.
        assert_eq!(
            cache
                .latest("chat", 1, &one)
                .map(|write| write.items.as_slice()),
            Some(items("x.").as_slice())
        );
        let two = vec![
            one[0].clone(),
            entry(MessageRole::Assistant, vec![todo_part("b", items("xx"))]),
        ];
        assert_eq!(
            cache
                .latest("chat", 2, &two)
                .map(|write| write.items.as_slice()),
            Some(items("xx").as_slice())
        );
        // Another chat never reuses the first chat's list.
        assert_eq!(cache.latest("other", 2, &[]), None);
    }
}

#[cfg(test)]
mod composer_tests {
    use super::*;
    use crate::state::AppState;

    fn assistant_with_todo(list: Vec<TodoItem>) -> SessionMessageEntry {
        SessionMessageEntry {
            id: "m".into(),
            role: MessageRole::Assistant,
            parts: vec![MessagePart::Tool {
                id: "t".into(),
                call: ToolCall::Todo { items: list },
                is_error: false,
                resolved: true,
                output: None,
                diff: None,
                output_ref: None,
                output_bytes: None,
                diff_ref: None,
                diff_stats: None,
                subagent_ref: None,
                subagent_status: None,
                subagent_tail: None,
            }],
            created_at: 0,
            device_id: "d".into(),
            status: None,
            continuation_of: None,
            duration_ms: None,
            excluded: false,
        }
    }

    fn items(done: usize, total: usize) -> Vec<TodoItem> {
        (0..total)
            .map(|i| {
                TodoItem::new(
                    format!("step {i}"),
                    if i < done {
                        TodoStatus::Completed
                    } else {
                        TodoStatus::Pending
                    },
                )
            })
            .collect()
    }

    fn window(
        cx: &mut gpui::TestAppContext,
    ) -> (
        tempfile::TempDir,
        gpui::Entity<AppState>,
        gpui::WindowHandle<Composer>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::app_menus::init(cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx);
        });
        let state = cx.new(|_| AppState::new());
        let handle = {
            let state = state.clone();
            cx.add_window(move |_, cx| Composer::new(state, cx))
        };
        (dir, state, handle)
    }

    fn show_todo(
        state: &gpui::Entity<AppState>,
        handle: &gpui::WindowHandle<Composer>,
        chat: &str,
        entries: Vec<SessionMessageEntry>,
        cx: &mut gpui::TestAppContext,
    ) {
        state.update(cx, |state, _| {
            state.selected_chat = Some(chat.into());
            state.apply_transcript(entries);
        });
        handle
            .update(cx, |composer, _, _| composer.current_key = chat.into())
            .unwrap();
    }

    /// Whether the tray is on screen for the selected chat right now.
    fn rendered(handle: &gpui::WindowHandle<Composer>, cx: &mut gpui::TestAppContext) -> bool {
        handle
            .update(cx, |composer, window, cx| {
                composer.render_todo_panel(window, cx).is_some()
            })
            .unwrap()
    }

    fn replay(
        state: &gpui::Entity<AppState>,
        entries: Vec<SessionMessageEntry>,
        cx: &mut gpui::TestAppContext,
    ) {
        state.update(cx, |state, cx| {
            state
                .receive_transcript_update(
                    rcode_doc::TranscriptUpdate {
                        replay_baseline: Some(TranscriptBaseline::capture(&entries)),
                        frame: rcode_doc::TranscriptFrame::Reset { reset: entries },
                        context_usage: None,
                        goal: None,
                    },
                    cx,
                )
                .unwrap();
        });
    }

    #[gpui::test]
    fn removed_and_completed_trays_stay_hidden_on_repeated_history_updates(
        cx: &mut gpui::TestAppContext,
    ) {
        for dismissed in [true, false] {
            let (_dir, state, handle) = window(cx);
            show_todo(
                &state,
                &handle,
                "a",
                vec![assistant_with_todo(items(1, 3))],
                cx,
            );
            assert!(rendered(&handle, cx));
            if dismissed {
                handle
                    .update(cx, |composer, _, _| {
                        composer.todo_panels.get_mut("a").unwrap().dismiss()
                    })
                    .unwrap();
            } else {
                show_todo(
                    &state,
                    &handle,
                    "a",
                    vec![assistant_with_todo(items(3, 3))],
                    cx,
                );
            }
            assert!(!rendered(&handle, cx));
            replay(&state, vec![assistant_with_todo(items(0, 3))], cx);
            for _ in 0..3 {
                assert!(!rendered(&handle, cx));
            }
            // Navigation and another historical reset also preserve retirement.
            show_todo(&state, &handle, "b", vec![], cx);
            assert!(!rendered(&handle, cx));
            state.update(cx, |state, _| state.selected_chat = Some("a".into()));
            replay(&state, vec![assistant_with_todo(items(1, 3))], cx);
            for _ in 0..3 {
                assert!(!rendered(&handle, cx));
            }
            let mut new_work = assistant_with_todo(items(1, 3));
            new_work.id = "new-message".into();
            new_work.created_at = 1;
            show_todo(&state, &handle, "a", vec![new_work], cx);
            assert!(rendered(&handle, cx));
            handle
                .read_with(cx, |composer, _| {
                    assert!(!composer.todo_panels["a"].is_expanded())
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn backfilled_checklist_does_not_mount_a_new_tray(cx: &mut gpui::TestAppContext) {
        let (_dir, state, handle) = window(cx);
        show_todo(&state, &handle, "a", vec![], cx);
        assert!(!rendered(&handle, cx));
        replay(&state, vec![assistant_with_todo(items(0, 3))], cx);
        for _ in 0..3 {
            assert!(!rendered(&handle, cx));
        }
    }

    #[gpui::test]
    fn tray_appears_only_when_the_chat_has_a_todo(cx: &mut gpui::TestAppContext) {
        let (_dir, state, handle) = window(cx);
        show_todo(&state, &handle, "a", Vec::new(), cx);
        assert!(!rendered(&handle, cx));
        show_todo(
            &state,
            &handle,
            "a",
            vec![assistant_with_todo(items(1, 3))],
            cx,
        );
        assert!(rendered(&handle, cx));
        // The agent clearing its list (an empty write) takes the tray away.
        show_todo(
            &state,
            &handle,
            "a",
            vec![assistant_with_todo(Vec::new())],
            cx,
        );
        assert!(!rendered(&handle, cx));
    }

    #[gpui::test]
    fn new_tray_is_one_line_and_header_click_expands_it(cx: &mut gpui::TestAppContext) {
        let (_dir, state, handle) = window(cx);
        show_todo(
            &state,
            &handle,
            "a",
            vec![assistant_with_todo(items(0, 5))],
            cx,
        );
        let mut visual = gpui::VisualTestContext::from_window(handle.into(), cx);
        handle
            .update(&mut visual, |composer, window, cx| {
                window.resize(gpui::size(px(900.0), px(700.0)));
                composer.set_available_width(900.0, cx);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        visual.executor().advance_clock(Duration::from_millis(500));
        visual.run_until_parked();
        let compact = visual
            .debug_bounds("todo-panel")
            .expect("collapsed todo header");
        // The tray also reserves the strip hidden behind the input surface.
        assert!(
            compact.size.height <= px(ATTACHED_PANEL_HEIGHT),
            "{compact:?}"
        );
        visual.simulate_click(compact.center(), Default::default());
        visual.run_until_parked();
        visual.executor().advance_clock(Duration::from_millis(500));
        visual.run_until_parked();
        let expanded = visual
            .debug_bounds("todo-panel")
            .expect("expanded todo tray");
        assert!(
            expanded.size.height > compact.size.height + px(40.0),
            "{compact:?} -> {expanded:?}"
        );
        handle
            .read_with(&visual, |composer, _| {
                assert!(composer.todo_panels["a"].is_expanded())
            })
            .unwrap();
    }

    #[gpui::test]
    fn open_state_is_remembered_per_chat(cx: &mut gpui::TestAppContext) {
        let (_dir, state, handle) = window(cx);
        let entries = || vec![assistant_with_todo(items(1, 3))];
        show_todo(&state, &handle, "a", entries(), cx);
        assert!(rendered(&handle, cx));
        handle
            .update(cx, |composer, _, _| {
                assert!(!composer.todo_panels["a"].is_expanded());
                composer.todo_panels.get_mut("a").unwrap().toggle();
            })
            .unwrap();
        // Another chat starts from the automatic state, and A keeps its choice.
        show_todo(&state, &handle, "b", entries(), cx);
        assert!(rendered(&handle, cx));
        show_todo(&state, &handle, "a", entries(), cx);
        assert!(rendered(&handle, cx));
        handle
            .read_with(cx, |composer, _| {
                assert!(composer.todo_panels["a"].is_expanded());
                assert!(!composer.todo_panels["b"].is_expanded());
            })
            .unwrap();
    }

    #[gpui::test]
    fn finished_list_disappears_and_new_pending_work_reappears(cx: &mut gpui::TestAppContext) {
        let (_dir, state, handle) = window(cx);
        let finished = || vec![assistant_with_todo(items(3, 3))];
        show_todo(&state, &handle, "a", finished(), cx);
        assert!(!rendered(&handle, cx));

        // A new list (more work) is shown again.
        show_todo(
            &state,
            &handle,
            "a",
            vec![assistant_with_todo(items(3, 5))],
            cx,
        );
        assert!(rendered(&handle, cx));
    }
}
