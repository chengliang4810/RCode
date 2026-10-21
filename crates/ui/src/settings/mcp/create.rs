use super::form::{EditorMode, FormError, ServerForm, Transport, parse_json};
use super::*;

pub(super) struct CreateEditor {
    mode: EditorMode,
    template: ServerForm,
    name: Entity<ComposerInput>,
    command: Entity<ComposerInput>,
    args: Entity<ComposerInput>,
    env: Entity<ComposerInput>,
    url: Entity<ComposerInput>,
    headers: Entity<ComposerInput>,
    json: Entity<ComposerInput>,
    show_options: bool,
    saving: bool,
    error: Option<SharedString>,
    context: Option<String>,
    _events: Vec<Subscription>,
}

impl CreateEditor {
    fn set_saving(&mut self, saving: bool, cx: &mut Context<McpPage>) {
        self.saving = saving;
        for input in [
            &self.name,
            &self.command,
            &self.args,
            &self.env,
            &self.url,
            &self.headers,
            &self.json,
        ] {
            input.update(cx, |input, cx| {
                input.read_only = saving;
                cx.notify();
            });
        }
    }

    fn form(&self, cx: &App) -> ServerForm {
        let mut form = self.template.clone();
        form.name = self.name.read(cx).text().to_string();
        form.command = self.command.read(cx).text().to_string();
        form.args = self.args.read(cx).text().to_string();
        form.env = self.env.read(cx).text().to_string();
        form.url = self.url.read(cx).text().to_string();
        form.headers = self.headers.read(cx).text().to_string();
        form
    }
}

fn input(
    placeholder: &str,
    height: Option<f32>,
    cx: &mut Context<McpPage>,
) -> (Entity<ComposerInput>, Subscription) {
    let input = cx.new(|cx| {
        let input = if height.is_some() {
            ComposerInput::with_context(placeholder, "SettingsTextarea", cx)
        } else {
            ComposerInput::new(placeholder, cx)
        }
        .with_tab_stop();
        match height {
            Some(height) => input.with_viewport_height(height),
            None => input.with_single_line(),
        }
    });
    let events = cx.subscribe(&input, |page: &mut McpPage, _, event, cx| match event {
        ComposerInputEvent::Submitted => page.save_create(cx),
        ComposerInputEvent::Edited => {
            if let Some(editor) = page.create.as_mut() {
                editor.error = None;
            }
            cx.notify();
        }
        _ => {}
    });
    (input, events)
}

