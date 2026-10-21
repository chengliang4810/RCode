//! Settings → Plugins: what is installed, what each one contributes, and the
//! switches that turn one off. Installation is exclusively through the marketplace.
//!
//! A plugin is a directory with a manifest and the contribution directories
//! beside it (skills, agents, hooks, MCP servers). The page shows that
//! projection rather than a per-contribution editor: the plugin owns its files,
//! and an edit here would be undone by the next install. What the page does own
//! is the lifecycle — switch one off, remove it — and the
//! honest report of what a broken one failed to provide.

use gpui::{
    AnyElement, Context, Entity, SharedString, Subscription, Task, Window, div, prelude::*, px,
};

use rcode_plugins::Plugin;
use rcode_presentation::TextKey;
use rcode_rpc::methods;

use crate::popover;
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

/// How a failure is described, one message per operation.
#[derive(Debug, Clone)]
enum Failure {
    Toggle { name: String },
    Uninstall,
}

impl Failure {
    fn message(&self, error: &str) -> String {
        let localizer = crate::l10n::localizer();
        match self {
            Self::Toggle { name } => localizer.format_with(
                TextKey::SettingsPluginsToggleFailed,
                &[("name", name.as_str()), ("error", error)],
            ),
            Self::Uninstall => {
                localizer.format_with(TextKey::SettingsPluginsUninstallFailed, &[("error", error)])
            }
        }
    }
}

pub struct PluginsPage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    plugins: Vec<Plugin>,
    loading: bool,
    load_error: Option<SharedString>,
    request: u64,
    load_task: Option<Task<()>>,
    write_task: Option<Task<()>>,
    busy_plugin: Option<String>,
    error: Option<SharedString>,
    _observe: Subscription,
}

