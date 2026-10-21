//! Appshots settings share the shortcut recorder and persistence events with
//! keyboard settings, while keeping capture setup on its own route.
use super::*;
use crate::appshots::{AppshotPlatform, CapabilityState};
use crate::settings::widgets;
use rcode_presentation::TextKey;

fn row(
    theme: &Theme,
    first: bool,
    title: &str,
    description: &str,
    control: gpui::AnyElement,
) -> gpui::Div {
    widgets::card_row(theme, first)
        .flex_wrap()
        .child(
            div()
                .flex_1()
                .min_w(px(160.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(widgets::row_title(theme, title.to_string()))
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(12.0))
                        .line_height(crate::typography::ui_rems(16.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(description.to_string())),
                ),
        )
        .child(control)
}

impl ShortcutsPage {
    pub(super) fn render_appshots(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::of(cx).for_settings_surface();
        let accent = theme.accent;
        let capabilities = self.appshot_capabilities;
        let localizer = crate::l10n::localizer();
        let toggle = |id: &'static str, label: String, enabled: bool| {
            widgets::toggle_switch(&theme, enabled, id)
                .id(id)
                .role(gpui::Role::Switch)
                .aria_label(label)
                .aria_toggled(if enabled {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .tab_index(0)
                .focus_visible(move |style| style.border_2().border_color(accent))
        };
        let capture = toggle(
            "appshots-enabled",
            localizer.text(TextKey::SettingsAppshotsCapture),
            self.appshots_enabled,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.appshots_enabled = !this.appshots_enabled;
            this.commit_appshots(cx);
            cx.notify();
        }));
        let sound = toggle(
            "appshot-sound-enabled",
            localizer.text(TextKey::SettingsAppshotsSound),
            self.appshot_sound_enabled,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.appshot_sound_enabled = !this.appshot_sound_enabled;
            this.commit_appshots(cx);
            cx.notify();
        }));
        let destination = widgets::select(
            "appshot-destination",
            localizer.text(TextKey::SettingsAppshotsDestination),
            &theme,
            |page: &mut Self| &mut page.destination_select,
        )
        .options(
            AppshotDestination::ALL
                .into_iter()
                .map(|destination| widgets::SelectOption::new(destination.label_text())),
            AppshotDestination::ALL
                .into_iter()
                .position(|destination| destination == self.appshot_destination)
                .unwrap_or_default(),
        )
        .width(148.0)
        .on_select(|page, ix, _, cx| {
            page.appshot_destination = AppshotDestination::ALL[ix];
            page.commit_appshots(cx);
            cx.notify();
        })
        .render(&self.destination_select, cx);
        let destination_description = match self.appshot_destination {
            AppshotDestination::Automatic => {
                localizer.text(TextKey::SettingsAppshotsDestinationAuto)
            }
            AppshotDestination::LastSession => {
                localizer.text(TextKey::SettingsAppshotsDestinationLast)
            }
            AppshotDestination::NewSession => {
                localizer.text(TextKey::SettingsAppshotsDestinationNew)
            }
        };
        let shortcut = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(widgets::badge(&theme, capabilities.global_shortcut.badge()))
            .child(self.render_binding_control(
                ShortcutId::CaptureAppshot,
                0,
                self.recording,
                &theme,
                cx,
            ));
        let action = |id: &'static str, label: String| {
            widgets::ghost_action(&theme)
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label.clone())
                .tab_index(0)
                .focus_visible(move |style| style.border_2().border_color(accent))
                .cursor_pointer()
                .child(label)
        };
        let mut window_access = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(widgets::badge(&theme, capabilities.window_capture.badge()));
        if self.appshots_enabled
            && capabilities.window_capture == CapabilityState::PermissionRequired
        {
            let label = if self.capture_access_prompted {
                localizer.text(TextKey::SettingsAppshotsOpenSettings)
            } else {
                localizer.text(TextKey::SettingsAppshotsAllowWindow)
            };
            window_access = window_access.child(action("appshots-capture-access", label).on_click(
                cx.listener(|this, _, _, cx| {
                    if this.capture_access_prompted {
                        if let Some(url) = crate::appshots::capture_settings_url() {
                            cx.open_url(url);
                        }
                    } else {
                        this.capture_access_prompted = true;
                        crate::appshots::request_capture_access();
                    }
                    this.appshot_capabilities = crate::appshots::capabilities();
                    cx.notify();
                }),
            ));
        }
        let mut text_access = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(widgets::badge(
                &theme,
                capabilities.application_text.badge(),
            ));
        if self.appshots_enabled
            && capabilities.application_text == CapabilityState::PermissionRequired
            && crate::appshots::semantic_settings_url().is_some()
        {
            let label = if self.semantic_access_prompted {
                localizer.text(TextKey::SettingsAppshotsOpenSettings)
            } else {
                localizer.text(TextKey::SettingsAppshotsEnableText)
            };
            text_access = text_access.child(action("appshots-semantic-access", label).on_click(
                cx.listener(|this, _, _, cx| {
                    if this.semantic_access_prompted {
                        if let Some(url) = crate::appshots::semantic_settings_url() {
                            cx.open_url(url);
                        }
                    } else {
                        this.semantic_access_prompted = true;
                        crate::appshots::request_semantic_access();
                    }
                    this.appshot_capabilities = crate::appshots::capabilities();
                    cx.notify();
                }),
            ));
        }
        let card = widgets::section_card(&theme)
            .child(row(
                &theme,
                true,
                &localizer.text(TextKey::SettingsAppshotsCapture),
                &localizer.text(TextKey::SettingsAppshotsCaptureDetail),
                capture.into_any_element(),
            ))
            .child(row(
                &theme,
                false,
                &localizer.text(TextKey::SettingsAppshotsSound),
                &localizer.text(TextKey::SettingsAppshotsSoundDetail),
                sound.into_any_element(),
            ))
            .child(row(
                &theme,
                false,
                &if capabilities.platform == AppshotPlatform::LinuxWayland {
                    localizer.text(TextKey::SettingsAppshotsShortcutPreferred)
                } else {
                    localizer.text(TextKey::SettingsAppshotsShortcut)
                },
                capabilities.shortcut_description(),
                shortcut.into_any_element(),
            ))
            .child(row(
                &theme,
                false,
                &localizer.text(TextKey::SettingsAppshotsDestination),
                &destination_description,
                destination.into_any_element(),
            ))
            .child(row(
                &theme,
                false,
                &localizer.text(TextKey::SettingsAppshotsWindowCapture),
                capabilities.capture_description(),
                window_access.into_any_element(),
            ))
            .child(row(
                &theme,
                false,
                &localizer.text(TextKey::SettingsAppshotsApplicationText),
                capabilities.semantic_description(),
                text_access.into_any_element(),
            ));
        let helper: SharedString = if self.recording.is_some() {
            localizer.text(TextKey::SettingsShortcutsPressEscape).into()
        } else {
            self.conflict_notice.clone().unwrap_or_default()
        };
        let refresh = action(
            "appshots-refresh-permissions",
            localizer.text(TextKey::SettingsAppshotsCheckAgain),
        )
        .aria_label(localizer.text(TextKey::SettingsAppshotsCheckAgainAria))
        .on_click(cx.listener(|this, _, _, cx| {
            this.appshot_capabilities = crate::appshots::capabilities();
            cx.notify();
        }));
        let scrollbar = self.render_scrollbar(&theme, cx);
        div()
            .id("appshots-settings-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .on_drag_move(cx.listener(Self::on_bar_drag_move))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("appshots-settings-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .track_focus(&self.focus)
                        .tab_group()
                        .tab_index(0)
                        .tab_stop(false)
                        .on_key_down(|event, window, cx| {
                            let key = &event.keystroke;
                            if key.key == "tab"
                                && !key.modifiers.control
                                && !key.modifiers.alt
                                && !key.modifiers.platform
                            {
                                if key.modifiers.shift {
                                    window.focus_prev(cx);
                                } else {
                                    window.focus_next(cx);
                                }
                                cx.stop_propagation();
                            }
                        })
                        .child(
                            widgets::page_column()
                                .child(widgets::page_header(
                                    &theme,
                                    &localizer.text(TextKey::SettingsAppshotsTitle),
                                    None,
                                ))
                                .child(
                                    widgets::page_subtitle(
                                        &theme,
                                        capabilities.setup_description(),
                                    )
                                    .line_height(px(20.0)),
                                )
                                .child(card)
                                .child(
                                    div()
                                        .min_h(px(20.0))
                                        .mt(px(8.0))
                                        .px(px(8.0))
                                        .text_size(px(12.0))
                                        .text_color(theme.text_muted)
                                        .child(helper),
                                )
                                .child(
                                    div()
                                        .mt(px(12.0))
                                        .pl(px(8.0))
                                        .flex()
                                        .flex_wrap()
                                        .items_center()
                                        .gap(px(12.0))
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_size(px(12.0))
                                                .text_color(theme.text_muted)
                                                .child(
                                                    localizer
                                                        .text(TextKey::SettingsAppshotsCheckHint),
                                                ),
                                        )
                                        .child(refresh),
                                ),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
            .into_any_element()
    }
}
