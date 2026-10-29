//! The model menu's pinned entry into provider/model settings.
use super::*;

pub(super) const FOOTER_HEIGHT: f32 = compact::COMPACT_ROW_HEIGHT + 2.0 * popover::CARD_INSET + 1.0;

impl Pickers {
    pub(super) fn model_management_only(&self, cx: &App) -> bool {
        self.needs_model_setup(cx)
            || (self.no_agents_available() && self.effective_harness(cx).is_none())
    }

    pub(super) fn open_model_management(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(cx);
        window.dispatch_action(Box::new(crate::shell::OpenProviderSettings), cx);
    }

    pub(super) fn render_model_management(
        &self,
        separator: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).for_popup();
        let label = crate::l10n::localizer().text(TextKey::PickersManageModels);
        div()
            .flex_none()
            .h(px(FOOTER_HEIGHT))
            .p(px(popover::CARD_INSET))
            .when(separator, |el| {
                el.border_t_1().border_color(crate::theme::hairline(0.08))
            })
            .child(
                popover::menu_row(&theme, false, "model-management")
                    .id("model-management")
                    .debug_selector(|| "model-management".into())
                    .h(px(compact::COMPACT_ROW_HEIGHT))
                    .py_0()
                    .role(gpui::Role::MenuItem)
                    .aria_label(label.clone())
                    .tab_index(0)
                    .focus_visible(move |style| style.bg(theme.element_hover))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_model_management(window, cx);
                        cx.stop_propagation();
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.open_model_management(window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        crate::icons::icon(crate::icons::SETTINGS)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    )
                    .child(div().flex_1().child(label)),
            )
            .into_any_element()
    }
}
