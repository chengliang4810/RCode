//! Settings → Subagents: personal definitions and contributions from installed plugins.

use gpui::{
    AnyElement, App, Context, Entity, SharedString, Subscription, Task, Window, div, prelude::*, px,
};

use super::subagent_models::model_items;
use rcode_harness::provider_store::ProviderSettingsView;
use rcode_presentation::TextKey;
use rcode_proto::HarnessId;
use rcode_proto::invocation::{SkillScope, SubagentDefinition};
use rcode_rpc::methods;

use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::popover;
use crate::settings::{personal_catalog_target, widgets};
use crate::state::AppState;
use crate::theme::Theme;

/// The listing is the native agent's definitions — the ones its `Task` tool
/// can actually run.
const LISTING_HARNESS: HarnessId = HarnessId::Native;

/// Height of the dialog's prompt editor (about six lines) before it scrolls
/// internally.
const PROMPT_EDITOR_HEIGHT: f32 = 140.0;

/// The model aliases a definition may name, in the order the picker shows
/// them. `inherit` first because it is the default and the way to remove an
/// override.
pub(crate) const MODEL_ALIASES: [&str; 1] = ["inherit"];

/// Which managed root a listing row came from, as the page groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DefinitionGroup {
    Builtin,
    Project,
    User,
    /// A definition an installed plugin contributes. Shown, not editable.
    Plugin,
    /// Older catalogs without a scope cannot establish a personal origin.
    Unknown,
}

impl DefinitionGroup {
    fn of(definition: &SubagentDefinition) -> Self {
        if definition.path.starts_with("builtin:") {
            return Self::Builtin;
        }
        match definition.scope {
            Some(SkillScope::Project) => Self::Project,
            Some(SkillScope::User) => Self::User,
            Some(SkillScope::Plugin) => Self::Plugin,
            None => Self::Unknown,
        }
    }

    fn writable(self) -> bool {
        matches!(self, Self::User)
    }
}

/// Rows grouped by managed root, each group name-sorted. Pure.
pub(crate) struct DefinitionGroups {
    pub builtin: Vec<SubagentDefinition>,
    pub user: Vec<SubagentDefinition>,
    pub plugin: Vec<SubagentDefinition>,
}

impl DefinitionGroups {
    fn len(&self) -> usize {
        self.builtin.len() + self.user.len() + self.plugin.len()
    }
}

fn sorted_by_name(mut rows: Vec<SubagentDefinition>) -> Vec<SubagentDefinition> {
    rows.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    rows
}

/// Split a listing into its managed-root groups. Pure.
pub(crate) fn group_definitions(definitions: Vec<SubagentDefinition>) -> DefinitionGroups {
    let mut groups = DefinitionGroups {
        builtin: Vec::new(),
        user: Vec::new(),
        plugin: Vec::new(),
    };
    for definition in definitions {
        match DefinitionGroup::of(&definition) {
            DefinitionGroup::Builtin => groups.builtin.push(definition),
            DefinitionGroup::Project | DefinitionGroup::Unknown => {}
            DefinitionGroup::User => groups.user.push(definition),
            DefinitionGroup::Plugin => groups.plugin.push(definition),
        }
    }
    groups.user = sorted_by_name(groups.user);
    groups.plugin = sorted_by_name(groups.plugin);
    groups
}

/// The name grammar the managed roots enforce (a name is a file stem and the
/// `Task` tool's argument): letters, digits, dash and underscore. Mirrors
/// `rcode_agent::skills::validate_name`, which the definition writer uses.
/// Pure.
pub(crate) fn valid_definition_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty()
        && !trimmed.chars().all(|c| c == '.')
        && trimmed
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// Which field a create/edit form refuses, if any. The description is
/// required: it is what the parent model reads when deciding to delegate.
/// Pure.
pub(crate) fn form_error(name: &str, description: &str) -> Option<TextKey> {
    if !valid_definition_name(name) {
        Some(TextKey::SettingsSubagentsNameInvalid)
    } else if description.trim().is_empty() {
        Some(TextKey::SettingsSubagentsDescriptionRequired)
    } else {
        None
    }
}

/// A comma-separated tool list, as the dialog edits it.
pub(crate) fn parse_tool_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The tool list back in the dialog's spelling.
pub(crate) fn format_tool_list(tools: &[String]) -> String {
    tools.join(", ")
}

/// The alias a row's selector shows: the effective value, which is the
/// definition's own `model:` unless the settings override it. Pure.
pub(crate) fn displayed_alias(definition: &SubagentDefinition) -> &str {
    match definition.model.as_deref() {
        Some(value) if rcode_harness::providers::ModelSelection::parse(value).is_some() => value,
        _ => "inherit",
    }
}

/// What a failed write should say, one message per operation.
#[derive(Debug, Clone)]
enum WriteFailure {
    Create,
    Update,
    Delete,
    Toggle { name: String },
    Model { name: String },
}

