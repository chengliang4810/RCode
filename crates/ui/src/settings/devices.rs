//! Local external access and the execution hosts this desktop connects to.

use gpui::{App, ClipboardItem, Context, Entity, Subscription, Task, Window, div, prelude::*, px};
use rcode_presentation::TextKey;
use serde::Deserialize;
use serde_json::{Value, json};

use super::widgets;
use crate::{composer::ComposerInput, icons, popover, state::AppState, theme::Theme};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteAccess {
    enabled: bool,
    port: u16,
    has_api_key: bool,
    error: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteHost {
    id: String,
    name: String,
    url: String,
    enabled: bool,
    connected: bool,
    error: Option<String>,
}

struct HostEditor {
    id: Option<String>,
    name: Entity<ComposerInput>,
    url: Entity<ComposerInput>,
    key: Entity<ComposerInput>,
    key_visible: bool,
    loading_key: bool,
    generation: u64,
}

pub struct DevicesPage {
    state: Entity<AppState>,
    access: Option<RemoteAccess>,
    port: Entity<ComposerInput>,
    key: Entity<ComposerInput>,
    key_visible: bool,
    hosts: Vec<RemoteHost>,
    editor: Option<HostEditor>,
    editor_generation: u64,
    loading: bool,
    saving: bool,
    error: Option<String>,
    loaded_context: Option<usize>,
    context_generation: u64,
    scroll: widgets::PageScroll,
    _load: Option<Task<()>>,
    _write: Option<Task<()>>,
    _key_load: Option<Task<()>>,
    _poll: Option<Task<()>>,
    _observe: Subscription,
}

fn input(
    value: &str,
    placeholder: &str,
    secret: bool,
    cx: &mut Context<DevicesPage>,
) -> Entity<ComposerInput> {
    cx.new(|cx| {
        let mut input = ComposerInput::new(placeholder, cx)
            .with_single_line()
            .with_tab_stop();
        input.set_text(value, cx);
        input.set_masked(secret, cx);
        input
    })
}

fn field(theme: &Theme, label: TextKey, input: &Entity<ComposerInput>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(widgets::field_label(
            theme,
            crate::l10n::localizer().text(label),
        ))
        .child(popover::dialog_field(input.clone().into_any_element()))
}

fn valid_key(key: &str) -> bool {
    key.len() >= 8
        && key.bytes().all(|c| c.is_ascii_alphanumeric())
        && key.bytes().any(|c| c.is_ascii_alphabetic())
        && key.bytes().any(|c| c.is_ascii_digit())
}

impl DevicesPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let port = input("27655", "27655", false, cx);
        let key = input("", "API Key", true, cx);
        let observe = cx.observe(&state, |page: &mut Self, _, cx| {
            if page.context(cx) != page.loaded_context {
                page.load(cx);
            }
        });
        let mut page = Self {
            state,
            access: None,
            port,
            key,
            key_visible: false,
            hosts: Vec::new(),
            editor: None,
            editor_generation: 0,
            loading: false,
            saving: false,
            error: None,
            loaded_context: None,
            context_generation: 0,
            scroll: widgets::PageScroll::default(),
            _load: None,
            _write: None,
            _key_load: None,
            _poll: None,
            _observe: observe,
        };
        page.load(cx);
        page.set_active(true, cx);
        page
    }

    pub(crate) fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if !active {
            self._poll = None;
            return;
        }
        if self._poll.is_some() {
            return;
        }
        self._poll = Some(cx.spawn(async move |page, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                let Ok((engine, context, generation)) = page.update(cx, |page, cx| {
                    (
                        page.state.read(cx).engine().cloned(),
                        page.context(cx),
                        page.context_generation,
                    )
                }) else {
                    return;
                };
                let Some(engine) = engine else {
                    continue;
                };
                let result = engine.client().call("ListRemoteHosts", json!({})).await;
                if page
                    .update(cx, |page, cx| {
                        if !page.is_current_context(context, generation, cx) {
                            return;
                        }
                        if let Ok(hosts) = result.and_then(|v| {
                            serde_json::from_value(v)
                                .map_err(|e| rcode_rpc::RpcError::Failed(e.to_string()))
                        }) {
                            page.hosts = hosts;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    return;
                }
            }
        }));
    }

    fn context(&self, cx: &App) -> Option<usize> {
        self.state
            .read(cx)
            .engine()
            .map(|engine| engine.client() as *const _ as usize)
    }

    fn is_current_context(&self, context: Option<usize>, generation: u64, cx: &App) -> bool {
        self.context(cx) == context
            && self.loaded_context == context
            && self.context_generation == generation
    }

    fn sync_context(&mut self, cx: &mut Context<Self>) {
        let context = self.context(cx);
        if self.loaded_context == context {
            return;
        }
        self.loaded_context = context;
        self.context_generation = self.context_generation.wrapping_add(1);
        self._load = None;
        self._write = None;
        self._key_load = None;
        let was_polling = self._poll.take().is_some();
        self.access = None;
        self.hosts.clear();
        self.editor = None;
        self.editor_generation = self.editor_generation.wrapping_add(1);
        self.key_visible = false;
        self.error = None;
        self.loading = false;
        self.set_saving(false, cx);
        self.port
            .update(cx, |input, cx| input.set_text("27655", cx));
        self.key.update(cx, |input, cx| {
            input.set_text("", cx);
            input.set_masked(true, cx);
        });
        if was_polling {
            self.set_active(true, cx);
        }
        cx.notify();
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.sync_context(cx);
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let context = self.loaded_context;
        let generation = self.context_generation;
        let port_revision = self.port.read(cx).edit_revision();
        let key_revision = self.key.read(cx).edit_revision();
        self.loading = true;
        self.error = None;
        self._load = Some(cx.spawn(async move |page, cx| {
            let result = async {
                let access: RemoteAccess = serde_json::from_value(
                    engine
                        .client()
                        .call("GetRemoteAccess", json!({}))
                        .await
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                let hosts: Vec<RemoteHost> = serde_json::from_value(
                    engine
                        .client()
                        .call("ListRemoteHosts", json!({}))
                        .await
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                let key = if access.has_api_key {
                    engine
                        .client()
                        .call("GetRemoteAccessApiKey", json!({}))
                        .await
                        .map_err(|e| e.to_string())?["apiKey"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned()
                } else {
                    String::new()
                };
                Ok::<_, String>((access, hosts, key))
            }
            .await;
            page.update(cx, |page, cx| {
                if !page.is_current_context(context, generation, cx) {
                    return;
                }
                page.loading = false;
                match result {
                    Ok((access, hosts, key)) => {
                        if page.port.read(cx).edit_revision() == port_revision {
                            page.port.update(cx, |input, cx| {
                                input.set_text(access.port.to_string(), cx)
                            });
                        }
                        if page.key.read(cx).edit_revision() == key_revision {
                            page.key.update(cx, |input, cx| input.set_text(key, cx));
                        }
                        page.access = Some(access);
                        page.hosts = hosts;
                    }
                    Err(error) => page.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn set_saving(&mut self, saving: bool, cx: &mut Context<Self>) {
        self.saving = saving;
        for input in [&self.port, &self.key] {
            input.update(cx, |input, cx| {
                input.read_only = saving;
                cx.notify();
            });
        }
        if let Some(editor) = &self.editor {
            for input in [&editor.name, &editor.url, &editor.key] {
                input.update(cx, |input, cx| {
                    input.read_only = saving;
                    cx.notify();
                });
            }
        }
    }

    fn save_access(&mut self, enabled: bool, replace_key: bool, cx: &mut Context<Self>) {
        if self.saving || self.loading {
            return;
        }
        let text = crate::l10n::localizer();
        let port = if enabled {
            match self.port.read(cx).text().parse::<u16>() {
                Ok(port) if port != 0 => port,
                _ => {
                    self.error = Some(text.text(TextKey::SettingsDevicesInvalidPort));
                    cx.notify();
                    return;
                }
            }
        } else {
            // Closing access must not depend on an unfinished settings draft.
            self.access.as_ref().map_or(27655, |access| access.port)
        };
        let mut params = json!({"enabled":enabled,"port":port});
        if enabled && replace_key {
            let key = self.key.read(cx).text();
            if !valid_key(key) {
                self.error = Some(text.text(TextKey::SettingsDevicesInvalidKey));
                cx.notify();
                return;
            }
            params["apiKey"] = json!(key);
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let context = self.context(cx);
        let generation = self.context_generation;
        self.set_saving(true, cx);
        self.error = None;
        self._write = Some(cx.spawn(async move |page, cx| {
            let result = async {
                let access: RemoteAccess = serde_json::from_value(
                    engine
                        .client()
                        .call("SetRemoteAccess", params)
                        .await
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                let key = engine
                    .client()
                    .call("GetRemoteAccessApiKey", json!({}))
                    .await
                    .map_err(|e| e.to_string())?["apiKey"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                Ok::<_, String>((access, key))
            }
            .await;
            page.update(cx, |page, cx| {
                if !page.is_current_context(context, generation, cx) {
                    return;
                }
                page.set_saving(false, cx);
                match result {
                    Ok((access, key)) => {
                        page.key.update(cx, |input, cx| input.set_text(key, cx));
                        page.access = Some(access);
                    }
                    Err(error) => page.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn edit_host(&mut self, host: Option<RemoteHost>, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.editor_generation = self.editor_generation.wrapping_add(1);
        let generation = self.editor_generation;
        let (id, name, url) = host
            .map(|host| (Some(host.id), host.name, host.url))
            .unwrap_or((None, String::new(), String::new()));
        self.editor = Some(HostEditor {
            id: id.clone(),
            name: input(&name, "", false, cx),
            url: input(&url, "ws://host:27655", false, cx),
            key: input("", "API Key", true, cx),
            key_visible: false,
            loading_key: id.is_some(),
            generation,
        });
        self.error = None;
        if let Some(id) = id {
            let key_revision = self.editor.as_ref().unwrap().key.read(cx).edit_revision();
            let Some(engine) = self.state.read(cx).engine().cloned() else {
                return;
            };
            let context = self.context(cx);
            let context_generation = self.context_generation;
            self._key_load = Some(cx.spawn(async move |page, cx| {
                let result = engine
                    .client()
                    .call("GetRemoteHostApiKey", json!({"id":id}))
                    .await;
                page.update(cx, |page, cx| {
                    if !page.is_current_context(context, context_generation, cx) {
                        return;
                    }
                    let Some(editor) = page.editor.as_mut().filter(|e| e.generation == generation)
                    else {
                        return;
                    };
                    editor.loading_key = false;
                    match result {
                        Ok(value) => {
                            // A delayed credential load may not replace a newer draft.
                            if editor.key.read(cx).edit_revision() == key_revision {
                                editor.key.update(cx, |input, cx| {
                                    input.set_text(value["apiKey"].as_str().unwrap_or_default(), cx)
                                });
                            }
                        }
                        Err(error) => page.error = Some(error.to_string()),
                    }
                    cx.notify();
                })
                .ok();
            }));
        }
        cx.notify();
    }

    fn save_host(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let name = editor.name.read(cx).text().trim().to_owned();
        let url = editor.url.read(cx).text().trim().to_owned();
        let key = editor.key.read(cx).text().to_owned();
        let valid_url = url::Url::parse(&url).is_ok_and(|url| {
            matches!(url.scheme(), "ws" | "wss" | "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        });
        if name.is_empty() || !valid_url {
            self.error = Some(crate::l10n::localizer().text(TextKey::SettingsDevicesInvalidHost));
            cx.notify();
            return;
        }
        if (!key.is_empty() || editor.id.is_none()) && !valid_key(&key) {
            self.error = Some(crate::l10n::localizer().text(TextKey::SettingsDevicesInvalidKey));
            cx.notify();
            return;
        }
        let mut params = json!({"name":name,"url":url});
        if let Some(id) = &editor.id {
            params["id"] = json!(id);
        }
        if !key.is_empty() {
            params["apiKey"] = json!(key);
        }
        self.write_host("SaveRemoteHost", params, true, cx);
    }

    fn write_host(
        &mut self,
        method: &'static str,
        params: Value,
        close_editor: bool,
        cx: &mut Context<Self>,
    ) {
        if self.saving {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let context = self.context(cx);
        let generation = self.context_generation;
        self.set_saving(true, cx);
        self.error = None;
        self._write = Some(cx.spawn(async move |page, cx| {
            let result = async {
                engine.client().call(method, params).await?;
                engine.client().call("ListRemoteHosts", json!({})).await
            }
            .await;
            page.update(cx, |page, cx| {
                if !page.is_current_context(context, generation, cx) {
                    return;
                }
                page.set_saving(false, cx);
                match result.and_then(|v| {
                    serde_json::from_value(v)
                        .map_err(|e| rcode_rpc::RpcError::Failed(e.to_string()))
                }) {
                    Ok(hosts) => {
                        page.hosts = hosts;
                        if close_editor {
                            page.editor = None;
                        }
                    }
                    Err(error) => page.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    fn render_access(&mut self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let text = crate::l10n::localizer();
        let enabled = self.access.as_ref().is_some_and(|access| access.enabled);
        let busy = self.loading || self.saving;
        let mut card = widgets::section_card(theme).mt_0().child(
            widgets::card_row(theme, true)
                .child(
                    widgets::row_title(theme, text.text(TextKey::SettingsDevicesExternalAccess))
                        .flex_1(),
                )
                .child(
                    div()
                        .id("devices-external-access")
                        .role(gpui::Role::Switch)
                        .aria_label(text.text(TextKey::SettingsDevicesExternalAccess))
                        .aria_toggled(if enabled {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                        .tab_index(0)
                        .cursor_pointer()
                        .opacity(if busy { 0.5 } else { 1.0 })
                        .on_click(
                            cx.listener(move |page, _, _, cx| {
                                page.save_access(!enabled, false, cx)
                            }),
                        )
                        .on_key_down(cx.listener(move |page, event: &gpui::KeyDownEvent, _, cx| {
                            if !event.is_held
                                && matches!(event.keystroke.key.as_str(), "enter" | "space")
                            {
                                cx.stop_propagation();
                                page.save_access(!enabled, false, cx);
                            }
                        }))
                        .child(widgets::toggle_switch(
                            theme,
                            enabled,
                            "devices-external-toggle",
                        )),
                ),
        );
        card = card.child(div().px(px(16.0)).pb(px(16.0)).child(field(
            theme,
            TextKey::SettingsDevicesPort,
            &self.port,
        )));
        if enabled {
            let visible = self.key_visible;
            card = card.child(
                div()
                    .px(px(16.0))
                    .pb(px(16.0))
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap(px(8.0))
                            .child(
                                field(theme, TextKey::SettingsDevicesApiKey, &self.key)
                                    .flex_1()
                                    .min_w_0(),
                            )
                            .child(
                                widgets::ghost_action(theme)
                                    .id("devices-key-visibility")
                                    .role(gpui::Role::Button)
                                    .tab_index(0)
                                    .aria_label(text.text(if visible {
                                        TextKey::SettingsDevicesHideKey
                                    } else {
                                        TextKey::SettingsDevicesShowKey
                                    }))
                                    .on_click(cx.listener(|page, _, _, cx| {
                                        page.key_visible = !page.key_visible;
                                        page.key.update(cx, |input, cx| {
                                            input.set_masked(!page.key_visible, cx)
                                        });
                                        cx.notify();
                                    }))
                                    .child(
                                        icons::icon(if visible {
                                            icons::EYE
                                        } else {
                                            icons::EYE_CLOSED
                                        })
                                        .size(px(16.0)),
                                    ),
                            )
                            .child(
                                widgets::text_action(
                                    theme,
                                    widgets::ActionTone::Quiet,
                                    text.text(TextKey::SettingsDevicesCopyKey),
                                )
                                .id("devices-key-copy")
                                .role(gpui::Role::Button)
                                .tab_index(0)
                                .on_click(cx.listener(
                                    |page, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            page.key.read(cx).text().to_owned(),
                                        ));
                                    },
                                )),
                            ),
                    )
                    .child(
                        widgets::text_action(
                            theme,
                            widgets::ActionTone::Outlined,
                            text.text(TextKey::ActionSave),
                        )
                        .id("devices-access-save")
                        .role(gpui::Role::Button)
                        .tab_index(0)
                        .opacity(if busy { 0.5 } else { 1.0 })
                        .on_click(cx.listener(|page, _, _, cx| page.save_access(true, true, cx))),
                    ),
            );
        }
        if let Some(error) = self.access.as_ref().and_then(|access| access.error.clone()) {
            card = card.child(widgets::error_strip(theme, error));
        }
        card
    }

    fn render_host_editor(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let editor = self.editor.as_ref()?;
        let text = crate::l10n::localizer();
        let visible = editor.key_visible;
        Some(
            widgets::section_card(theme)
                .mt_0()
                .p(px(16.0))
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(field(theme, TextKey::SettingsDevicesHostName, &editor.name))
                .child(field(theme, TextKey::SettingsDevicesHostUrl, &editor.url))
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap(px(8.0))
                        .child(
                            field(theme, TextKey::SettingsDevicesApiKey, &editor.key)
                                .flex_1()
                                .min_w_0(),
                        )
                        .child(
                            widgets::ghost_action(theme)
                                .id("devices-host-key-visibility")
                                .role(gpui::Role::Button)
                                .tab_index(0)
                                .aria_label(text.text(if visible {
                                    TextKey::SettingsDevicesHideKey
                                } else {
                                    TextKey::SettingsDevicesShowKey
                                }))
                                .on_click(cx.listener(|page, _, _, cx| {
                                    if let Some(editor) = page.editor.as_mut() {
                                        editor.key_visible = !editor.key_visible;
                                        editor.key.update(cx, |input, cx| {
                                            input.set_masked(!editor.key_visible, cx)
                                        });
                                        cx.notify();
                                    }
                                }))
                                .child(
                                    icons::icon(if visible {
                                        icons::EYE
                                    } else {
                                        icons::EYE_CLOSED
                                    })
                                    .size(px(16.0)),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(
                            widgets::text_action(
                                theme,
                                widgets::ActionTone::Outlined,
                                text.text(TextKey::ActionSave),
                            )
                            .id("devices-host-save")
                            .role(gpui::Role::Button)
                            .tab_index(0)
                            .opacity(if self.saving { 0.5 } else { 1.0 })
                            .on_click(cx.listener(|page, _, _, cx| page.save_host(cx))),
                        )
                        .child(
                            widgets::text_action(
                                theme,
                                widgets::ActionTone::Quiet,
                                text.text(TextKey::ActionCancel),
                            )
                            .id("devices-host-cancel")
                            .role(gpui::Role::Button)
                            .tab_index(0)
                            .on_click(cx.listener(|page, _, _, cx| {
                                if !page.saving {
                                    page.editor = None;
                                    page.error = None;
                                    cx.notify();
                                }
                            })),
                        ),
                ),
        )
    }
}

impl popover::ScrollRailHost for DevicesPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }
    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for DevicesPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let text = crate::l10n::localizer();
        let access = self.render_access(&theme, cx);
        let mut hosts = div().flex().flex_col().gap(px(8.0));
        for (index, host) in self.hosts.clone().into_iter().enumerate() {
            let edit = host.clone();
            let toggle = host.clone();
            let remove = host.clone();
            let status = if !host.enabled {
                TextKey::SettingsDevicesDisconnected
            } else if host.connected {
                TextKey::SettingsDevicesConnected
            } else if host.error.is_some() {
                TextKey::SettingsDevicesConnectionFailed
            } else {
                TextKey::SettingsDevicesConnecting
            };
            let mut row = widgets::section_card(&theme).mt_0().p(px(12.0)).flex().flex_col().gap(px(6.0))
                .child(div().flex().items_center().gap(px(8.0))
                    .child(div().flex_1().min_w_0().flex().flex_col().gap(px(3.0))
                        .child(widgets::row_title(&theme, host.name))
                        .child(div().text_color(theme.text_muted).truncate().child(host.url)))
                    .child(widgets::badge(&theme, text.text(status)))
                    .child(widgets::text_action(&theme, widgets::ActionTone::Quiet, text.text(TextKey::SettingsDevicesEdit))
                        .id(("devices-host-edit", index)).role(gpui::Role::Button).tab_index(0)
                        .on_click(cx.listener(move |page, _, _, cx| page.edit_host(Some(edit.clone()), cx))))
                    .child(widgets::text_action(&theme, widgets::ActionTone::Quiet,
                        text.text(if host.enabled { TextKey::SettingsDevicesDisconnect } else { TextKey::SettingsDevicesConnect }))
                        .id(("devices-host-toggle", index)).role(gpui::Role::Button).tab_index(0)
                        .on_click(cx.listener(move |page, _, _, cx| page.write_host("SaveRemoteHost",
                            json!({"id":toggle.id,"name":toggle.name,"url":toggle.url,"enabled":!toggle.enabled}), false, cx))))
                    .child(widgets::text_action(&theme, widgets::ActionTone::Quiet, text.text(TextKey::SettingsDevicesRemove))
                        .id(("devices-host-remove", index)).role(gpui::Role::Button).tab_index(0)
                        .on_click(cx.listener(move |page, _, _, cx| {
                            let close_editor = page.editor.as_ref().is_some_and(|editor|
                                editor.id.as_deref() == Some(remove.id.as_str()));
                            page.write_host("RemoveRemoteHost", json!({"id":remove.id}), close_editor, cx);
                        }))));
            if let Some(error) = host.error {
                row = row.child(div().text_color(theme.danger_muted).child(error));
            }
            hosts = hosts.child(row);
        }
        if self.hosts.is_empty() {
            hosts = hosts.child(
                div()
                    .text_color(theme.text_muted)
                    .child(text.text(TextKey::SettingsDevicesNoRemoteHosts)),
            );
        }
        if let Some(editor) = self.render_host_editor(&theme, cx) {
            hosts = hosts.child(editor);
        }
        hosts = hosts.child(
            widgets::text_action(
                &theme,
                widgets::ActionTone::Quiet,
                text.text(TextKey::SettingsDevicesAddHost),
            )
            .id("devices-add-host")
            .role(gpui::Role::Button)
            .tab_index(0)
            .on_click(cx.listener(|page, _, _, cx| page.edit_host(None, cx))),
        );
        let mut column = widgets::page_column()
            .child(widgets::page_header(
                &theme,
                &text.text(TextKey::ShellSettingsNavDevices),
                None,
            ))
            .child(widgets::section(
                &theme,
                text.text(TextKey::SettingsDevicesLocal),
                access,
            ))
            .child(widgets::section(
                &theme,
                text.text(TextKey::SettingsDevicesRemote),
                hosts,
            ));
        if let Some(error) = &self.error {
            column = column
                .child(widgets::error_strip(&theme, error.clone()))
                .child(
                    widgets::text_action(
                        &theme,
                        widgets::ActionTone::Quiet,
                        text.text(TextKey::ActionRetry),
                    )
                    .id("devices-retry")
                    .on_click(cx.listener(|page, _, _, cx| page.load(cx))),
                );
        }
        if self.loading {
            column = column.child(
                div()
                    .text_color(theme.text_muted)
                    .child(text.text(TextKey::ActionLoading)),
            );
        }
        let scrollbar = popover::rail(self, "devices-scrollbar", &theme, cx);
        div()
            .id("devices-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("devices-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(column),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
    }
}

#[cfg(test)]
#[path = "devices_tests.rs"]
mod tests;
