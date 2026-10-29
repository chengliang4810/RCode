//! Goal state comes from the session document; buttons submit host commands.
use super::*;
use rcode_presentation::TextKey;
use rcode_proto::goal::GoalStatus;

impl Composer {
    fn update_goal(&mut self, action: &'static str, cx: &mut Context<Self>) {
        if self.goal_action_task.is_some() {
            return;
        }
        let state = self.state.read(cx);
        let Some(chat_id) = state.selected_chat.clone() else {
            return;
        };
        let Some(engine) = state.engine().cloned() else {
            return;
        };
        let Some(goal) = &state.goal else {
            return;
        };
        let mut params = serde_json::json!({"chatId":chat_id,"action":action,"goalId":goal.id});
        if let Some(device) = state.selected_chat_row().map(|chat| chat.device_id.clone()) {
            params["targetDeviceId"] = device.into();
        }
        self.goal_action_task = Some(cx.spawn(async move |this, cx| {
            let result = engine.client().call(methods::UPDATE_GOAL, params).await;
            this.update(cx, |this, cx| {
                this.goal_action_task = None;
                if this.state.read(cx).selected_chat.as_deref() == Some(chat_id.as_str()) {
                    if let Err(error) = result {
                        this.set_failure(error.to_string());
                        this.failure_key = Some(chat_id.clone());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    pub(super) fn render_goal_banner(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let goal = self.state.read(cx).goal.clone()?;
        self.state.read(cx).selected_chat.as_ref()?;
        if goal.status == GoalStatus::Active && self.goal_clock_task.is_none() {
            self.goal_clock_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                this.update(cx, |this, cx| {
                    this.goal_clock_task = None;
                    cx.notify();
                })
                .ok();
            }));
        } else if goal.status != GoalStatus::Active {
            self.goal_clock_task = None;
        }
        let localizer = crate::l10n::localizer();
        let elapsed = goal.elapsed_ms(crate::state::clock_now().timestamp_millis()) / 1000;
        let (unit, count) = duration_unit(elapsed);
        let duration = localizer.format_with(unit, &[("count", &count.to_string())]);
        let status = localizer.text(match goal.status {
            GoalStatus::Active => TextKey::ConversationGoalActiveTitle,
            GoalStatus::Paused => TextKey::ConversationGoalPausedTitle,
            GoalStatus::Complete => TextKey::ConversationGoalCompleteTitle,
        });
        let iteration = localizer.format_with(
            TextKey::ConversationGoalIteration,
            &[("count", &goal.iteration.to_string())],
        );
        let (action, label) = if goal.status == GoalStatus::Active {
            ("pause", localizer.text(TextKey::ConversationGoalPause))
        } else {
            ("resume", localizer.text(TextKey::ConversationGoalResume))
        };
        let toggle_view = cx.entity().downgrade();
        let clear_view = cx.entity().downgrade();
        let button = |id: &'static str, label: String, glyph: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .size(px(24.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .text_color(theme.text_muted)
                .hover(|style| style.bg(theme.ink(0.08)))
                .cursor_pointer()
                .role(gpui::Role::Button)
                .aria_label(label.clone())
                .tab_index(0)
                .focus_visible(|style| style.border_1().border_color(theme.accent))
                .tooltip(crate::settings::widgets::text_tooltip(label))
                .child(
                    crate::icons::icon(glyph)
                        .size(px(13.0))
                        .text_color(theme.text_muted),
                )
        };
        let banner = attached_panel_surface(theme)
            .id("composer-goal-surface")
            .h(px(ATTACHED_PANEL_HEIGHT))
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .pb(px(ATTACHED_PANEL_OVERLAP))
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(theme.text)
            .child(
                crate::icons::icon(crate::icons::GOAL)
                    .size(px(13.0))
                    .text_color(theme.text_muted),
            )
            .child(
                div()
                    .flex_none()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(status),
            )
            .child(
                div()
                    .id("goal-objective")
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(goal.objective.clone())
                    .tooltip(crate::settings::widgets::text_tooltip(format!(
                        "{} · {iteration}",
                        goal.objective
                    ))),
            )
            .child(
                div()
                    .id("goal-duration")
                    .debug_selector(|| "goal-duration".into())
                    .flex_none()
                    .text_color(theme.text_muted)
                    .child(format!("· {duration}")),
            )
            .child(
                button(
                    "goal-clear",
                    localizer.text(TextKey::ConversationGoalClear),
                    crate::icons::TRASH_BIN_MINIMALISTIC,
                )
                .on_click(cx.listener(|this, _, _, cx| this.update_goal("clear", cx)))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                        this.update_goal("clear", cx);
                    }
                }))
                .on_a11y_action(
                    gpui::AccessibleAction::Click,
                    move |_, _, cx| {
                        clear_view
                            .update(cx, |this, cx| this.update_goal("clear", cx))
                            .ok();
                    },
                ),
            )
            .when(goal.status != GoalStatus::Complete, |row| {
                row.child(
                    button(
                        "goal-toggle",
                        label,
                        if action == "pause" {
                            crate::icons::STOP
                        } else {
                            crate::icons::ACTION_PLAY
                        },
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.update_goal(action, cx)))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            this.update_goal(action, cx);
                        }
                    }))
                    .on_a11y_action(
                        gpui::AccessibleAction::Click,
                        move |_, _, cx| {
                            toggle_view
                                .update(cx, |this, cx| this.update_goal(action, cx))
                                .ok();
                        },
                    ),
                )
            })
            .child(
                button(
                    "goal-edit",
                    localizer.text(TextKey::ConversationGoalEdit),
                    crate::icons::EXPAND_ARROWS,
                )
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposerEvent::OpenGoal)))
                .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                        cx.emit(ComposerEvent::OpenGoal);
                    }
                }))
                .on_a11y_action(gpui::AccessibleAction::Click, {
                    let view = cx.entity().downgrade();
                    move |_, _, cx| {
                        view.update(cx, |_, cx| cx.emit(ComposerEvent::OpenGoal))
                            .ok();
                    }
                }),
            )
            .when(goal.status == GoalStatus::Paused, |row| {
                if let Some(reason) = goal.reason.filter(|reason| !reason.is_empty()) {
                    row.tooltip(crate::settings::widgets::text_tooltip(reason))
                } else {
                    row
                }
            });
        Some(
            clipped_composer_panel(banner, ATTACHED_PANEL_OVERLAP)
                .id("composer-goal")
                .debug_selector(|| "composer-goal".into())
                .absolute()
                .top_0()
                .left(px(ATTACHED_PANEL_INSET))
                .right(px(ATTACHED_PANEL_INSET))
                .h(px(ATTACHED_PANEL_HEIGHT))
                .into_any_element(),
        )
    }
}

