//! Alternate presentation of the same model and option mutations.
use super::*;

pub(super) const COMPACT_WIDTH: f32 = 256.0;
/// One-line model rows (plus the 2px gap) so about seven fit at once.
pub(super) const COMPACT_ROW_HEIGHT: f32 = 32.0;
pub(super) const COMPACT_LIST_ROWS: f32 = 7.0;
/// The list fits its rows, up to [`COMPACT_LIST_ROWS`]; empty and loading
/// states keep room for a few skeleton rows.
pub(super) fn compact_list_height(rows: usize) -> f32 {
    let rows = if rows == 0 {
        4.0
    } else {
        (rows as f32).min(COMPACT_LIST_ROWS)
    };
    rows * (COMPACT_ROW_HEIGHT + popover::MENU_GAP) + 2.0 * popover::CARD_INSET
}
// A 28px rail under a 44x32 pill thumb that overhangs it by 2px, so the
// handle reads as the thing to grab. Stops and pointer input share the
// thumb's centre inset; the fill runs a rail radius past the centre, always
// hidden under the thumb.
const SLIDER_HEIGHT: f32 = 36.0;
const RAIL_HEIGHT: f32 = 28.0;
const RAIL_TOP: f32 = (SLIDER_HEIGHT - RAIL_HEIGHT) / 2.0;
const THUMB_WIDTH: f32 = 44.0;
const THUMB_HEIGHT: f32 = 32.0;
const THUMB_INSET: f32 = THUMB_WIDTH / 2.0;
const FILL_PAST: f32 = RAIL_HEIGHT / 2.0;
/// Both header buttons fill the header's height, so they sit one card inset
/// from the card's top and sides and their radius is concentric with its
/// corners. 8pt of padding around the two-line title puts its text 12pt
/// from the card's top and leading edges.
const HEADER_HEIGHT: f32 = 48.0;
/// A model without an effort ladder titles the header with its name alone.
const HEADER_HEIGHT_SINGLE: f32 = 36.0;
/// Flush with the card inset, a 15pt glyph centred in 32pt ends 12pt from
/// the card edge: the slider rail's end. The provider button mirrors it on
/// the leading side.
const FAST_BUTTON_WIDTH: f32 = 32.0;
/// The header inside the card inset, then the slider block and the space
/// above the option rows. The slider's 4pt below its box leaves the rail
/// 12pt from the card's bottom when no options follow.
fn header_block(effort: bool) -> f32 {
    2.0 * popover::CARD_INSET
        + if effort {
            HEADER_HEIGHT
        } else {
            HEADER_HEIGHT_SINGLE
        }
}
const SLIDER_TOP: f32 = 2.0;
const SLIDER_BOTTOM: f32 = 4.0;
const EFFORT_BLOCK: f32 = SLIDER_TOP + SLIDER_HEIGHT + SLIDER_BOTTOM;
const OPTIONS_GAP: f32 = 4.0;
/// The card's 1pt border, top and bottom, sits inside its height.
const CARD_BORDERS: f32 = 2.0;
/// The model list's chrome above the rows: the back button with the
/// provider tab strip (one row plus the card inset above and below, 40),
/// then the search row (40).
pub(super) const LIST_HEADER: f32 = 80.0;
/// Option rows (26pt plus a 2pt gap) all show up to this many, then scroll.
const OPTIONS_MAX_ROWS: usize = 6;
/// The rows' 4pt top inset, then the rows; the panel's inset closes below.
fn options_height(count: usize) -> f32 {
    2.0 + count.min(OPTIONS_MAX_ROWS) as f32 * 28.0
}

/// Option ids Cursor uses for an effort ladder (its models carry none).
const EFFORT_OPTION_IDS: [&str; 3] = ["effort", "reasoning", "reasoning_effort"];

pub(super) struct CompactEffort {
    pub labels: Vec<SharedString>,
    pub selected: usize,
    stops: EffortStops,
}

