use super::*;
use rcode_harness::model_catalog::{self, ModelMetadata};

impl ProvidersPage {
    pub(super) fn model_metadata(
        &self,
        provider: usize,
        model: &str,
        cx: &Context<Self>,
    ) -> ModelMetadata {
        let state = self.state.read(cx);
        let Some(root) = state.data_dir.as_deref() else {
            return ModelMetadata::default();
        };
        let Some(provider_edit) = self
            .draft
            .as_ref()
            .and_then(|draft| draft.providers.get(provider))
        else {
            return ModelMetadata::default();
        };
        let endpoint = self
            .form
            .as_ref()
            .filter(|form| form.index == provider)
            .map(|form| trimmed(form.base_url.read(cx).text()))
            .unwrap_or_else(|| provider_edit.base_url.clone())
            .unwrap_or_else(|| provider_edit.protocol.default_endpoint().to_owned());
        model_catalog::cached(root).lookup(
            model,
            &provider_edit.id,
            &endpoint,
            provider_edit.protocol,
        )
    }

    pub(super) fn autofill_model_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = &self.model_dialog else {
            return;
        };
        let name = dialog.model.read(cx).text().trim().to_owned();
        if dialog.catalog_model == name {
            return;
        }
        let metadata = self.model_metadata(dialog.provider, &name, cx);
        let dialog = self.model_dialog.as_mut().unwrap();
        dialog.catalog_model = name;
        dialog.context_window.update(cx, |input, cx| {
            input.set_text(metadata.context_window.to_string(), cx)
        });
        dialog.max_tokens.update(cx, |input, cx| {
            input.set_text(metadata.max_tokens.to_string(), cx)
        });
        dialog.input_format = metadata.input_format;
        dialog.reasoning_levels = metadata.reasoning_levels;
        dialog.error = None;
        cx.notify();
    }
}
