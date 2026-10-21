//! Settings → Skills: personal definitions and contributions from installed plugins.

use gpui::{
    AnyElement, App, Context, Entity, SharedString, Subscription, Task, Window, div, prelude::*, px,
};

use rcode_presentation::TextKey;
use rcode_proto::HarnessId;
use rcode_proto::invocation::{Skill, SkillScope};
use rcode_rpc::methods;

use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::popover;
use crate::settings::{personal_catalog_target, widgets};
use crate::state::AppState;
use crate::theme::Theme;

/// The listing is the native agent's catalog — the one whose skill roots the
/// write RPCs manage. Other harnesses' provider catalogs are not files this
/// page could edit.
const LISTING_HARNESS: HarnessId = HarnessId::Native;

/// Height of the dialog's instruction editor (about four lines) before it
/// scrolls internally.
const BODY_EDITOR_HEIGHT: f32 = 96.0;

/// Which managed root a listing row came from, as the page groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkillGroup {
    Project,
    User,
    /// A directory an installed plugin contributes. Shown, not editable: the
    /// plugin owns those files, and a re-install replaces them.
    Plugin,
    /// Older catalogs without a scope cannot establish a personal origin.
    Unknown,
}

impl SkillGroup {
    fn of(skill: &Skill) -> Self {
        match skill.scope {
            Some(SkillScope::Project) => Self::Project,
            Some(SkillScope::User) => Self::User,
            Some(SkillScope::Plugin) => Self::Plugin,
            None => Self::Unknown,
        }
    }

    /// Whether the page can open create/edit/delete for a row from this
    /// group — the ops that name a root on the wire.
    fn writable(self) -> bool {
        matches!(self, Self::User)
    }
}

/// Rows grouped by managed root, each group name-sorted so the page never
/// depends on the scan's directory order. Pure.
pub(crate) struct SkillGroups {
    pub user: Vec<Skill>,
    pub plugin: Vec<Skill>,
}

impl SkillGroups {
    fn len(&self) -> usize {
        self.user.len() + self.plugin.len()
    }
}

fn sorted_by_name(mut rows: Vec<Skill>) -> Vec<Skill> {
    rows.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    rows
}

/// Split a listing into its managed-root groups. Pure.
pub(crate) fn group_skills(skills: Vec<Skill>) -> SkillGroups {
    let mut groups = SkillGroups {
        user: Vec::new(),
        plugin: Vec::new(),
    };
    for skill in skills {
        match SkillGroup::of(&skill) {
            SkillGroup::Project | SkillGroup::Unknown => {}
            SkillGroup::User => groups.user.push(skill),
            SkillGroup::Plugin => groups.plugin.push(skill),
        }
    }
    groups.user = sorted_by_name(groups.user);
    groups.plugin = sorted_by_name(groups.plugin);
    groups
}

/// The name grammar the managed roots enforce. A name becomes a directory
/// segment, so the page mirrors `rcode_agent::skills::validate_name` here —
/// letters, digits, dash and underscore, and never only dots — rather than
/// round-tripping a request the engine must refuse. Pure.
pub(crate) fn valid_skill_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty()
        && !trimmed.chars().all(|c| c == '.')
        && trimmed
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// Which field a create/edit form refuses, if any. The description is
/// required at both ends: it is what the model reads when deciding whether
/// to load the skill. Pure.
pub(crate) fn form_error(name: &str, description: &str) -> Option<TextKey> {
    if !valid_skill_name(name) {
        Some(TextKey::SettingsSkillsNameInvalid)
    } else if description.trim().is_empty() {
        Some(TextKey::SettingsSkillsDescriptionRequired)
    } else {
        None
    }
}

/// What a failed write should say, one message per operation.
#[derive(Debug, Clone)]
enum WriteFailure {
    Create,
    Update,
    Delete,
    Toggle { name: String },
}