impl WriteFailure {
    fn message(&self, error: &str) -> String {
        let localizer = crate::l10n::localizer();
        match self {
            Self::Create => {
                localizer.format_with(TextKey::SettingsSubagentsCreateFailed, &[("error", error)])
            }
            Self::Update => {
                localizer.format_with(TextKey::SettingsSubagentsUpdateFailed, &[("error", error)])
            }
            Self::Delete => {
                localizer.format_with(TextKey::SettingsSubagentsDeleteFailed, &[("error", error)])
            }
            Self::Toggle { name } => localizer.format_with(
                TextKey::SettingsSubagentsToggleFailed,
                &[("name", name.as_str()), ("error", error)],
            ),
            Self::Model { name } => localizer.format_with(
                TextKey::SettingsSubagentsModelFailed,
                &[("name", name.as_str()), ("error", error)],
            ),
        }
    }
}

/// Personal create/edit inputs; `origin` identifies the row being edited.
struct DefinitionEditor {
    creating: bool,
    origin: Option<SubagentDefinition>,
    name: Entity<ComposerInput>,
    description: Entity<ComposerInput>,
    prompt: Entity<ComposerInput>,
    tools: Entity<ComposerInput>,
    disallowed_tools: Entity<ComposerInput>,
    model: String,
    invalid: Option<TextKey>,
    _events: Vec<Subscription>,
}

fn dialog_input(
    placeholder: TextKey,
    height: Option<f32>,
    cx: &mut Context<SubagentsPage>,
) -> (Entity<ComposerInput>, Subscription) {
    let localizer = crate::l10n::localizer();
    let input = cx.new(|cx| {
        let input = ComposerInput::new(localizer.text(placeholder), cx);
        match height {
            Some(height) => input.with_viewport_height(height),
            None => input.with_single_line(),
        }
    });
    let events = cx.subscribe(
        &input,
        |page: &mut SubagentsPage, _, event, cx| match event {
            ComposerInputEvent::Submitted => page.save_editor(cx),
            ComposerInputEvent::Edited => {
                if let Some(editor) = page.editor.as_mut() {
                    editor.invalid = None;
                }
                cx.notify();
            }
            _ => {}
        },
    );
    (input, events)
}

/// The Settings → Subagents page.
pub struct SubagentsPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    definitions: Vec<SubagentDefinition>,
    provider_view: Option<ProviderSettingsView>,
    model_selects: std::collections::BTreeMap<String, widgets::CascadeState>,
    editor_model_select: widgets::CascadeState,
    effort_selects: std::collections::BTreeMap<String, widgets::SelectState>,
    loading: bool,
    load_error: Option<SharedString>,
    loaded_context: Option<String>,
    request: u64,
    load_task: Option<Task<()>>,
    write_task: Option<Task<()>>,
    busy_path: Option<String>,
    editor: Option<DefinitionEditor>,
    confirm_delete: Option<SubagentDefinition>,
    error: Option<SharedString>,
    _observe: Subscription,
}

