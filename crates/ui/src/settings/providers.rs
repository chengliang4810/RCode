//! Settings → Providers: the vendors and models this device runs against.
//!
//! # Two panes
//!
//! A list on the left, the selected provider's form on the right. The list is
//! the *summary* — a name and whether anything needs attention — and everything
//! editable lives in one form at a time.
//!
//! One form rather than a form per provider because a text input entity per
//! field per provider is a page whose size follows the configuration, and GPUI
//! wants those entities created and owned explicitly. Selecting a provider
//! rebuilds the form's inputs; leaving one folds what was typed back into the
//! draft, so switching away never loses an edit.
//!
//! # Models are added in a dialog
//!
//! Model fields are entered and validated together. Inline rows would mean
//! more input entities per model — the same unbounded-form problem, one
//! level down — and a half-typed model row is not a thing worth keeping in the
//! draft.
//!
//! # Saving
//!
//! Text edits are saved on blur; completed choices are saved immediately, one
//! request at a time. Leaving the page also submits its valid pending edits.
//! The save carries the revision the draft was written against. A refusal is a
//! conflict, not a failure: the file changed, the draft is still good, and the
//! user chooses between reloading and discarding. That is why the conflict is
//! its own state rather than a message in the error strip.
//!
//! # Credentials
//!
//! The catalogue carries only [`CredentialView`]. The selected form reads its
//! key separately into a masked input. Loading or revealing it is not an edit:
//! [`CredentialEdit::Keep`] preserves it until the user changes the value.

use gpui::{
    AnyElement, Context, Entity, Focusable, SharedString, Subscription, Task, Window, div,
    prelude::*, px,
};
use rcode_harness::provider_store::{
    CredentialUpdate, CredentialView, ModelView, ProviderDraft, ProviderDraftEntry,
    ProviderSettingsView, ProviderView,
};
use rcode_harness::providers::{InputFormat, ModelConfig, Protocol};
use rcode_presentation::{Localizer, TextKey};
use rcode_proto::ReasoningLevel;
use rcode_rpc::methods;

use crate::composer::ComposerInput;
use crate::popover::{self, Loadable};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

#[cfg(test)]
#[path = "providers_tests.rs"]
mod tests;

#[path = "providers_fetch.rs"]
mod fetch;
use fetch::ModelFetchDialog;
#[path = "providers_models.rs"]
mod model_order;
use model_order::ModelDrag;
#[path = "providers_catalog.rs"]
mod catalog;

/// The list rail's width. Narrow enough to leave the form room beside the
/// settings nav that is already there.
const LIST_WIDTH: f32 = 208.0;
/// This page is wider than the single-column settings pages: a two-pane editor
/// does not fit in the 760 they use.
const PAGE_WIDTH: f32 = 1000.0;

// ---------------------------------------------------------------------------
// Draft
// ---------------------------------------------------------------------------

/// The whole configuration as the user has it, plus the revision it was read
/// at. Plain data: the text being typed lives in the open form's input
/// entities and is folded back here when the selection changes or a save
/// begins.
struct Draft {
    revision: String,
    providers: Vec<ProviderEdit>,
}

/// One provider in the draft.
#[derive(Clone, PartialEq)]
struct ProviderEdit {
    id: String,
    label: Option<String>,
    protocol: Protocol,
    base_url: Option<String>,
    credential: CredentialEdit,
    models: Vec<ModelEdit>,
}

impl ProviderEdit {
    fn from_view(view: &ProviderView) -> Self {
        Self {
            id: view.id.clone(),
            label: view.label.clone(),
            protocol: protocol_from_wire(&view.protocol),
            base_url: view.base_url.clone(),
            // Existing provider identifiers are internal and never edited here.
            credential: CredentialEdit::Keep,
            models: view.models.iter().map(ModelEdit::from_view).collect(),
        }
    }

    /// A provider added on this page. It starts with no models; the empty list
    /// says what to do next.
    fn added(id: String, label: String) -> Self {
        Self {
            id,
            label: Some(label),
            protocol: Protocol::OpenAiChat,
            base_url: None,
            credential: CredentialEdit::Keep,
            models: Vec::new(),
        }
    }

    /// What the rail row shows, falling back to the id the way the picker
    /// falls back to the vendor model name.
    fn display_label(&self) -> &str {
        self.label
            .as_deref()
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .unwrap_or(&self.id)
    }

    /// What the rail's status dot reports: nothing when the provider is usable.
    fn attention(&self) -> Option<ProviderAttention> {
        self.models
            .is_empty()
            .then_some(ProviderAttention::Incomplete)
    }
}

/// Why a rail row is flagged.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ProviderAttention {
    Incomplete,
}

/// One model in the draft. Ids are the identity, and the store replaces the
/// whole list, so there is nothing else to carry.
#[derive(Default, Clone, PartialEq)]
struct ModelEdit {
    id: String,
    model: String,
    label: Option<String>,
    context_window: String,
    max_tokens: String,
    input_format: InputFormat,
    /// The ladder, in the order the picker offers it. Empty means the model has
    /// no effort chip, which is the honest answer for a model that does not
    /// reason.
    reasoning_levels: Vec<ReasoningLevel>,
}

impl ModelEdit {
    fn apply_metadata(&mut self, metadata: rcode_harness::model_catalog::ModelMetadata) {
        self.context_window = metadata.context_window.to_string();
        self.max_tokens = metadata.max_tokens.to_string();
        self.input_format = metadata.input_format;
        self.reasoning_levels = metadata.reasoning_levels;
    }