impl McpPage {
    pub(super) fn open_create(&mut self, cx: &mut Context<Self>) {
        if self.create.is_some() || self.editor.is_some() {
            return;
        }
        let (name, e1) = input("my-mcp-server", None, cx);
        let (command, e2) = input("npx", None, cx);
        let (args, e3) = input("-y @modelcontextprotocol/server-memory", Some(48.0), cx);
        let (env, e4) = input(r#"{"MY_API_KEY": "your-key"}"#, Some(80.0), cx);
        let (url, e5) = input("https://example.com/mcp", None, cx);
        let (headers, e6) = input(r#"{"Authorization": "Bearer your-token"}"#, Some(80.0), cx);
        let (json, e7) = input(
            r#"{"mcpServers": {"my-server": {"command": "npx", "args": []}}}"#,
            Some(240.0),
            cx,
        );
        self.create = Some(CreateEditor {
            mode: EditorMode::Form,
            template: ServerForm::default(),
            name,
            command,
            args,
            env,
            url,
            headers,
            json,
            show_options: false,
            saving: false,
            error: None,
            context: self.context(cx),
            _events: vec![e1, e2, e3, e4, e5, e6, e7],
        });
        cx.notify();
    }

    pub(super) fn close_create(&mut self, cx: &mut Context<Self>) {
        if self.create.as_ref().is_some_and(|editor| !editor.saving) {
            self.create = None;
            cx.notify();
        }
    }

    fn set_create_mode(&mut self, mode: EditorMode, cx: &mut Context<Self>) {
        let Some(editor) = self.create.as_mut() else {
            return;
        };
        if editor.mode == mode || editor.saving {
            return;
        }
        let result = match mode {
            EditorMode::Json => editor.form(cx).json_draft().map(|text| {
                editor.json.update(cx, |input, cx| input.set_text(text, cx));
            }),
            EditorMode::Form => {
                parse_json(&editor.json.read(cx).text(), &editor.name.read(cx).text()).map(
                    |(name, config)| {
                        let form = ServerForm::from_entry(name, config);
                        for (input, text) in [
                            (&editor.name, &form.name),
                            (&editor.command, &form.command),
                            (&editor.args, &form.args),
                            (&editor.env, &form.env),
                            (&editor.url, &form.url),
                            (&editor.headers, &form.headers),
                        ] {
                            input.update(cx, |input, cx| input.set_text(text.clone(), cx));
                        }
                        editor.show_options = !form.env.is_empty() || !form.headers.is_empty();
                        editor.template = form;
                    },
                )
            }
        };
        match result {
            Ok(()) => {
                editor.mode = mode;
                editor.error = None;
            }
            Err(error) => editor.error = Some(error.message().into()),
        }
        cx.notify();
    }

    fn set_create_transport(&mut self, transport: Transport, cx: &mut Context<Self>) {
        if let Some(editor) = self.create.as_mut()
            && !editor.saving
        {
            editor.template.transport = transport;
            editor.error = None;
            cx.notify();
        }
    }

    fn save_create(&mut self, cx: &mut Context<Self>) {
        let context = self.context(cx);
        let Some(editor) = self.create.as_mut() else {
            return;
        };
        if editor.saving {
            return;
        }
        if editor.context != context {
            editor.error = Some(
                crate::l10n::localizer()
                    .text(TextKey::SettingsMcpContextChanged)
                    .into(),
            );
            cx.notify();
            return;
        }
        let result = match editor.mode {
            EditorMode::Form => editor.form(cx).submission(),
            EditorMode::Json => {
                parse_json(&editor.json.read(cx).text(), &editor.name.read(cx).text())
            }
        };
        let (name, config) = match result {
            Ok(entry) => entry,
            Err(error) => {
                editor.error = Some(error.message().into());
                cx.notify();
                return;
            }
        };
        if self
            .servers
            .iter()
            .any(|row| row.source == rcode_mcp::McpSource::User && row.name == name)
        {
            editor.error = Some(
                FormError(TextKey::SettingsMcpNameExists, None)
                    .message()
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            editor.error = Some(
                crate::l10n::localizer()
                    .text(TextKey::SettingsMcpContextChanged)
                    .into(),
            );
            cx.notify();
            return;
        };
        editor.set_saving(true, cx);
        editor.error = None;
        let mut params = self.target_params();
        params["name"] = name.into();
        params["server"] = config;
        params["scope"] = serde_json::json!(SkillScope::User);
        params["createOnly"] = true.into();
        self.write_task = Some(cx.spawn(async move |page, cx| {
            let result = engine
                .client()
                .call(methods::SET_MCP_SERVER, params)
                .await
                .and_then(|value| {
                    serde_json::from_value::<Vec<ServerReport>>(value)
                        .map_err(|error| rcode_rpc::RpcError::Failed(error.to_string()))
                });
            page.update(cx, |page, cx| {
                match result {
                    Ok(servers) if page.context(cx) == context => {
                        page.servers = servers;
                        page.create = None;
                        page.load_error = None;
                        // Invalidate a listing started before this write.
                        page.request = page.request.wrapping_add(1);
                        page.loading = false;
                    }
                    result => {
                        if let Some(editor) = page.create.as_mut() {
                            editor.set_saving(false, cx);
                            editor.error = Some(
                                match result {
                                    Err(error) => crate::l10n::localizer().format_with(
                                        TextKey::SettingsMcpWriteFailed,
                                        &[("error", error.to_string().as_str())],
                                    ),
                                    Ok(_) => crate::l10n::localizer()
                                        .text(TextKey::SettingsMcpContextChanged),
                                }
                                .into(),
                            );
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    pub(super) fn render_create(
        &mut self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let editor = self.create.as_ref()?;
        let theme = Theme::of(cx).for_popup();
        let localizer = crate::l10n::localizer();
        let mode = editor.mode;
        let transport = editor.template.transport;
        let saving = editor.saving;
        let accent = theme.accent;
        let choice = |id: &'static str, label: String, selected: bool| {
            widgets::text_action(
                &theme,
                if selected {
                    widgets::ActionTone::Solid
                } else {
                    widgets::ActionTone::Quiet
                },
                label,
            )
            .id(id)
            .role(gpui::Role::Button)
            .tab_index(0)
            .aria_toggled(if selected {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .focus_visible(move |s| s.border_2().border_color(accent))
        };
        let field = |label: TextKey, input: &Entity<ComposerInput>| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(widgets::field_label(&theme, localizer.text(label)))
                .child(popover::dialog_field(input.clone().into_any_element()))
        };
        let mut body = div()
            .id("mcp-create-fields")
            .min_h(px(0.0))
            .max_h(px((f32::from(viewport.height) * 0.9 - 180.0).max(120.0)))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .mt(px(12.0));
        if mode == EditorMode::Json {
            body = body.child(field(TextKey::SettingsMcpJsonLabel, &editor.json));
        } else {
            body = body
                .child(field(TextKey::SettingsMcpName, &editor.name))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(widgets::field_label(
                            &theme,
                            localizer.text(TextKey::SettingsMcpTransport),
                        ))
                        .child(
                            div()
                                .flex()
                                .gap(px(8.0))
                                .child(
                                    choice(
                                        "mcp-create-stdio",
                                        "stdio".into(),
                                        transport == Transport::Stdio,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.set_create_transport(Transport::Stdio, cx)
                                        },
                                    )),
                                )
                                .child(
                                    choice(
                                        "mcp-create-http",
                                        "HTTP".into(),
                                        transport == Transport::Http,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.set_create_transport(Transport::Http, cx)
                                        },
                                    )),
                                ),
                        ),
                );
            if transport == Transport::Stdio {
                body = body
                    .child(field(TextKey::SettingsMcpCommand, &editor.command))
                    .child(field(TextKey::SettingsMcpArgs, &editor.args));
            } else {
                body = body.child(field(TextKey::SettingsMcpUrl, &editor.url));
            }
            let option_key = if transport == Transport::Stdio {
                TextKey::SettingsMcpEnv
            } else {
                TextKey::SettingsMcpHeaders
            };
            body = body.child(
                widgets::text_action(
                    &theme,
                    widgets::ActionTone::Quiet,
                    localizer.text(option_key),
                )
                .id("mcp-create-options")
                .role(gpui::Role::Button)
                .tab_index(0)
                .aria_toggled(if editor.show_options {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .focus_visible(|s| s.border_2().border_color(theme.accent))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(editor) = this.create.as_mut()
                        && !editor.saving
                    {
                        editor.show_options = !editor.show_options;
                        cx.notify();
                    }
                })),
            );
            if editor.show_options {
                body = body.child(popover::dialog_field(
                    if transport == Transport::Stdio {
                        editor.env.clone()
                    } else {
                        editor.headers.clone()
                    }
                    .into_any_element(),
                ));
            }
        }
        let card =
            popover::dialog_card(&theme)
                .w(px(520.0))
                .max_w(px(f32::from(viewport.width) * 0.95))
                .max_h(px(f32::from(viewport.height) * 0.9))
                .id("mcp-create-card")
                .role(gpui::Role::Dialog)
                .aria_label(localizer.text(TextKey::SettingsMcpNew))
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        this.close_create(cx);
                        cx.stop_propagation();
                    }
                }))
                .child(popover::dialog_title(
                    &theme,
                    localizer.text(TextKey::SettingsMcpNew),
                ))
                .child(
                    div()
                        .mt(px(12.0))
                        .flex()
                        .gap(px(8.0))
                        .child(
                            choice(
                                "mcp-create-form",
                                localizer.text(TextKey::SettingsMcpForm),
                                mode == EditorMode::Form,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_create_mode(EditorMode::Form, cx)
                            })),
                        )
                        .child(
                            choice("mcp-create-json", "JSON".into(), mode == EditorMode::Json)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_create_mode(EditorMode::Json, cx)
                                })),
                        ),
                )
                .child(body)
                .when_some(editor.error.clone(), |el, error| {
                    el.child(widgets::error_strip(&theme, error))
                })
                .child(
                    div()
                        .mt(px(16.0))
                        .flex()
                        .justify_end()
                        .gap(px(8.0))
                        .child(
                            popover::btn_ghost(
                                &theme,
                                &localizer.text(TextKey::ActionCancel),
                                "mcp-create-cancel",
                            )
                            .id("mcp-create-cancel")
                            .when(!saving, |el| {
                                el.on_click(cx.listener(|this, _, _, cx| this.close_create(cx)))
                            }),
                        )
                        .child(
                            popover::btn_primary(
                                &theme,
                                &localizer.text(if saving {
                                    TextKey::SettingsMcpSaving
                                } else {
                                    TextKey::ActionSave
                                }),
                            )
                            .id("mcp-create-save")
                            .when(!saving, |el| {
                                el.on_click(cx.listener(|this, _, _, cx| this.save_create(cx)))
                            }),
                        ),
                )
                .into_any_element();
        Some(popover::modal("mcp-create-dialog", viewport, card))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn mode_switches_keep_invalid_json_and_populate_http_form(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::app_menus::init(cx);
            crate::composer::init(cx, crate::settings::ComposerSendBehavior::default());
        });
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            let mut page = McpPage::new(state, cx);
            page.open_create(cx);
            page
        });
        window
            .update(cx, |page, _, cx| {
                let editor = page.create.as_ref().unwrap();
                editor
                    .name
                    .update(cx, |input, cx| input.set_text("files", cx));
                editor
                    .command
                    .update(cx, |input, cx| input.set_text("node", cx));
                editor.args.update(cx, |input, cx| {
                    input.set_text(r#""C:\Program Files\server.js""#, cx)
                });
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |page, window, cx| {
                page.set_create_mode(EditorMode::Json, cx);
                let editor = page.create.as_ref().unwrap();
                assert!(editor.mode == EditorMode::Json);
                let (_, config) = parse_json(&editor.json.read(cx).text(), "").unwrap();
                assert_eq!(
                    config["args"],
                    serde_json::json!([r"C:\Program Files\server.js"])
                );
                use gpui::Focusable;
                window.focus(&editor.json.focus_handle(cx), cx);
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "enter");
        window
            .update(cx, |page, _, cx| {
                let editor = page.create.as_ref().unwrap();
                assert!(editor.json.read(cx).text().ends_with('\n'));
                assert!(
                    editor.error.is_none(),
                    "Enter in JSON must insert a newline, not submit"
                );
                editor
                    .json
                    .update(cx, |input, cx| input.set_text("{ invalid", cx));
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |page, _, cx| {
                page.set_create_mode(EditorMode::Form, cx);
                let editor = page.create.as_ref().unwrap();
                assert!(editor.mode == EditorMode::Json);
                assert_eq!(editor.json.read(cx).text(), "{ invalid");
                assert!(editor.error.is_some());
            })
            .unwrap();
        window.update(cx, |page, _, cx| {
            page.create.as_ref().unwrap().json.update(cx, |input, cx| input.set_text(
                r#"{"mcpServers":{"docs":{"type":"http","url":"https://example.invalid/mcp","headers":{"X-Example":"value"}}}}"#, cx));
        }).unwrap();
        cx.run_until_parked();
        window
            .update(cx, |page, _, cx| {
                page.set_create_mode(EditorMode::Form, cx);
                let editor = page.create.as_ref().unwrap();
                assert!(editor.mode == EditorMode::Form);
                assert!(editor.template.transport == Transport::Http);
                assert_eq!(editor.name.read(cx).text(), "docs");
                assert_eq!(editor.url.read(cx).text(), "https://example.invalid/mcp");
                assert!(editor.show_options);
                assert!(editor.error.is_none());
                // A failed save (no connected engine here) keeps the complete draft open.
                page.save_create(cx);
                let editor = page.create.as_ref().unwrap();
                assert!(editor.error.is_some());
                assert_eq!(editor.name.read(cx).text(), "docs");
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
    }
}
