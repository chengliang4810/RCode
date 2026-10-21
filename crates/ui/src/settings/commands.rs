//! Personal prompt commands. The filesystem is owned by the native harness.
use super::widgets::{self, ActionTone};
use crate::{composer::ComposerInput, popover, state::AppState, theme::Theme};
use gpui::{AnyElement, Context, Entity, Subscription, Task, Window, div, prelude::*, px};
use rcode_presentation::TextKey;
use rcode_proto::custom_commands::{CommandMutation, CommandSource, CustomCommand};
use rcode_rpc::methods;

struct Editor {
    name: Entity<ComposerInput>,
    body: Entity<ComposerInput>,
    origin: Option<CustomCommand>,
}

pub struct CommandsPage {
    state: Entity<AppState>,
    commands: Vec<CustomCommand>,
    editor: Option<Editor>,
    deleting: Option<CustomCommand>,
    error: Option<String>,
    busy: bool,
    context: Option<String>,
    task: Option<Task<()>>,
    _state: Subscription,
}

impl CommandsPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&state, |page: &mut Self, _, cx| {
            if !page.busy && page.current_context(cx) != page.context {
                page.load(cx);
            }
        });
        let mut page = Self {
            state,
            commands: Vec::new(),
            editor: None,
            deleting: None,
            error: None,
            busy: false,
            context: None,
            task: None,
            _state: sub,
        };
        page.load(cx);
        page
    }

    fn current_context(&self, cx: &gpui::App) -> Option<String> {
        self.state
            .read(cx)
            .engine()
            .map(|engine| format!("{:p}", engine.client()))
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.call(None, cx);
    }

    fn call(&mut self, mutation: Option<CommandMutation>, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.context = self.current_context(cx);
        let context = self.context.clone();
        self.busy = true;
        self.error = None;
        let saved = match &mutation {
            Some(CommandMutation::Save { name, content, .. }) => {
                Some((name.clone(), content.clone()))
            }
            _ => None,
        };
        let deleting = matches!(&mutation, Some(CommandMutation::Delete { .. }));
        let (method, params) = match mutation {
            Some(mutation) => (
                methods::MUTATE_CUSTOM_COMMAND,
                serde_json::to_value(mutation).expect("command mutation"),
            ),
            None => (
                methods::LIST_CUSTOM_COMMANDS,
                serde_json::json!({"personal": true}),
            ),
        };
        self.task = Some(cx.spawn(async move |page, cx| {
            let result = engine
                .client()
                .call(method, params)
                .await
                .map_err(|e| e.to_string())
                .and_then(|v| {
                    serde_json::from_value::<Vec<CustomCommand>>(v).map_err(|e| e.to_string())
                });
            page.update(cx, |page, cx| {
                page.busy = false;
                if page.current_context(cx) != context {
                    page.load(cx);
                    return;
                }
                match result {
                    Ok(commands) => {
                        page.commands = commands;
                        if let Some((name, content)) = &saved {
                            let changed = page
                                .editor
                                .as_ref()
                                .is_some_and(|editor| editor.body.read(cx).text() != content);
                            if changed {
                                if let Some(editor) = &mut page.editor {
                                    editor.origin = page
                                        .commands
                                        .iter()
                                        .find(|c| {
                                            c.source == CommandSource::User && &c.name == name
                                        })
                                        .cloned();
                                }
                            } else {
                                page.editor = None;
                            }
                        }
                        if deleting {
                            page.deleting = None;
                        }
                    }
                    Err(error) => page.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn edit(&mut self, origin: Option<CustomCommand>, cx: &mut Context<Self>) {
        if self.busy || self.editor.is_some() {
            return;
        }
        let name = cx.new(|cx| {
            ComposerInput::with_context(
                crate::l10n::localizer().text(TextKey::SettingsCommandsName),
                "SettingsTextarea",
                cx,
            )
            .with_tab_stop()
        });
        let body = cx.new(|cx| {
            ComposerInput::with_context(
                crate::l10n::localizer().text(TextKey::SettingsCommandsBody),
                "SettingsTextarea",
                cx,
            )
            .with_viewport_height(240.0)
            .with_text_metrics(13.0, 20.0)
            .with_tab_stop()
        });
        if let Some(command) = &origin {
            name.update(cx, |input, cx| input.set_text(command.name.clone(), cx));
            body.update(cx, |input, cx| input.set_text(command.content.clone(), cx));
        }
        self.editor = Some(Editor { name, body, origin });
        self.deleting = None;
        self.error = None;
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &self.editor else {
            return;
        };
        let name = editor
            .origin
            .as_ref()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| {
                editor
                    .name
                    .read(cx)
                    .text()
                    .trim()
                    .trim_start_matches('/')
                    .to_lowercase()
            });
        let content = editor.body.read(cx).text().to_owned();
        let expected_revision = editor.origin.as_ref().map(|c| c.revision.clone());
        self.call(
            Some(CommandMutation::Save {
                name,
                content,
                expected_revision,
            }),
            cx,
        );
    }

    pub fn dismiss_on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if self.busy {
            return self.editor.is_some() || self.deleting.is_some();
        }
        let had_dialog = self.editor.take().is_some() | self.deleting.take().is_some();
        if had_dialog {
            cx.notify();
        }
        had_dialog
    }

    fn render_editor(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editor = self.editor.as_ref()?;
        let l = crate::l10n::localizer();
        let mut card = widgets::section_card(theme)
            .mt(px(16.0))
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(10.0));
        if let Some(origin) = &editor.origin {
            card = card.child(widgets::row_title(theme, format!("/{}", origin.name)));
        } else {
            card = card
                .child(widgets::field_label(
                    theme,
                    l.text(TextKey::SettingsCommandsName),
                ))
                .child(popover::dialog_field(
                    editor.name.clone().into_any_element(),
                ));
        }
        Some(
            card.child(widgets::field_label(
                theme,
                l.text(TextKey::SettingsCommandsBody),
            ))
            .child(popover::dialog_field(
                editor.body.clone().into_any_element(),
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        widgets::text_action(
                            theme,
                            ActionTone::Quiet,
                            l.text(TextKey::ActionCancel),
                        )
                        .id("commands-cancel")
                        .on_click(cx.listener(|page, _, _, cx| {
                            page.dismiss_on_escape(cx);
                        })),
                    )
                    .child(
                        widgets::text_action(theme, ActionTone::Solid, l.text(TextKey::ActionSave))
                            .id("commands-save")
                            .debug_selector(|| "commands-save".into())
                            .on_click(cx.listener(|page, _, _, cx| page.save(cx))),
                    ),
            )
            .into_any_element(),
        )
    }
}

impl Render for CommandsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let l = crate::l10n::localizer();
        let editor = self.render_editor(&theme, cx);
        let mut column = widgets::page_column()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(widgets::page_header(
                        &theme,
                        &l.text(TextKey::SettingsCommands),
                        Some(self.commands.len()),
                    ))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.0))
                            .child(
                                widgets::text_action(
                                    &theme,
                                    ActionTone::Quiet,
                                    l.text(TextKey::SettingsCommandsReload),
                                )
                                .id("commands-reload")
                                .on_click(cx.listener(|page, _, _, cx| page.load(cx))),
                            )
                            .child(
                                widgets::text_action(
                                    &theme,
                                    ActionTone::Outlined,
                                    l.text(TextKey::SettingsCommandsNew),
                                )
                                .id("commands-new")
                                .debug_selector(|| "commands-new".into())
                                .on_click(cx.listener(|page, _, _, cx| page.edit(None, cx))),
                            ),
                    ),
            )
            .children(editor)
            .when_some(self.error.clone(), |el, error| {
                el.child(widgets::error_strip(&theme, error).mt(px(12.0)))
            });
        if let Some(command) = self.deleting.clone() {
            column = column.child(
                widgets::section_card(&theme)
                    .mt(px(16.0))
                    .p(px(16.0))
                    .child(div().child(format!(
                        "{} /{}",
                        l.text(TextKey::SettingsCommandsDelete),
                        command.name
                    )))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .child(
                                widgets::text_action(
                                    &theme,
                                    ActionTone::Quiet,
                                    l.text(TextKey::ActionCancel),
                                )
                                .id("commands-keep")
                                .on_click(cx.listener(
                                    |page, _, _, cx| {
                                        page.dismiss_on_escape(cx);
                                    },
                                )),
                            )
                            .child(
                                widgets::text_action(
                                    &theme,
                                    ActionTone::Danger,
                                    l.text(TextKey::SettingsCommandsDelete),
                                )
                                .id("commands-confirm-delete")
                                .on_click(cx.listener(
                                    move |page, _, _, cx| {
                                        page.call(
                                            Some(CommandMutation::Delete {
                                                name: command.name.clone(),
                                                expected_revision: command.revision.clone(),
                                            }),
                                            cx,
                                        );
                                    },
                                )),
                            ),
                    ),
            );
        }
        if self.commands.is_empty() {
            column = column.child(
                div()
                    .mt(px(24.0))
                    .text_color(theme.text_muted)
                    .child(l.text(if self.busy {
                        TextKey::SettingsCommandsLoading
                    } else {
                        TextKey::SettingsCommandsEmpty
                    })),
            );
        }
        for (index, command) in self.commands.clone().into_iter().enumerate() {
            let editable = command.source == CommandSource::User;
            let editing = command.clone();
            let toggling = command.clone();
            let deleting = command.clone();
            let mut row = widgets::section_card(&theme)
                .mt(px(12.0))
                .p(px(14.0))
                .flex()
                .items_center()
                .gap(px(12.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(widgets::row_title(
                            &theme,
                            format!(
                                "/{}{}",
                                command.name,
                                command
                                    .input_hint
                                    .as_ref()
                                    .map(|hint| format!(" {hint}"))
                                    .unwrap_or_default()
                            ),
                        ))
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(12.0))
                                .text_color(theme.text_muted)
                                .child(command.description),
                        ),
                );
            if editable {
                row = row
                    .child(
                        widgets::text_action(
                            &theme,
                            ActionTone::Quiet,
                            l.text(if command.enabled {
                                TextKey::SettingsCommandsDisable
                            } else {
                                TextKey::SettingsCommandsEnable
                            }),
                        )
                        .id(("commands-toggle", index))
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.call(
                                Some(CommandMutation::SetEnabled {
                                    name: toggling.name.clone(),
                                    enabled: !toggling.enabled,
                                    expected_revision: toggling.revision.clone(),
                                }),
                                cx,
                            );
                        })),
                    )
                    .child(
                        widgets::text_action(
                            &theme,
                            ActionTone::Quiet,
                            l.text(TextKey::SettingsCommandsEdit),
                        )
                        .id(("commands-edit", index))
                        .on_click(
                            cx.listener(move |page, _, _, cx| page.edit(Some(editing.clone()), cx)),
                        ),
                    )
                    .child(
                        widgets::text_action(
                            &theme,
                            ActionTone::Quiet,
                            l.text(TextKey::SettingsCommandsDelete),
                        )
                        .id(("commands-delete", index))
                        .on_click(cx.listener(move |page, _, _, cx| {
                            if !page.busy && page.editor.is_none() {
                                page.deleting = Some(deleting.clone());
                                page.editor = None;
                                cx.notify();
                            }
                        })),
                    );
            } else {
                row = row.child(widgets::badge(
                    &theme,
                    l.text(TextKey::SettingsCommandsPlugin),
                ));
            }
            column = column.child(row.when(!command.enabled, |el| el.opacity(0.55)));
        }
        div()
            .id("commands-page")
            .size_full()
            .overflow_y_scroll()
            .child(column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn command_editor_preserves_markdown_and_cancels_without_writing(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::app_menus::init(cx);
        });
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            CommandsPage::new(state, cx)
        });
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
            window.draw(cx).clear();
        });
        let new = visual.debug_bounds("commands-new").unwrap().center();
        visual.simulate_click(new, Default::default());
        window
            .read_with(&visual, |page, _| {
                assert!(page.editor.as_ref().unwrap().origin.is_none())
            })
            .unwrap();
        let command = CustomCommand {
            name: "review".into(),
            description: "Review".into(),
            input_hint: None,
            source: CommandSource::User,
            enabled: true,
            revision: "revision".into(),
            content: "---\ndescription: Review\n---\nReview $ARGUMENTS\n".into(),
        };
        window
            .update(&mut visual, |page, _, cx| {
                assert!(page.dismiss_on_escape(cx));
                page.edit(Some(command.clone()), cx);
                let editor = page.editor.as_ref().unwrap();
                assert_eq!(editor.body.read(cx).text(), command.content);
                assert_eq!(editor.name.read(cx).text(), command.name);
                // Another row or New must not discard the current draft.
                page.edit(None, cx);
                assert!(page.editor.as_ref().unwrap().origin.is_some());
                assert!(page.dismiss_on_escape(cx));
                assert!(!page.dismiss_on_escape(cx));
            })
            .unwrap();
    }
}