enum EffortStops {
    Reasoning(Vec<ReasoningLevel>),
    Option {
        id: String,
        choices: Vec<String>,
        default: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CompactPage {
    Panel,
    Models,
}

#[derive(Default)]
pub(super) struct CompactMotion {
    effort: Option<ScalarTransition>,
    press: Option<ScalarTransition>,
    drag_fraction: Option<f32>,
    /// The slider's visual clock is independent of provider Fast options.
    shimmer_phase: f32,
    shimmer_last_frame: Option<std::time::Instant>,
    fast: Option<ScalarTransition>,
    height: Option<ScalarTransition>,
    page: Option<CompactPage>,
    reveal: Option<ScalarTransition>,
    pub frame_height: f32,
}

pub(super) struct ScalarTransition {
    from: f32,
    to: f32,
    started: std::time::Instant,
}
impl ScalarTransition {
    pub(super) fn value(&self, now: std::time::Instant) -> f32 {
        let progress = motion::RESIZE.progress(
            now.duration_since(self.started).as_secs_f32()
                / motion::RESIZE
                    .total()
                    .mul_f32(motion::speed_scale())
                    .as_secs_f32(),
        );
        motion::lerp(self.from, self.to, progress)
    }
    pub(super) fn sample(
        state: &mut Option<Self>,
        target: f32,
        now: std::time::Instant,
        reduced: bool,
    ) -> (f32, bool) {
        let Some(transition) = state else {
            *state = Some(Self {
                from: target,
                to: target,
                started: now,
            });
            return (target, false);
        };
        if reduced {
            transition.from = target;
            transition.to = target;
            return (target, false);
        }
        if transition.to != target {
            transition.from = transition.value(now);
            transition.to = target;
            transition.started = now;
        }
        let value = transition.value(now);
        (value, (value - target).abs() > 0.001)
    }
}

impl Pickers {
    pub(super) fn render_compact_model_row(
        &mut self,
        ix: usize,
        row: &ModelRowData,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).for_popup();
        let selected = self.row_is_selected(row, cx);
        let (icon, tint) = harness_brand_icon(row.harness);
        // The brand mark names the provider; a description appears only to
        // tell identically named rows apart.
        let attribution: Option<SharedString> = row
            .model
            .description
            .as_deref()
            .map(str::trim)
            .filter(|d| row.ambiguous && !d.is_empty())
            .map(|d| SharedString::from(d.to_owned()));
        let details: SharedString = match row.model.description.as_deref() {
            Some(description) => format!(
                "{} · {}\n{}",
                row.model.label, row.harness_name, description
            )
            .into(),
            None => format!("{} · {}", row.model.label, row.harness_name).into(),
        };
        let accessible_name = details.clone();
        let item = div()
            .id(("model-row", ix))
            .role(gpui::Role::ListBoxOption)
            .aria_label(accessible_name)
            .aria_selected(selected)
            .tooltip(move |_, cx| cx.new(|_| PickerHint(details.clone())).into())
            .tooltip_show_delay(std::time::Duration::from_millis(500))
            .h(px(COMPACT_ROW_HEIGHT))
            .pl(px(8.0))
            .pr(px(4.0))
            .rounded(px(popover::MENU_ITEM_RADIUS))
            .flex()
            .items_center()
            .gap(px(if row.harness == HarnessId::Native {
                0.0
            } else {
                8.0
            }))
            .cursor_pointer()
            .text_color(theme.text)
            .when(selected, |el| {
                el.bg(crate::theme::card_selected_bg())
                    .shadow(crate::theme::card_selected_shadows())
            })
            .when(!selected && self.active == ix, |el| {
                el.bg(crate::theme::ink(0.05))
            })
            .when(self.compact_keyboard && self.active == ix, |el| {
                el.aria_active_descendant()
            })
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && (this.active != ix || this.compact_keyboard) {
                    this.compact_keyboard = false;
                    this.active = ix;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.activate_model_index(ix, cx)))
            .when(row.harness != HarnessId::Native, |item| {
                item.child(
                    crate::icons::icon(icon)
                        .size(px(14.0))
                        .flex_none()
                        .text_color(tint.unwrap_or(theme.text_muted)),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_baseline()
                    .gap(px(6.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .child(
                        self.render_model_name(ix, row, cx)
                            .max_w_full()
                            .font_weight(gpui::FontWeight::MEDIUM),
                    )
                    .when_some(attribution, |el, attribution| {
                        el.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(crate::typography::ui_rems(11.0))
                                .text_color(theme.text_muted)
                                .child(attribution),
                        )
                    }),
            )
            .child(model_selected_mark(ix, selected, theme.text));
        div()
            .pb(px(popover::MENU_GAP))
            .child(item)
            .into_any_element()
    }

    pub(super) fn compact_model_picker(&self, cx: &App) -> bool {
        self.title.is_none() && crate::settings::compact_model_picker(cx)
    }

    /// The parameter page only exists when it contains a usable control.
    pub(super) fn compact_panel_available(&self, cx: &App) -> bool {
        let hidden = self.compact_hidden_options(cx);
        self.compact_effort(cx).is_some()
            || self.compact_fast_choice(cx).is_some()
            || self.setting_groups(cx).iter().any(|group| Self::compact_option_visible(&group.id, &hidden))
    }

    pub(super) fn show_compact_models(&mut self, cx: &mut Context<Self>) {
        // Browse every offered provider, just as the standard picker's rail
        // allows. rail_descriptors still limits existing chats to their provider.
        // A foreign-provider row switches the provider before picking its model.
        let rail = if self.harness_locked(cx) {
            ModelRail::Harness
        } else {
            ModelRail::All
        };
        self.show_compact_list(
            rail,
            crate::l10n::localizer().text(TextKey::PickersSearchModels),
            cx,
        );
    }

    fn show_compact_list(&mut self, rail: ModelRail, placeholder: String, cx: &mut Context<Self>) {
        self.setting_menu = None;
        self.compact_model_list = true;
        self.model_rail = rail;
        self.reset_compact_search(placeholder, cx);
        // Open on the current model: its provider's group at the top when
        // the model sits near the start of it, else centred on the model.
        let selected = self.selected_model_index(cx);
        self.active = selected;
        let start = self
            .compact_groups(cx)
            .into_iter()
            .rev()
            .find(|(_, start)| *start <= selected)
            .map_or(0, |(_, start)| start);
        if selected - start < COMPACT_LIST_ROWS as usize - 2 {
            // One row of the previous group stays above, under the edge
            // fade, so the group's first row is never washed out and the
            // list shows there is more above.
            self.model_scroll.scroll_to_item_strict_with_offset(
                start,
                gpui::ScrollStrategy::Top,
                usize::from(start > 0),
            );
        } else {
            self.model_scroll
                .scroll_to_item_strict(selected, gpui::ScrollStrategy::Center);
        }
        self.focus_on_mount = true;
        cx.notify();
    }

    /// Back from the model list to the panel.
    pub(super) fn show_compact_panel(&mut self, cx: &mut Context<Self>) {
        if !self.compact_panel_available(cx) {
            self.animate_close(cx);
            return;
        }
        self.provider_menu.close();
        self.compact_model_list = false;
        self.focus_on_mount = true;
        cx.notify();
    }

    /// Clear the shared filter for a new page without letting the clear's
    /// Edited event reset the highlight anchored right after.
    fn reset_compact_search(&mut self, placeholder: String, cx: &mut Context<Self>) {
        self.search_reset_muted = !self.search.read(cx).text().is_empty();
        self.search.update(cx, |input, cx| {
            input.set_placeholder(placeholder, cx);
            if !input.text().is_empty() {
                input.set_text("", cx);
            }
        });
    }

    /// Picking a provider runs its last-used model at that model's last
    /// settings (the remembered defaults take over once the harness moves).
    pub(super) fn pick_compact_provider(&mut self, harness: HarnessId, cx: &mut Context<Self>) {
        self.model_rail = ModelRail::Harness;
        self.pick_harness(harness, cx);
        self.show_compact_panel(cx);
    }

    /// Where each provider's group starts in catalog order.
    pub(super) fn compact_groups(&self, cx: &App) -> Vec<(HarnessId, usize)> {
        let rows = self.model_rows(cx);
        let mut groups: Vec<(HarnessId, usize)> = Vec::new();
        for (ix, row) in rows.iter().enumerate() {
            let group = row.harness;
            if groups.last().is_none_or(|(last, _)| *last != group) {
                groups.push((group, ix));
            }
        }
        groups
    }

    /// The model list's top row: back to the panel, then the providers as
    /// tabs. Each tab jumps the list to its group, and the group at the top
    /// of the scroll lights its tab. The strip fades at both edges over the
    /// tabs scrolled past them.
    pub(super) fn compact_model_back_header(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let theme = Theme::of(cx).for_popup();
        let has_panel = self.compact_panel_available(cx);
        let searching = !self.search.read(cx).text().trim().is_empty();
        let groups = if searching {
            Vec::new()
        } else {
            self.compact_groups(cx)
        };
        // Rows share one pitch, so the scroll offset names the top row.
        let top = (f32::from(-self.model_scroll_base().offset().y)
            / (COMPACT_ROW_HEIGHT + popover::MENU_GAP))
            .round()
            .max(0.0) as usize;
        let current = groups
            .iter()
            .rev()
            .find(|(_, start)| *start <= top)
            .map(|(group, _)| *group);
        // Many providers overflow the strip: keep the viewed tab in sight
        // as the list scrolls, without fighting a manual sideways scroll.
        let viewed_ix = groups.iter().position(|(group, _)| current == Some(*group));
        if viewed_ix != self.compact_strip_viewed {
            self.compact_strip_viewed = viewed_ix;
            if let Some(ix) = viewed_ix {
                self.compact_strip_scroll.scroll_to_item(ix);
            }
        }
        let show_strip =
            !groups.is_empty() && !(groups.len() == 1 && groups[0].0 == HarnessId::Native);
        let strip = show_strip.then(|| {
            let tabs = div()
                .id("compact-group-strip")
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .items_center()
                .gap(px(popover::MENU_GAP))
                .overflow_x_scroll()
                .track_scroll(&self.compact_strip_scroll)
                .children(groups.into_iter().map(|(group, start)| {
                    let viewed = current == Some(group);
                    let (icon, tint) = harness_brand_icon(group);
                    let name: SharedString = self
                        .rail_descriptors(cx)
                        .into_iter()
                        .find(|d| d.id == group)
                        .map(|d| d.name)
                        .unwrap_or_default()
                        .into();
                    let hint = name.clone();
                    div()
                        .id(SharedString::from(format!("compact-group-{start}")))
                        .role(gpui::Role::Button)
                        .aria_label(crate::l10n::localizer().format_with(
                            TextKey::AuditSettingsModelsJump,
                            &[("name", name.as_ref())],
                        ))
                        .tooltip(move |_, cx| cx.new(|_| PickerHint(hint.clone())).into())
                        // A row-height square with the rows' corners, so the
                        // strip sits on the card's grid like the list below.
                        .flex_none()
                        .size(px(COMPACT_ROW_HEIGHT))
                        .rounded(px(popover::MENU_ITEM_RADIUS))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .when(viewed, |el| el.bg(crate::theme::ink(0.08)))
                        .when(!viewed, |el| {
                            el.opacity(0.7)
                                .hover(|s| s.bg(crate::theme::ink(0.05)).opacity(1.0))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.compact_keyboard = false;
                            this.active = start;
                            this.model_scroll
                                .scroll_to_item_strict(start, gpui::ScrollStrategy::Top);
                            cx.notify();
                        }))
                        .when(group != HarnessId::Native, |el| {
                            el.child(crate::icons::icon(icon).size(px(14.0)).text_color(
                                tint.unwrap_or(if viewed { theme.text } else { theme.text_muted }),
                            ))
                        })
                }));
            // Each side fades over the tabs hidden past it, up to one tab
            // pitch, reaching zero exactly at the clip edge.
            crate::edge_fade::edge_faded(COMPACT_ROW_HEIGHT + popover::MENU_GAP, false, false, tabs)
                .fade_left(true)
                .fade_right(true)
                .fade_scroll_x(&self.compact_strip_scroll)
        });
        // The card's inset on every side of one row-height strip: the same
        // 4pt edge and 32pt pitch as the list rows and the panel's buttons.
        div()
            .h(px(COMPACT_ROW_HEIGHT + 2.0 * popover::CARD_INSET))
            .flex_none()
            .px(px(popover::CARD_INSET))
            .flex()
            .items_center()
            .gap(px(popover::MENU_GAP))
            .child(
                div()
                    .id("compact-model-back")
                    .role(gpui::Role::Button)
                    .aria_label(crate::l10n::localizer().text(if has_panel { TextKey::ActionBack } else { TextKey::ShellProjectClose }))
                    .flex_none()
                    .size(px(COMPACT_ROW_HEIGHT))
                    .rounded(px(popover::MENU_ITEM_RADIUS))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(crate::theme::ink(0.05)))
                    .on_click(cx.listener(|this, _, _, cx| this.show_compact_panel(cx)))
                    .child(
                        crate::icons::icon(if has_panel { crate::icons::ALT_ARROW_LEFT } else { crate::icons::CLOSE_CIRCLE })
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    ),
            )
            .children(strip)
    }

    pub(super) fn compact_fast_choice(&self, cx: &App) -> Option<(String, String, bool, bool)> {
        let (option, on, off) = self
            .selected_model(cx)?
            .options
            .iter()
            .find_map(|o| fast_mode_values(o).map(|(on, off)| (o, on, off)))?;
        let options = self.explicit_options(cx);
        let fast = options
            .get(&option.id)
            .and_then(|v| v.as_str())
            .unwrap_or(&option.default_choice)
            == on;
        let next = if fast { off } else { on };
        Some((
            option.id.clone(),
            next.to_owned(),
            next == option.default_choice,
            fast,
        ))
    }

    /// The selected model's fast-mode option id, whichever form it takes.
    pub(super) fn fast_option_id(&self, cx: &App) -> Option<String> {
        self.selected_model(cx)?
            .options
            .iter()
            .find(|o| fast_mode_values(o).is_some())
            .map(|o| o.id.clone())
    }

    /// The panel's title for a [`ModelName`]. Loading also draws a ghost
    /// bar in the name's place, as the composer chip does.
    fn compact_title_text(name: &ModelName) -> SharedString {
        match name {
            ModelName::Named(label) => label.clone(),
            ModelName::Loading => crate::l10n::localizer()
                .text(TextKey::AuditModelsLoading)
                .into(),
            ModelName::None { no_agents: true } => crate::l10n::localizer()
                .text(TextKey::AuditModelsNoAgents)
                .into(),
            ModelName::None { .. } => crate::l10n::localizer()
                .text(TextKey::AuditModelsSelect)
                .into(),
        }
    }

    /// The slider's stops: the model's reasoning ladder, else an option
    /// shaped like one (Cursor's `effort`/`reasoning` choices), so every
    /// model with an effort gets the same slider.
    pub(super) fn compact_effort(&self, cx: &App) -> Option<CompactEffort> {
        if self.model_unavailable(cx) {
            return None;
        }
        let levels = self.trait_ladder(cx);
        if !levels.is_empty() {
            let current = self.effective_reasoning(cx);
            return Some(CompactEffort {
                labels: levels.iter().map(|l| reasoning_label(*l).into()).collect(),
                selected: levels.iter().position(|l| Some(*l) == current).unwrap_or(0),
                stops: EffortStops::Reasoning(levels),
            });
        }
        let option = self
            .selected_model(cx)?
            .options
            .iter()
            .find(|o| EFFORT_OPTION_IDS.contains(&o.id.as_str()) && o.choices.len() > 1)?;
        let options = self.explicit_options(cx);
        let current = options
            .get(&option.id)
            .and_then(|v| v.as_str())
            .unwrap_or(&option.default_choice);
        Some(CompactEffort {
            labels: option
                .choices
                .iter()
                .map(|c| {
                    SharedString::from(labels::choice_label(
                        &option.id,
                        &c.id,
                        &c.label,
                        crate::l10n::localizer(),
                    ))
                })
                .collect(),
            selected: option
                .choices
                .iter()
                .position(|c| c.id == current)
                .unwrap_or(0),
            stops: EffortStops::Option {
                id: option.id.clone(),
                choices: option.choices.iter().map(|c| c.id.clone()).collect(),
                default: option.default_choice.clone(),
            },
        })
    }

    /// Option ids the compact card draws as its own controls, not rows.
    pub(super) fn compact_hidden_options(&self, cx: &App) -> Vec<String> {
        let effort = match self.compact_effort(cx).map(|e| e.stops) {
            Some(EffortStops::Option { id, .. }) => Some(id),
            _ => None,
        };
        self.fast_option_id(cx).into_iter().chain(effort).collect()
    }

    pub(super) fn pick_effort_at(&mut self, x: gpui::Pixels, cx: &mut Context<Self>) {
        let Some(bounds) = self.effort_bounds else {
            return;
        };
        let Some(effort) = self.compact_effort(cx) else {
            return;
        };
        let count = effort.labels.len();
        let Some(index) = effort_index(
            f32::from(x - bounds.left()),
            f32::from(bounds.size.width),
            count,
        ) else {
            return;
        };
        if self.effort_dragging {
            let fraction =
                effort_fraction(f32::from(x - bounds.left()), f32::from(bounds.size.width));
            self.compact_motion.drag_fraction = Some(if count > 1 { fraction } else { 0.0 });
            cx.notify();
        }
        self.pick_compact_effort(effort, index, cx);
    }

    /// Effort and fast mode have their own controls in the compact card.
    pub(super) fn compact_option_visible(id: &ModelSetting, hidden: &[String]) -> bool {
        !matches!(id, ModelSetting::Reasoning)
            && !matches!(id, ModelSetting::Option(id) if hidden.contains(id))
    }

    pub(super) fn pick_compact_effort(
        &mut self,
        effort: CompactEffort,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if index == effort.selected {
            return;
        }
        match effort.stops {
            EffortStops::Reasoning(levels) => {
                let Some(level) = levels.get(index).copied() else {
                    return;
                };
                self.pick_reasoning(level, cx);
            }
            EffortStops::Option {
                id,
                choices,
                default,
            } => {
                let Some(choice) = choices.get(index).cloned() else {
                    return;
                };
                let is_default = choice == default;
                self.pick_option(id, choice, is_default, cx);
            }
        }
        crate::haptics::selection_step();
    }

    pub(super) fn render_compact_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Catalog updates can remove the selected model's last setting while
        // the menu is open. Keep navigation on the actual model list.
        if !self.compact_model_list && !self.compact_panel_available(cx) {
            self.compact_model_list = true;
            self.focus_on_mount = true;
        }
        let hidden = self.compact_hidden_options(cx);
        let options = self
            .setting_groups(cx)
            .iter()
            .filter(|g| Self::compact_option_visible(&g.id, &hidden))
            .count();
        let effort = self.compact_effort(cx).is_some();
        let panel_height = CARD_BORDERS
            + header_block(effort && self.mounted_kind() != Some(PickerKind::Effort))
            + if effort { EFFORT_BLOCK } else { 0.0 }
            + if options == 0 {
                0.0
            } else {
                OPTIONS_GAP + options_height(options)
            };
        let page = if self.compact_model_list {
            CompactPage::Models
        } else {
            CompactPage::Panel
        };
        let target = self.menu_geometry().height.min(match page {
            CompactPage::Models => {
                if self.provider_mode(cx) {
                    self.provider_menu_height(cx)
                } else {
                    LIST_HEADER + compact_list_height(self.displayed_model_count(cx)) + CARD_BORDERS
                }
            }
            CompactPage::Panel => panel_height,
        });
        let (height, moving) = ScalarTransition::sample(
            &mut self.compact_motion.height,
            target,
            std::time::Instant::now(),
            cx.reduce_motion(),
        );
        self.compact_motion.frame_height = height;
        if moving {
            window.request_animation_frame();
        }
        let content = match page {
            CompactPage::Models => self.render_harness_model_popover(cx),
            CompactPage::Panel => self.render_compact_model_panel(window, cx),
        };
        let now = std::time::Instant::now();
        if self.compact_motion.page.is_some_and(|last| last != page) {
            self.compact_motion.reveal = Some(ScalarTransition {
                from: 0.0,
                to: 1.0,
                started: now,
            });
        }
        self.compact_motion.page = Some(page);
        let (reveal, revealing) = ScalarTransition::sample(
            &mut self.compact_motion.reveal,
            1.0,
            now,
            cx.reduce_motion(),
        );
        if revealing {
            window.request_animation_frame();
        }
        // The new page fades in while the card resizes to it: the swap
        // never shows at full strength and nothing slides sideways against
        // the vertical resize. Initial opening uses the popover entrance.
        let content = div().opacity(reveal).child(content).into_any_element();
        self.popover_frame_flush(COMPACT_WIDTH, content, cx)
    }