impl SubagentsPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&state, |page, _, cx| page.sync_context(cx));
        let mut page = Self {
            state,
            scroll: widgets::PageScroll::default(),
            definitions: Vec::new(),
            provider_view: None,
            model_selects: Default::default(),
            editor_model_select: Default::default(),
            effort_selects: Default::default(),
            loading: false,
            load_error: None,
            loaded_context: None,
            request: 0,
            load_task: None,
            write_task: None,
            busy_path: None,
            editor: None,
            confirm_delete: None,
            error: None,
            _observe: observe,
        };
        page.reload(cx);
        page
    }

    fn context(&self, cx: &App) -> Option<String> {
        let state = self.state.read(cx);
        let engine = state.engine()?;
        Some(format!(
            "{:?}:{:p}:{target}",
            state.connection,
            engine.client(),
            target = personal_catalog_target()
        ))
    }

    fn target_params(&self) -> serde_json::Value {
        let mut target = personal_catalog_target();
        target["harness"] = serde_json::json!(LISTING_HARNESS);
        target
    }

    fn sync_context(&mut self, cx: &mut Context<Self>) {
        if self.context(cx) == self.loaded_context {
            return;
        }
        self.reload(cx);
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.loaded_context = self.context(cx);
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.loading = false;
            self.load_error = None;
            cx.notify();
            return;
        };
        let params = self.target_params();
        self.request = self.request.wrapping_add(1);
        let request = self.request;
        self.loading = true;
        self.load_error = None;
        self.load_task = Some(cx.spawn(async move |page, cx| {
            let result = engine.client().call(methods::LIST_SUBAGENTS, params).await;
            let providers = engine
                .client()
                .call(methods::GET_PROVIDER_SETTINGS, serde_json::json!({}))
                .await
                .ok()
                .and_then(|value| serde_json::from_value::<ProviderSettingsView>(value).ok());
            page.update(cx, |page, cx| {
                if page.request != request {
                    return;
                }
                page.loading = false;
                page.provider_view = providers;
                let decoded = result.and_then(|value| {
                    serde_json::from_value::<Option<Vec<SubagentDefinition>>>(value)
                        .map_err(|error| rcode_rpc::RpcError::Failed(error.to_string()))
                });
                match decoded {
                    Ok(definitions) => page.definitions = definitions.unwrap_or_default(),
                    Err(err) => {
                        let message = err.to_string();
                        page.load_error = Some(
                            crate::l10n::localizer()
                                .format_with(
                                    TextKey::SettingsSubagentsLoadFailed,
                                    &[("error", message.as_str())],
                                )
                                .into(),
                        );
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn write(
        &mut self,
        method: &'static str,
        params: serde_json::Value,
        failure: WriteFailure,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let context = self.context(cx).unwrap_or_default();
        self.write_task = Some(cx.spawn(async move |page, cx| {
            let result = engine.client().call(method, params).await;
            page.update(cx, |page, cx| {
                page.busy_path = None;
                match result {
                    Ok(value) => {
                        let decoded =
                            serde_json::from_value::<Option<Vec<SubagentDefinition>>>(value)
                                .map_err(|error| rcode_rpc::RpcError::Failed(error.to_string()));
                        match decoded {
                            Ok(definitions)
                                if page.context(cx).as_deref() == Some(context.as_str()) =>
                            {
                                page.definitions = definitions.unwrap_or_default();
                            }
                            Ok(_) => {}
                            Err(err) => {
                                let message = err.to_string();
                                page.error = Some(failure.message(&message).into());
                            }
                        }
                    }
                    Err(err) => {
                        let message = err.to_string();
                        page.error = Some(failure.message(&message).into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    // ---- actions ----

    fn open_create(&mut self, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            return;
        }
        let (name, name_events) = dialog_input(TextKey::SettingsSubagentsNamePlaceholder, None, cx);
        let (description, description_events) =
            dialog_input(TextKey::SettingsSubagentsDescriptionPlaceholder, None, cx);
        let (prompt, prompt_events) = dialog_input(
            TextKey::SettingsSubagentsPromptPlaceholder,
            Some(PROMPT_EDITOR_HEIGHT),
            cx,
        );
        let (tools, tools_events) =
            dialog_input(TextKey::SettingsSubagentsToolsPlaceholder, None, cx);
        let (disallowed_tools, disallowed_events) = dialog_input(
            TextKey::SettingsSubagentsDisallowedToolsPlaceholder,
            None,
            cx,
        );
        self.editor = Some(DefinitionEditor {
            creating: true,
            origin: None,
            name,
            description,
            prompt,
            tools,
            disallowed_tools,
            model: "inherit".to_owned(),
            invalid: None,
            _events: vec![
                name_events,
                description_events,
                prompt_events,
                tools_events,
                disallowed_events,
            ],
        });
        cx.notify();
    }

    fn open_edit(&mut self, definition: &SubagentDefinition, cx: &mut Context<Self>) {
        if self.editor.is_some() || !DefinitionGroup::of(definition).writable() {
            return;
        }
        let (name, name_events) = dialog_input(TextKey::SettingsSubagentsNamePlaceholder, None, cx);
        name.update(cx, |input, cx| input.set_text(definition.name.clone(), cx));
        let (description, description_events) =
            dialog_input(TextKey::SettingsSubagentsDescriptionPlaceholder, None, cx);
        description.update(cx, |input, cx| {
            input.set_text(definition.description.clone(), cx)
        });
        let (prompt, prompt_events) = dialog_input(
            TextKey::SettingsSubagentsPromptPlaceholder,
            Some(PROMPT_EDITOR_HEIGHT),
            cx,
        );
        prompt.update(cx, |input, cx| {
            input.set_text(definition.system_prompt.clone().unwrap_or_default(), cx)
        });
        let (tools, tools_events) =
            dialog_input(TextKey::SettingsSubagentsToolsPlaceholder, None, cx);
        tools.update(cx, |input, cx| {
            input.set_text(format_tool_list(&definition.tools), cx)
        });
        let (disallowed_tools, disallowed_events) = dialog_input(
            TextKey::SettingsSubagentsDisallowedToolsPlaceholder,
            None,
            cx,
        );
        disallowed_tools.update(cx, |input, cx| {
            input.set_text(format_tool_list(&definition.disallowed_tools), cx)
        });
        self.editor = Some(DefinitionEditor {
            creating: false,
            origin: Some(definition.clone()),
            name,
            description,
            prompt,
            tools,
            disallowed_tools,
            model: displayed_alias(definition).to_owned(),
            invalid: None,
            _events: vec![
                name_events,
                description_events,
                prompt_events,
                tools_events,
                disallowed_events,
            ],
        });
        cx.notify();
    }

    fn close_editor(&mut self, cx: &mut Context<Self>) {
        if self.editor.take().is_some() {
            cx.notify();
        }
    }

    fn save_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let name = editor.name.read(cx).text().trim().to_string();
        let description = editor.description.read(cx).text().trim().to_string();
        let prompt = editor.prompt.read(cx).text().to_string();
        let tools = parse_tool_list(&editor.tools.read(cx).text());
        let disallowed_tools = parse_tool_list(&editor.disallowed_tools.read(cx).text());
        if let Some(key) = form_error(&name, &description) {
            if let Some(editor) = self.editor.as_mut() {
                editor.invalid = Some(key);
            }
            cx.notify();
            return;
        }
        let Some(editor) = self.editor.take() else {
            return;
        };
        let mut params = self.target_params();
        if editor.creating {
            params["name"] = name.into();
            params["description"] = description.into();
            params["systemPrompt"] = prompt.into();
            params["tools"] = serde_json::json!(tools);
            params["disallowedTools"] = serde_json::json!(disallowed_tools);
            params["model"] = editor.model.clone().into();
            params["scope"] = serde_json::json!(SkillScope::User);
            self.write(methods::CREATE_SUBAGENT, params, WriteFailure::Create, cx);
        } else {
            let Some(origin) = editor.origin.as_ref() else {
                return;
            };
            params["name"] = origin.name.clone().into();
            params["newName"] = name.into();
            params["description"] = description.into();
            // An absent prompt keeps the definition's instructions; an empty
            // one is not an edit anyone means.
            if !prompt.trim().is_empty() {
                params["systemPrompt"] = prompt.into();
            }
            params["tools"] = serde_json::json!(tools);
            params["disallowedTools"] = serde_json::json!(disallowed_tools);
            params["model"] = editor.model.clone().into();
            params["scope"] = serde_json::json!(SkillScope::User);
            self.write(methods::UPDATE_SUBAGENT, params, WriteFailure::Update, cx);
        }
        cx.notify();
    }

    fn set_editor_model(&mut self, alias: &str, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.as_mut()
            && editor.model != alias
        {
            editor.model = alias.to_owned();
            cx.notify();
        }
    }

    fn set_enabled(
        &mut self,
        definition: &SubagentDefinition,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let mut params = self.target_params();
        params["definitionPath"] = definition.path.clone().into();
        params["enabled"] = enabled.into();
        if let Some(scope) = definition.scope {
            params["scope"] = serde_json::json!(scope);
        }
        self.busy_path = Some(definition.path.clone());
        self.write(
            methods::SET_SUBAGENT_ENABLED,
            params,
            WriteFailure::Toggle {
                name: definition.name.clone(),
            },
            cx,
        );
    }

    fn set_model(&mut self, definition: &SubagentDefinition, alias: &str, cx: &mut Context<Self>) {
        if self.busy_path.is_some() { return; }
        let mut params = self.target_params();
        params["definitionPath"] = definition.path.clone().into();
        params["model"] = alias.into();
        if let Some(scope) = definition.scope {
            params["scope"] = serde_json::json!(scope);
        }
        self.busy_path = Some(definition.path.clone());
        self.write(
            methods::SET_SUBAGENT_MODEL,
            params,
            WriteFailure::Model {
                name: definition.name.clone(),
            },
            cx,
        );
    }

    fn render_effort_control(
        &mut self,
        definition: &SubagentDefinition,
        busy: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use rcode_harness::providers::ModelSelection;
        let current = displayed_alias(definition);
        let Some(selection) = ModelSelection::parse(current) else {
            return div().into_any_element();
        };
        let levels = super::subagent_models::reasoning_levels(self.provider_view.as_ref(), current);
        if levels.is_empty() {
            return div().into_any_element();
        }
        let selected = selection.reasoning
            .and_then(|level| levels.iter().position(|candidate| *candidate == level))
            .map(|index| index + 1).unwrap_or(0);
        let mut choices = vec![widgets::SelectOption::new(
            crate::l10n::localizer().text(TextKey::PickersDefault),
        )];
        for level in &levels {
            let color = theme.text_muted;
            choices.push(widgets::SelectOption::new(crate::pickers::reasoning_label(*level))
                .leading(move || crate::icons::icon(crate::icons::BRAIN)
                    .size(px(16.0)).text_color(color).into_any_element()));
        }
        let key = definition.path.clone();
        let state_key = key.clone();
        self.effort_selects.entry(key.clone()).or_default();
        let target = definition.clone();
        widgets::select(
            format!("subagents-effort-{key}"),
            crate::l10n::localizer().text(TextKey::PickersReasoningEffort),
            theme,
            move |page: &mut Self| page.effort_selects.entry(key.clone()).or_default(),
        )
            .options(choices, selected)
            .width(112.0)
            .on_select(move |page, index, _, cx| {
                let mut next = selection.clone();
                next.reasoning = index.checked_sub(1)
                    .and_then(|index| levels.get(index)).copied();
                page.set_model(&target, &next.selection(), cx);
            })
            .render(self.effort_selects.get(&state_key).unwrap(), cx)
            .when(busy, |el| el.opacity(0.4))
            .into_any_element()
    }

    fn request_delete(&mut self, definition: SubagentDefinition, cx: &mut Context<Self>) {
        self.confirm_delete = Some(definition);
        cx.notify();
    }

    fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        let Some(definition) = self.confirm_delete.take() else {
            return;
        };
        let mut params = self.target_params();
        params["name"] = definition.name.clone().into();
        if let Some(scope) = definition.scope {
            params["scope"] = serde_json::json!(scope);
        }
        self.write(methods::DELETE_SUBAGENT, params, WriteFailure::Delete, cx);
        cx.notify();
    }

    fn close_confirm_delete(&mut self, cx: &mut Context<Self>) {
        if self.confirm_delete.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn dismiss_on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if self.editor.take().is_some() || self.confirm_delete.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    // ---- render ----

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    fn render_editor_dialog(
        &mut self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let theme = Theme::of(cx).for_popup();
        let localizer = crate::l10n::localizer();
        let editor = self.editor.as_ref()?;
        let title = if editor.creating {
            localizer.text(TextKey::SettingsSubagentsCreateTitle)
        } else {
            localizer.text(TextKey::SettingsSubagentsEditTitle)
        };
        let model = editor.model.clone();
        let invalid = editor.invalid;
        let name = editor.name.clone();
        let description = editor.description.clone();
        let prompt = editor.prompt.clone();
        let tools = editor.tools.clone();
        let disallowed_tools = editor.disallowed_tools.clone();
        let field = |label: TextKey, input: Entity<ComposerInput>| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(widgets::field_label(&theme, localizer.text(label)))
                .child(popover::dialog_field(input.into_any_element()))
        };
        let options = model_items(self.provider_view.as_ref(), &model);
        let alias_row = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(widgets::field_label(
                &theme,
                localizer.text(TextKey::SettingsSubagentsModelLabel),
            ))
            .child(
                widgets::cascade(
                    "subagents-editor-model",
                    localizer.text(TextKey::SettingsSubagentsModelLabel),
                    &theme,
                    |page: &mut Self| &mut page.editor_model_select,
                )
                .options(options, &model)
                .width(400.0)
                .on_select(move |page, value, _, cx| page.set_editor_model(value, cx))
                .render(&self.editor_model_select, cx),
            );

        let card = div()
            .w(px(560.0))
            .max_h(px(f32::from(viewport.height) * 0.9))
            .p(px(20.0))
            .rounded(px(16.0))
            .bg(popover::surface_bg(&theme))
            .border_1()
            .border_color(crate::theme::hairline(0.10))
            .text_color(theme.text)
            .flex()
            .flex_col()
            .id("subagents-editor-card")
            .role(gpui::Role::Dialog)
            .aria_label(title.clone())
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.close_editor(cx);
                    cx.stop_propagation();
                }
            }))
            .child(popover::dialog_title(&theme, &title))
            .child(
                div()
                    .mt(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(field(TextKey::SettingsSubagentsNameLabel, name))
                    .child(field(
                        TextKey::SettingsSubagentsDescriptionLabel,
                        description,
                    ))
                    .child(field(TextKey::SettingsSubagentsPromptLabel, prompt))
                    .child(field(TextKey::SettingsSubagentsToolsLabel, tools))
                    .child(field(
                        TextKey::SettingsSubagentsDisallowedToolsLabel,
                        disallowed_tools,
                    ))
                    .child(alias_row)
                    .when_some(invalid, |el, key| {
                        el.child(
                            div()
                                .text_size(crate::typography::ui_rems(12.0))
                                .text_color(theme.danger_muted)
                                .child(SharedString::from(localizer.text(key))),
                        )
                    }),
            )
            .child(
                div()
                    .mt(px(16.0))
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        widgets::text_action(
                            &theme,
                            widgets::ActionTone::Quiet,
                            localizer.text(TextKey::ActionCancel),
                        )
                        .id("subagents-editor-cancel")
                        .tab_index(0)
                        .role(gpui::Role::Button)
                        .focus_visible(|s| s.border_2().border_color(theme.accent))
                        .on_click(cx.listener(|this, _, _, cx| this.close_editor(cx))),
                    )
                    .child(
                        widgets::text_action(
                            &theme,
                            widgets::ActionTone::Solid,
                            localizer.text(TextKey::ActionSave),
                        )
                        .id("subagents-editor-save")
                        .tab_index(0)
                        .role(gpui::Role::Button)
                        .focus_visible(|s| s.border_2().border_color(theme.accent))
                        .on_click(cx.listener(|this, _, _, cx| this.save_editor(cx))),
                    ),
            )
            .into_any_element();
        Some(popover::modal("subagents-editor-dialog", viewport, card))
    }

    fn render_confirm_delete_dialog(
        &mut self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let theme = Theme::of(cx).for_popup();
        let localizer = crate::l10n::localizer();
        let definition = self.confirm_delete.as_ref()?;
        let name = definition.name.clone();
        let card = popover::dialog_card(&theme)
            .id("subagents-delete-card")
            .role(gpui::Role::Dialog)
            .aria_label(localizer.text(TextKey::SettingsSubagentsDeleteTitle))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.close_confirm_delete(cx);
                    cx.stop_propagation();
                }
            }))
            .child(popover::dialog_title(
                &theme,
                &localizer.text(TextKey::SettingsSubagentsDeleteTitle),
            ))
            .child(div().mt(px(6.0)).child(popover::dialog_body(
                &theme,
                localizer.format_with(
                    TextKey::SettingsSubagentsDeleteBody,
                    &[("name", name.as_str())],
                ),
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
                            "subagents-delete-cancel",
                        )
                        .id("subagents-delete-cancel")
                        .on_click(cx.listener(|this, _, _, cx| this.close_confirm_delete(cx))),
                    )
                    .child(
                        popover::btn_danger(&theme, &localizer.text(TextKey::ActionDelete))
                            .id("subagents-delete-confirm")
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_delete(cx))),
                    ),
            )
            .into_any_element();
        Some(popover::modal("subagents-delete-dialog", viewport, card))
    }

    fn render_definition_row(
        &mut self,
        definition: &SubagentDefinition,
        ix: usize,
        first: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let localizer = crate::l10n::localizer();
        let group = DefinitionGroup::of(definition);
        let writable = group.writable();
        let busy = self.busy_path.as_deref() == Some(definition.path.as_str());
        let enabled = definition.enabled;
        let name = definition.name.clone();
        let path = definition.path.clone();
        let for_toggle = definition.clone();
        let for_edit = definition.clone();
        let for_delete = definition.clone();
        let accent = theme.accent;
        let builtin = group == DefinitionGroup::Builtin;
        let mut meta = Vec::new();
        if !definition.description.trim().is_empty() {
            meta.push(
                div()
                    .min_w_0()
                    .child(SharedString::from(definition.description.clone()))
                    .into_any_element(),
            );
        }
        if !builtin && !definition.tools.is_empty() {
            meta.push(
                div()
                    .child(SharedString::from(localizer.format_with(
                        TextKey::SettingsSubagentsToolsSummary,
                        &[("tools", format_tool_list(&definition.tools).as_str())],
                    )))
                    .into_any_element(),
            );
        }
        if !builtin && !definition.disallowed_tools.is_empty() {
            meta.push(
                div()
                    .child(SharedString::from(localizer.format_with(
                        TextKey::SettingsSubagentsDisallowedToolsSummary,
                        &[(
                            "tools",
                            format_tool_list(&definition.disallowed_tools).as_str(),
                        )],
                    )))
                    .into_any_element(),
            );
        }
        if !builtin { meta.push(
            div()
                .id(("subagents-path", ix))
                .min_w_0()
                .truncate()
                .font_family(theme.font_mono.clone())
                .child(SharedString::from(path.clone()))
                .tooltip(widgets::text_tooltip_above(SharedString::from(path)))
                .into_any_element(),
        ); }
        let scope_badge = widgets::badge(
            theme,
            match group {
                DefinitionGroup::Builtin => localizer.text(TextKey::SettingsSubagentsBuiltin),
                DefinitionGroup::Project => localizer.text(TextKey::SettingsSkillsScopeProject),
                DefinitionGroup::User => localizer.text(TextKey::SettingsSkillsScopeUser),
                DefinitionGroup::Plugin => localizer.text(TextKey::SettingsSubagentsPluginSection),
                DefinitionGroup::Unknown => localizer.text(TextKey::SettingsSkillsOtherSection),
            },
        );
        let model_control: AnyElement = if group != DefinitionGroup::Unknown {
            let current = displayed_alias(definition);
            let options = model_items(self.provider_view.as_ref(), current);
            let target = definition.clone();
            let select_key = definition.path.clone();
            let state_key = select_key.clone();
            self.model_selects.entry(select_key.clone()).or_default();
            widgets::cascade(
                format!("subagents-model-{select_key}"),
                localizer.text(TextKey::SettingsSubagentsModelLabel),
                theme,
                move |page: &mut Self| page.model_selects.entry(select_key.clone()).or_default(),
            )
            .options(options, current)
            .show_group(false)
            .enabled(!busy)
            .width(if current == "inherit" { 124.0 } else { 230.0 })
            .on_select(move |page, value, _, cx| page.set_model(&target, value, cx))
            .render(self.model_selects.get(&state_key).unwrap(), cx)
            .into_any_element()
        } else {
            div().into_any_element()
        };
        let effort_control = self.render_effort_control(definition, busy, theme, cx);
        let tool_badge = if definition.tools.is_empty() || definition.tools.iter().any(|tool| tool == "*") {
            localizer.text(TextKey::SettingsSubagentsAllTools)
        } else {
            localizer.format_with(TextKey::SettingsSubagentsToolCount, &[("count", definition.tools.len().to_string().as_str())])
        };
        widgets::card_row(theme, first)
            .child(
                div().relative().flex_none().size(px(40.0)).rounded(px(12.0))
                    .bg(theme.wash(0.04)).flex().items_center().justify_center()
                    .child(crate::icons::icon(crate::icons::BOT).size(px(18.0)).text_color(theme.text_muted))
                    .when(builtin, |el| el.child(div().absolute().right(px(-2.0)).bottom(px(-2.0)).size(px(10.0)).rounded_full().border_2().border_color(theme.panel_bg())
                        .bg(if definition.name == "Explore" { gpui::rgb(0x22d3ee) } else { gpui::rgb(0x60a5fa) }))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .flex()
                    .flex_col()
                    .child(div().flex().items_center().gap(px(8.0))
                        .child(widgets::row_title(theme, definition.name.clone()))
                        .child(widgets::badge(theme, tool_badge)))
                    .child(widgets::meta_line(theme, meta)),
            )
            .when(!builtin, |row| row.child(scope_badge))
            .child(model_control)
            .child(effort_control)
            .when(writable, |row| {
                row.child({
                    let mut toggle = div()
                        .id(("subagents-toggle", ix))
                        .flex_none()
                        .w(px(widgets::SWITCH_WIDTH))
                        .h(px(40.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .role(gpui::Role::Switch)
                        .aria_label(localizer.format_with(
                            if enabled {
                                TextKey::SettingsSubagentsDisableAria
                            } else {
                                TextKey::SettingsSubagentsEnableAria
                            },
                            &[("name", name.as_str())],
                        ))
                        .aria_toggled(if enabled {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                        .child(widgets::toggle_switch(
                            theme,
                            enabled,
                            SharedString::from(format!("subagents-switch-{ix}")),
                        ));
                    if writable && !busy {
                        toggle =
                            toggle
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_enabled(&for_toggle, !enabled, cx);
                                }));
                    } else {
                        toggle = toggle.opacity(0.4);
                    }
                    toggle
                })
            })
            .when(writable, |row| {
                row.child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .child({
                            let mut edit = widgets::text_action(
                                theme,
                                widgets::ActionTone::Quiet,
                                localizer.text(TextKey::SettingsSkillsEdit),
                            )
                            .id(("subagents-edit", ix))
                            .role(gpui::Role::Button)
                            .aria_label(localizer.format_with(
                                TextKey::SettingsSubagentsEditAria,
                                &[("name", name.as_str())],
                            ))
                            .tab_index(0)
                            .focus_visible(move |s| s.border_2().border_color(accent));
                            if writable {
                                edit = edit.on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_edit(&for_edit, cx);
                                }));
                            } else {
                                edit = edit.opacity(0.4);
                            }
                            edit
                        })
                        .child({
                            let mut delete = widgets::text_action(
                                theme,
                                widgets::ActionTone::Quiet,
                                localizer.text(TextKey::ActionDelete),
                            )
                            .id(("subagents-delete", ix))
                            .role(gpui::Role::Button)
                            .aria_label(localizer.format_with(
                                TextKey::SettingsSubagentsDeleteAria,
                                &[("name", name.as_str())],
                            ))
                            .tab_index(0)
                            .text_color(theme.danger_muted)
                            .focus_visible(move |s| s.border_2().border_color(accent));
                            if writable {
                                delete = delete.on_click(cx.listener(move |this, _, _, cx| {
                                    let definition = for_delete.clone();
                                    this.request_delete(definition, cx);
                                }));
                            } else {
                                delete = delete.opacity(0.4);
                            }
                            delete
                        }),
                )
            })
    }

    fn render_group(
        &mut self,
        rows: &[SubagentDefinition],
        label: TextKey,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if rows.is_empty() {
            return None;
        }
        let mut block = widgets::section_card(theme).mt(px(8.0));
        for (ix, definition) in rows.iter().enumerate() {
            block = block.child(self.render_definition_row(definition, ix, ix == 0, theme, cx));
        }
        Some(
            div()
                .flex()
                .flex_col()
                .child(widgets::section_label(
                    theme,
                    format!("{}  {}", crate::l10n::localizer().text(label), crate::l10n::localizer().format_with(TextKey::SettingsSubagentsGroupCount, &[("count", rows.len().to_string().as_str())])),
                ))
                .child(block)
                .into_any_element(),
        )
    }
}

