//! Personal MCP servers, with form and JSON creation and plugin contributions.

mod create;
mod form;

use gpui::{
    AnyElement, App, Context, Entity, SharedString, Subscription, Task, Window, div, prelude::*, px,
};

use rcode_mcp::ServerReport;
use rcode_presentation::TextKey;
use rcode_proto::invocation::SkillScope;
use rcode_rpc::methods;

use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::popover;
use crate::settings::{personal_catalog_target, widgets};
use crate::state::AppState;
use crate::theme::Theme;

/// Which root a row came from, as the page groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServerGroup {
    Project,
    User,
    /// A server an installed plugin contributes. Shown, not editable.
    Plugin,
}

impl ServerGroup {
    fn of(report: &ServerReport) -> Self {
        match report.source {
            rcode_mcp::McpSource::Project => Self::Project,
            rcode_mcp::McpSource::User => Self::User,
            rcode_mcp::McpSource::Plugin => Self::Plugin,
        }
    }
}

/// Rows grouped by root, each group name-sorted. Pure.
pub(crate) struct ServerGroups {
    pub user: Vec<ServerReport>,
    pub plugin: Vec<ServerReport>,
}

impl ServerGroups {
    fn len(&self) -> usize {
        self.user.len() + self.plugin.len()
    }
}

pub(crate) fn group_servers(servers: Vec<ServerReport>) -> ServerGroups {
    let mut groups = ServerGroups {
        user: Vec::new(),
        plugin: Vec::new(),
    };
    for server in servers {
        match ServerGroup::of(&server) {
            ServerGroup::Project => {}
            ServerGroup::User => groups.user.push(server),
            ServerGroup::Plugin => groups.plugin.push(server),
        }
    }
    for group in [&mut groups.user, &mut groups.plugin] {
        group.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.name.cmp(&b.name))
        });
    }
    groups
}

pub struct McpPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    servers: Vec<ServerReport>,
    loading: bool,
    load_error: Option<SharedString>,
    loaded_context: Option<String>,
    request: u64,
    load_task: Option<Task<()>>,
    write_task: Option<Task<()>>,
    /// The server whose JSON editor is open.
    editor: Option<(String, SkillScope, Entity<ComposerInput>, Vec<Subscription>)>,
    create: Option<create::CreateEditor>,
    error: Option<SharedString>,
    _observe: Subscription,
}

