use super::*;
use rcode_proto::goal::SessionGoal;

/// A chat-owned draft survives tab switches and closing/reopening the tab.
pub(super) struct GoalEditor {
    state: Entity<AppState>,
    chat_id: String,
    device_id: Option<String>,
    origin: SessionGoal,
    input: Entity<ComposerInput>,
    error: Option<String>,
    busy: bool,
    task: Option<Task<()>>,
    focus_pending: bool,
    _state: Subscription,
    _input: Subscription,
}

impl GoalEditor {
    fn new(
        state: Entity<AppState>,
        chat_id: String,
        origin: SessionGoal,
        cx: &mut Context<Self>,
    ) -> Self {
        let device_id = state
            .read(cx)
            .selected_chat_row()
            .map(|row| row.device_id.clone());
        let input = cx.new(|cx| {
            ComposerInput::with_context(
                crate::l10n::localizer().text(TextKey::ConversationGoalEdit),
                "SettingsTextarea",
                cx,
            )
            .with_viewport_height(280.0)
            .with_text_metrics(14.0, 22.0)
            .with_tab_stop()
        });
        input.update(cx, |input, cx| input.set_text(origin.objective.clone(), cx));
        let input_subscription = cx.subscribe(&input, |_: &mut Self, _, _, cx| cx.notify());
        let state_subscription = cx.observe(&state, |this: &mut Self, _, cx| this.refresh(cx));
        Self {
            state,
            chat_id,
            device_id,
            origin,
            input,
            error: None,
            busy: false,
            task: None,
            focus_pending: true,
            _state: state_subscription,
            _input: input_subscription,
        }
    }

    pub(super) fn dirty(&self, cx: &App) -> bool {
        self.input.read(cx).text() != self.origin.objective
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.input.focus_handle(cx), cx);
    }

    fn current(&self, cx: &App) -> Option<SessionGoal> {
        let state = self.state.read(cx);
        (state.selected_chat.as_deref() == Some(self.chat_id.as_str()))
            .then(|| state.goal.clone())
            .flatten()
    }

    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.busy && !self.dirty(cx) {
            if let Some(goal) = self.current(cx) {
                if goal.id != self.origin.id || goal.objective != self.origin.objective {
                    self.input
                        .update(cx, |input, cx| input.set_text(goal.objective.clone(), cx));
                }
                self.origin = goal;
            }
        }
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if let Some(goal) = self.current(cx) {
            self.input
                .update(cx, |input, cx| input.set_text(goal.objective.clone(), cx));
            self.origin = goal;
            self.error = None;
        }
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.busy || !self.dirty(cx) {
            return;
        }
        let objective = self.input.read(cx).text().trim().to_owned();
        if objective.is_empty() || objective.chars().count() > 4000 {
            self.error = Some(crate::l10n::localizer().text(TextKey::ConversationGoalInvalid));
            cx.notify();
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let mut params = serde_json::json!({"chatId": self.chat_id, "goalId": self.origin.id,
            "action": "edit", "objective": objective, "revision": self.origin.revision});
        if let Some(device) = &self.device_id {
            params["targetDeviceId"] = device.clone().into();
        }
        let submitted = self.input.read(cx).text().to_owned();
        self.busy = true;
        self.error = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::UPDATE_GOAL, params)
                .await
                .map_err(|error| error.to_string())
                .and_then(|reply| {
                    serde_json::from_value::<SessionGoal>(reply["goal"].clone())
                        .map_err(|error| error.to_string())
                });
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(goal) => {
                        if this.input.read(cx).text() == submitted {
                            this.input
                                .update(cx, |input, cx| input.set_text(goal.objective.clone(), cx));
                        }
                        this.origin = goal;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
}

impl Render for GoalEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_pending {
            self.focus_pending = false;
            self.focus(window, cx);
        }
        let theme = Theme::of(cx).clone();
        let l = crate::l10n::localizer();
        let missing = self.current(cx).is_none();
        let enabled = !missing && !self.busy && self.dirty(cx);
        let view = cx.entity().downgrade();
        let save = crate::settings::widgets::text_action(
            &theme,
            crate::settings::widgets::ActionTone::Solid,
            l.text(if self.busy {
                TextKey::ActionLoading
            } else {
                TextKey::ConversationGoalSave
            }),
        )
        .id("goal-editor-save")
        .debug_selector(|| "goal-editor-save".into())
        .role(gpui::Role::Button)
        .tab_index(0)
        .aria_label(l.text(TextKey::ConversationGoalSave))
        .when(!enabled, |el| el.opacity(0.45))
        .on_click(cx.listener(move |this, _, _, cx| {
            if enabled {
                this.save(cx);
            }
        }))
        .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
            if enabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                this.save(cx);
            }
        }))
        .on_a11y_action(gpui::AccessibleAction::Click, move |_, _, cx| {
            if enabled {
                view.update(cx, |this, cx| this.save(cx)).ok();
            }
        });
        div()
            .id("goal-editor")
            .debug_selector(|| "goal-editor".into())
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, _: &SaveFile, _, cx| this.save(cx)))
            .text_color(theme.text)
            .text_size(crate::typography::ui_rems(13.0))
            .child(
                crate::surface_chrome::toolbar(&theme)
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        crate::settings::widgets::text_action(
                            &theme,
                            crate::settings::widgets::ActionTone::Quiet,
                            l.text(TextKey::ConversationGoalReload),
                        )
                        .id("goal-editor-reload")
                        .role(gpui::Role::Button)
                        .tab_index(0)
                        .on_click(cx.listener(|this, _, _, cx| this.reload(cx)))
                        .on_key_down(cx.listener(
                            |this, event: &gpui::KeyDownEvent, _, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    cx.stop_propagation();
                                    this.reload(cx);
                                }
                            },
                        )),
                    )
                    .child(save),
            )
            .child(
                div()
                    .id("goal-editor-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(16.0))
                    .child(crate::popover::dialog_field(
                        self.input.clone().into_any_element(),
                    ))
                    .when(missing, |el| {
                        el.child(
                            div()
                                .mt(px(10.0))
                                .child(l.text(TextKey::ConversationGoalMissing)),
                        )
                    })
                    .children(
                        self.error
                            .clone()
                            .map(|error| div().mt(px(10.0)).text_color(theme.danger).child(error)),
                    ),
            )
    }
}