#[cfg(feature = "appshots-fixture")]
impl Composer {
    pub fn fixture_goal_surface_bounds(&self) -> Option<gpui::Bounds<gpui::Pixels>> {
        self.surface_bounds.get()
    }
    pub fn fixture_goal_action(&mut self, action: &'static str, cx: &mut Context<Self>) {
        self.update_goal(action, cx);
    }
    pub fn fixture_todo_visible(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.render_todo_panel(window, cx).is_some()
    }
    pub fn fixture_todo_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        if let Some(panel) = self.state.read(cx).selected_chat.as_ref()
            .and_then(|chat| self.todo_panels.get_mut(chat))
            && panel.is_expanded() != expanded
        {
            panel.toggle();
            cx.notify();
        }
    }
}

fn duration_unit(seconds: u64) -> (TextKey, u64) {
    match seconds {
        0..60 => (TextKey::ConversationGoalDurationSeconds, seconds),
        60..3600 => (TextKey::ConversationGoalDurationMinutes, seconds / 60),
        3600..86400 => (TextKey::ConversationGoalDurationHours, seconds / 3600),
        _ => (TextKey::ConversationGoalDurationDays, seconds / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duration_changes_units_at_each_boundary() {
        for (seconds, key, count) in [
            (0, TextKey::ConversationGoalDurationSeconds, 0),
            (59, TextKey::ConversationGoalDurationSeconds, 59),
            (60, TextKey::ConversationGoalDurationMinutes, 1),
            (3599, TextKey::ConversationGoalDurationMinutes, 59),
            (3600, TextKey::ConversationGoalDurationHours, 1),
            (86399, TextKey::ConversationGoalDurationHours, 23),
            (86400, TextKey::ConversationGoalDurationDays, 1),
            (172800, TextKey::ConversationGoalDurationDays, 2),
        ] {
            assert_eq!(duration_unit(seconds), (key, count));
        }
    }
}
