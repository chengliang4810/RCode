use std::collections::HashSet;

use rcode_harness::provider_store::DiscoveredModel;

use super::*;

pub(super) struct ModelFetchDialog {
    pub(super) provider_id: String,
    pub(super) models: Loadable<Vec<DiscoveredModel>>,
    pub(super) selected: HashSet<String>,
    focus: gpui::FocusHandle,
    scroll: gpui::UniformListScrollHandle,
    _request: Option<Task<()>>,
}

impl ProvidersPage {
    pub(super) fn open_model_fetch(
        &mut self,
        provider: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.model_drag.is_some() {
            return;
        }
        self.commit_form(cx);
        let Some(provider_id) = self
            .draft
            .as_ref()
            .and_then(|draft| draft.providers.get(provider))
            .map(|p| p.id.clone())
        else {
            return;
        };
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.fetch_dialog = Some(ModelFetchDialog {
            provider_id,
            models: Loadable::Idle,
            selected: HashSet::new(),
            focus,
            scroll: gpui::UniformListScrollHandle::new(),
            _request: None,
        });
        self.continue_model_fetch(cx);
        cx.notify();
    }

    pub(super) fn continue_model_fetch(&mut self, cx: &mut Context<Self>) {
        if !self
            .fetch_dialog
            .as_ref()
            .is_some_and(|dialog| matches!(dialog.models, Loadable::Idle))
        {
            return;
        }
        if self.conflict.is_some() || self.error.is_some() {
            return;
        }
        if self.has_pending_save() {
            self.save(cx);
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.fetch_dialog.as_mut().unwrap().models =
                Loadable::Error(crate::l10n::localizer().text(TextKey::ProvidersModelsUnavailable));
            cx.notify();
            return;
        };
        let Some(draft) = &self.draft else { return };
        let dialog = self.fetch_dialog.as_mut().unwrap();
        let params = serde_json::json!({"providerId": dialog.provider_id, "expectedRevision": draft.revision});
        dialog.models = Loadable::Loading;
        dialog._request = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::FETCH_PROVIDER_MODELS, params)
                .await
                .map_err(|error| error.to_string())
                .and_then(|value| {
                    serde_json::from_value::<Vec<DiscoveredModel>>(value)
                        .map_err(|error| error.to_string())
                });
            this.update(cx, |page, cx| {
                if let Some(dialog) = page.fetch_dialog.as_mut() {
                    match result {
                        Ok(models) => {
                            dialog.selected =
                                models.iter().map(|model| model.model.clone()).collect();
                            dialog.models = Loadable::Ready(models);
                        }
                        Err(error) => {
                            if error.contains(rcode_harness::provider_store::CONFLICT_CODE) {
                                page.conflict = Some(error.clone());
                            }
                            dialog.models = Loadable::Error(error);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    pub(super) fn close_model_fetch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fetch_dialog = None;
        if let Some(form) = &self.form {
            window.focus(&form.label.focus_handle(cx), cx);
        }
        cx.notify();
    }

    pub(super) fn retry_model_fetch(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.fetch_dialog.as_mut() {
            dialog.models = Loadable::Idle;
            dialog._request = None;
        }
        self.error = None;
        self.continue_model_fetch(cx);
        cx.notify();
    }

    pub(super) fn toggle_fetched_model(&mut self, model: &str, cx: &mut Context<Self>) {
        if let Some(dialog) = self.fetch_dialog.as_mut() {
            if !dialog.selected.remove(model) {
                dialog.selected.insert(model.to_owned());
            }
        }
        cx.notify();
    }

    pub(super) fn toggle_all_fetched_models(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.fetch_dialog.as_mut()
            && let Loadable::Ready(models) = &dialog.models
        {
            if dialog.selected.len() == models.len() {
                dialog.selected.clear();
            } else {
                dialog.selected = models.iter().map(|model| model.model.clone()).collect();
            }
        }
        cx.notify();
    }

    pub(super) fn import_fetched_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = &self.fetch_dialog else {
            return;
        };
        let Loadable::Ready(models) = &dialog.models else {
            return;
        };
        if dialog.selected.is_empty() {
            return;
        }
        let Some(provider) = self.draft.as_ref().and_then(|draft| {
            draft
                .providers
                .iter()
                .position(|p| p.id == dialog.provider_id)
        }) else {
            return;
        };
        let selected = models
            .iter()
            .filter(|model| dialog.selected.contains(&model.model))
            .cloned()
            .collect::<Vec<_>>();
        let mut changed = false;
        for model in selected {
            if self.draft.as_ref().unwrap().providers[provider]
                .models
                .iter()
                .any(|existing| existing.model == model.model)
            {
                continue;
            }
            let id = self.new_model_id();
            let metadata = self.model_metadata(provider, &model.model, cx);
            let mut edit = ModelEdit {
                id,
                model: model.model,
                label: model.display_name.filter(|name| !name.trim().is_empty()).or(metadata.name.clone()),
                ..ModelEdit::default()
            };
            edit.apply_metadata(metadata);
            self.draft.as_mut().unwrap().providers[provider]
                .models
                .push(edit);
            changed = true;
        }
        self.close_model_fetch(window, cx);
        if changed {
            self.schedule_save(cx);
        }
        cx.notify();
    }

    pub(super) fn render_fetch_dialog(
        &mut self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let dialog = self.fetch_dialog.as_ref()?;
        let theme = Theme::of(cx).for_popup();
        let localizer = crate::l10n::localizer();
        let models = dialog.models.clone();
        let scroll = dialog.scroll.clone();
        let focus = dialog.focus.clone();
        let selected_count = dialog.selected.len();
        let ready = matches!(&models, Loadable::Ready(models) if !models.is_empty());
        let can_import = ready && selected_count > 0;
        let error = self
            .conflict
            .clone()
            .or_else(|| self.error.clone())
            .or_else(|| match &models {
                Loadable::Error(error) => Some(error.clone()),
                _ => None,
            });
        let body: AnyElement =
            match models {
                Loadable::Ready(models) if !models.is_empty() => {
                    let all = selected_count == models.len();
                    let mixed = selected_count > 0 && !all;
                    let entity = cx.entity();
                    div()
                        .flex()
                        .flex_col()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(theme.border)
                        .id("providers-fetch-table")
                        .role(gpui::Role::Table)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(10.0))
                                .px(px(10.0))
                                .h(px(38.0))
                                .border_b_1()
                                .border_color(theme.border)
                                .child(
                                    fetch_checkbox(
                                        &theme,
                                        "providers-fetch-all".into(),
                                        localizer.text(TextKey::ProvidersModelsSelectAll).into(),
                                        all,
                                        mixed,
                                    )
                                    .on_click(cx.listener(
                                        |page, _, _, cx| page.toggle_all_fetched_models(cx),
                                    )),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .text_color(theme.text_muted)
                                        .child(localizer.text(TextKey::ProvidersModelName)),
                                )
                                .child(
                                    div()
                                        .w(px(120.0))
                                        .text_color(theme.text_muted)
                                        .child(localizer.text(TextKey::ProvidersModelsOwner)),
                                ),
                        )
                        .child(
                            gpui::uniform_list(
                                "providers-fetch-rows",
                                models.len(),
                                move |range, _, app| {
                                    entity.update(app, |page, cx| {
                                        range
                                            .filter_map(|index| page.render_fetched_row(index, cx))
                                            .collect::<Vec<_>>()
                                    })
                                },
                            )
                            .h(px((models.len() as f32 * 40.0).min(
                                (f32::from(viewport.height) - 240.0).clamp(80.0, 360.0),
                            )))
                            .track_scroll(&scroll),
                        )
                        .into_any_element()
                }
                Loadable::Ready(_) => div()
                    .py(px(20.0))
                    .text_color(theme.text_muted)
                    .child(localizer.text(TextKey::ProvidersModelsEmpty))
                    .into_any_element(),
                Loadable::Idle | Loadable::Loading if error.is_none() => div()
                    .py(px(20.0))
                    .text_color(theme.text_muted)
                    .child(localizer.text(TextKey::ProvidersModelsLoading))
                    .into_any_element(),
                _ => div().into_any_element(),
            };
        let mut card = popover::dialog_card(&theme)
            .id("providers-fetch-dialog-card")
            .track_focus(&focus)
            .role(gpui::Role::Dialog)
            .aria_label(localizer.text(TextKey::ProvidersModelsFetch))
            .w(px(660.0))
            .max_w(viewport.width - px(40.0))
            .flex()
            .flex_col()
            .gap(px(14.0))
            .on_key_down(cx.listener(|page, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    page.close_model_fetch(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(popover::dialog_title(
                &theme,
                &localizer.text(TextKey::ProvidersModelsFetch),
            ))
            .child(body);
        if let Some(error) = error {
            card = card.child(widgets::error_strip(&theme, error).when(
                self.conflict.is_none(),
                |strip| {
                    strip.child(
                        widgets::text_action(
                            &theme,
                            widgets::ActionTone::Outlined,
                            localizer.text(TextKey::ActionRetry),
                        )
                        .id("providers-fetch-retry")
                        .tab_index(0)
                        .role(gpui::Role::Button)
                        .on_click(cx.listener(|page, _, _, cx| page.retry_model_fetch(cx))),
                    )
                },
            ));
        }
        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.0))
                .child(
                    widgets::text_action(
                        &theme,
                        widgets::ActionTone::Quiet,
                        localizer.text(TextKey::ActionCancel),
                    )
                    .id("providers-fetch-cancel")
                    .tab_index(0)
                    .role(gpui::Role::Button)
                    .on_click(
                        cx.listener(|page, _, window, cx| page.close_model_fetch(window, cx)),
                    ),
                )
                .child(
                    widgets::text_action(
                        &theme,
                        widgets::ActionTone::Solid,
                        localizer.text(TextKey::ProvidersModelsImport),
                    )
                    .id("providers-fetch-import")
                    .debug_selector(|| "providers-fetch-import".into())
                    .tab_index(0)
                    .role(gpui::Role::Button)
                    .when(!can_import, |button| button.opacity(0.5))
                    .on_click(
                        cx.listener(|page, _, window, cx| page.import_fetched_models(window, cx)),
                    ),
                ),
        );
        Some(popover::modal(
            "providers-fetch-dialog",
            viewport,
            card.into_any_element(),
        ))
    }

    fn render_fetched_row(&mut self, index: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.fetch_dialog.as_ref()?;
        let model = dialog.models.ready()?.get(index)?.clone();
        let selected = dialog.selected.contains(&model.model);
        let exists = self
            .draft
            .as_ref()?
            .providers
            .iter()
            .find(|p| p.id == dialog.provider_id)?
            .models
            .iter()
            .any(|existing| existing.model == model.model);
        let theme = Theme::of(cx).for_popup();
        let name = model.model.clone();
        Some(
            div()
                .id(("providers-fetch-row", index))
                .h(px(40.0))
                .flex()
                .items_center()
                .gap(px(10.0))
                .px(px(10.0))
                .border_b_1()
                .border_color(theme.border)
                .child(
                    fetch_checkbox(
                        &theme,
                        format!("providers-fetch-check-{index}").into(),
                        model.model.clone().into(),
                        selected,
                        false,
                    )
                    .on_click(
                        cx.listener(move |page, _, _, cx| page.toggle_fetched_model(&name, cx)),
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.text)
                                .child(model.model),
                        )
                        .when(exists, |cell| {
                            cell.child(widgets::badge(
                                &theme,
                                crate::l10n::localizer().text(TextKey::ProvidersModelsAdded),
                            ))
                        }),
                )
                .child(
                    div()
                        .w(px(120.0))
                        .truncate()
                        .text_color(theme.text_muted)
                        .child(model.owned_by.unwrap_or_else(|| "—".into())),
                )
                .into_any_element(),
        )
    }
}

fn fetch_checkbox(
    theme: &Theme,
    id: SharedString,
    label: SharedString,
    checked: bool,
    mixed: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id.clone())
        .debug_selector(move || id.to_string())
        .tab_index(0)
        .role(gpui::Role::CheckBox)
        .aria_label(label)
        .aria_toggled(if mixed {
            gpui::Toggled::Mixed
        } else if checked {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .size(px(28.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .child(
            div()
                .size(px(16.0))
                .rounded(px(3.0))
                .border_1()
                .border_color(if checked || mixed {
                    theme.accent
                } else {
                    theme.border
                })
                .bg(if checked || mixed {
                    theme.accent
                } else {
                    theme.input_glass_bg()
                })
                .flex()
                .items_center()
                .justify_center()
                .when(checked, |square| {
                    square.child(
                        crate::icons::icon(crate::icons::CHECK)
                            .size(px(12.0))
                            .text_color(theme.on_solid),
                    )
                })
                .when(mixed, |square| {
                    square.child(div().w(px(8.0)).h(px(2.0)).bg(theme.on_solid))
                }),
        )
}
