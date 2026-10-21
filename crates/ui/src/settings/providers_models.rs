//! Reorder a provider's models without changing their identities or the active model.
use super::*;
use crate::{
    motion,
    terminal::panel::{drop_index, reorder_tabs, slide_offset},
};
use gpui::AnimationExt;

const ROW_HEIGHT: f32 = 60.0;
#[derive(Clone, PartialEq, Eq)]
struct Payload {
    owner: gpui::EntityId,
    provider: String,
    from: usize,
    models: Vec<String>,
}
pub(super) struct ModelDrag {
    payload: Payload,
    over: usize,
    previous: usize,
    epoch: usize,
    pointer: f32,
    grab: f32,
}
struct Ghost;
impl Render for Ghost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}
impl ProvidersPage {
    fn accepts_model_drag(&self, payload: &Payload, cx: &Context<Self>) -> bool {
        !self.has_pending_save()
            && self.conflict.is_none()
            && self.model_dialog.is_none()
            && self.fetch_dialog.is_none()
            && self.confirming_delete.is_none()
            && payload.owner == cx.entity().entity_id()
            && payload.from < payload.models.len()
            && self
                .selected
                .and_then(|index| self.draft.as_ref()?.providers.get(index))
                .is_some_and(|provider| {
                    provider.id == payload.provider
                        && provider
                            .models
                            .iter()
                            .map(|model| &model.id)
                            .eq(payload.models.iter())
                })
    }
    fn begin_model_drag(&mut self, payload: &Payload, grab: f32, cx: &mut Context<Self>) {
        if !self.accepts_model_drag(payload, cx) {
            return;
        }
        self.model_drag = Some(ModelDrag {
            payload: payload.clone(),
            over: payload.from,
            previous: payload.from,
            epoch: 0,
            pointer: payload.from as f32 * ROW_HEIGHT + grab,
            grab: grab.clamp(0.0, ROW_HEIGHT),
        });
        cx.notify();
    }
    fn move_model_drag(&mut self, payload: &Payload, pointer: f32, cx: &mut Context<Self>) {
        if !self.accepts_model_drag(payload, cx) {
            self.model_drag = None;
            cx.notify();
            return;
        }
        let Some(drag) = self
            .model_drag
            .as_mut()
            .filter(|drag| drag.payload == *payload)
        else {
            return;
        };
        let over = drop_index(pointer, ROW_HEIGHT, payload.models.len());
        if drag.over != over {
            drag.previous = drag.over;
            drag.over = over;
            drag.epoch = drag.epoch.wrapping_add(1);
        }
        drag.pointer = pointer;
        cx.notify();
    }
    fn commit_model_order(&mut self, payload: &Payload, to: usize, cx: &mut Context<Self>) {
        if !self.accepts_model_drag(payload, cx) || to >= payload.models.len() || to == payload.from
        {
            return;
        }
        let Some(provider) = self.draft.as_mut().and_then(|draft| {
            draft
                .providers
                .iter_mut()
                .find(|provider| provider.id == payload.provider)
        }) else {
            return;
        };
        reorder_tabs(&mut provider.models, payload.from, to);
        self.schedule_save(cx);
    }
    fn drop_model_drag(&mut self, payload: &Payload, cx: &mut Context<Self>) {
        if let Some(drag) = self
            .model_drag
            .take()
            .filter(|drag| drag.payload == *payload)
        {
            self.commit_model_order(payload, drag.over, cx);
        }
        cx.notify();
    }
    pub(super) fn render_model_list(
        &mut self,
        provider: usize,
        models: Vec<(usize, ModelEdit)>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let localizer = crate::l10n::localizer();
        if models.is_empty() {
            return div()
                .w_full()
                .px(px(14.0))
                .py(px(18.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    crate::icons::icon(crate::icons::INFO_CIRCLE)
                        .size(px(15.0))
                        .flex_none()
                        .text_color(theme.text_faint),
                )
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(12.5))
                        .text_color(theme.text_muted)
                        .child(localizer.text(TextKey::ProvidersModelEmpty)),
                );
        }
        let provider_id = self
            .draft
            .as_ref()
            .and_then(|draft| draft.providers.get(provider))
            .map(|provider| provider.id.clone())
            .unwrap_or_default();
        let snapshot = models
            .iter()
            .map(|(_, model)| model.id.clone())
            .collect::<Vec<_>>();
        let mut list = div()
            .id("providers-model-list")
            .debug_selector(|| "providers-model-list".into())
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(10.0))
            .border_1()
            .border_color(theme.border)
            .overflow_hidden();
        for (slot, model) in &models {
            let row = self.model_order_row(
                provider,
                *slot,
                model,
                &provider_id,
                &snapshot,
                false,
                theme,
                cx,
            );
            let row = if let Some(drag) = &self.model_drag {
                if *slot == drag.payload.from {
                    div().opacity(0.0).child(row).into_any_element()
                } else {
                    let start = slide_offset(*slot, drag.payload.from, drag.previous) * ROW_HEIGHT;
                    let end = slide_offset(*slot, drag.payload.from, drag.over) * ROW_HEIGHT;
                    if cx.reduce_motion() {
                        div().relative().top(px(end)).child(row).into_any_element()
                    } else {
                        div()
                            .child(row)
                            .with_animation(
                                SharedString::from(format!(
                                    "provider-model-{slot}-slide-{}",
                                    drag.epoch
                                )),
                                motion::TAB_SLIDE.animation(),
                                move |el, t| el.relative().top(px(motion::lerp(start, end, t))),
                            )
                            .into_any_element()
                    }
                }
            } else {
                row.into_any_element()
            };
            list = list.child(row);
        }
        if let Some(drag) = &self.model_drag {
            if let Some((slot, model)) = models.get(drag.payload.from) {
                let top = (drag.pointer - drag.grab)
                    .clamp(0.0, models.len().saturating_sub(1) as f32 * ROW_HEIGHT);
                list = list.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(px(top + 1.0))
                        .bg(popover::surface_bg(theme))
                        .shadow_md()
                        .child(self.model_order_row(
                            provider,
                            *slot,
                            model,
                            &provider_id,
                            &snapshot,
                            true,
                            theme,
                            cx,
                        )),
                );
            }
        }
        list = list
            .on_drag_move::<Payload>(cx.listener(
                |page, event: &gpui::DragMoveEvent<Payload>, window, cx| {
                    let payload = event.drag(cx).clone();
                    if payload.owner == cx.entity().entity_id() {
                        page.move_model_drag(
                            &payload,
                            f32::from(event.event.position.y - event.bounds.top()) - 1.0,
                            cx,
                        );
                        cx.set_active_drag_cursor_style(gpui::CursorStyle::ClosedHand, window);
                    }
                },
            ))
            .on_drop::<Payload>(
                cx.listener(|page, payload, _, cx| page.drop_model_drag(payload, cx)),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|page, _, _, cx| {
                    if page.model_drag.take().is_some() {
                        cx.notify();
                    }
                }),
            );
        div().w_full().child(list)
    }
    fn model_order_row(
        &self,
        provider: usize,
        slot: usize,
        model: &ModelEdit,
        provider_id: &str,
        snapshot: &[String],
        lifted: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let localizer = crate::l10n::localizer();
        let payload = Payload {
            owner: cx.entity().entity_id(),
            provider: provider_id.into(),
            from: slot,
            models: snapshot.to_vec(),
        };
        let enabled = !lifted && self.accepts_model_drag(&payload, cx) && snapshot.len() > 1;
        let editable = !lifted && self.model_drag.is_none();
        let suffix = if lifted { "lifted" } else { "row" };
        let row_id: SharedString = format!("providers-model-{slot}-{suffix}").into();
        let handle_id: SharedString = format!("providers-model-{slot}-{suffix}-drag").into();
        let entity = cx.entity();
        let keyboard = payload.clone();
        let display = model.label.as_deref().unwrap_or(&model.model).to_owned();
        div()
            .id(row_id.clone())
            .debug_selector(move || row_id.to_string())
            .h(px(ROW_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(14.0))
            .when(slot + 1 < snapshot.len(), |el| {
                el.border_b_1().border_color(widgets::row_divider(theme))
            })
            .child(
                div()
                    .id(handle_id.clone())
                    .debug_selector(move || handle_id.to_string())
                    .w(px(16.0))
                    .h_full()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .role(gpui::Role::Button)
                    .tab_index(0)
                    .aria_label(localizer.text(TextKey::ProvidersModelReorder))
                    .cursor(if enabled {
                        gpui::CursorStyle::OpenHand
                    } else {
                        gpui::CursorStyle::Arrow
                    })
                    .when(!enabled, |el| el.opacity(0.35))
                    .child(
                        crate::icons::icon(crate::icons::QUEUE_DRAG_HANDLE)
                            .size(px(13.0))
                            .text_color(theme.text_muted),
                    )
                    .when(enabled, |el| {
                        el.on_drag(payload, move |payload, point, _, cx| {
                            entity.update(cx, |page, cx| {
                                page.begin_model_drag(payload, f32::from(point.y), cx)
                            });
                            cx.stop_propagation();
                            cx.new(|_| Ghost)
                        })
                    })
                    .on_key_down(cx.listener(move |page, event: &gpui::KeyDownEvent, _, cx| {
                        if page.model_drag.is_some() {
                            return;
                        }
                        let to = match event.keystroke.key.as_str() {
                            "up" => slot.checked_sub(1),
                            "down" => Some(slot + 1),
                            _ => return,
                        };
                        if let Some(to) = to {
                            page.commit_model_order(&keyboard, to, cx);
                        }
                        cx.stop_propagation();
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_size(crate::typography::ui_rems(13.0))
                            .text_color(theme.text)
                            .child(display.clone()),
                    )
                    .when(display != model.model, |el| {
                        el.child(
                            div()
                                .truncate()
                                .text_size(crate::typography::ui_rems(11.5))
                                .text_color(theme.text_faint)
                                .child(model.model.clone()),
                        )
                    }),
            )
            .when_some(context_badge(&model.context_window), |el, badge| {
                el.child(widgets::badge(theme, badge))
            })
            .when(model.input_format.accepts_images(), |el| {
                el.child(widgets::badge(
                    theme,
                    localizer.text(TextKey::ProvidersModelInputVisionBadge),
                ))
            })
            .child(
                widgets::ghost_action(theme)
                    .id(("providers-model-edit", slot))
                    .tab_index(0)
                    .role(gpui::Role::Button)
                    .aria_label(localizer.text(TextKey::ProvidersModelEdit))
                    .tooltip(widgets::text_tooltip(
                        localizer.text(TextKey::ProvidersModelEdit),
                    ))
                    .when(editable, |el| {
                        el.on_click(cx.listener(move |page, _, window, cx| {
                            page.open_model_dialog(provider, Some(slot), window, cx)
                        }))
                    })
                    .child(
                        crate::icons::icon(crate::icons::PEN)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    ),
            )
            .child(
                widgets::ghost_action(theme)
                    .id(("providers-model-delete", slot))
                    .tab_index(0)
                    .role(gpui::Role::Button)
                    .aria_label(localizer.text(TextKey::ProvidersModelDelete))
                    .tooltip(widgets::text_tooltip(
                        localizer.text(TextKey::ProvidersModelDelete),
                    ))
                    .when(editable, |el| {
                        el.on_click(
                            cx.listener(move |page, _, _, cx| {
                                page.delete_model(provider, slot, cx)
                            }),
                        )
                    })
                    .child(
                        crate::icons::icon(crate::icons::TRASH_BIN_MINIMALISTIC)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    ),
            )
    }
}