impl WriteFailure {
    fn message(&self, error: &str) -> String {
        let localizer = crate::l10n::localizer();
        match self {
            Self::Create => {
                localizer.format_with(TextKey::SettingsSkillsCreateFailed, &[("error", error)])
            }
            Self::Update => {
                localizer.format_with(TextKey::SettingsSkillsUpdateFailed, &[("error", error)])
            }
            Self::Delete => {
                localizer.format_with(TextKey::SettingsSkillsDeleteFailed, &[("error", error)])
            }
            Self::Toggle { name } => localizer.format_with(
                TextKey::SettingsSkillsToggleFailed,
                &[("name", name.as_str()), ("error", error)],
            ),
        }
    }
}

/// Personal create/edit inputs; `origin` identifies the row being edited.
struct SkillEditor {
    creating: bool,
    origin: Option<Skill>,
    name: Entity<ComposerInput>,
    description: Entity<ComposerInput>,
    /// Empty for an edit means "keep the current instructions" — the page
    /// only sends what the user actually typed.
    body: Entity<ComposerInput>,
    invalid: Option<TextKey>,
    _events: Vec<Subscription>,
}

/// One dialog input: a submit that saves the dialog and an edit that clears
/// a shown validation error. `height` caps a multiline editor, which then
/// scrolls internally instead of growing the dialog.
fn dialog_input(
    placeholder: TextKey,
    height: Option<f32>,
    cx: &mut Context<SkillsPage>,
) -> (Entity<ComposerInput>, Subscription) {
    let localizer = crate::l10n::localizer();
    let input = cx.new(|cx| {
        let input = ComposerInput::new(localizer.text(placeholder), cx);
        match height {
            Some(height) => input.with_viewport_height(height),
            None => input.with_single_line(),
        }
    });
    let events = cx.subscribe(&input, |page: &mut SkillsPage, _, event, cx| match event {
        ComposerInputEvent::Submitted => page.save_editor(cx),
        ComposerInputEvent::Edited => {
            if let Some(editor) = page.editor.as_mut() {
                editor.invalid = None;
            }
            cx.notify();
        }
        _ => {}
    });
    (input, events)
}

/// The Settings → Skills page. One listing per engine connection; writes
/// replace the list from their reply so the page repaints in one round trip.
pub struct SkillsPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    skills: Vec<Skill>,
    loading: bool,
    load_error: Option<SharedString>,
    /// Identity of the engine connection that supplied the personal catalog.
    loaded_context: Option<String>,
    request: u64,
    load_task: Option<Task<()>>,
    write_task: Option<Task<()>>,
    /// Skill directory with an in-flight enable toggle.
    busy_toggle: Option<String>,
    editor: Option<SkillEditor>,
    confirm_delete: Option<Skill>,
    error: Option<SharedString>,
    _observe: Subscription,
}