    fn from_view(view: &ModelView) -> Self {
        // Sorted on read as well as on insert, so the chip row shows the same
        // order a save would write whatever order the file happens to hold.
        let mut reasoning_levels = view.reasoning_levels.clone();
        reasoning_levels.sort();
        Self {
            id: view.id.clone(),
            model: view.model.clone(),
            label: view.label.clone(),
            context_window: view
                .context_window
                .map(|v| v.to_string())
                .unwrap_or_default(),
            max_tokens: view.max_tokens.map(|v| v.to_string()).unwrap_or_default(),
            input_format: view.input_format,
            reasoning_levels,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
enum CredentialEdit {
    Keep,
    Replace(String),
}

// ---------------------------------------------------------------------------
// Live inputs
// ---------------------------------------------------------------------------

/// The selected provider's form, as input entities.
///
/// Built when the selection changes and read back when it changes again or a
/// save begins, so the entity count is bounded by one provider's fields rather
/// than by the size of the configuration.
struct Form {
    index: usize,
    label: Entity<ComposerInput>,
    base_url: Entity<ComposerInput>,
    secret: Entity<ComposerInput>,
    acknowledged_secret: Option<String>,
    secret_visible: bool,
    secret_loaded: bool,
    _key_task: Option<Task<()>>,
    _edits: Vec<Subscription>,
}

/// The add/edit-model dialog's fields.
struct ModelDialog {
    /// Which provider the model belongs to.
    provider: usize,
    /// The model's slot when editing; `None` when adding.
    slot: Option<usize>,
    /// Internal identity, generated once for a new model and never edited.
    id: String,
    model: Entity<ComposerInput>,
    /// Last name queried; opening an existing model preserves its saved overrides.
    catalog_model: String,
    context_window: Entity<ComposerInput>,
    max_tokens: Entity<ComposerInput>,
    input_format: InputFormat,
    reasoning_levels: Vec<ReasoningLevel>,
    /// Whether the level list under the "+" is showing.
    ///
    /// An inline list rather than a popover: this form is already inside a
    /// modal, and a menu layered over one has to win a stacking fight the
    /// dialog itself does not have.
    reasoning_add_open: bool,
    /// Set when the dialog was submitted with something wrong.
    error: Option<String>,
    _edits: Vec<Subscription>,
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

pub struct ProvidersPage {
    state: Entity<AppState>,
    saved: Loadable<ProviderSettingsView>,
    draft: Option<Draft>,
    /// Which provider the form shows, by index into the draft.
    selected: Option<usize>,
    form: Option<Form>,
    model_dialog: Option<ModelDialog>,
    fetch_dialog: Option<ModelFetchDialog>,
    /// The provider id awaiting deletion confirmation.
    confirming_delete: Option<String>,
    saving: bool,
    edit_generation: u64,
    saved_generation: u64,
    /// Set when a save was refused because the file moved.
    conflict: Option<String>,
    error: Option<String>,
    protocol_select: widgets::SelectState,
    model_drag: Option<ModelDrag>,
    task: Option<Task<()>>,
    _state: Subscription,
}

impl ProvidersPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        // Settings can open before the engine connects: load once it does.
        let state_sub = cx.observe(&state, |this: &mut Self, _, cx| {
            if matches!(this.saved, Loadable::Idle) {
                this.load(cx);
            }
        });
        let mut page = Self {
            state,
            saved: Loadable::Idle,
            draft: None,
            selected: None,
            form: None,
            model_dialog: None,
            fetch_dialog: None,
            confirming_delete: None,
            saving: false,
            edit_generation: 0,
            saved_generation: 0,
            conflict: None,
            error: None,
            protocol_select: widgets::SelectState::default(),
            model_drag: None,
            task: None,
            _state: state_sub,
        };
        page.load(cx);
        page
    }

    /// Read the configuration.
    fn load(&mut self, cx: &mut Context<Self>) {
        self.model_drag = None;
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if matches!(self.saved, Loadable::Idle) {
            self.saved = Loadable::Loading;
        }
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::GET_PROVIDER_SETTINGS, serde_json::json!({}))
                .await
                .map_err(|error| error.to_string())
                .and_then(|value| {
                    serde_json::from_value::<ProviderSettingsView>(value)
                        .map_err(|error| error.to_string())
                });
            this.update(cx, |page, cx| {
                match result {
                    Ok(view) => {
                        page.saved = Loadable::Ready(view);
                        // A reload discards the draft: it was written against a
                        // revision that is no longer current, and keeping it
                        // would mean the next save conflicts again.
                        page.draft = None;
                        page.form = None;
                        page.selected = None;
                        page.model_dialog = None;
                        page.fetch_dialog = None;
                        page.conflict = None;
                        page.error = None;
                        page.edit_generation += 1;
                        page.saved_generation = page.edit_generation;
                    }
                    Err(error) => page.saved = Loadable::Error(error),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// The draft, created from the stored view on first edit.
    fn draft(&mut self) -> Option<&mut Draft> {
        if self.draft.is_none() {
            let view = self.saved.ready()?;
            self.draft = Some(Draft {
                revision: view.revision.clone(),
                providers: view.providers.iter().map(ProviderEdit::from_view).collect(),
            });
        }
        self.draft.as_mut()
    }

    /// Show `index` in the form, folding whatever was in it back into the
    /// draft first.
    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected == Some(index) {
            return;
        }
        self.model_drag = None;
        self.fold_form(cx);
        self.close_model_dialog(cx);
        self.confirming_delete = None;
        self.selected = Some(index);
        self.build_form(index, window, cx);
        cx.notify();
    }

    /// Build the form's inputs from the draft's `index`.
    fn build_form(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((label, base_url, secret)) = self
            .draft()
            .and_then(|draft| draft.providers.get(index))
            .map(|edit| {
                (
                    edit.label.clone().unwrap_or_default(),
                    edit.base_url.clone().unwrap_or_default(),
                    match &edit.credential {
                        CredentialEdit::Keep => String::new(),
                        CredentialEdit::Replace(value) => value.clone(),
                    },
                )
            })
        else {
            self.form = None;
            return;
        };

        let label = text_input(&label, TextKey::ProvidersLabel, cx);
        let base_url = text_input_with(
            &base_url,
            Protocol::OpenAiChat.default_endpoint().to_owned(),
            cx,
        );
        let has_saved_key = self
            .draft
            .as_ref()
            .and_then(|draft| draft.providers.get(index))
            .and_then(|provider| {
                self.saved
                    .ready()?
                    .providers
                    .iter()
                    .find(|saved| saved.id == provider.id)
            })
            .is_some_and(|provider| {
                matches!(
                    provider.credential,
                    CredentialView::Stored | CredentialView::Environment { set: true, .. }
                )
            });
        let secret_loaded = !secret.is_empty() || !has_saved_key;
        let secret = text_input_with(
            &secret,
            if has_saved_key {
                "********".into()
            } else {
                crate::l10n::localizer().text(TextKey::ProvidersApiKey)
            },
            cx,
        );
        secret.update(cx, |input, cx| input.set_masked(true, cx));

        let edits = [&label, &base_url, &secret]
            .into_iter()
            .map(|input| {
                cx.on_blur(&input.focus_handle(cx), window, |page, _, cx| {
                    page.commit_form(cx);
                })
            })
            .collect();
        let focus = label.focus_handle(cx);
        self.form = Some(Form {
            index,
            label,
            base_url,
            secret,
            acknowledged_secret: None,
            secret_visible: false,
            secret_loaded,
            _key_task: None,
            _edits: edits,
        });
        self.load_form_key(cx);
        window.focus(&focus, cx);
    }

    /// Fold the open form's fields back into the draft and drop the inputs.
    fn fold_form(&mut self, cx: &mut Context<Self>) {
        self.commit_form(cx);
        self.form = None;
    }

    fn commit_form(&mut self, cx: &mut Context<Self>) {
        if self.sync_form(cx) {
            self.schedule_save(cx);
        }
    }

    /// Called before Settings is hidden, including routes that do not move focus.
    pub(crate) fn flush_pending(&mut self, cx: &mut Context<Self>) {
        self.model_drag = None;
        self.commit_form(cx);
        self.sync_existing_model(cx);
        self.save(cx);
    }

    pub(crate) fn has_pending_save(&self) -> bool {
        self.saving || self.edit_generation != self.saved_generation
    }

    fn sync_form(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(form) = self.form.as_ref() else {
            return false;
        };
        let index = form.index;
        let label = trimmed(form.label.read(cx).text());
        let base_url = trimmed(form.base_url.read(cx).text());
        let secret = form.secret.read(cx).text().trim().to_owned();
        let credential =
            if secret.is_empty() || form.acknowledged_secret.as_deref() == Some(secret.as_str()) {
                CredentialEdit::Keep
            } else {
                CredentialEdit::Replace(secret)
            };
        let Some(edit) = self
            .draft()
            .and_then(|draft| draft.providers.get_mut(index))
        else {
            return false;
        };
        if edit.label == label && edit.base_url == base_url && edit.credential == credential {
            return false;
        }
        edit.label = label;
        edit.base_url = base_url;
        edit.credential = credential;
        true
    }

    /// Add a provider with a generated internal id and a unique suggested name.
    fn add_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_model_dialog(cx);
        self.fold_form(cx);
        let base_name = crate::l10n::localizer().text(TextKey::ProvidersNew);
        let Some(draft) = self.draft() else {
            return;
        };
        let mut suffix = 1;
        let label = loop {
            let candidate = if suffix == 1 {
                base_name.clone()
            } else {
                crate::l10n::localizer().format_with(
                    TextKey::ProvidersNewNumbered,
                    &[("number", suffix.to_string().as_str())],
                )
            };
            if !draft.providers.iter().any(|provider| {
                provider
                    .label
                    .as_deref()
                    .is_some_and(|name| name == candidate)
            }) {
                break candidate;
            }
            suffix += 1;
        };
        let id = loop {
            let candidate = format!("p_{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
            if !draft
                .providers
                .iter()
                .any(|provider| provider.id == candidate)
            {
                break candidate;
            }
        };
        draft.providers.push(ProviderEdit::added(id, label));
        let index = draft.providers.len() - 1;
        self.protocol_select = widgets::SelectState::default();
        self.confirming_delete = None;
        self.selected = Some(index);
        self.build_form(index, window, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    fn delete_provider(&mut self, provider_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.fold_form(cx);
        let index = self.draft.as_ref().and_then(|draft| {
            draft
                .providers
                .iter()
                .position(|provider| provider.id == provider_id)
        });
        let Some(index) = index else {
            self.confirming_delete = None;
            cx.notify();
            return;
        };
        if let Some(draft) = self.draft() {
            draft.providers.remove(index);
        }
        self.confirming_delete = None;
        self.selected = None;
        self.protocol_select = widgets::SelectState::default();
        self.ensure_form(window, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    fn ensure_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.draft() else { return };
        let count = draft.providers.len();
        if count == 0 {
            self.selected = None;
            self.form = None;
        } else if self.form.is_none() {
            let index = self.selected.filter(|index| *index < count).unwrap_or(0);
            self.selected = Some(index);
            self.build_form(index, window, cx);
        }
    }

    fn close_delete_dialog(&mut self, cx: &mut Context<Self>) {
        self.confirming_delete = None;
        cx.notify();
    }

    fn toggle_reveal_key(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.form.as_mut() {
            form.secret_visible = !form.secret_visible;
            let masked = !form.secret_visible;
            form.secret
                .update(cx, |input, cx| input.set_masked(masked, cx));
        }
        self.load_form_key(cx);
        cx.notify();
    }

    fn load_form_key(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_ref() else {
            return;
        };
        if form.secret_loaded || form._key_task.is_some() {
            return;
        }
        let secret = form.secret.clone();
        let revision = secret.read(cx).edit_revision();
        let Some(provider_id) = self
            .draft
            .as_ref()
            .and_then(|draft| draft.providers.get(form.index))
            .map(|provider| provider.id.clone())
        else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let task = cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(
                    methods::GET_PROVIDER_API_KEY,
                    serde_json::json!({ "providerId": provider_id }),
                )
                .await
                .map_err(|error| error.to_string());
            this.update(cx, |page, cx| {
                let Some(form) = page.form.as_mut().filter(|form| form.secret == secret) else {
                    return;
                };
                form._key_task = None;
                if secret.read(cx).edit_revision() != revision {
                    form.secret_loaded = true;
                    return;
                }
                match result {
                    Ok(value) => {
                        let key = value
                            .get("apiKey")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        form.acknowledged_secret = Some(key.clone());
                        form.secret_loaded = true;
                        secret.update(cx, |input, cx| {
                            input.set_text(key, cx);
                            input.set_placeholder(
                                crate::l10n::localizer().text(TextKey::ProvidersApiKey),
                                cx,
                            );
                        });
                    }
                    Err(error) => page.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        });
        self.form.as_mut().unwrap()._key_task = Some(task);
    }

    /// What the dialog's model accepts as input.

    fn open_model_dialog(
        &mut self,
        provider: usize,
        slot: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.model_drag.is_some() {
            return;
        }
        let mut existing = slot
            .and_then(|slot| {
                self.draft
                    .as_ref()
                    .and_then(|draft| draft.providers.get(provider))
                    .and_then(|edit| edit.models.get(slot))
                    .cloned()
            })
            .unwrap_or_default();
        if slot.is_none() {
            existing.apply_metadata(rcode_harness::model_catalog::ModelMetadata::default());
        }
        let id = if slot.is_some() {
            existing.id.clone()
        } else {
            self.new_model_id()
        };
        let model = text_input(&existing.model, TextKey::ProvidersModelNameHint, cx);
        let context_window = text_input(
            &existing.context_window,
            TextKey::ProvidersModelContextHint,
            cx,
        );
        let max_tokens = text_input(
            &existing.max_tokens,
            TextKey::ProvidersModelMaxTokensHint,
            cx,
        );
        let mut edits: Vec<_> = [&model, &context_window, &max_tokens]
            .into_iter()
            .map(|input| {
                cx.on_blur(&input.focus_handle(cx), window, |page, _, cx| {
                    if page
                        .model_dialog
                        .as_ref()
                        .is_some_and(|dialog| dialog.slot.is_some())
                    {
                        page.sync_model_dialog(true, cx);
                    }
                })
            })
            .collect();
        edits.push(cx.subscribe(&model, |page, _, event, cx| {
            if matches!(event, crate::composer::ComposerInputEvent::Edited) {
                page.autofill_model_dialog(cx);
            }
        }));
        let focus = model.focus_handle(cx);
        self.model_dialog = Some(ModelDialog {
            provider,
            slot,
            id,
            model,
            catalog_model: existing.model.clone(),
            context_window,
            max_tokens,
            input_format: existing.input_format,
            reasoning_levels: existing.reasoning_levels.clone(),
            reasoning_add_open: false,
            error: None,
            _edits: edits,
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    fn close_model_dialog(&mut self, cx: &mut Context<Self>) {
        if self
            .model_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.slot.is_some())
            && !self.sync_model_dialog(true, cx)
        {
            return;
        }
        if self.model_dialog.take().is_some() {
            cx.notify();
        }
    }

    /// What the dialog's model accepts as input.
    fn set_dialog_input_format(&mut self, input_format: InputFormat, cx: &mut Context<Self>) {
        if let Some(dialog) = self.model_dialog.as_mut() {
            dialog.input_format = input_format;
        }
        self.sync_existing_model(cx);
        cx.notify();
    }

    /// Take a level out of the dialog's ladder. The chip is the only control
    /// for its own entry, so clicking it is the removal.
    fn toggle_dialog_level(&mut self, level: ReasoningLevel, cx: &mut Context<Self>) {
        if let Some(dialog) = self.model_dialog.as_mut() {
            if let Some(index) = dialog
                .reasoning_levels
                .iter()
                .position(|other| *other == level)
            {
                dialog.reasoning_levels.remove(index);
                cx.notify();
            }
        }
        self.sync_existing_model(cx);
    }

    /// Put a level into the dialog's ladder, in ladder order.
    ///
    /// Sorted on insert rather than appended: the list is what the composer
    /// offers, so appending would produce a chip row reading `high, low`.
    /// There is nothing to express with a custom order, so the page decides it.
    fn add_dialog_level(&mut self, level: ReasoningLevel, cx: &mut Context<Self>) {
        if let Some(dialog) = self.model_dialog.as_mut() {
            if !dialog.reasoning_levels.contains(&level) {
                dialog.reasoning_levels.push(level);
                dialog.reasoning_levels.sort();
            }
            cx.notify();
        }
        self.sync_existing_model(cx);
    }

    fn toggle_reasoning_add(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.model_dialog.as_mut() {
            dialog.reasoning_add_open = !dialog.reasoning_add_open;
            cx.notify();
        }
    }

    fn sync_existing_model(&mut self, cx: &mut Context<Self>) {
        if self
            .model_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.slot.is_some())
        {
            self.sync_model_dialog(true, cx);
        }
    }

    fn new_model_id(&self) -> String {
        loop {
            let candidate = format!("m_{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
            let taken = self.draft.as_ref().is_some_and(|draft| {
                draft
                    .providers
                    .iter()
                    .flat_map(|provider| &provider.models)
                    .any(|model| model.id == candidate)
            });
            if !taken {
                return candidate;
            }
        }
    }

    /// Creating a model needs a vendor name; editing keeps its internal identity.
    fn commit_model(&mut self, cx: &mut Context<Self>) {
        if self.sync_model_dialog(true, cx) {
            self.model_dialog = None;
            cx.notify();
        }
    }

    fn sync_model_dialog(&mut self, show_errors: bool, cx: &mut Context<Self>) -> bool {
        // Also query here so an immediate Add after typing uses the same metadata.
        self.autofill_model_dialog(cx);
        let Some(dialog) = self.model_dialog.as_ref() else {
            return false;
        };
        let localizer = crate::l10n::localizer();
        let id = dialog.id.clone();
        let model = dialog.model.read(cx).text().trim().to_owned();
        let context_window = dialog.context_window.read(cx).text().trim().to_owned();
        let max_tokens = dialog.max_tokens.read(cx).text().trim().to_owned();
        let (provider, slot) = (dialog.provider, dialog.slot);
        let input_format = dialog.input_format;
        let reasoning_levels = dialog.reasoning_levels.clone();

        // Validate while the editable fields are still visible in the dialog.
        let error = if model.is_empty() {
            Some(localizer.text(TextKey::ProvidersModelNameRequired))
        } else if [&context_window, &max_tokens]
            .into_iter()
            .any(|value| !value.is_empty() && !value.parse::<u32>().is_ok_and(|count| count > 0))
        {
            Some(localizer.text(TextKey::ProvidersModelCountInvalid))
        } else if parse_count(&context_window)
            .zip(parse_count(&max_tokens))
            .is_some_and(|(context, output)| output > context)
        {
            Some(localizer.text(TextKey::ProvidersModelOutputExceedsContext))
        } else {
            None
        };
        if let Some(error) = error {
            if show_errors && let Some(dialog) = self.model_dialog.as_mut() {
                dialog.error = Some(error);
            }
            cx.notify();
            return false;
        }

        let model = ModelEdit {
            id,
            model,
            label: slot.and_then(|slot| {
                self.draft
                    .as_ref()?
                    .providers
                    .get(provider)?
                    .models
                    .get(slot)?
                    .label
                    .clone()
            }),
            context_window,
            max_tokens,
            input_format,
            reasoning_levels,
        };
        let mut changed = false;
        if let Some(edit) = self
            .draft()
            .and_then(|draft| draft.providers.get_mut(provider))
        {
            match slot {
                Some(slot) if slot < edit.models.len() => {
                    if edit.models[slot] != model {
                        edit.models[slot] = model;
                        changed = true;
                    }
                }
                _ => {
                    edit.models.push(model);
                    changed = true;
                }
            }
        }
        if let Some(dialog) = self.model_dialog.as_mut() {
            dialog.error = None;
        }
        if changed {
            self.schedule_save(cx);
        }
        cx.notify();
        true
    }

    fn delete_model(&mut self, provider: usize, slot: usize, cx: &mut Context<Self>) {
        if self.model_drag.is_some() {
            return;
        }
        if let Some(edit) = self
            .draft()
            .and_then(|draft| draft.providers.get_mut(provider))
            && slot < edit.models.len()
        {
            edit.models.remove(slot);
            self.schedule_save(cx);
        }
        cx.notify();
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.edit_generation += 1;
        self.error = None;
        self.queue_save(cx);
        cx.notify();
    }

    fn queue_save(&self, cx: &mut Context<Self>) {
        let generation = self.edit_generation;
        // Coalesce changes in one UI event, without a typing timeout. Retain
        // the editor until the write finishes if Settings has already closed.
        let page = cx.entity().clone();
        cx.spawn(async move |_, cx| {
            page.update(cx, |page, cx| {
                if page.edit_generation == generation {
                    page.save(cx);
                }
            });
        })
        .detach();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving
            || self.model_drag.is_some()
            || self.conflict.is_some()
            || self.edit_generation == self.saved_generation
        {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(draft) = self.draft.as_ref() else {
            return;
        };
        let generation = self.edit_generation;
        let credentials: Vec<_> = draft
            .providers
            .iter()
            .map(|provider| (provider.id.clone(), provider.credential.clone()))
            .collect();
        let params = serde_json::json!({
            "expectedRevision": draft.revision,
            "draft": ProviderDraft {
                default_model: None,
                providers: draft
                    .providers
                    .iter()
                    .map(|edit| ProviderDraftEntry {
                        id: edit.id.clone(),
                        label: edit.label.clone(),
                        protocol: edit.protocol,
                        base_url: edit.base_url.clone(),
                        models: edit
                            .models
                            .iter()
                            .map(|model| ModelConfig {
                                id: model.id.clone(),
                                model: model.model.clone(),
                                label: model.label.clone(),
                                context_window: parse_count(&model.context_window),
                                max_tokens: parse_count(&model.max_tokens),
                                input_format: model.input_format,
                                reasoning_levels: model.reasoning_levels.clone(),
                            })
                            .collect(),
                        credential: match &edit.credential {
                            CredentialEdit::Keep => CredentialUpdate::Keep,
                            CredentialEdit::Replace(value) => {
                                CredentialUpdate::ReplaceLiteral { value: value.clone() }
                            }
                        },
                    })
                    .collect(),
                // Not edited on this page. `None` leaves a search endpoint and
                // token the user configured by hand exactly as they were.
                web_search: None,
            },
        });
        self.saving = true;
        self.conflict = None;
        self.error = None;
        let page = cx.entity().clone();
        cx.spawn(async move |_, cx| {
            let result = engine
                .client()
                .call(methods::SAVE_PROVIDER_SETTINGS, params)
                .await;
            page.update(cx, |page, cx| {
                page.saving = false;
                match result {
                    Ok(value) => match serde_json::from_value::<ProviderSettingsView>(value) {
                        Ok(view) => {
                            if let Some(draft) = page.draft.as_mut() {
                                draft.revision = view.revision.clone();
                                for (id, submitted) in &credentials {
                                    if let Some(provider) =
                                        draft.providers.iter_mut().find(|p| &p.id == id)
                                        && &provider.credential == submitted
                                    {
                                        provider.credential = CredentialEdit::Keep;
                                        if let CredentialEdit::Replace(value) = submitted
                                            && let Some(form) = page.form.as_mut().filter(|form| {
                                                draft
                                                    .providers
                                                    .get(form.index)
                                                    .is_some_and(|p| &p.id == id)
                                            })
                                            && form.secret.read(cx).text().trim() == value
                                        {
                                            // Keep the text and cursor stable if the user
                                            // resumes typing after this automatic save.
                                            form.acknowledged_secret = Some(value.clone());
                                        }
                                    }
                                }
                            }
                            page.saved = Loadable::Ready(view);
                            page.saved_generation = generation;
                            page.conflict = None;
                            // The composer's model chip is reading a cached
                            // catalogue that this save just replaced. Without
                            // this, a model added here is invisible until the
                            // application restarts.
                            crate::pickers::bump_harness_catalog(cx);
                        }
                        Err(error) => page.error = Some(error.to_string()),
                    },
                    // A conflict is a state, not a failure: the draft is kept
                    // so the user can copy anything they need out of it before
                    // deciding.
                    Err(error)
                        if error
                            .to_string()
                            .contains(rcode_harness::provider_store::CONFLICT_CODE) =>
                    {
                        page.conflict = Some(error.to_string());
                    }
                    Err(error) => page.error = Some(error.to_string()),
                }
                if page.edit_generation > generation && page.conflict.is_none() {
                    page.queue_save(cx);
                } else if page.error.is_none() && page.conflict.is_none() {
                    page.continue_model_fetch(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for ProvidersPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.model_drag.is_some() && !cx.has_active_drag() {
            self.model_drag = None;
        }
        self.ensure_form(window, cx);
        // Save failures remain beside the unsaved edits; read-only actions
        // such as revealing a key keep their existing toast feedback.
        if self.edit_generation == self.saved_generation {
            crate::toast::take_error(&mut self.error, cx);
        }
        let theme = Theme::of(cx).for_settings_surface();
        let delete_dialog = self.render_delete_dialog(window.viewport_size(), cx);
        let dialog = self
            .render_fetch_dialog(window.viewport_size(), cx)
            .or_else(|| self.render_model_dialog(window.viewport_size(), cx))
            .or(delete_dialog);
        let body = match self.saved.clone() {
            Loadable::Ready(view) => self.render_page(&view, &theme, cx).into_any_element(),
            Loadable::Error(error) => div()
                .child(widgets::error_strip(&theme, error))
                .into_any_element(),
            Loadable::Idle | Loadable::Loading => div()
                .text_color(theme.text_muted)
                .child(crate::l10n::localizer().text(TextKey::ActionLoading))
                .into_any_element(),
        };
        div()
            .size_full()
            .on_key_down(cx.listener(|page, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && page.model_drag.take().is_some() {
                    cx.stop_active_drag(window);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(body)
            .children(dialog)
    }
}

impl ProvidersPage {
    fn render_page(
        &mut self,
        view: &ProviderSettingsView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let localizer = crate::l10n::localizer();
        div()
            .size_full()
            .id("providers-page")
            .overflow_y_scroll()
            .child(
                div()
                    .w_full()
                    .max_w(px(PAGE_WIDTH))
                    .mx_auto()
                    .px(px(40.0))
                    .pt(px(Theme::TITLEBAR_HEIGHT + 16.0))
                    .pb(px(48.0))
                    .flex()
                    .flex_col()
                    .child(self.render_header(theme, cx))
                    .children(self.conflict.clone().map(|conflict| {
                        widgets::error_strip(theme, conflict).child(
                            widgets::text_action(
                                theme,
                                widgets::ActionTone::Outlined,
                                localizer.text(TextKey::ProvidersConflictReload),
                            )
                            .id("providers-conflict-reload")
                            .tab_index(0)
                            .role(gpui::Role::Button)
                            .ml(px(8.0))
                            .on_click(cx.listener(|page, _, _, cx| page.load(cx))),
                        )
                    }))
                    .children(self.error.clone().map(|error| {
                        widgets::error_strip(theme, error).child(
                            widgets::text_action(
                                theme,
                                widgets::ActionTone::Outlined,
                                localizer.text(TextKey::ActionRetry),
                            )
                            .id("providers-save-retry")
                            .tab_index(0)
                            .role(gpui::Role::Button)
                            .on_click(cx.listener(|page, _, _, cx| page.save(cx))),
                        )
                    }))
                    .child(self.render_panes(view, theme, cx)),
            )
    }

    fn render_header(&mut self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let localizer = crate::l10n::localizer();
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(12.0))
            .pb(px(16.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(26.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(localizer.text(TextKey::ProvidersTitle)),
                    )
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(13.0))
                            .text_color(theme.text_muted)
                            .child(localizer.text(TextKey::ProvidersSubtitle)),
                    ),
            )
            .child(
                widgets::text_action(
                    theme,
                    widgets::ActionTone::Solid,
                    localizer.text(TextKey::ProvidersAddShort),
                )
                .id("providers-add")
                .debug_selector(|| "providers-add".into())
                .tab_index(0)
                .role(gpui::Role::Button)
                .on_click(cx.listener(|page, _, window, cx| page.add_provider(window, cx))),
            )
    }

    /// The two panes: the rail, a hairline, and the selected provider's form.
    fn render_panes(
        &mut self,
        view: &ProviderSettingsView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div()
            .w_full()
            .min_h(px(520.0))
            .flex()
            .flex_row()
            .rounded(px(14.0))
            .border_1()
            .border_color(theme.border)
            .bg(theme.wash(0.02))
            .overflow_hidden()
            .child(self.render_rail(view, theme, cx))
            .child(div().w(px(1.0)).flex_none().bg(theme.border))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(self.render_form(view, theme, cx)),
            )
    }

    fn render_rail(
        &mut self,
        view: &ProviderSettingsView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let localizer = crate::l10n::localizer();
        let selected = self.selected;
        // Read from the draft when there is one so a row reflects an edit as
        // soon as the form changes, and from the stored view otherwise.
        let rows: Vec<(usize, String, Option<ProviderAttention>)> = match self.draft.as_ref() {
            Some(draft) => draft
                .providers
                .iter()
                .enumerate()
                .map(|(index, edit)| (index, edit.display_label().to_owned(), edit.attention()))
                .collect(),
            None => view
                .providers
                .iter()
                .enumerate()
                .map(|(index, stored)| {
                    (
                        index,
                        stored
                            .label
                            .as_deref()
                            .map(str::trim)
                            .filter(|label| !label.is_empty())
                            .unwrap_or(&stored.id)
                            .to_owned(),
                        None,
                    )
                })
                .collect(),
        };
        let mut rail = div()
            .w(px(LIST_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .py(px(12.0));
        for (index, label, attention) in rows {
            rail = rail.child(self.render_rail_row(index, label, attention, selected, theme, cx));
        }
        // "New provider" is the list's last entry rather than a toolbar
        // action: adding one is part of the list.
        rail.child(
            div()
                .id("providers-rail-add")
                .mx(px(8.0))
                .mt(px(4.0))
                .px(px(8.0))
                .py(px(7.0))
                .rounded(px(8.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .cursor_pointer()
                .tab_index(0)
                .role(gpui::Role::Button)
                .aria_label(localizer.text(TextKey::ProvidersNew))
                .text_size(crate::typography::ui_rems(13.0))
                .text_color(theme.text_muted)
                .hover(|style| style.bg(theme.glass_hover()).text_color(theme.text))
                .on_click(cx.listener(|page, _, window, cx| page.add_provider(window, cx)))
                .child(
                    crate::icons::icon(crate::icons::PLUS)
                        .size(px(14.0))
                        .flex_none()
                        .text_color(theme.text_faint),
                )
                .child(localizer.text(TextKey::ProvidersNew)),
        )
    }

    fn render_rail_row(
        &mut self,
        index: usize,
        label: String,
        attention: Option<ProviderAttention>,
        selected: Option<usize>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_selected = selected == Some(index);
        div()
            .id(("providers-rail-row", index))
            .mx(px(8.0))
            .px(px(8.0))
            .py(px(7.0))
            .rounded(px(8.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .cursor_pointer()
            .tab_index(0)
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .text_size(crate::typography::ui_rems(13.0))
            .text_color(if is_selected {
                theme.text
            } else {
                theme.text_muted
            })
            .when(is_selected, |el| el.bg(theme.wash(0.06)))
            .hover(|style| style.bg(theme.glass_hover()))
            .on_click(cx.listener(move |page, _, window, cx| page.select(index, window, cx)))
            .child(
                crate::icons::icon(crate::icons::WIDGET)
                    .size(px(14.0))
                    .flex_none()
                    .text_color(theme.text_faint),
            )
            .child(div().flex_1().min_w_0().truncate().child(label))
            .when_some(attention, |el, _| {
                el.child(
                    div()
                        .size(px(6.0))
                        .flex_none()
                        .rounded_full()
                        .bg(theme.warning),
                )
            })
            .into_any_element()
    }

    /// The selected provider's form, or the empty state when nothing is
    /// selected.
    fn render_form(
        &mut self,
        view: &ProviderSettingsView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let localizer = crate::l10n::localizer();
        let Some(index) = self.selected else {
            return empty_pane(theme, localizer);
        };
        let Some((provider_id, protocol, display, model_count, models)) = self
            .draft
            .as_ref()
            .and_then(|draft| draft.providers.get(index))
            .map(|edit| {
                (
                    edit.id.clone(),
                    edit.protocol,
                    edit.display_label().to_owned(),
                    edit.models.len(),
                    edit.models
                        .iter()
                        .enumerate()
                        .map(|(slot, model)| (slot, model.clone()))
                        .collect::<Vec<_>>(),
                )
            })
        else {
            return empty_pane(theme, localizer);
        };
        let credential_view = view
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .map(|provider| provider.credential.clone())
            .unwrap_or(CredentialView::None);

        let mut pane = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(18.0))
            .px(px(24.0))
            .py(px(20.0));

        // Header: the name, editable in place, and the row's own actions.
        pane = pane.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(
                    crate::icons::icon(crate::icons::WIDGET)
                        .size(px(18.0))
                        .flex_none()
                        .text_color(theme.text_faint),
                )
                .child(
                    div().flex_1().min_w(px(120.0)).child(
                        self.form
                            .as_ref()
                            .map(|form| title_input(theme, &form.label))
                            .unwrap_or_else(|| {
                                div()
                                    .text_size(crate::typography::ui_rems(17.0))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child(display.clone())
                            }),
                    ),
                )
                .child(
                    widgets::ghost_action(theme)
                        .id("providers-delete")
                        .tab_index(0)
                        .role(gpui::Role::Button)
                        .aria_label(localizer.text(TextKey::ProvidersDelete))
                        .tooltip(widgets::text_tooltip(
                            localizer.text(TextKey::ProvidersDelete),
                        ))
                        .on_click(cx.listener(move |page, _, _, cx| {
                            let provider_id = page
                                .draft
                                .as_ref()
                                .and_then(|draft| draft.providers.get(index))
                                .map(|provider| provider.id.clone());
                            if let Some(provider_id) = provider_id {
                                page.confirming_delete = Some(provider_id);
                                cx.notify();
                            }
                        }))
                        .child(
                            crate::icons::icon(crate::icons::TRASH_BIN_MINIMALISTIC)
                                .size(px(15.0))
                                .text_color(theme.text_muted),
                        ),
                ),
        );

        // Where and how: the endpoint, and the wire it speaks.
        pane = pane.child(
            div()
                .flex()
                .flex_row()
                .gap(px(12.0))
                .child(field(
                    theme,
                    localizer.text(TextKey::ProvidersBaseUrlLabel),
                    self.form
                        .as_ref()
                        .map(|form| text_field_of(theme, &form.base_url))
                        .unwrap_or_else(|| div()),
                ))
                .child(field(
                    theme,
                    localizer.text(TextKey::ProvidersApiFormat),
                    self.render_protocol_select(index, protocol, theme, cx),
                )),
        );

        let secret_visible = self.form.as_ref().is_some_and(|form| form.secret_visible);
        pane = pane.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(widgets::field_label(
                            theme,
                            localizer.text(TextKey::ProvidersApiKey),
                        ))
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(12.0))
                                .text_color(theme.text_faint)
                                .child(credential_detail(localizer, &credential_view)),
                        )
                        .child(div().flex_1())
                        .child(
                            widgets::ghost_action(theme)
                                .id("providers-reveal")
                                .debug_selector(|| "providers-reveal".into())
                                .tab_index(0)
                                .role(gpui::Role::Button)
                                .aria_label(localizer.text(if secret_visible {
                                    TextKey::ProvidersConceal
                                } else {
                                    TextKey::ProvidersReveal
                                }))
                                .tooltip(widgets::text_tooltip(localizer.text(if secret_visible {
                                    TextKey::ProvidersConceal
                                } else {
                                    TextKey::ProvidersReveal
                                })))
                                .on_click(cx.listener(move |page, _, _, cx| {
                                    page.toggle_reveal_key(cx);
                                }))
                                .child(
                                    crate::icons::icon(if secret_visible {
                                        crate::icons::EYE_CLOSED
                                    } else {
                                        crate::icons::EYE
                                    })
                                    .size(px(15.0))
                                    .text_color(
                                        if secret_visible {
                                            theme.text
                                        } else {
                                            theme.text_muted
                                        },
                                    ),
                                ),
                        ),
                )
                .child(
                    div().flex().flex_row().items_center().gap(px(6.0)).child(
                        div().flex_1().min_w_0().child(
                            self.form
                                .as_ref()
                                .map(|form| text_field_of(theme, &form.secret))
                                .unwrap_or_else(|| div()),
                        ),
                    ),
                ),
        );

        // Models.
        pane = pane.child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(widgets::field_label(
                            theme,
                            localizer.text(TextKey::ProvidersModelList),
                        ))
                        .child(
                            div()
                                .text_size(crate::typography::ui_rems(12.0))
                                .text_color(theme.text_faint)
                                .child(localizer.format_count(
                                    TextKey::ProvidersModelCountBadge,
                                    "count",
                                    model_count as i64,
                                )),
                        )
                        .child(div().flex_1())
                        .child(
                            widgets::text_action(
                                theme,
                                widgets::ActionTone::Outlined,
                                localizer.text(TextKey::ProvidersModelsFetch),
                            )
                            .id("providers-model-fetch")
                            .debug_selector(|| "providers-model-fetch".into())
                            .tab_index(0)
                            .role(gpui::Role::Button)
                            .on_click(cx.listener(
                                move |page, _, window, cx| page.open_model_fetch(index, window, cx),
                            )),
                        )
                        .child(
                            widgets::text_action(
                                theme,
                                widgets::ActionTone::Outlined,
                                localizer.text(TextKey::ProvidersModelAdd),
                            )
                            .id("providers-model-add")
                            .debug_selector(|| "providers-model-add".into())
                            .tab_index(0)
                            .role(gpui::Role::Button)
                            .on_click(cx.listener(
                                move |page, _, window, cx| {
                                    page.open_model_dialog(index, None, window, cx)
                                },
                            )),
                        ),
                )
                .child(self.render_model_list(index, models, theme, cx)),
        );

        pane
    }

    fn render_protocol_select(
        &mut self,
        index: usize,
        protocol: Protocol,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let localizer = crate::l10n::localizer();
        let options: Vec<widgets::SelectOption> = PROTOCOLS
            .iter()
            .map(|candidate| widgets::SelectOption::new(localizer.text(protocol_key(*candidate))))
            .collect();
        let selected = PROTOCOLS
            .iter()
            .position(|candidate| *candidate == protocol)
            .unwrap_or_default();
        widgets::select(
            "providers-protocol",
            localizer.text(TextKey::ProvidersApiFormat),
            theme,
            |page: &mut Self| &mut page.protocol_select,
        )
        .options(options, selected)
        .width(280.0)
        .on_select(move |page, selected, _, cx| {
            let Some(protocol) = PROTOCOLS.get(selected).copied() else {
                return;
            };
            if let Some(edit) = page
                .draft()
                .and_then(|draft| draft.providers.get_mut(index))
            {
                edit.protocol = protocol;
            }
            page.schedule_save(cx);
            cx.notify();
        })
        .render(&self.protocol_select, cx)
    }

    /// Confirmation for removing a provider.
    fn render_delete_dialog(
        &mut self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let provider_id = self.confirming_delete.clone()?;
        let name = self
            .draft
            .as_ref()?
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)?
            .display_label()
            .to_owned();
        let theme = Theme::of(cx).for_popup();
        let localizer = crate::l10n::localizer();
        let card = popover::dialog_card(&theme)
            .id("providers-delete-dialog-card")
            .role(gpui::Role::Dialog)
            .aria_label(localizer.text(TextKey::ProvidersDelete))
            .on_key_down(cx.listener(|page, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    page.close_delete_dialog(cx);
                    cx.stop_propagation();
                }
            }))
            .child(popover::dialog_title(
                &theme,
                &localizer.text(TextKey::ProvidersDelete),
            ))
            .child(div().mt(px(6.0)).child(popover::dialog_body(
                &theme,
                localizer.format_with(TextKey::ProvidersDeleteConfirm, &[("name", name.as_str())]),
            )))
            .child(
                div()
                    .mt(px(16.0))
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        popover::btn_ghost(
                            &theme,
                            &localizer.text(TextKey::ActionCancel),
                            "providers-delete-cancel",
                        )
                        .id("providers-delete-cancel")
                        .on_click(cx.listener(|page, _, _, cx| page.close_delete_dialog(cx))),
                    )
                    .child(
                        popover::btn_danger(&theme, &localizer.text(TextKey::ActionDelete))
                            .id("providers-delete-confirm")
                            .on_click(cx.listener(move |page, _, window, cx| {
                                page.delete_provider(&provider_id, window, cx)
                            })),
                    ),
            )
            .into_any_element();
        Some(popover::modal("providers-delete-dialog", viewport, card))
    }

    /// The add/edit-model dialog.
    fn render_model_dialog(
        &mut self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let dialog = self.model_dialog.as_ref()?;
        let theme = Theme::of(cx).for_popup();
        let localizer = crate::l10n::localizer();
        let editing = dialog.slot.is_some();
        let model = dialog.model.clone();
        let context_window = dialog.context_window.clone();
        let max_tokens = dialog.max_tokens.clone();
        let input_format = dialog.input_format;
        let reasoning_levels = dialog.reasoning_levels.clone();
        let reasoning_add_open = dialog.reasoning_add_open;
        let error = dialog
            .error
            .clone()
            .or_else(|| self.error.clone())
            .or_else(|| self.conflict.clone());

        let card = popover::dialog_card(&theme)
            .id("providers-model-dialog-card")
            .w(px(440.0))
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(popover::dialog_title(
                &theme,
                &localizer.text(if editing {
                    TextKey::ProvidersModelDialogEditTitle
                } else {
                    TextKey::ProvidersModelDialogAddTitle
                }),
            ))
            .child(field(
                &theme,
                localizer.text(TextKey::ProvidersModelName),
                text_field_of(&theme, &model),
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(12.0))
                    .child(field(
                        &theme,
                        localizer.text(TextKey::ProvidersModelContext),
                        text_field_of(&theme, &context_window),
                    ))
                    .child(field(
                        &theme,
                        localizer.text(TextKey::ProvidersModelMaxTokens),
                        text_field_of(&theme, &max_tokens),
                    )),
            )
            .child(field(
                &theme,
                localizer.text(TextKey::ProvidersModelInput),
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.0))
                    .child(input_choice(
                        &theme,
                        localizer.text(TextKey::ProvidersModelInputText),
                        "providers-model-input-text",
                        input_format == InputFormat::Text,
                        cx.listener(|page, _, _, cx| {
                            page.set_dialog_input_format(InputFormat::Text, cx)
                        }),
                    ))
                    .child(input_choice(
                        &theme,
                        localizer.text(TextKey::ProvidersModelInputTextAndImages),
                        "providers-model-input-images",
                        input_format == InputFormat::TextAndImages,
                        cx.listener(|page, _, _, cx| {
                            page.set_dialog_input_format(InputFormat::TextAndImages, cx)
                        }),
                    )),
            ))
            .child(field(
                &theme,
                localizer.text(TextKey::ProvidersModelReasoning),
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .items_center()
                            .gap(px(6.0))
                            .children(reasoning_levels.iter().copied().map(|level| {
                                // Clicking a chip removes it: the chip is the
                                // list entry, and a separate remove glyph per
                                // chip would be a second target for the same
                                // edit.
                                widgets::badge(&theme, crate::pickers::reasoning_label(level))
                                    .id(("providers-model-level", level as usize))
                                    .cursor_pointer()
                                    .tab_index(0)
                                    .role(gpui::Role::Button)
                                    .aria_label(localizer.format_with(
                                        TextKey::ProvidersModelReasoningRemove,
                                        &[("level", &crate::pickers::reasoning_label(level))],
                                    ))
                                    .tooltip(widgets::text_tooltip(localizer.format_with(
                                        TextKey::ProvidersModelReasoningRemove,
                                        &[("level", &crate::pickers::reasoning_label(level))],
                                    )))
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        page.toggle_dialog_level(level, cx)
                                    }))
                            }))
                            .child(
                                widgets::ghost_action(&theme)
                                    .id("providers-model-level-add")
                                    .tab_index(0)
                                    .role(gpui::Role::Button)
                                    .aria_label(localizer.text(TextKey::ProvidersModelReasoningAdd))
                                    .tooltip(widgets::text_tooltip(
                                        localizer.text(TextKey::ProvidersModelReasoningAdd),
                                    ))
                                    .on_click(
                                        cx.listener(|page, _, _, cx| page.toggle_reasoning_add(cx)),
                                    )
                                    .child(crate::icons::icon(crate::icons::PLUS).size(px(14.0))),
                            ),
                    )
                    .when(reasoning_add_open, |el| {
                        el.child(
                            div().flex().flex_row().flex_wrap().gap(px(6.0)).children(
                                crate::pickers::ALL_REASONING_LEVELS
                                    .iter()
                                    .copied()
                                    .filter(|level| !reasoning_levels.contains(level))
                                    .map(|level| {
                                        widgets::text_action(
                                            &theme,
                                            widgets::ActionTone::Quiet,
                                            crate::pickers::reasoning_label(level),
                                        )
                                        .id(("providers-model-level-option", level as usize))
                                        .tab_index(0)
                                        .role(gpui::Role::Button)
                                        .on_click(
                                            cx.listener(move |page, _, _, cx| {
                                                page.add_dialog_level(level, cx)
                                            }),
                                        )
                                    }),
                            ),
                        )
                    })
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(theme.text_faint)
                            .child(localizer.text(TextKey::ProvidersModelReasoningHint)),
                    ),
            ))
            .children(error.map(|error| widgets::error_strip(&theme, error)))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.0))
                    .when(!editing, |row| {
                        row.child(
                            widgets::text_action(
                                &theme,
                                widgets::ActionTone::Quiet,
                                localizer.text(TextKey::ActionCancel),
                            )
                            .id("providers-model-dialog-cancel")
                            .tab_index(0)
                            .role(gpui::Role::Button)
                            .on_click(cx.listener(|page, _, _, cx| page.close_model_dialog(cx))),
                        )
                    })
                    .child(
                        widgets::text_action(
                            &theme,
                            widgets::ActionTone::Solid,
                            localizer.text(if editing {
                                TextKey::ProvidersModelDone
                            } else {
                                TextKey::ProvidersModelAdd
                            }),
                        )
                        .id("providers-model-dialog-commit")
                        .tab_index(0)
                        .role(gpui::Role::Button)
                        .on_click(cx.listener(|page, _, _, cx| page.commit_model(cx))),
                    ),
            );

        Some(popover::modal(
            "providers-model-dialog",
            viewport,
            card.into_any_element(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The pane shown when no provider is selected.
fn empty_pane(theme: &Theme, localizer: &Localizer) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .p(px(32.0))
        .child(
            div()
                .text_size(crate::typography::ui_rems(14.0))
                .text_color(theme.text_muted)
                .child(localizer.text(TextKey::ProvidersEmptyTitle)),
        )
        .child(
            div()
                .text_size(crate::typography::ui_rems(12.5))
                .text_color(theme.text_faint)
                .child(localizer.text(TextKey::ProvidersEmptyBody)),
        )
}

/// The context window as a badge, when the vendor published one.
///
/// A count is a fact about the model rather than a label, so it is formatted
/// rather than translated: `1M` reads the same in both languages.
fn context_badge(context_window: &str) -> Option<String> {
    let value: u32 = context_window.trim().parse().ok()?;
    if value == 0 {
        return None;
    }
    Some(if value >= 1_000_000 {
        let millions = value as f32 / 1_000_000.0;
        if millions.fract().abs() < 0.05 {
            format!("{}M", millions.round() as u32)
        } else {
            format!("{millions:.1}M")
        }
    } else if value >= 1_000 {
        let thousands = value as f32 / 1_000.0;
        if thousands.fract().abs() < 0.05 {
            format!("{}K", thousands.round() as u32)
        } else {
            format!("{thousands:.1}K")
        }
    } else {
        value.to_string()
    })
}

/// What the credential row reports about the stored key.
fn credential_detail(localizer: &Localizer, view: &CredentialView) -> String {
    match view {
        CredentialView::None => localizer.text(TextKey::ProvidersCredentialNone),
        CredentialView::Stored => localizer.text(TextKey::ProvidersCredentialStored),
        CredentialView::Environment { .. } => localizer.text(TextKey::ProvidersCredentialStored),
    }
}

/// A single-line text field in the settings look.
fn text_field_of(theme: &Theme, input: &Entity<ComposerInput>) -> gpui::Div {
    div()
        .h(px(34.0))
        .w_full()
        .min_w(px(120.0))
        .overflow_hidden()
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.input_glass_bg())
        .px(px(10.0))
        .py(px(6.0))
        .text_size(crate::typography::ui_rems(13.0))
        .child(input.clone())
}

/// The provider name, styled as the pane's title: no frame until it is
/// hovered, because it reads as a heading rather than as a field.
fn title_input(theme: &Theme, input: &Entity<ComposerInput>) -> gpui::Div {
    div()
        .h(px(30.0))
        .w_full()
        .min_w(px(120.0))
        .flex()
        .items_center()
        .overflow_hidden()
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border.opacity(0.0))
        .hover(|style| style.border_color(theme.border))
        .px(px(6.0))
        .py(px(2.0))
        .text_size(crate::typography::ui_rems(17.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(input.clone())
}

/// A labelled field: the label above, the input below.
fn field(theme: &Theme, label: impl Into<SharedString>, control: impl IntoElement) -> gpui::Div {
    div()
        .flex_1()
        .min_w(px(140.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(widgets::field_label(theme, label))
        .child(control)
}

/// One option of the dialog's input-format pair.
///
/// Two buttons rather than a select: both choices fit, and a dropdown would
/// hide the alternative behind a click to save nothing.
fn input_choice(
    theme: &Theme,
    label: impl Into<SharedString>,
    id: &'static str,
    selected: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    widgets::text_action(
        theme,
        if selected {
            widgets::ActionTone::Solid
        } else {
            widgets::ActionTone::Quiet
        },
        label,
    )
    .id(id)
    .tab_index(0)
    .role(gpui::Role::Button)
    .on_click(on_click)
}

/// A count typed into a text field. Blank is "unknown", which is a real state
/// for both counts — see `ModelConfig::context_window`.
fn parse_count(text: &str) -> Option<u32> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse().ok()
}

fn trimmed(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn protocol_key(protocol: Protocol) -> TextKey {
    match protocol {
        Protocol::OpenAiChat => TextKey::ProvidersProtocolChat,
        Protocol::OpenAiResponses => TextKey::ProvidersProtocolResponses,
        Protocol::AnthropicMessages => TextKey::ProvidersProtocolMessages,
    }
}

/// The wire name `ProviderView` carries, back to the value the config uses.
///
/// A name this build does not know reads as the plain Chat Completions wire
/// rather than failing the page: a provider written by a newer build should
/// still be visible and editable here.
fn protocol_from_wire(wire: &str) -> Protocol {
    match wire {
        "open_ai_responses" => Protocol::OpenAiResponses,
        "anthropic_messages" => Protocol::AnthropicMessages,
        _ => Protocol::OpenAiChat,
    }
}

const PROTOCOLS: [Protocol; 3] = [
    Protocol::OpenAiChat,
    Protocol::OpenAiResponses,
    Protocol::AnthropicMessages,
];

/// A text input for the page, pre-filled and single-line.
fn text_input(
    value: &str,
    placeholder: TextKey,
    cx: &mut Context<ProvidersPage>,
) -> Entity<ComposerInput> {
    text_input_with(value, crate::l10n::localizer().text(placeholder), cx)
}

/// [`text_input`] for a placeholder that is already resolved — an endpoint
/// field suggests a URL rather than naming itself.
fn text_input_with(
    value: &str,
    placeholder: String,
    cx: &mut Context<ProvidersPage>,
) -> Entity<ComposerInput> {
    let input = cx.new(|cx| {
        ComposerInput::new(placeholder, cx)
            .with_single_line()
            .with_text_metrics(13.0, 18.0)
            .with_accessibility_role(gpui::Role::TextInput)
    });
    let value = value.to_owned();
    input.update(cx, |input, cx| input.set_text(value, cx));
    input
}