impl popover::ScrollRailHost for SubagentsPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for SubagentsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::toast::take_error(&mut self.error, cx);
        let theme = Theme::of(cx).for_settings_surface();
        let localizer = crate::l10n::localizer();
        let groups = group_definitions(self.definitions.clone());
        let count = groups.len();
        let loading = self.loading;
        let load_error = self.load_error.clone();
        let editor_dialog = self.render_editor_dialog(window.viewport_size(), cx);
        let delete_dialog = self.render_confirm_delete_dialog(window.viewport_size(), cx);

        let new_definition = widgets::action_button(&theme, widgets::ActionTone::Solid)
            .id("subagents-new")
            .role(gpui::Role::Button)
            .aria_label(localizer.text(TextKey::SettingsSubagentsNewAria))
            .tab_index(0)
            .focus_visible(|s| s.border_2().border_color(theme.accent))
            .on_click(cx.listener(|this, _, _, cx| this.open_create(cx)))
            .child(
                crate::icons::icon(crate::icons::PLUS)
                    .size(px(14.0))
                    .text_color(theme.on_solid),
            )
            .child(SharedString::from(
                localizer.text(TextKey::SettingsSubagentsNew),
            ));
        let reload = widgets::ghost_action(&theme)
            .id("subagents-reload")
            .role(gpui::Role::Button)
            .aria_label(localizer.text(TextKey::SettingsAppearanceThemeReload))
            .tab_index(0)
            .focus_visible(|s| s.border_2().border_color(theme.accent))
            .on_click(cx.listener(|this, _, _, cx| this.reload(cx)))
            .child(
                crate::icons::icon(crate::icons::REFRESH)
                    .size(px(14.0))
                    .text_color(theme.text_muted),
            );

        let body: AnyElement = if loading && self.definitions.is_empty() {
            div()
                .mt(px(24.0))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(popover::skeleton_rows(
                    "subagents-skeleton",
                    &theme,
                    4,
                    cx.entity_id(),
                    cx,
                ))
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(
                            localizer.text(TextKey::SettingsSubagentsLoading),
                        )),
                )
                .into_any_element()
        } else if count == 0 {
            if let Some(error) = load_error {
                div()
                    .mt(px(24.0))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(10.0))
                    .child(widgets::error_strip(&theme, error))
                    .child(
                        widgets::text_action(
                            &theme,
                            widgets::ActionTone::Outlined,
                            localizer.text(TextKey::ActionRetry),
                        )
                        .id("subagents-retry")
                        .tab_index(0)
                        .role(gpui::Role::Button)
                        .focus_visible(|s| s.border_2().border_color(theme.accent))
                        .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    )
                    .into_any_element()
            } else {
                div()
                    .mt(px(96.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .text_center()
                    .text_color(theme.text_muted)
                    .child(
                        crate::icons::icon(crate::icons::MAGIC_STICK_3)
                            .size(px(28.0))
                            .text_color(theme.text_muted.opacity(0.2)),
                    )
                    .child(
                        div()
                            .mt(px(12.0))
                            .text_size(crate::typography::ui_rems(14.0))
                            .child(SharedString::from(
                                localizer.text(TextKey::SettingsSubagentsEmpty),
                            )),
                    )
                    .child(
                        div()
                            .mt(px(4.0))
                            .text_size(crate::typography::ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(
                                localizer.text(TextKey::SettingsSubagentsEmptyDetail),
                            )),
                    )
                    .into_any_element()
            }
        } else {
            div()
                .mt(px(16.0))
                .flex()
                .flex_col()
                .when_some(load_error, |el, error| {
                    el.child(widgets::error_strip(&theme, error))
                })
                .children(self.render_group(
                    &groups.builtin,
                    TextKey::SettingsSubagentsBuiltin,
                    &theme,
                    cx,
                ))
                .children(self.render_group(
                    &groups.user,
                    TextKey::SettingsSubagentsUserSection,
                    &theme,
                    cx,
                ))
                .children(self.render_group(
                    &groups.plugin,
                    TextKey::SettingsSubagentsPluginSection,
                    &theme,
                    cx,
                ))
                .into_any_element()
        };

        let scrollbar = popover::rail(self, "subagents-page-scrollbar", &theme, cx);
        div()
            .id("subagents-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("subagents-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(
                            widgets::page_column()
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .gap(px(16.0))
                                        .child(widgets::page_header(
                                            &theme,
                                            &localizer.text(TextKey::SettingsSubagents),
                                            Some(count),
                                        ))
                                        .child(
                                            div()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap(px(8.0))
                                                .child(reload)
                                                .child(new_definition),
                                        ),
                                )
                                .child(body),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
            .when_some(editor_dialog, |el, dialog| el.child(dialog))
            .when_some(delete_dialog, |el, dialog| el.child(dialog))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(name: &str, path: &str, scope: Option<SkillScope>) -> SubagentDefinition {
        SubagentDefinition {
            name: name.into(),
            path: path.into(),
            description: format!("the {name} agent"),
            enabled: true,
            tools: Vec::new(),
            disallowed_tools: Vec::new(),
            model: None,
            scope,
            system_prompt: None,
        }
    }

    #[test]
    fn personal_listing_excludes_project_and_unknown_scopes() {
        let groups = group_definitions(vec![
            definition("alpha", "/project/alpha", Some(SkillScope::Project)),
            definition("zeta", "/user/zeta", Some(SkillScope::User)),
            definition("alpha", "/user/alpha", Some(SkillScope::User)),
            definition("plugin", "/plugins/plugin", Some(SkillScope::Plugin)),
            definition("legacy", "/legacy", None),
        ]);
        let names: Vec<&str> = groups.user.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["alpha", "zeta"]);
        assert_eq!(groups.plugin.len(), 1);
        assert_eq!(groups.len(), 3);
        assert!(DefinitionGroup::of(&groups.user[0]).writable());
        assert!(!DefinitionGroup::of(&groups.plugin[0]).writable());
    }

    #[test]
    fn an_empty_listing_groups_into_empty_groups() {
        let groups = group_definitions(Vec::new());
        assert_eq!(groups.len(), 0);
        assert!(groups.user.is_empty());
    }

    #[test]
    fn definition_names_follow_the_managed_roots_grammar() {
        for name in ["reviewer", "my_agent-2", "Reviewer2", "-_"] {
            assert!(valid_definition_name(name), "{name}");
        }
        for name in ["", "   ", "a b", "a/b", "a.md", "..", "...", "a\nb"] {
            assert!(!valid_definition_name(name), "{name}");
        }
    }

    #[test]
    fn the_form_refuses_the_name_before_the_description() {
        assert_eq!(
            form_error("bad name", ""),
            Some(TextKey::SettingsSubagentsNameInvalid)
        );
        assert_eq!(
            form_error("good-name", "   "),
            Some(TextKey::SettingsSubagentsDescriptionRequired)
        );
        assert_eq!(form_error("good-name", "what it does"), None);
    }

    #[test]
    fn tool_lists_round_trip_through_the_dialog_spelling() {
        assert_eq!(
            parse_tool_list(" Read, Grep ,, Glob "),
            vec!["Read".to_owned(), "Grep".to_owned(), "Glob".to_owned()]
        );
        assert!(parse_tool_list("   ").is_empty());
        assert_eq!(
            format_tool_list(&parse_tool_list("Read, Grep")),
            "Read, Grep"
        );
    }

    #[test]
    fn the_model_selector_shows_the_effective_alias() {
        let mut row = definition("reviewer", "/p/reviewer.md", Some(SkillScope::Project));
        // No override: inherit.
        assert_eq!(displayed_alias(&row), "inherit");
        row.model = Some("haiku".into());
        assert_eq!(displayed_alias(&row), "inherit");
        row.model = Some("fable".into());
        assert_eq!(displayed_alias(&row), "inherit");
        // An alias this build does not know reads as inherit rather than as a
        // token the selector cannot show.
        row.model = Some("nope".into());
        assert_eq!(displayed_alias(&row), "inherit");
    }

    #[test]
    fn every_alias_the_picker_offers_parses_at_the_engine() {
        // The wire vocabulary and the picker are one list; a new alias must be
        // added to both, and this fails when only one moves.
        for alias in MODEL_ALIASES {
            assert!(
                rcode_harness::native_subagents::parse_model_alias(alias).is_ok(),
                "{alias}"
            );
        }
    }
}