impl PluginsPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&state, |page, _, cx| {
            if page.load_task.is_none() && page.plugins.is_empty() {
                page.reload(cx);
            } else {
                cx.notify();
            }
        });
        let mut page = Self {
            state,
            scroll: widgets::PageScroll::default(),
            plugins: Vec::new(),
            loading: false,
            load_error: None,
            request: 0,
            load_task: None,
            write_task: None,
            busy_plugin: None,
            error: None,
            _observe: observe,
        };
        page.reload(cx);
        page
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.loading = false;
            self.load_error = None;
            cx.notify();
            return;
        };
        self.request = self.request.wrapping_add(1);
        let request = self.request;
        self.loading = true;
        self.load_error = None;
        self.load_task = Some(cx.spawn(async move |page, cx| {
            let result = engine
                .client()
                .call(methods::LIST_PLUGINS, serde_json::json!({}))
                .await;
            page.update(cx, |page, cx| {
                if page.request != request {
                    return;
                }
                page.loading = false;
                page.load_task = None;
                match result.and_then(|value| {
                    serde_json::from_value::<Vec<Plugin>>(value)
                        .map_err(|error| rcode_rpc::RpcError::Failed(error.to_string()))
                }) {
                    Ok(plugins) => page.plugins = plugins,
                    Err(err) => {
                        page.load_error = Some(
                            crate::l10n::localizer()
                                .format_with(
                                    TextKey::SettingsPluginsLoadFailed,
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

    fn write(
        &mut self,
        method: &'static str,
        params: serde_json::Value,
        failure: Failure,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.write_task = Some(cx.spawn(async move |page, cx| {
            let result = engine.client().call(method, params).await;
            page.update(cx, |page, cx| {
                page.busy_plugin = None;
                match result {
                    Ok(value) => {
                        if let Ok(plugins) = serde_json::from_value::<Vec<Plugin>>(value) {
                            page.plugins = plugins;
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

    fn set_enabled(&mut self, plugin: &Plugin, enabled: bool, cx: &mut Context<Self>) {
        self.busy_plugin = Some(plugin.name.clone());
        self.write(
            methods::SET_PLUGIN_ENABLED,
            serde_json::json!({ "name": plugin.name, "enabled": enabled }),
            Failure::Toggle {
                name: plugin.name.clone(),
            },
            cx,
        );
    }

    fn uninstall(&mut self, plugin: &Plugin, cx: &mut Context<Self>) {
        self.busy_plugin = Some(plugin.name.clone());
        self.write(
            methods::UNINSTALL_PLUGIN,
            serde_json::json!({ "name": plugin.name }),
            Failure::Uninstall,
            cx,
        );
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    fn render_plugin_row(
        &mut self,
        plugin: &Plugin,
        ix: usize,
        first: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let localizer = crate::l10n::localizer();
        let name = plugin.name.clone();
        let enabled = plugin.enabled;
        let busy = self.busy_plugin.is_some();
        let for_toggle = plugin.clone();
        let for_uninstall = plugin.clone();
        let accent = theme.accent;
        let mut meta = Vec::new();
        let summary = plugin.contributions.summary();
        if !summary.is_empty() {
            meta.push(
                div()
                    .min_w_0()
                    .child(SharedString::from(summary))
                    .into_any_element(),
            );
        }
        if let Some(description) = &plugin.description {
            meta.push(
                div()
                    .min_w_0()
                    .truncate()
                    .child(SharedString::from(description.clone()))
                    .into_any_element(),
            );
        }
        if let Some(version) = &plugin.version {
            meta.push(
                div()
                    .child(SharedString::from(format!("v{version}")))
                    .into_any_element(),
            );
        }
        widgets::card_row(theme, first)
            .child(
                crate::icons::icon(crate::icons::PUZZLE)
                    .size(px(16.0))
                    .text_color(theme.text_muted.opacity(0.7)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .flex()
                    .flex_col()
                    .child(widgets::row_title(theme, plugin.name.clone()))
                    .child(widgets::meta_line(theme, meta)),
            )
            .child({
                let mut toggle = div()
                    .id(("plugin-toggle", ix))
                    .flex_none()
                    .w(px(widgets::SWITCH_WIDTH))
                    .h(px(40.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .role(gpui::Role::Switch)
                    .aria_label(localizer.format_with(
                        if enabled {
                            TextKey::SettingsPluginsDisableAria
                        } else {
                            TextKey::SettingsPluginsEnableAria
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
                        SharedString::from(format!("plugins-switch-{ix}")),
                    ));
                if busy {
                    toggle = toggle.opacity(0.4);
                } else {
                    toggle =
                        toggle
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_enabled(&for_toggle, !enabled, cx);
                            }));
                }
                toggle
            })
            .child(
                widgets::text_action(
                    theme,
                    widgets::ActionTone::Quiet,
                    localizer.text(TextKey::SettingsPluginsRemove),
                )
                .id(("plugin-remove", ix))
                .role(gpui::Role::Button)
                .tab_index(0)
                .text_color(theme.danger_muted)
                .focus_visible(move |s| s.border_2().border_color(accent))
                .when(!busy, |action| {
                    action.on_click(cx.listener(move |this, _, _, cx| {
                        this.uninstall(&for_uninstall, cx);
                    }))
                })
                .when(busy, |action| action.opacity(0.4)),
            )
    }
}

impl popover::ScrollRailHost for PluginsPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for PluginsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::toast::take_error(&mut self.error, cx);
        let theme = Theme::of(cx).for_settings_surface();
        let localizer = crate::l10n::localizer();
        let count = self.plugins.len();
        let loading = self.loading;
        let load_error = self.load_error.clone();
        let plugins = self.plugins.clone();

        let body: AnyElement = if loading && plugins.is_empty() {
            div()
                .mt(px(24.0))
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(SharedString::from(
                    localizer.text(TextKey::SettingsPluginsLoading),
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
                    .child(SharedString::from(
                        localizer.text(TextKey::SettingsPluginsEmpty),
                    ))
                    .child(
                        div()
                            .mt(px(4.0))
                            .text_size(crate::typography::ui_rems(12.0))
                            .child(SharedString::from(
                                localizer.text(TextKey::SettingsPluginsEmptyDetail),
                            )),
                    )
                    .into_any_element()
            }
        } else {
            let mut card = widgets::section_card(&theme).mt(px(16.0));
            for (ix, plugin) in plugins.iter().enumerate() {
                card = card.child(self.render_plugin_row(plugin, ix, ix == 0, &theme, cx));
            }
            div()
                .flex()
                .flex_col()
                .when_some(load_error, |el, error| {
                    el.child(widgets::error_strip(&theme, error))
                })
                .child(card)
                .into_any_element()
        };

        let scrollbar = popover::rail(self, "plugins-page-scrollbar", &theme, cx);
        div()
            .id("plugins-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("plugins-page")
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
                                            &localizer.text(TextKey::SettingsPlugins),
                                            Some(count),
                                        )),
                                )
                                .child(body),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
    }
}