    /// The panel's shortcuts, live only while it is the picker's page: Up
    /// and Down open the model list on the model beside the selected one,
    /// Tab cycles providers, Left/Right (Home/End) set the effort, F toggles
    /// fast mode.
    pub(super) fn compact_panel_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        self.compact_keyboard = true;
        match event.keystroke.key.as_str() {
            "escape" => self.animate_close(cx),
            "f" if !event.keystroke.modifiers.modified() => {
                let Some((option, choice, default, _)) = self.compact_fast_choice(cx) else {
                    return;
                };
                self.pick_option(option, choice, default, cx);
            }
            "up" | "down" => {
                self.show_compact_models(cx);
                let delta = if event.keystroke.key == "up" { -1 } else { 1 };
                if let Some(next) =
                    popover::menu_step(Some(self.active), self.model_rows_len(cx), delta)
                {
                    self.active = next;
                    self.model_scroll
                        .scroll_to_item(next, gpui::ScrollStrategy::Nearest);
                }
            }
            "enter" | "space" => self.show_compact_models(cx),
            "tab" => {
                let delta = if event.keystroke.modifiers.shift {
                    -1
                } else {
                    1
                };
                self.cycle_compact_provider(delta, cx);
            }
            "left" | "right" | "home" | "end" => {
                if let Some(effort) = self.compact_effort(cx) {
                    let last = effort.labels.len().saturating_sub(1);
                    let next = match event.keystroke.key.as_str() {
                        "left" => effort.selected.saturating_sub(1),
                        "right" => (effort.selected + 1).min(last),
                        "home" => 0,
                        _ => last,
                    };
                    self.pick_compact_effort(effort, next, cx);
                }
            }
            _ => return,
        }
        cx.notify();
        cx.stop_propagation();
    }

    /// Tab on the panel: the next provider on offer, wrapping, at its
    /// last-used model. A chat's fixed provider stays put.
    fn cycle_compact_provider(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.harness_locked(cx) {
            return;
        }
        let providers = self.rail_descriptors(cx);
        let current = self
            .effective_harness(cx)
            .and_then(|harness| providers.iter().position(|d| d.id == harness));
        if let Some(next) = popover::menu_step(current, providers.len(), delta) {
            self.pick_compact_provider(providers[next].id, cx);
        }
    }

    pub(super) fn render_compact_model_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).for_popup();
        let effort_only = self.mounted_kind() == Some(PickerKind::Effort);
        let (levels, selected) = self
            .compact_effort(cx)
            .map_or((Vec::new(), 0), |e| (e.labels, e.selected));
        let name = self.model_name(cx);
        let label = if self.needs_model_setup(cx) && self.effective_model_id(cx).is_none() {
            crate::l10n::localizer()
                .text(TextKey::PickersAddModel)
                .into()
        } else {
            Self::compact_title_text(&name)
        };
        let label = self.model_chip_label(label, cx);
        let name_element: AnyElement = if name == ModelName::Loading {
            popover::skeleton_bar(72.0, cx.entity_id(), cx)
        } else {
            div()
                .min_w_0()
                .truncate()
                .child(label.clone())
                .into_any_element()
        };
        let effort: SharedString = levels.get(selected).cloned().unwrap_or_else(|| {
            crate::l10n::localizer()
                .text(TextKey::PickersDefault)
                .into()
        });
        let model_accessible: SharedString = format!(
            "{} · {} · {}",
            effort,
            label,
            crate::l10n::localizer().text(TextKey::PickersChangeModel)
        )
        .into();
        let now = std::time::Instant::now();
        let reduced = cx.reduce_motion();
        // The Fast-style treatment is a local visual preference, not a model option.
        let intensity = 1.0;
        let palette = crate::settings::effort_theme(cx).palette(&theme);
        let dt = self
            .compact_motion
            .shimmer_last_frame
            .replace(now)
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32().min(0.05));
        let alive =
            !levels.is_empty() && !reduced && window.is_window_active() && self.open.is_open();
        if alive {
            self.compact_motion.shimmer_phase =
                (self.compact_motion.shimmer_phase + dt * (0.32 + 0.22 * intensity)).fract();
            window.request_animation_frame();
        }
        let phase = self.compact_motion.shimmer_phase;
        let breath = if alive {
            0.5 + 0.5 * (std::f32::consts::TAU * phase).sin()
        } else {
            0.5
        };
        // The effort titles the model's name as one button filling the row
        // up to fast mode, its text on the rail's and option rows' edge; fast
        // mode is a small icon button whose glyph ends where the rail does.
        // The chevron leans toward the list on hover.
        let model_key: SharedString = format!("compact-model-link-{}", cx.entity_id()).into();
        let hover = motion::hover_t(&model_key);
        let lean = motion::EASE_OUT_QUINT.eval(hover);
        let title = div()
            .id("compact-select-model")
            .debug_selector(|| "compact-select-model".into())
            .role(gpui::Role::Button)
            .aria_label(model_accessible)
            .flex_1()
            .min_w_0()
            .h_full()
            .px(px(8.0))
            .rounded(px(popover::MENU_ITEM_RADIUS))
            .bg(crate::theme::ink(0.05 * hover))
            .flex()
            .flex_col()
            .items_start()
            .justify_center()
            .cursor_pointer()
            .on_hover(motion::hover_listener(model_key))
            .on_click(cx.listener(|this, _, window, cx| {
                if this.open_kind() == Some(PickerKind::Effort) {
                    this.toggle(PickerKind::HarnessModel, window, cx);
                    cx.stop_propagation();
                    return;
                }
                if this.needs_model_setup(cx) {
                    this.toggle(PickerKind::HarnessModel, window, cx);
                    return;
                }
                this.compact_keyboard = false;
                this.show_compact_models(cx);
            }))
            .when(!levels.is_empty(), |el| {
                // The slider retitles the panel as it drags; the level name
                // rolls between values like the composer chips.
                el.child(
                    div()
                        .text_size(crate::typography::ui_rems(14.0))
                        .line_height(px(17.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(crate::roll_text::roll_text(
                            format!("compact-effort-title-{}", cx.entity_id()),
                            effort.clone(),
                            reduced,
                        )),
                )
            })
            .child(
                div()
                    .max_w_full()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(3.0))
                    .map(|el| {
                        if levels.is_empty() {
                            // The name is the whole title: no effort above it.
                            el.text_size(crate::typography::ui_rems(14.0))
                                .line_height(px(17.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(theme.text)
                        } else {
                            el.text_size(crate::typography::ui_rems(12.0))
                                .line_height(px(15.0))
                                .text_color(motion::mix(theme.text_muted, theme.text, hover))
                        }
                    })
                    .child(name_element)
                    .child(
                        // Leans 3pt toward the list and firms up on hover.
                        div()
                            .flex_none()
                            .relative()
                            .left(px(3.0 * lean))
                            .opacity(0.55 + 0.45 * lean)
                            .child(
                                crate::icons::icon(crate::icons::ALT_ARROW_RIGHT)
                                    .size(px(10.0))
                                    .text_color(motion::mix(theme.text_muted, theme.text, lean)),
                            ),
                    ),
            );
        let title = if effort_only {
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .px(px(8.0))
                .flex()
                .items_center()
                .text_size(crate::typography::ui_rems(14.0))
                .line_height(px(17.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(crate::roll_text::roll_text(
                    format!("compact-effort-title-{}", cx.entity_id()),
                    effort.clone(),
                    reduced,
                ))
                .into_any_element()
        } else {
            title.into_any_element()
        };
        let mut fast_button = None;
        if effort_only {
            // Reserve the original Fast affordance without changing model options.
            fast_button = Some(
                div()
                    .id("compact-fast-preview")
                    .debug_selector(|| "compact-fast-preview".into())
                    .role(gpui::Role::Image)
                    .aria_label(crate::l10n::localizer().text(TextKey::PickersFastMode))
                    .w(px(FAST_BUTTON_WIDTH))
                    .h_full()
                    .flex_none()
                    .rounded(px(popover::MENU_ITEM_RADIUS))
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        crate::icons::icon(crate::icons::FAST_TIER)
                            .size(px(15.0))
                            .text_color(theme.text_muted),
                    ),
            );
        } else if let Some((option, choice, default, fast)) = self.compact_fast_choice(cx) {
            let (fast_t, fading) = ScalarTransition::sample(
                &mut self.compact_motion.fast,
                if fast { 1.0 } else { 0.0 },
                now,
                reduced,
            );
            if fading {
                window.request_animation_frame();
            }
            let fast_key: SharedString = format!("compact-fast-{}", cx.entity_id()).into();
            let fast_hover = motion::hover_t(&fast_key);
            fast_button = Some(
                div()
                    .id("compact-fast")
                    .role(gpui::Role::Button)
                    .aria_label(crate::l10n::localizer().text(TextKey::PickersFastMode))
                    .aria_toggled(if fast {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .tooltip(move |_, cx| {
                        cx.new(|_| {
                            PickerHint(
                                crate::l10n::localizer()
                                    .text(if fast {
                                        TextKey::AuditSettingsModelsFastOn
                                    } else {
                                        TextKey::AuditSettingsModelsFastOff
                                    })
                                    .into(),
                            )
                        })
                        .into()
                    })
                    // Bare like the model button beside it: a plate only on
                    // hover. The glyph's colour is the state, and the
                    // slider's fill carries the accent.
                    .w(px(FAST_BUTTON_WIDTH))
                    .h_full()
                    .flex_none()
                    .rounded(px(popover::MENU_ITEM_RADIUS))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .bg(crate::theme::ink(0.05 * fast_hover))
                    .on_hover(motion::hover_listener(fast_key))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.compact_keyboard = false;
                        this.pick_option(option.clone(), choice.clone(), default, cx);
                    }))
                    .child({
                        let color = motion::mix(
                            motion::mix(theme.text_muted, theme.text, fast_hover),
                            theme.accent,
                            fast_t,
                        );
                        // On fills the bolt: the solid twin fades in over the
                        // outline, which shares its silhouette.
                        div()
                            .relative()
                            .size(px(15.0))
                            .child(
                                crate::icons::icon(crate::icons::FAST_TIER)
                                    .size(px(15.0))
                                    .text_color(color),
                            )
                            .when(fast_t > 0.0, |el| {
                                el.child(
                                    crate::icons::icon(crate::icons::FAST_TIER_BOLD)
                                        .absolute()
                                        .top_0()
                                        .left_0()
                                        .size(px(15.0))
                                        .text_color(color.opacity(fast_t)),
                                )
                            })
                    }),
            );
        }
        let header_height = if levels.is_empty() || effort_only {
            HEADER_HEIGHT_SINGLE
        } else {
            HEADER_HEIGHT
        };
        let header = div()
            .h(px(header_height))
            .flex_none()
            .flex()
            .gap(px(popover::CARD_INSET))
            .child(title)
            .children(fast_button);
        let mut panel = div()
            .p(px(popover::CARD_INSET))
            .flex()
            .flex_col()
            .child(header);
        if !levels.is_empty() {
            let target = if levels.len() > 1 {
                selected as f32 / (levels.len() - 1) as f32
            } else {
                0.0
            };
            let (mut fraction, moving) =
                ScalarTransition::sample(&mut self.compact_motion.effort, target, now, reduced);
            if self.effort_dragging {
                fraction = self.compact_motion.drag_fraction.unwrap_or(target);
                // Keep the release animation rooted at the actual pointer, not the previous stop.
                self.compact_motion.effort = Some(ScalarTransition {
                    from: fraction,
                    to: fraction,
                    started: now,
                });
            }
            if moving {
                window.request_animation_frame();
            }
            let (press, pressing) = ScalarTransition::sample(
                &mut self.compact_motion.press,
                if self.effort_dragging { 1.0 } else { 0.0 },
                now,
                reduced,
            );
            if pressing {
                window.request_animation_frame();
            }
            let entity = cx.entity().downgrade();
            let drag_entity = entity.clone();
            let slider = div()
                .id("compact-effort-slider")
                .debug_selector(|| "compact-effort-slider".into())
                .role(gpui::Role::Slider)
                .aria_label(crate::l10n::localizer().text(TextKey::PickersReasoningEffort))
                .aria_value(effort.clone())
                .aria_numeric_value(selected as f64)
                .aria_min_numeric_value(0.0)
                .aria_max_numeric_value(levels.len().saturating_sub(1) as f64)
                .aria_numeric_value_step(1.0)
                .aria_orientation(gpui::Orientation::Horizontal)
                .aria_description(crate::l10n::localizer().text(TextKey::PickersReasoningKeys))
                .relative()
                .h(px(SLIDER_HEIGHT))
                .cursor_pointer()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        this.compact_keyboard = false;
                        this.active = NO_ACTIVE_ROW;
                        this.effort_dragging = true;
                        this.pick_effort_at(event.position.x, cx);
                        cx.notify();
                        cx.stop_propagation();
                    }),
                )
                .child(
                    gpui::canvas(
                        move |bounds, _, cx| {
                            let _ = entity.update(cx, |this, _| this.effort_bounds = Some(bounds));
                        },
                        move |_, _, window, _| {
                            let release = drag_entity.clone();
                            window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
                                if phase == gpui::DispatchPhase::Bubble {
                                    let _ = release.update(cx, |this, cx| {
                                        if this.effort_dragging {
                                            this.effort_dragging = false;
                                            this.compact_motion.drag_fraction = None;
                                            cx.notify();
                                        }
                                    });
                                }
                            });
                            let entity = drag_entity.clone();
                            window.on_mouse_event(
                                move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                                    if phase == gpui::DispatchPhase::Bubble
                                        && event.pressed_button == Some(gpui::MouseButton::Left)
                                    {
                                        let _ = entity.update(cx, |this, cx| {
                                            if this.effort_dragging {
                                                this.pick_effort_at(event.position.x, cx);
                                            }
                                        });
                                    }
                                },
                            );
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
                .child(crate::glass::light(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(px(RAIL_TOP))
                        .h(px(RAIL_HEIGHT))
                        .rounded_full(),
                    &theme,
                    1.0,
                ))
                // A contained glow: it must not wash over the option rows.
                .child(effort_fill(
                    &theme,
                    fraction,
                    levels.len(),
                    Shimmer {
                        intensity,
                        phase,
                        breath,
                        maximum: levels.len() > 1 && selected == levels.len() - 1,
                        palette,
                    },
                ))
                .child({
                    // A plain handle that keeps its size; a press only lifts
                    // it on a softer, wider drop.
                    let (w, h) = (THUMB_WIDTH, THUMB_HEIGHT);
                    // The visual theme breathes a soft halo under the thumb.
                    let live = intensity * (0.55 + 0.45 * breath);
                    let place = |el: gpui::Div| {
                        el.absolute()
                            .left(gpui::relative(fraction))
                            .ml(px(-w / 2.0))
                            .w(px(w))
                            .h(px(h))
                            .rounded_full()
                    };
                    div()
                        .absolute()
                        .left(px(THUMB_INSET))
                        .right(px(THUMB_INSET))
                        .top(px((SLIDER_HEIGHT - h) / 2.0))
                        .when(intensity > 0.001, |el| {
                            el.child(place(div()).shadow(vec![gpui::BoxShadow {
                                color: if palette.light {
                                    gpui::black().opacity(0.12 * live)
                                } else {
                                    palette.color_at(1.0).opacity(0.35 * live)
                                },
                                offset: gpui::point(px(0.0), px(1.0)),
                                blur_radius: px(10.0),
                                spread_radius: px(0.0),
                                inset: false,
                            }]))
                        })
                        .when(press > 0.001, |el| {
                            el.child(place(div()).shadow(vec![gpui::BoxShadow {
                                color: gpui::black().opacity(0.12 * press),
                                offset: gpui::point(px(0.0), px(2.0 * press)),
                                blur_radius: px(8.0),
                                spread_radius: px(0.0),
                                inset: false,
                            }]))
                        })
                        .child(crate::glass::thumb(place(div()), &theme))
                });
            // The rail starts on the same edge as the title and row labels.
            panel = panel.child(
                div()
                    .px(px(8.0))
                    .pt(px(SLIDER_TOP))
                    .pb(px(SLIDER_BOTTOM))
                    .child(slider),
            );
        }
        let hidden = self.compact_hidden_options(cx);
        let option_count = self
            .setting_groups(cx)
            .iter()
            .filter(|g| Self::compact_option_visible(&g.id, &hidden))
            .count();
        if option_count > 0 {
            let options = self.render_traits_sections(cx);
            let scrollbar = popover::rail(self, "compact-options-scrollbar", &theme, cx);
            let chrome = CARD_BORDERS
                + header_block(!levels.is_empty() && !effort_only)
                + if levels.is_empty() { 0.0 } else { EFFORT_BLOCK }
                + OPTIONS_GAP;
            // Space, not a rule, separates the options from the effort group.
            panel = panel.child(
                div().mt(px(OPTIONS_GAP)).child(
                    popover::menu_scroll_host("compact-options-host")
                        .on_hover(cx.listener(Self::on_menu_list_hover))
                        .child(popover::faded_menu_list(
                            &self.menu_scroll,
                            popover::menu_scroll_list("compact-options", &self.menu_scroll)
                                .max_h(px(options_height(option_count)
                                    .min((self.menu_geometry().height - chrome).max(0.0))))
                                .child(options),
                        ))
                        .children(scrollbar),
                ),
            );
        }
        panel.into_any_element()
    }
}

