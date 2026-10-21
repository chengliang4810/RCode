//! Direct access to the selected model's reasoning levels.
use super::*;

impl Pickers {
    pub(super) fn model_chip_label(&self, label: SharedString, cx: &App) -> SharedString {
        if self.title.is_some() || self.effective_harness(cx) != Some(HarnessId::Native) {
            return label;
        }
        if let Some(model) = self
            .provider_menu
            .view
            .as_ref()
            .into_iter()
            .flat_map(|view| &view.providers)
            .flat_map(|provider| &provider.models)
            .find(|model| Some(model.id.as_str()) == self.effective_model_id(cx))
        {
            return model
                .label
                .as_deref()
                .unwrap_or(&model.model)
                .to_owned()
                .into();
        }
        // Native catalog labels are provider + separator + model while the
        // provider settings snapshot is still loading.
        label
            .split_once(" · ")
            .map(|(_, model)| SharedString::from(model.to_owned()))
            .unwrap_or(label)
    }

    pub(super) fn direct_effort_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        self.compact_keyboard = true;
        match event.keystroke.key.as_str() {
            "left" | "right" | "home" | "end" => {
                if let Some(effort) = self.compact_effort(cx) {
                    let last = effort.labels.len().saturating_sub(1);
                    let index = match event.keystroke.key.as_str() {
                        "left" => effort.selected.saturating_sub(1),
                        "right" => (effort.selected + 1).min(last),
                        "home" => 0,
                        _ => last,
                    };
                    self.pick_compact_effort(effort, index, cx);
                }
            }
            "enter" | "escape" => self.animate_close(cx),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}