impl SkillsPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&state, |page, _, cx| page.sync_context(cx));
        let mut page = Self {
            state,
            scroll: widgets::PageScroll::default(),
            skills: Vec::new(),
            loading: false,
            load_error: None,
            loaded_context: None,
            request: 0,
            load_task: None,
            write_task: None,
            busy_toggle: None,
            editor: None,
            confirm_delete: None,
            error: None,
            _observe: observe,
        };
        page.reload(cx);
        page
    }

    /// Personal catalogs are invalidated only when the engine connection changes.
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

    /// Reload when the engine connection changes. Called
    /// from the state observer, so it is a no-op string compare in steady
    /// state.
    fn sync_context(&mut self, cx: &mut Context<Self>) {
        if self.context(cx) == self.loaded_context {
            return;
        }
        self.reload(cx);
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.loaded_context = self.context(cx);
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            // Not connected yet; the observer fires again when one connects.
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
            let result = engine.client().call(methods::LIST_SKILLS, params).await;
            page.update(cx, |page, cx| {
                if page.request != request {
                    return;
                }
                page.loading = false;
                let decoded = result.and_then(|value| {
                    serde_json::from_value::<Option<Vec<Skill>>>(value)
                        .map_err(|error| rcode_rpc::RpcError::Failed(error.to_string()))
                });
                match decoded {
                    Ok(skills) => page.skills = skills.unwrap_or_default(),
                    Err(err) => {
                        let message = err.to_string();
                        page.load_error = Some(
                            crate::l10n::localizer()
                                .format_with(
                                    TextKey::SettingsSkillsLoadFailed,
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

    /// Run one skill write and apply the fresh listing it returns. The reply
    /// is only applied while the page still addresses the workspace the write
    /// was aimed at — a mid-flight switch must not repaint from stale roots.
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
                page.busy_toggle = None;
                match result {
                    Ok(value) => {
                        let decoded = serde_json::from_value::<Option<Vec<Skill>>>(value)
                            .map_err(|error| rcode_rpc::RpcError::Failed(error.to_string()));
                        match decoded {
                            Ok(skills) if page.context(cx).as_deref() == Some(context.as_str()) => {
                                page.skills = skills.unwrap_or_default();
                            }
                            // A parse failure is a write whose result cannot be
                            // confirmed; an unrelated workspace means the
                            // reply is not this view's to show. Both leave the
                            // rows to the next listing.
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
        let (name, name_events) = dialog_input(TextKey::SettingsSkillsNamePlaceholder, None, cx);
        let (description, description_events) =
            dialog_input(TextKey::SettingsSkillsDescriptionPlaceholder, None, cx);
        let (body, body_events) = dialog_input(
            TextKey::SettingsSkillsBodyPlaceholder,
            Some(BODY_EDITOR_HEIGHT),
            cx,
        );
        self.editor = Some(SkillEditor {
            creating: true,
            origin: None,
            name,
            description,
            body,
            invalid: None,
            _events: vec![name_events, description_events, body_events],
        });
        cx.notify();
    }

    fn open_edit(&mut self, skill: &Skill, cx: &mut Context<Self>) {
        if self.editor.is_some() || !SkillGroup::of(skill).writable() {
            return;
        }
        let (name, name_events) = dialog_input(TextKey::SettingsSkillsNamePlaceholder, None, cx);
        name.update(cx, |input, cx| input.set_text(skill.name.clone(), cx));
        let (description, description_events) =
            dialog_input(TextKey::SettingsSkillsDescriptionPlaceholder, None, cx);
        description.update(cx, |input, cx| {
            input.set_text(skill.description.clone(), cx)
        });
        let (body, body_events) = dialog_input(
            TextKey::SettingsSkillsBodyPlaceholder,
            Some(BODY_EDITOR_HEIGHT),
            cx,
        );
        self.editor = Some(SkillEditor {
            creating: false,
            origin: Some(skill.clone()),
            name,
            description,
            body,
            invalid: None,
            _events: vec![name_events, description_events, body_events],
        });
        cx.notify();
    }

    fn close_editor(&mut self, cx: &mut Context<Self>) {
        if self.editor.take().is_some() {
            cx.notify();
        }
    }

    /// Validate and send the dialog. Like the devices rename dialog, the
    /// editor closes up front and a failed write comes back as a toast —
    /// keeping it open would need optimistic on-page rows to confirm into.
    fn save_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let name = editor.name.read(cx).text().trim().to_string();
        let description = editor.description.read(cx).text().trim().to_string();
        let body = editor.body.read(cx).text().to_string();
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
            params["body"] = body.into();
            params["scope"] = serde_json::json!(SkillScope::User);
            self.write(methods::CREATE_SKILL, params, WriteFailure::Create, cx);
        } else {
            let Some(origin) = editor.origin.as_ref() else {
                return;
            };
            params["name"] = origin.name.clone().into();
            params["newName"] = name.into();
            params["description"] = description.into();
            params["scope"] = serde_json::json!(SkillScope::User);
            // An absent body keeps the skill's instruction; only a typed one
            // is an edit anyone means.
            if !body.trim().is_empty() {
                params["body"] = body.into();
            }
            self.write(methods::UPDATE_SKILL, params, WriteFailure::Update, cx);
        }
        cx.notify();
    }

    fn set_enabled(&mut self, skill: &Skill, enabled: bool, cx: &mut Context<Self>) {
        let mut params = self.target_params();
        params["skillPath"] = skill.path.clone().into();
        params["enabled"] = enabled.into();
        if let Some(scope) = skill.scope {
            params["scope"] = serde_json::json!(scope);
        }
        self.busy_toggle = Some(skill.path.clone());
        self.write(
            methods::SET_SKILL_ENABLED,
            params,
            WriteFailure::Toggle {
                name: skill.name.clone(),
            },
            cx,
        );
    }

    fn request_delete(&mut self, skill: Skill, cx: &mut Context<Self>) {
        self.confirm_delete = Some(skill);
        cx.notify();
    }

    fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        let Some(skill) = self.confirm_delete.take() else {
            return;
        };
        let mut params = self.target_params();
        params["name"] = skill.name.clone().into();
        if let Some(scope) = skill.scope {
            params["scope"] = serde_json::json!(scope);
        }
        self.write(methods::DELETE_SKILL, params, WriteFailure::Delete, cx);
        cx.notify();
    }

    fn close_confirm_delete(&mut self, cx: &mut Context<Self>) {
        if self.confirm_delete.take().is_some() {
            cx.notify();
        }
    }

    /// Open the create dialog. Exposed so a shell test can prove Escape closes
    /// a settings dialog before Settings itself.
    #[cfg(test)]
    pub(crate) fn open_create_for_test(&mut self, cx: &mut Context<Self>) {
        self.open_create(cx);
    }

    /// Escape that reached Settings unclaimed closes the page's dialog first,
    /// so it never closes Settings under the dialog. Returns whether it did.
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
            localizer.text(TextKey::SettingsSkillsCreateTitle)
        } else {
            localizer.text(TextKey::SettingsSkillsEditTitle)
        };
        let creating = editor.creating;
        let invalid = editor.invalid;
        let name = editor.name.clone();
        let description = editor.description.clone();
        let body = editor.body.clone();
        let card = div()
            .w(px(520.0))
            .max_h(px(f32::from(viewport.height) * 0.9))
            .p(px(20.0))
            .rounded(px(16.0))
            .bg(popover::surface_bg(&theme))
            .border_1()
            .border_color(crate::theme::hairline(0.10))
            .text_color(theme.text)
            .flex()
            .flex_col()
            .id("skills-editor-card")
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
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(widgets::field_label(
                                &theme,
                                localizer.text(TextKey::SettingsSkillsNameLabel),
                            ))
                            .child(popover::dialog_field(name.into_any_element())),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(widgets::field_label(
                                &theme,
                                localizer.text(TextKey::SettingsSkillsDescriptionLabel),
                            ))
                            .child(popover::dialog_field(description.into_any_element())),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(widgets::field_label(
                                &theme,
                                localizer.text(TextKey::SettingsSkillsBodyLabel),
                            ))
                            .child(popover::dialog_field(body.into_any_element()))
                            .when(!creating, |el| {
                                el.child(
                                    div()
                                        .text_size(crate::typography::ui_rems(11.5))
                                        .text_color(theme.text_muted)
                                        .child(SharedString::from(
                                            localizer
                                                .text(TextKey::SettingsSkillsBodyKeepsExisting),
                                        )),
                                )
                            }),
                    )
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
                        .id("skills-editor-cancel")
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
                        .id("skills-editor-save")
                        .tab_index(0)
                        .role(gpui::Role::Button)
                        .focus_visible(|s| s.border_2().border_color(theme.accent))
                        .on_click(cx.listener(|this, _, _, cx| this.save_editor(cx))),
                    ),
            )
            .into_any_element();
        Some(popover::modal("skills-editor-dialog", viewport, card))
    }

    fn render_confirm_delete_dialog(
        &mut self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let theme = Theme::of(cx).for_popup();
        let localizer = crate::l10n::localizer();
        let skill = self.confirm_delete.as_ref()?;
        let name = skill.name.clone();
        let card = popover::dialog_card(&theme)
            .id("skills-delete-card")
            .role(gpui::Role::Dialog)
            .aria_label(localizer.text(TextKey::SettingsSkillsDeleteTitle))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.close_confirm_delete(cx);
                    cx.stop_propagation();
                }
            }))
            .child(popover::dialog_title(
                &theme,
                &localizer.text(TextKey::SettingsSkillsDeleteTitle),
            ))
            .child(div().mt(px(6.0)).child(popover::dialog_body(
                &theme,
                localizer.format_with(
                    TextKey::SettingsSkillsDeleteBody,
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
                            "skills-delete-cancel",
                        )
                        .id("skills-delete-cancel")
                        .on_click(cx.listener(|this, _, _, cx| this.close_confirm_delete(cx))),
                    )
                    .child(
                        popover::btn_danger(&theme, &localizer.text(TextKey::ActionDelete))
                            .id("skills-delete-confirm")
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_delete(cx))),
                    ),
            )
            .into_any_element();
        Some(popover::modal("skills-delete-dialog", viewport, card))
    }

    fn render_skill_row(
        &mut self,
        skill: &Skill,
        ix: usize,
        first: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let localizer = crate::l10n::localizer();
        let group = SkillGroup::of(skill);
        let writable = group.writable();
        let busy = self.busy_toggle.as_deref() == Some(skill.path.as_str());
        let enabled = skill.enabled;
        let name = skill.name.clone();
        let path = skill.path.clone();
        let skill_for_toggle = skill.clone();
        let skill_for_edit = skill.clone();
        let skill_for_delete = skill.clone();
        let accent = theme.accent;
        let mut meta = Vec::new();
        if !skill.description.trim().is_empty() {
            meta.push(
                div()
                    .min_w_0()
                    .child(SharedString::from(skill.description.clone()))
                    .into_any_element(),
            );
        }
        meta.push(
            div()
                .id(("skills-path", ix))
                .min_w_0()
                .truncate()
                .font_family(theme.font_mono.clone())
                .child(SharedString::from(path.clone()))
                .tooltip(widgets::text_tooltip_above(SharedString::from(path)))
                .into_any_element(),
        );
        let scope_badge = widgets::badge(
            theme,
            match group {
                SkillGroup::Project => localizer.text(TextKey::SettingsSkillsScopeProject),
                SkillGroup::User => localizer.text(TextKey::SettingsSkillsScopeUser),
                SkillGroup::Plugin => localizer.text(TextKey::SettingsSkillsPluginSection),
                SkillGroup::Unknown => localizer.text(TextKey::SettingsSkillsOtherSection),
            },
        );
        widgets::card_row(theme, first)
            .child(
                crate::icons::icon(crate::icons::MAGIC_STICK_3)
                    .size(px(16.0))
                    .text_color(theme.text_muted.opacity(0.7)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .flex()
                    .flex_col()
                    .child(widgets::row_title(theme, skill.name.clone()))
                    .child(widgets::meta_line(theme, meta)),
            )
            .child(scope_badge)
            .child({
                // The enabled switch. Unknown-scope rows keep their listing's
                // flag visible but out of reach: the toggle names a root on
                // the wire, and this row's root is not known.
                let mut toggle = div()
                    .id(("skills-toggle", ix))
                    .flex_none()
                    .w(px(widgets::SWITCH_WIDTH))
                    .h(px(40.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .role(gpui::Role::Switch)
                    .aria_label(localizer.format_with(
                        if enabled {
                            TextKey::SettingsSkillsDisableAria
                        } else {
                            TextKey::SettingsSkillsEnableAria
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
                        SharedString::from(format!("skills-switch-{ix}")),
                    ));
                if writable && !busy {
                    toggle =
                        toggle
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_enabled(&skill_for_toggle, !enabled, cx);
                            }));
                } else {
                    toggle = toggle.opacity(0.4);
                }
                toggle
            })
            .child(
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
                        .id(("skills-edit", ix))
                        .role(gpui::Role::Button)
                        .aria_label(localizer.format_with(
                            TextKey::SettingsSkillsEditAria,
                            &[("name", name.as_str())],
                        ))
                        .tab_index(0)
                        .focus_visible(move |s| s.border_2().border_color(accent));
                        if writable {
                            edit = edit.on_click(cx.listener(move |this, _, _, cx| {
                                this.open_edit(&skill_for_edit, cx);
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
                        .id(("skills-delete", ix))
                        .role(gpui::Role::Button)
                        .aria_label(localizer.format_with(
                            TextKey::SettingsSkillsDeleteAria,
                            &[("name", name.as_str())],
                        ))
                        .tab_index(0)
                        .text_color(theme.danger_muted)
                        .focus_visible(move |s| s.border_2().border_color(accent));
                        if writable {
                            delete = delete.on_click(cx.listener(move |this, _, _, cx| {
                                let skill = skill_for_delete.clone();
                                this.request_delete(skill, cx);
                            }));
                        } else {
                            delete = delete.opacity(0.4);
                        }
                        delete
                    }),
            )
    }

    fn render_group(
        &mut self,
        rows: &[Skill],
        label: TextKey,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if rows.is_empty() {
            return None;
        }
        let mut block = widgets::section_card(theme).mt(px(8.0));
        for (ix, skill) in rows.iter().enumerate() {
            block = block.child(self.render_skill_row(skill, ix, ix == 0, theme, cx));
        }
        Some(
            div()
                .flex()
                .flex_col()
                .child(widgets::section_label(
                    theme,
                    crate::l10n::localizer().text(label),
                ))
                .child(block)
                .into_any_element(),
        )
    }
}

impl popover::ScrollRailHost for SkillsPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for SkillsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::toast::take_error(&mut self.error, cx);
        let theme = Theme::of(cx).for_settings_surface();
        let localizer = crate::l10n::localizer();
        let groups = group_skills(self.skills.clone());
        let count = groups.len();
        let loading = self.loading;
        let load_error = self.load_error.clone();
        let editor_dialog = self.render_editor_dialog(window.viewport_size(), cx);
        let delete_dialog = self.render_confirm_delete_dialog(window.viewport_size(), cx);

        let new_skill = widgets::action_button(&theme, widgets::ActionTone::Solid)
            .id("skills-new")
            .role(gpui::Role::Button)
            .aria_label(localizer.text(TextKey::SettingsSkillsNewAria))
            .tab_index(0)
            .focus_visible(|s| s.border_2().border_color(theme.accent))
            .on_click(cx.listener(|this, _, _, cx| this.open_create(cx)))
            .child(
                crate::icons::icon(crate::icons::PLUS)
                    .size(px(14.0))
                    .text_color(theme.on_solid),
            )
            .child(SharedString::from(
                localizer.text(TextKey::SettingsSkillsNew),
            ));
        let reload = widgets::ghost_action(&theme)
            .id("skills-reload")
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

        let body: AnyElement = if loading && self.skills.is_empty() {
            div()
                .mt(px(24.0))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(popover::skeleton_rows(
                    "skills-skeleton",
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
                            localizer.text(TextKey::SettingsSkillsLoading),
                        )),
                )
                .into_any_element()
        } else if count == 0 {
            if let Some(error) = load_error {
                // Nothing loaded: the error carries the retry.
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
                        .id("skills-retry")
                        .tab_index(0)
                        .role(gpui::Role::Button)
                        .focus_visible(|s| s.border_2().border_color(theme.accent))
                        .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    )
                    .into_any_element()
            } else {
                // Centered empty state, the archived page's shape.
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
                                localizer.text(TextKey::SettingsSkillsEmpty),
                            )),
                    )
                    .child(
                        div()
                            .mt(px(4.0))
                            .text_size(crate::typography::ui_rems(12.0))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(
                                localizer.text(TextKey::SettingsSkillsEmptyDetail),
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
                    &groups.user,
                    TextKey::SettingsSkillsUserSection,
                    &theme,
                    cx,
                ))
                .children(self.render_group(
                    &groups.plugin,
                    TextKey::SettingsSkillsPluginSection,
                    &theme,
                    cx,
                ))
                .into_any_element()
        };

        let scrollbar = popover::rail(self, "skills-page-scrollbar", &theme, cx);
        div()
            .id("skills-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("skills-page")
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
                                            &localizer.text(TextKey::SettingsSkills),
                                            Some(count),
                                        ))
                                        .child(
                                            div()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap(px(8.0))
                                                .child(reload)
                                                .child(new_skill),
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

    fn skill(name: &str, path: &str, scope: Option<SkillScope>) -> Skill {
        Skill {
            name: name.into(),
            path: path.into(),
            description: format!("the {name} skill"),
            enabled: true,
            command: None,
            scope,
        }
    }

    #[test]
    fn personal_listing_excludes_project_and_unknown_scopes() {
        let groups = group_skills(vec![
            skill("alpha", "/project/alpha", Some(SkillScope::Project)),
            skill("zeta", "/user/zeta", Some(SkillScope::User)),
            skill("alpha", "/user/alpha", Some(SkillScope::User)),
            skill("plugin", "/plugins/plugin", Some(SkillScope::Plugin)),
            skill("legacy", "/legacy", None),
        ]);
        let names: Vec<&str> = groups.user.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["alpha", "zeta"]);
        assert_eq!(groups.plugin.len(), 1);
        assert_eq!(groups.len(), 3);
        assert!(SkillGroup::of(&groups.user[0]).writable());
        assert!(!SkillGroup::of(&groups.plugin[0]).writable());
    }

    #[test]
    fn an_empty_listing_groups_into_empty_groups() {
        let groups = group_skills(Vec::new());
        assert_eq!(groups.len(), 0);
        assert!(groups.user.is_empty());
    }

    /// The page mirrors the managed roots' name grammar (a name is a
    /// directory segment), so an invalid name never leaves the UI.
    #[test]
    fn skill_names_follow_the_managed_roots_grammar() {
        for name in ["review", "my_skill-2", "Review2", "-_"] {
            assert!(valid_skill_name(name), "{name}");
        }
        // Unicode letters are alphanumeric to both ends.
        assert!(valid_skill_name("评价"));
        // Surrounding whitespace is trimmed before the check.
        assert!(valid_skill_name(" my_skill-2 "));
        for name in ["", "   ", "a b", "a/b", "a.b", "..", "...", "a\nb"] {
            assert!(!valid_skill_name(name), "{name}");
        }
    }

    #[test]
    fn the_form_refuses_the_name_before_the_description() {
        assert_eq!(
            form_error("bad name", ""),
            Some(TextKey::SettingsSkillsNameInvalid)
        );
        assert_eq!(
            form_error("good-name", "   "),
            Some(TextKey::SettingsSkillsDescriptionRequired)
        );
        assert_eq!(form_error("good-name", "what it does"), None);
    }

    /// The write params the page assembles carry the row's own identity: the
    /// scope names the root, the path addresses the toggle. Wire-shaped, so a
    /// change of the proto's serde spelling fails here first.
    #[test]
    fn scope_serializes_to_the_wire_tokens_the_engine_reads() {
        assert_eq!(
            serde_json::to_value(SkillScope::Project).unwrap(),
            "project"
        );
        assert_eq!(serde_json::to_value(SkillScope::User).unwrap(), "user");
        assert_eq!(
            serde_json::json!(LISTING_HARNESS),
            serde_json::json!("native")
        );
    }
}