/// The accent glass fill from the rail's start to just past the thumb, with
/// the stops painted over rail and fill in one coordinate space.
#[derive(Clone, Copy)]
struct Shimmer {
    /// Visual strength; independent of the provider's Fast mode.
    intensity: f32,
    /// 0–1 position of the band's sweep.
    phase: f32,
    /// 0–1 breathing of the glow.
    breath: f32,
    maximum: bool,
    palette: crate::effort_theme::EffortPalette,
}

fn effort_fill(theme: &Theme, fraction: f32, count: usize, shimmer: Shimmer) -> AnyElement {
    let Shimmer {
        intensity,
        phase,
        breath,
        maximum,
        palette,
    } = shimmer;
    // A contained glow that breathes; it must not wash over the option rows.
    let mut fill_theme = theme.clone();
    fill_theme.accent_strong = palette.colors[0];
    let fill = crate::glass::accent_plate(&fill_theme, 1.0, intensity * (0.2 + 0.25 * breath));
    // Quiet stops: a hint of the ladder, not a second focal point.
    let filled_stop = palette
        .spark()
        .opacity(if palette.light { 0.45 } else { 0.55 });
    let open_stop = theme.text_muted.opacity(0.28);
    gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let width = f32::from(bounds.size.width);
            let run = (width - 2.0 * THUMB_INSET).max(0.0);
            let center = THUMB_INSET + fraction * run;
            let rect = |x: f32, y: f32, w: f32, h: f32| {
                gpui::Bounds::new(
                    bounds.origin + gpui::point(px(x), px(y)),
                    gpui::size(px(w), px(h)),
                )
            };
            fill.paint(
                window,
                rect(0.0, RAIL_TOP, (center + FILL_PAST).min(width), RAIL_HEIGHT),
                RAIL_HEIGHT / 2.0,
            );
            // A band of light flows through the whole fill toward
            // the thumb. Two slow waves drift at different speeds, shading
            // the fill between a lifted accent and a hue-shifted one, and the
            // light gathers just before the thumb. It is painted in thin
            // strips whose gradients join end to end; the end strips carry
            // the fill's corner radius, so it never leaves the rounded ends.
            let fill_end = (center + FILL_PAST).min(width);
            if intensity > 0.001 && fill_end > RAIL_HEIGHT {
                let inset = 1.0;
                let height = RAIL_HEIGHT - 2.0 * inset;
                let radius = height / 2.0;
                let (from, to) = (inset, fill_end - inset);
                let tau = std::f32::consts::TAU;
                let flow = |x: f32| -> gpui::Hsla {
                    let base = palette.color_at((x - from) / (to - from).max(1.0));
                    let slow = 0.5 + 0.5 * (tau * (x / 150.0 - 2.0 * phase)).sin();
                    let quick = 0.5 + 0.5 * (tau * (x / 64.0 - 3.0 * phase) + 1.7).sin();
                    let glow = (0.65 * slow + 0.35 * quick).powi(2);
                    let charge = (-(center - x).max(0.0) / 40.0).exp() * (0.6 + 0.4 * breath);
                    let lead = ((x - from) / 28.0).clamp(0.0, 1.0);
                    let alpha = intensity * ((0.08 + 0.34 * glow) * lead + 0.22 * charge);
                    motion::mix(base, palette.spark(), alpha.clamp(0.0, 0.55))
                };
                let mut edges = vec![from, (from + radius).min(to)];
                let inner_end = (to - radius).max(edges[1]);
                let mut x = edges[1];
                while x + 4.0 < inner_end {
                    x += 4.0;
                    edges.push(x);
                }
                if inner_end > *edges.last().unwrap() {
                    edges.push(inner_end);
                }
                if to > *edges.last().unwrap() {
                    edges.push(to);
                }
                let last = edges.len().saturating_sub(2);
                for (i, pair) in edges.windows(2).enumerate() {
                    let (x0, x1) = (pair[0], pair[1]);
                    if x1 - x0 < 0.1 {
                        continue;
                    }
                    let r = px(radius.min(x1 - x0));
                    let zero = px(0.0);
                    let corners = gpui::Corners {
                        top_left: if i == 0 { r } else { zero },
                        bottom_left: if i == 0 { r } else { zero },
                        top_right: if i == last { r } else { zero },
                        bottom_right: if i == last { r } else { zero },
                    };
                    window.paint_quad(gpui::quad(
                        rect(x0, RAIL_TOP + inset, x1 - x0, height),
                        corners,
                        gpui::linear_gradient(
                            90.0,
                            gpui::linear_color_stop(flow(x0), 0.0),
                            gpui::linear_color_stop(flow(x1), 1.0),
                        ),
                        px(0.0),
                        gpui::transparent_black(),
                        gpui::BorderStyle::default(),
                    ));
                }

                // Short, deterministic speed streaks stay inside the filled capsule.
                let spark = palette.spark();
                let start = RAIL_HEIGHT / 2.0;
                let end = fill_end - RAIL_HEIGHT / 2.0;
                let span = end - start;
                if span > 10.0 {
                    let streaks = ((span / 32.0) as usize).clamp(2, 8);
                    for index in 0..streaks {
                        let seed = index as f32;
                        let progress = (phase * (3.0 + seed % 3.0) + seed * 0.381).fract();
                        let head = start + span * progress;
                        let length = (10.0 + seed % 3.0 * 4.0).min(head - start);
                        let fade = (progress * 8.0).min((1.0 - progress) * 8.0).clamp(0.0, 1.0);
                        if length > 1.0 {
                            window.paint_quad(gpui::quad(
                                rect(
                                    head - length,
                                    RAIL_TOP + 7.0 + (seed * 7.0) % 15.0,
                                    length,
                                    1.2,
                                ),
                                px(0.6),
                                gpui::linear_gradient(
                                    90.0,
                                    gpui::linear_color_stop(spark.opacity(0.0), 0.0),
                                    gpui::linear_color_stop(spark.opacity(0.4 * fade), 1.0),
                                ),
                                px(0.0),
                                gpui::transparent_black(),
                                gpui::BorderStyle::default(),
                            ));
                        }
                    }
                    if maximum {
                        let particles = ((span / 14.0) as usize).clamp(4, 18);
                        for index in 0..particles {
                            let seed = index as f32;
                            let progress = (phase * (1.0 + seed % 2.0) + seed * 0.618).fract();
                            let fade = (progress * 10.0)
                                .min((1.0 - progress) * 10.0)
                                .clamp(0.0, 1.0);
                            let opacity =
                                (0.18 + 0.28 * (seed * 1.7 + phase * tau).sin().abs()) * fade;
                            window.paint_quad(gpui::quad(
                                rect(
                                    start + span * progress,
                                    RAIL_TOP + 5.0 + (seed * 11.7) % 18.0,
                                    2.0,
                                    2.0,
                                ),
                                px(1.0),
                                spark.opacity(opacity),
                                px(0.0),
                                gpui::transparent_black(),
                                gpui::BorderStyle::default(),
                            ));
                        }
                    }
                    // A brief lightning accent, inset from the capsule's rounded ends.
                    let flash = (phase * 2.0).fract();
                    if flash < 0.16 {
                        let alpha = (flash / 0.16 * std::f32::consts::PI).sin() * 0.65;
                        let x = start + span * 0.62;
                        let mut bolt = gpui::PathBuilder::stroke(px(1.2));
                        let point = |x, y| bounds.origin + gpui::point(px(x), px(y));
                        bolt.move_to(point(x + 3.0, RAIL_TOP + 6.0));
                        bolt.line_to(point(x - 2.0, RAIL_TOP + 14.0));
                        bolt.line_to(point(x + 2.0, RAIL_TOP + 14.0));
                        bolt.line_to(point(x - 3.0, RAIL_TOP + 22.0));
                        if let Ok(bolt) = bolt.build() {
                            window.paint_path(bolt, spark.opacity(alpha));
                        }
                    }
                }
            }
            for i in 0..count {
                let stop = if count > 1 {
                    i as f32 / (count - 1) as f32
                } else {
                    0.0
                };
                let x = THUMB_INSET + stop * run;
                // Filled stops follow the animated front, never jump ahead of it.
                let color = if stop <= fraction {
                    filled_stop
                } else {
                    open_stop
                };
                window.paint_quad(gpui::quad(
                    rect(x - 2.0, SLIDER_HEIGHT / 2.0 - 2.0, 4.0, 4.0),
                    px(2.0),
                    color,
                    px(0.0),
                    gpui::transparent_black(),
                    gpui::BorderStyle::default(),
                ));
            }
        },
    )
    .absolute()
    .inset_0()
    .into_any_element()
}