impl McpPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&state, |page, _, cx| page.sync_context(cx));
        let mut page = Self {
            state,
            scroll: widgets::PageScroll::default(),
            servers: Vec::new(),
            loading: false,
            load_error: None,
            loaded_context: None,
            request: 0,
            load_task: None,
            write_task: None,
            editor: None,
            create: None,
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
        personal_catalog_target()
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
            let result = engine
                .client()
                .call(methods::LIST_MCP_SERVERS, params)
                .await;
            page.update(cx, |page, cx| {
                if page.request != request {
                    return;
                }
                page.loading = false;
                match result.and_then(|value| {
                    serde_json::from_value::<Vec<ServerReport>>(value)
                        .map_err(|error| rcode_rpc::RpcError::Failed(error.to_string()))
                }) {
                    Ok(servers) => page.servers = servers,
                    Err(err) => {
                        page.load_error = Some(
                            crate::l10n::localizer()
                                .format_with(
                                    TextKey::SettingsMcpLoadFailed,
                                    &[("error", err.to_string().as_str())],
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

    fn open_editor(&mut self, name: &str, scope: SkillScope, cx: &mut Context<Self>) {
        if self.create.is_some() || self.editor.is_some() {
            return;
        }
        let localizer = crate::l10n::localizer();
        let input = cx.new(|cx| {
            let input = ComposerInput::new(localizer.text(TextKey::SettingsMcpJsonPlaceholder), cx);
            input.with_viewport_height(160.0)
        });
        let events = cx.subscribe(&input, |page: &mut McpPage, _, event, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                page.save_editor(cx);
            }
        });
        self.editor = Some((name.to_owned(), scope, input, vec![events]));
        cx.notify();
    }

    fn save_editor(&mut self, cx: &mut Context<Self>) {
        let Some((name, scope, input, _)) = self.editor.take() else {
            return;
        };
        let text = input.read(cx).text().to_string();
        let entry: serde_json::Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(
                    crate::l10n::localizer()
                        .format_with(
                            TextKey::SettingsMcpJsonInvalid,
                            &[("error", error.to_string().as_str())],
                        )
                        .into(),
                );
                cx.notify();
                return;
            }
        };
        let mut params = self.target_params();
        params["name"] = name.into();
        params["server"] = entry;
        params["scope"] = serde_json::json!(scope);
        self.write(methods::SET_MCP_SERVER, params, cx);
    }

    fn remove(&mut self, name: &str, scope: SkillScope, cx: &mut Context<Self>) {
        let mut params = self.target_params();
        params["name"] = name.into();
        params["server"] = serde_json::Value::Null;
        params["scope"] = serde_json::json!(scope);
        self.write(methods::SET_MCP_SERVER, params, cx);
    }

    fn write(&mut self, method: &'static str, params: serde_json::Value, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let context = self.context(cx).unwrap_or_default();
        self.write_task = Some(cx.spawn(async move |page, cx| {
            let result = engine.client().call(method, params).await;
            page.update(cx, |page, cx| {
                match result {
                    Ok(value) => {
                        if let Ok(servers) = serde_json::from_value::<Vec<ServerReport>>(value)
                            && page.context(cx).as_deref() == Some(context.as_str())
                        {
                            page.servers = servers;
                        }
                    }
                    Err(err) => {
                        let message = err.to_string();
                        page.error = Some(
                            crate::l10n::localizer()
                                .format_with(
                                    TextKey::SettingsMcpWriteFailed,
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

    pub(crate) fn dismiss_on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if self.create.is_some() {
            self.close_create(cx);
            return true;
        }
        if self.editor.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    fn render_editor(
        &mut self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let theme = Theme::of(cx).for_popup();
        let localizer = crate::l10n::localizer();
        let (name, _, input, _) = self.editor.as_ref()?;
        let name = name.clone();
        let input = input.clone();
        let card = popover::dialog_card(&theme)
            .id("mcp-editor-card")
            .role(gpui::Role::Dialog)
            .aria_label(localizer.text(TextKey::SettingsMcpEditTitle))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.editor = None;
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(popover::dialog_title(
                &theme,
                &localizer.text(TextKey::SettingsMcpEditTitle),
            ))
            .child(
                div()
                    .mt(px(8.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(format!("{name} · mcp.json"))),
            )
            .child(
                div()
                    .mt(px(12.0))
                    .child(popover::dialog_field(input.into_any_element())),
            )
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
                            "mcp-editor-cancel",
                        )
                        .id("mcp-editor-cancel")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.editor = None;
                            cx.notify();
                        })),
                    )
                    .child(
                        popover::btn_primary(&theme, &localizer.text(TextKey::ActionSave))
                            .id("mcp-editor-save")
                            .on_click(cx.listener(|this, _, _, cx| this.save_editor(cx))),
                    ),
            )
            .into_any_element();
        Some(popover::modal("mcp-editor-dialog", viewport, card))
    }

    fn render_row(
        &mut self,
        report: &ServerReport,
        ix: usize,
        first: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let localizer = crate::l10n::localizer();
        let name = report.name.clone();
        let scope = match report.source {
            rcode_mcp::McpSource::Project => SkillScope::Project,
            rcode_mcp::McpSource::User => SkillScope::User,
            // Plugin servers are edited in the plugin; the row's controls are
            // disabled below.
            rcode_mcp::McpSource::Plugin => SkillScope::Plugin,
        };
        let writable = report.source != rcode_mcp::McpSource::Plugin;
        let for_edit = name.clone();
        let for_remove = name.clone();
        let accent = theme.accent;
        let mut meta = Vec::new();
        meta.push(
            div()
                .min_w_0()
                .truncate()
                .font_family(theme.font_mono.clone())
                .child(SharedString::from(report.transport.clone()))
                .into_any_element(),
        );
        if let Some(problem) = &report.problem {
            meta.push(
                div()
                    .text_color(theme.danger_muted)
                    .child(SharedString::from(problem.clone()))
                    .into_any_element(),
            );
        }
        let status = if report.problem.is_some() {
            widgets::badge(theme, localizer.text(TextKey::SettingsMcpNotConnected))
        } else {
            widgets::badge_active(theme, localizer.text(TextKey::SettingsMcpConfigured))
        };
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
                    .child(widgets::row_title(theme, report.name.clone()))
                    .child(widgets::meta_line(theme, meta)),
            )
            .child(status)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        widgets::text_action(
                            theme,
                            widgets::ActionTone::Quiet,
                            localizer.text(TextKey::SettingsMcpEdit),
                        )
                        .id(("mcp-edit", ix))
                        .role(gpui::Role::Button)
                        .tab_index(0)
                        .focus_visible(move |s| s.border_2().border_color(accent))
                        .when(writable, |action| {
                            action.on_click(cx.listener(move |this, _, _, cx| {
                                this.open_editor(&for_edit, scope, cx);
                            }))
                        })
                        .when(!writable, |action| action.opacity(0.4)),
                    )
                    .child(
                        widgets::text_action(
                            theme,
                            widgets::ActionTone::Quiet,
                            localizer.text(TextKey::ActionDelete),
                        )
                        .id(("mcp-remove", ix))
                        .role(gpui::Role::Button)
                        .tab_index(0)
                        .text_color(theme.danger_muted)
                        .focus_visible(move |s| s.border_2().border_color(accent))
                        .when(writable, |action| {
                            action.on_click(cx.listener(move |this, _, _, cx| {
                                this.remove(&for_remove, scope, cx);
                            }))
                        })
                        .when(!writable, |action| action.opacity(0.4)),
                    ),
            )
    }

    fn render_group(
        &mut self,
        rows: &[ServerReport],
        label: TextKey,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if rows.is_empty() {
            return None;
        }
        let mut block = widgets::section_card(theme).mt(px(8.0));
        for (ix, report) in rows.iter().enumerate() {
            block = block.child(self.render_row(report, ix, ix == 0, theme, cx));
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

impl popover::ScrollRailHost for McpPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for McpPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::toast::take_error(&mut self.error, cx);
        let theme = Theme::of(cx).for_settings_surface();
        let localizer = crate::l10n::localizer();
        let groups = group_servers(self.servers.clone());
        let count = groups.len();
        let loading = self.loading;
        let load_error = self.load_error.clone();
        let dialog = self.render_editor(window.viewport_size(), cx);
        let create_dialog = self.render_create(window.viewport_size(), cx);

        let new_server = widgets::text_action(
            &theme,
            widgets::ActionTone::Solid,
            localizer.text(TextKey::SettingsMcpNew),
        )
        .id("mcp-new")
        .role(gpui::Role::Button)
        .tab_index(0)
        .aria_label(localizer.text(TextKey::SettingsMcpNew))
        .focus_visible(|s| s.border_2().border_color(theme.accent))
        .on_click(cx.listener(|this, _, _, cx| this.open_create(cx)));

        let reload = widgets::ghost_action(&theme)
            .id("mcp-reload")
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

        let body: AnyElement = if loading && self.servers.is_empty() {
            div()
                .mt(px(24.0))
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(
                    localizer.text(TextKey::SettingsMcpLoading),
                ))
                .into_any_element()
        } else if count == 0 {
            if let Some(error) = load_error {
                widgets::error_strip(&theme, error).into_any_element()
            } else {
                div()
                    .mt(px(96.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .text_center()
                    .text_color(theme.text_muted)
                    .child(div().text_size(crate::typography::ui_rems(14.0)).child(
                        SharedString::from(localizer.text(TextKey::SettingsMcpEmpty)),
                    ))
                    .child(
                        div()
                            .mt(px(4.0))
                            .text_size(crate::typography::ui_rems(12.0))
                            .child(SharedString::from(
                                localizer.text(TextKey::SettingsMcpEmptyDetail),
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
                    TextKey::SettingsMcpUserSection,
                    &theme,
                    cx,
                ))
                .children(self.render_group(
                    &groups.plugin,
                    TextKey::SettingsMcpPluginSection,
                    &theme,
                    cx,
                ))
                .into_any_element()
        };

        let scrollbar = popover::rail(self, "mcp-page-scrollbar", &theme, cx);
        div()
            .id("mcp-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("mcp-page")
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
                                            &localizer.text(TextKey::SettingsMcp),
                                            Some(count),
                                        ))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(8.0))
                                                .child(reload)
                                                .child(new_server),
                                        ),
                                )
                                .child(body),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
            .when_some(dialog, |el, dialog| el.child(dialog))
            .when_some(create_dialog, |el, dialog| el.child(dialog))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(name: &str, source: rcode_mcp::McpSource) -> ServerReport {
        ServerReport {
            name: name.into(),
            source,
            transport: "npx server".into(),
            enabled: true,
            connected: false,
            problem: None,
            tool_count: 0,
        }
    }

    #[test]
    fn personal_listing_excludes_projects_and_sorts_each_group() {
        let groups = group_servers(vec![
            report("alpha", rcode_mcp::McpSource::Project),
            report("zeta", rcode_mcp::McpSource::User),
            report("alpha", rcode_mcp::McpSource::User),
            report("plugin", rcode_mcp::McpSource::Plugin),
        ]);
        let names: Vec<&str> = groups.user.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["alpha", "zeta"]);
        assert_eq!(groups.plugin.len(), 1);
        assert_eq!(groups.len(), 3);
    }

    #[test]
    fn an_empty_listing_groups_into_empty_groups() {
        let groups = group_servers(Vec::new());
        assert_eq!(groups.len(), 0);
    }
}