impl Shell {
    pub(super) fn goal_editor(&self, id: u64) -> Option<Entity<GoalEditor>> {
        self.goal_editors
            .values()
            .find(|editor| editor.entity_id().as_u64() == id)
            .cloned()
    }

    pub(super) fn add_goal_surface(&mut self, cx: &mut Context<Self>) {
        self.add_goal_surface_for(self.state.clone(), cx);
    }

    pub(super) fn add_goal_surface_for(&mut self, state: Entity<AppState>, cx: &mut Context<Self>) {
        let Some(key) = state.read(cx).selected_chat.clone() else {
            return;
        };
        let Some(goal) = state.read(cx).goal.clone() else {
            return;
        };
        if !self.goal_editors.contains_key(&key) {
            let editor = cx.new(|cx| GoalEditor::new(state, key.clone(), goal, cx));
            let sub = cx.observe(&editor, |_, _, cx| cx.notify());
            self.goal_editors.insert(key.clone(), editor);
            self.goal_editor_subs.insert(key.clone(), sub);
        }
        self.composer
            .update(cx, |composer, _| composer.focus_pending = false);
        self.set_surfaces_open(true, cx);
        let surface = RightSurface::Goal(self.goal_editors[&key].entity_id().as_u64());
        self.goal_editors[&key].update(cx, |editor, cx| {
            editor.focus_pending = true;
            cx.notify();
        });
        let host = self.panel_key(cx);
        push_unique_right_surface(self.right_tabs.entry(host).or_default(), surface);
        self.set_right_active(surface, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn incoming_updates_preserve_dirty_drafts_and_reload_is_explicit(
        cx: &mut gpui::TestAppContext,
    ) {
        let goal = SessionGoal::new("first".into(), "Original".into(), 1000).unwrap();
        let state = cx.new(|_| {
            let mut state = AppState::new();
            state.selected_chat = Some("chat".into());
            state.goal = Some(goal.clone());
            state
        });
        let editor = cx.new(|cx| GoalEditor::new(state.clone(), "chat".into(), goal, cx));
        editor.update(cx, |editor, cx| {
            editor
                .input
                .update(cx, |input, cx| input.set_text("My draft", cx))
        });
        state.update(cx, |state, _| {
            state.goal = Some(
                SessionGoal::new("replacement".into(), "External change".into(), 2000).unwrap(),
            )
        });
        editor.update(cx, |editor, cx| {
            editor.refresh(cx);
            assert_eq!(editor.origin.id, "first");
            assert_eq!(editor.input.read(cx).text(), "My draft");
            editor.reload(cx);
            assert_eq!(editor.origin.id, "replacement");
            assert_eq!(editor.input.read(cx).text(), "External change");
            assert!(!editor.dirty(cx));
        });
        state.update(cx, |state, _| {
            state.selected_chat = Some("other-chat".into())
        });
        editor.update(cx, |editor, cx| {
            editor.refresh(cx);
            assert_eq!(editor.origin.id, "replacement");
            assert!(editor.current(cx).is_none());
        });
    }
}

#[cfg(feature = "appshots-fixture")]
impl Shell {
    pub fn fixture_open_goal_editor(&mut self, cx: &mut Context<Self>) {
        self.add_goal_surface(cx);
    }
    pub fn fixture_save_goal_editor(&mut self, objective: &str, cx: &mut Context<Self>) {
        let editor = self.goal_editors.get(&self.panel_key(cx)).unwrap().clone();
        editor.update(cx, |editor, cx| {
            editor
                .input
                .update(cx, |input, cx| input.set_text(objective, cx));
            editor.save(cx);
        });
    }
    pub fn fixture_goal_draft_survives_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = self.panel_key(cx);
        let editor = self.goal_editors[&key].clone();
        editor.update(cx, |editor, cx| {
            editor
                .input
                .update(cx, |input, cx| input.set_text("未保存的目标草稿", cx))
        });
        let surface = RightSurface::Goal(editor.entity_id().as_u64());
        self.close_right_surface(surface, window, cx);
        self.add_goal_surface(cx);
        self.goal_editors[&key] == editor
            && editor.read(cx).input.read(cx).text() == "未保存的目标草稿"
            && self
                .right_surface_rows(cx)
                .iter()
                .filter(|(s, ..)| *s == surface)
                .count()
                == 1
    }
}