fn effort_fraction(x: f32, width: f32) -> f32 {
    ((x - THUMB_INSET) / (width - THUMB_INSET * 2.0).max(1.0)).clamp(0.0, 1.0)
}

/// Slider stops include both endpoints and only advertised reasoning levels.
fn effort_index(x: f32, width: f32, count: usize) -> Option<usize> {
    (count > 0)
        .then(|| (effort_fraction(x, width) * count.saturating_sub(1) as f32).round() as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_transitions_start_settled_and_retarget_without_jumping() {
        let start = std::time::Instant::now();
        let mut state = None;
        assert_eq!(
            ScalarTransition::sample(&mut state, 0.5, start, false),
            (0.5, false)
        );
        assert_eq!(
            ScalarTransition::sample(&mut state, 1.0, start, false).0,
            0.5
        );
        let middle = start + motion::RESIZE.total().mul_f32(motion::speed_scale() * 0.5);
        let before = state.as_ref().unwrap().value(middle);
        let retargeted = ScalarTransition::sample(&mut state, 0.0, middle, false).0;
        assert!((before - retargeted).abs() < 0.0001);
        assert_eq!(
            ScalarTransition::sample(&mut state, 0.0, middle, true),
            (0.0, false)
        );
    }

    #[test]
    fn continuous_drag_and_stops_share_inset_geometry() {
        let width = 228.0;
        for count in 2..=8 {
            for index in 0..count {
                let fraction = index as f32 / (count - 1) as f32;
                let x = THUMB_INSET + fraction * (width - THUMB_INSET * 2.0);
                assert!((effort_fraction(x, width) - fraction).abs() < 0.00001);
                assert_eq!(effort_index(x, width, count), Some(index));
                assert!(x - 2.0 >= 0.0 && x + 2.0 <= width);
            }
        }
        assert!((effort_fraction(68.0, width) - 0.25).abs() < 0.00001);
        assert_eq!(effort_fraction(-100.0, width), 0.0);
        assert_eq!(effort_fraction(500.0, width), 1.0);
    }

    #[test]
    fn effort_stops_clamp_and_round_to_advertised_levels() {
        assert_eq!(effort_index(100.0, 200.0, 0), None);
        assert_eq!(effort_index(100.0, 200.0, 1), Some(0));
        assert_eq!(effort_index(-20.0, 200.0, 5), Some(0));
        assert_eq!(effort_index(100.0, 200.0, 5), Some(2));
        assert_eq!(effort_index(250.0, 200.0, 5), Some(4));
    }
}

struct PickerHint(SharedString);
impl Render for PickerHint {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_popup();
        let card = popover::popover_card(&theme)
            .max_w(px(320.0))
            .p(px(8.0))
            .text_size(crate::typography::ui_rems(12.0))
            .line_height(px(17.0))
            .text_color(theme.text)
            .child(self.0.clone());
        crate::frost::frosted(popover::CARD_RADIUS, crate::frost::MENU_BLUR, card)
    }
}
