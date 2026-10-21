//! Settings → Shortcuts (feature-inventory §1.4): a table of the rebindable
//! bindings — click a combo to record (Esc cancels), live conflict detection,
//! per-row Reset and Restore defaults. Changes emit [`ShortcutsEvent`]; the
//! shell persists them and re-applies the app keymap.

use std::path::PathBuf;

use gpui::{
    AnyElement, Context, Entity, EventEmitter, FocusHandle, Keystroke, SharedString, Window, div,
    prelude::*, px,
};
use rcode_engine::DataRoots;
use rcode_presentation::TextKey;

use crate::appshots::{AppshotCapabilities, AppshotDestination};
use crate::data_location::{self, LocationError};
use crate::popover::{self, ScrollRailHost};

#[path = "appshots.rs"]
mod appshots_page;
use crate::settings::widgets;
use crate::settings::{
    ComposerSendBehavior, KeymapConfig, PullRequestDestination, SavePolicy, ShortcutId,
    combo_from_keystroke, display_combo,
};
use crate::state::AppState;
use crate::theme::Theme;

/// Outcome of one keystroke while recording. Pure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordOutcome {
    /// Esc — abandon recording, keep the old combo.
    Cancelled,
    /// A bare modifier (or unusable key) — stay recording.
    Ignored,
    /// A full combo landed.
    Set(String),
}

pub fn record_key(key: &str, ctrl: bool, alt: bool, shift: bool, cmd: bool) -> RecordOutcome {
    if key.eq_ignore_ascii_case("escape") {
        return RecordOutcome::Cancelled;
    }
    match combo_from_keystroke(ctrl, alt, shift, cmd, key) {
        Some(combo) => RecordOutcome::Set(combo),
        None => RecordOutcome::Ignored,
    }
}

#[derive(Debug, Clone)]
pub enum ShortcutsEvent {
    /// The keymap changed — persist + re-apply.
    KeymapChanged(KeymapConfig),
    /// The Escape fallback changed — persist it locally.
    EscapeStopsActiveAgentChanged(bool),
    /// The composer send behavior changed — persist + re-apply.
    ComposerSendBehaviorChanged(ComposerSendBehavior),
    /// The display language changed — persist. It takes effect on the next
    /// launch, so nothing is re-applied here.
    LanguageChanged(rcode_presentation::LanguagePreference),
    /// The user confirmed the restart that a language change offered. The
    /// shell owns the process, so starting the new one is its job.
    RestartRequested,
    AppshotsChanged {
        enabled: bool,
        sound_enabled: bool,
        destination: AppshotDestination,
    },
}

pub struct ShortcutsPage {
    appshots_page: bool,
    general_page: bool,
    appshots_focus_pending: bool,
    scroll: crate::settings::widgets::PageScroll,
    /// Working copy (kept in sync with the shell via change events).
    keymap: KeymapConfig,
    escape_stops_active_agent: bool,
    composer_send_behavior: ComposerSendBehavior,
    recording: Option<ShortcutId>,
    recording_blur: Option<gpui::Subscription>,
    recording_interceptor: Option<gpui::Subscription>,
    /// A rejected record attempt ("{Combo} is already assigned to {label}.") —
    /// conflicts never persist; they're refused at record time, as in the original app.
    conflict_notice: Option<SharedString>,
    focus: FocusHandle,
    appshots_enabled: bool,
    appshot_sound_enabled: bool,
    appshot_destination: AppshotDestination,
    appshot_capabilities: AppshotCapabilities,
    send_select: widgets::SelectState,
    language_select: widgets::SelectState,
    /// The preference this page shows, which the shell owns and persists.
    language: rcode_presentation::LanguagePreference,
    /// The restart confirmation is open: a language change was just made and
    /// the new language needs a fresh process to show.
    language_restart_confirm: bool,
    /// The roots this process opened. The config root is where the `location`
    /// pointer lives; the data root is what the row reports.
    data_roots: DataRoots,
    /// The pointer as read when the page was built: `None` means the stores sit
    /// beside the settings, a path means they were moved.
    data_location: Option<String>,
    /// A change was just written and needs a fresh process to be read.
    data_location_restart_confirm: bool,
    /// A picked folder could not be used; the reason is shown on the row.
    data_location_error: Option<SharedString>,
    pull_request_destination_select: widgets::SelectState,
    destination_select: widgets::SelectState,
    capture_access_prompted: bool,
    semantic_access_prompted: bool,
    /// Settings → General's thread naming card (its own title-bound picker).
    thread_naming: Entity<crate::settings::thread_naming::ThreadNamingCard>,
    /// Where new worktrees are checked out, per device.
    worktrees: Entity<super::worktrees::WorktreeSettingsCard>,
    _state: Entity<AppState>,
}

impl EventEmitter<ShortcutsEvent> for ShortcutsPage {}

impl ShortcutsPage {
    pub fn new(
        state: Entity<AppState>,
        keymap: KeymapConfig,
        escape_stops_active_agent: bool,
        composer_send_behavior: ComposerSendBehavior,
        appshots_enabled: bool,
        appshot_sound_enabled: bool,
        appshot_destination: AppshotDestination,
        language: rcode_presentation::LanguagePreference,
        data_roots: DataRoots,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.on_release(|_, _| crate::appshots::set_recording(false))
            .detach();
        let data_location = data_location::read(data_roots.config());
        Self {
            appshots_page: false,
            general_page: false,
            appshots_focus_pending: false,
            scroll: crate::settings::widgets::PageScroll::default(),
            keymap,
            escape_stops_active_agent,
            composer_send_behavior,
            recording: None,
            recording_blur: None,
            recording_interceptor: None,
            conflict_notice: None,
            focus: cx.focus_handle(),
            appshots_enabled,
            appshot_sound_enabled,
            appshot_destination,
            appshot_capabilities: crate::appshots::capabilities(),
            send_select: widgets::SelectState::default(),
            language_select: widgets::SelectState::default(),
            language,
            language_restart_confirm: false,
            data_location,
            data_location_restart_confirm: false,
            data_location_error: None,
            data_roots,
            pull_request_destination_select: widgets::SelectState::default(),
            destination_select: widgets::SelectState::default(),
            capture_access_prompted: false,
            semantic_access_prompted: false,
            thread_naming: {
                let state = state.clone();
                cx.new(|cx| crate::settings::thread_naming::ThreadNamingCard::new(state, cx))
            },
            worktrees: cx.new(|cx| super::worktrees::WorktreeSettingsCard::new(state.clone(), cx)),
            _state: state,
        }
    }

    pub fn show_appshots(&mut self, appshots: bool) {
        self.show_section(appshots, false);
    }

    pub fn show_section(&mut self, appshots: bool, general: bool) {
        if self.appshots_page != appshots || self.general_page != general {
            self.stop_recording();
            self.conflict_notice = None;
            // The restart prompt belongs to the page that raised it; leaving
            // it pending would re-open it over an unrelated page.
            self.language_restart_confirm = false;
            self.appshots_page = appshots;
            self.general_page = general;
            self.appshots_focus_pending = appshots;
            // One scroll state serves these pages — rewind it so each opens
            // at the top instead of where the other was left.
            self.scroll.reset();
            if appshots {
                self.appshot_capabilities = crate::appshots::capabilities();
            }
        }
    }

    fn start_recording(&mut self, id: ShortcutId, window: &mut Window, cx: &mut Context<Self>) {
        self.recording = Some(id);
        crate::appshots::set_recording(true);
        let page = cx.entity().downgrade();
        // Bound actions run before Div key listeners. Intercept first so a
        // conflicting chord is recorded/refused instead of running its action.
        self.recording_interceptor = Some(cx.intercept_keystrokes(move |event, window, cx| {
            let _ = page.update(cx, |page, cx| {
                if page.focus.is_focused(window) {
                    page.record_keystroke(&event.keystroke, cx);
                }
            });
        }));
        self.recording_blur = Some(cx.on_blur(&self.focus, window, |this, _, cx| {
            this.stop_recording();
            cx.notify();
        }));
        self.conflict_notice = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn stop_recording(&mut self) {
        self.recording = None;
        self.recording_blur = None;
        self.recording_interceptor = None;
        crate::appshots::set_recording(false);
    }

    fn commit(&mut self, cx: &mut Context<Self>) {
        cx.emit(ShortcutsEvent::KeymapChanged(self.keymap.clone()));
        cx.notify();
    }

    fn set_escape_stops_active_agent(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.escape_stops_active_agent != enabled {
            self.escape_stops_active_agent = enabled;
            cx.emit(ShortcutsEvent::EscapeStopsActiveAgentChanged(enabled));
            cx.notify();
        }
    }

    /// Record the language choice.
    ///
    /// Nothing is re-rendered: the process fixes the language at startup,
    /// because changing it here would leave the windows and the native menu
    /// already drawn in the old one. So a change offers the restart that makes
    /// it visible — see [`Self::render_language_restart_dialog`].
    fn set_language(
        &mut self,
        preference: rcode_presentation::LanguagePreference,
        cx: &mut Context<Self>,
    ) {
        if self.language != preference {
            self.language = preference;
            cx.emit(ShortcutsEvent::LanguageChanged(preference));
            self.language_restart_confirm = true;
            cx.notify();
        }
    }

    /// Where the data lives, applied by writing the `location` pointer.
    ///
    /// Validated here and refused with a reason: the alternative is a file the
    /// *next* launch rejects, which the user would find out about by the app
    /// not starting. `None` means back to one directory.
    fn set_data_location(&mut self, target: Option<PathBuf>, cx: &mut Context<Self>) {
        let result = match &target {
            Some(target) => data_location::point_at(self.data_roots.config(), target),
            None => {
                data_location::clear(self.data_roots.config()).map_err(|_| LocationError::Directory)
            }
        };
        match result {
            Ok(()) => {
                self.data_location = target.map(|target| target.to_string_lossy().into_owned());
                self.data_location_error = None;
                self.data_location_restart_confirm = true;
            }
            Err(error) => {
                self.data_location_error = Some(
                    crate::l10n::localizer()
                        .format_with(
                            TextKey::SettingsDataLocationRejected,
                            &[("reason", error.reason())],
                        )
                        .into(),
                );
            }
        }
        cx.notify();
    }

    /// Pick the folder the stores should move to.
    fn pick_data_location(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(
                crate::l10n::localizer()
                    .text(TextKey::SettingsDataLocation)
                    .into(),
            ),
        });
        cx.spawn(async move |this, cx| {
            let path = match receiver.await {
                Ok(Ok(Some(mut paths))) => paths.pop(),
                _ => None,
            };
            let Some(path) = path else {
                return;
            };
            let _ = this.update(cx, |page, cx| page.set_data_location(Some(path), cx));
        })
        .detach();
    }

    fn set_composer_send_behavior(
        &mut self,
        behavior: ComposerSendBehavior,
        cx: &mut Context<Self>,
    ) {
        if self.composer_send_behavior != behavior {
            self.composer_send_behavior = behavior;
            self.conflict_notice = None;
            cx.emit(ShortcutsEvent::ComposerSendBehaviorChanged(behavior));
            cx.notify();
        }
    }

    fn commit_appshots(&self, cx: &mut Context<Self>) {
        cx.emit(ShortcutsEvent::AppshotsChanged {
            enabled: self.appshots_enabled,
            sound_enabled: self.appshot_sound_enabled,
            destination: self.appshot_destination,
        });
    }

    fn record_keystroke(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let Some(recording) = self.recording else {
            return;
        };
        let mods = &keystroke.modifiers;
        match record_key(
            &keystroke.key,
            mods.control,
            mods.alt,
            mods.shift,
            mods.platform,
        ) {
            RecordOutcome::Cancelled => {
                self.stop_recording();
                cx.notify();
            }
            RecordOutcome::Ignored => {}
            RecordOutcome::Set(combo) => {
                self.stop_recording();
                self.conflict_notice =
                    refusal(&self.keymap, self.composer_send_behavior, recording, &combo);
                if self.conflict_notice.is_none() {
                    self.keymap.set(recording, combo);
                    self.commit(cx);
                }
                cx.notify();
            }
        }
        cx.stop_propagation();
    }

    /// One shortcut row: label left, Reset when customized, and
    /// the click-to-record combo chip (recording inverts it to
    /// white-on-black). `ix` is the id's position in [`ShortcutId::ALL`]
    /// (unique element ids across the group cards); `gx` is the row's place
    /// in its own card (separator rule).
    fn render_row(
        &self,
        id: ShortcutId,
        ix: usize,
        gx: usize,
        recording: Option<ShortcutId>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        widgets::card_row(theme, gx == 0)
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .flex()
                    .flex_col()
                    .child(widgets::row_title(theme, id.label_text()))
                    .when(
                        matches!(id, ShortcutId::NextSession | ShortcutId::PrevSession),
                        |row| {
                            row.child(widgets::meta_line(
                                theme,
                                vec![
                                    div()
                                        .child(
                                            crate::l10n::localizer()
                                                .text(TextKey::SettingsShortcutsNavigatePane),
                                        )
                                        .into_any_element(),
                                ],
                            ))
                        },
                    ),
            )
            .child(self.render_binding_control(id, ix, recording, theme, cx))
    }

    fn render_binding_control(
        &self,
        id: ShortcutId,
        ix: usize,
        recording: Option<ShortcutId>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let page = cx.entity().downgrade();
        let reset_page = page.clone();
        binding_control(
            id,
            ix,
            self.keymap.get(id),
            recording == Some(id),
            theme,
            move |_, cx| {
                reset_page
                    .update(cx, |this, cx| {
                        this.keymap.reset(id);
                        this.stop_recording();
                        this.commit(cx);
                    })
                    .ok();
            },
            move |window, cx| {
                page.update(cx, |this, cx| this.start_recording(id, window, cx))
                    .ok();
            },
        )
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    /// The confirmation a language change raises.
    ///
    /// The process fixes its display language at startup, so a change here
    /// cannot repaint the windows or the native menu that already exist —
    /// without this the user changes a setting and sees nothing happen.
    /// Confirming asks the shell to start a fresh process; cancelling keeps
    /// the change (it is already persisted) and leaves the restart to them.
    fn render_language_restart_dialog(
        &self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.language_restart_confirm {
            return None;
        }
        Some(Self::restart_dialog(
            "language-restart",
            TextKey::SettingsLanguageRestartTitle,
            TextKey::SettingsLanguageRestartBody,
            TextKey::SettingsLanguageRestartAction,
            |page, _| page.language_restart_confirm = false,
            viewport,
            cx,
        ))
    }

    /// The same offer for a data location the next launch will read.
    fn render_data_location_restart_dialog(
        &self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.data_location_restart_confirm {
            return None;
        }
        Some(Self::restart_dialog(
            "data-location-restart",
            TextKey::SettingsDataLocationRestartTitle,
            TextKey::SettingsDataLocationRestartBody,
            TextKey::SettingsDataLocationRestartAction,
            |page, _| page.data_location_restart_confirm = false,
            viewport,
            cx,
        ))
    }

    /// Settings → General → Data location.
    ///
    /// The path shown is where the **stores** are, which is the config root
    /// until a pointer moves them. The row names the config folder on the
    /// default and says the keys stay there in both cases, because "which
    /// folder holds my conversations" is the question a user has and "which
    /// holds my API key" is the one they will have next.
    fn render_data_location_row(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let localizer = crate::l10n::localizer();
        let custom = self.data_location.is_some();
        let detail = if custom {
            localizer.text(TextKey::SettingsDataLocationDetailCustom)
        } else {
            localizer.text(TextKey::SettingsDataLocationDetailDefault)
        };
        let path = self
            .data_location
            .clone()
            .unwrap_or_else(|| self.data_roots.config().display().to_string());
        // "Default" or "custom", beside the path the stores are under.
        let badge_label: String = if custom {
            localizer.text(TextKey::SettingsDataLocation)
        } else {
            localizer.text(TextKey::SettingsDataLocationDefault)
        };

        let badge = div().flex_none().child(widgets::meta_line(
            theme,
            vec![div().child(badge_label).into_any_element()],
        ));

        let change = popover::btn_ghost(
            theme,
            &localizer.text(TextKey::SettingsDataLocationChange),
            "data-location-change",
        )
        .id("data-location-change")
        .debug_selector(|| "data-location-change".into())
        .role(gpui::Role::Button)
        .tab_index(0)
        .on_click(cx.listener(|this, _, _, cx| this.pick_data_location(cx)));

        let mut control = div().flex().flex_row().items_center().gap(px(8.0));
        if custom {
            control = control.child(
                popover::btn_ghost(
                    theme,
                    &localizer.text(TextKey::SettingsDataLocationUseDefault),
                    "data-location-default",
                )
                .id("data-location-default")
                .role(gpui::Role::Button)
                .tab_index(0)
                .on_click(cx.listener(|this, _, _, cx| this.set_data_location(None, cx))),
            );
        }
        let control = control.child(change);

        let mut meta = vec![
            div().child(path.clone()).into_any_element(),
            div().child(detail).into_any_element(),
        ];
        if let Some(error) = self.data_location_error.clone() {
            meta.push(div().child(error).into_any_element());
        }

        widgets::card_row(theme, false)
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .child(widgets::row_title(
                        theme,
                        localizer.text(TextKey::SettingsDataLocation),
                    ))
                    .child(widgets::meta_line(theme, meta)),
            )
            .child(badge)
            .child(control)
            .into_any_element()
    }

    /// One restart confirmation. Both switches this page offers — the display
    /// language and the data location — are read at startup, so both end in
    /// the same offer, and the shape lives here once.
    fn restart_dialog(
        id: &'static str,
        title: TextKey,
        body: TextKey,
        action: TextKey,
        dismiss: fn(&mut Self, &mut Context<Self>),
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).for_popup();
        let localizer = crate::l10n::localizer();
        let card = popover::dialog_card(&theme)
            .id(id)
            .role(gpui::Role::Dialog)
            .aria_label(localizer.text(title))
            .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    dismiss(this, cx);
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(popover::dialog_title(&theme, &localizer.text(title)))
            .child(
                div()
                    .mt(px(6.0))
                    .child(popover::dialog_body(&theme, localizer.text(body))),
            )
            .child(
                div()
                    .mt(px(16.0))
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        popover::btn_ghost(&theme, &localizer.text(TextKey::ActionCancel), id)
                            .id(SharedString::from(format!("{id}-cancel")))
                            .role(gpui::Role::Button)
                            .tab_index(0)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                dismiss(this, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        popover::btn_primary(&theme, &localizer.text(action))
                            .id(SharedString::from(format!("{id}-confirm")))
                            .role(gpui::Role::Button)
                            .tab_index(0)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                dismiss(this, cx);
                                cx.emit(ShortcutsEvent::RestartRequested);
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element();
        popover::modal(id, viewport, card)
    }

    // Kept for the appshots half of this page (appshots.rs), whose host still
    // carries the drag-move listener itself.
    fn on_bar_drag_move(
        &mut self,
        event: &gpui::DragMoveEvent<popover::MenuScrollbarDrag>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.rail_drag_to(event.event.position.y) {
            cx.notify();
        }
    }

    /// The shared rail under the id of whichever page is showing. Kept as a
    /// method because the appshots half of this page renders through it.
    fn render_scrollbar(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let id = if self.appshots_page {
            "appshots-settings-page-scrollbar"
        } else if self.general_page {
            "general-settings-page-scrollbar"
        } else {
            "shortcuts-page-scrollbar"
        };
        popover::rail(self, id, theme, cx)
    }
}

impl popover::ScrollRailHost for ShortcutsPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

/// Reset (when customized) plus the click-to-record combo chip; recording
/// inverts it to the accent wash. Shared by Settings → Shortcuts and the
/// feature pages that own a shortcut.
fn binding_control(
    id: ShortcutId,
    ix: usize,
    combo: &str,
    is_recording: bool,
    theme: &Theme,
    on_reset: impl Fn(&mut Window, &mut gpui::App) + 'static,
    on_record: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> gpui::Div {
    let accent = theme.accent;
    let localizer = crate::l10n::localizer();
    let combo = combo.to_string();
    let non_default = combo != id.default_combo();
    let chip_text: SharedString = if is_recording {
        localizer.text(TextKey::SettingsShortcutsPressKeys).into()
    } else {
        display_combo(&combo).into()
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(20.0))
        .when(non_default && !is_recording, |el| {
            el.child(
                div()
                    .id(("shortcut-reset", ix))
                    .role(gpui::Role::Button)
                    .aria_label(localizer.format_with(
                        TextKey::SettingsShortcutsResetAria,
                        &[("name", id.label_text().as_str())],
                    ))
                    .min_h(px(24.0))
                    .tab_index(0)
                    .focus_visible(move |style| style.border_2().border_color(accent))
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(theme.text_muted)
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme.text))
                    .on_click(move |_, window, cx| on_reset(window, cx))
                    .child(localizer.text(TextKey::ActionReset)),
            )
        })
        .child(
            div()
                .id(("shortcut-combo", ix))
                .role(gpui::Role::Button)
                .aria_label(localizer.format_with(
                    TextKey::SettingsShortcutsChangeAria,
                    &[
                        ("name", id.label_text().as_str()),
                        ("combo", &display_combo(&combo)),
                    ],
                ))
                .tab_index(0)
                .min_w(px(96.0))
                .h(px(widgets::SELECT_HEIGHT))
                .px(px(12.0))
                .rounded(px(8.0))
                // Same glass wash as the settings dropdowns; the
                // transparent edge is held for the focus ring.
                .border_1()
                .border_color(gpui::transparent_black())
                .focus_visible(move |style| style.border_color(accent))
                .flex()
                .items_center()
                .justify_center()
                .font_family(theme.font_mono.clone())
                .text_size(crate::typography::ui_rems(12.0))
                .cursor_pointer()
                .map(|el| {
                    if is_recording {
                        el.bg(accent.opacity(0.16))
                            .border_color(accent.opacity(0.55))
                            .text_color(theme.text)
                    } else {
                        let hover_key = format!("shortcut-combo-{ix}-hover");
                        el.bg(crate::motion::hover_blend(
                            &hover_key,
                            widgets::select_fill(theme, false),
                            widgets::select_fill(theme, true),
                        ))
                        .on_hover(crate::motion::hover_listener(hover_key))
                        .text_color(theme.text)
                    }
                })
                .on_click(move |_, window, cx| on_record(window, cx))
                .child(chip_text),
        )
}

/// Why `combo` cannot be bound to `id`, if it cannot. Conflicts never
/// persist; they're refused at record time, naming the owner (the original app
/// settings.shortcuts.tsx: "… is already assigned to …"). Pure.
fn refusal(
    keymap: &KeymapConfig,
    behavior: ComposerSendBehavior,
    id: ShortcutId,
    combo: &str,
) -> Option<SharedString> {
    let localizer = crate::l10n::localizer();
    if id == ShortcutId::CaptureAppshot && crate::appshots::validate_shortcut(combo).is_err() {
        let modifiers = localizer.text(if cfg!(target_os = "macos") {
            TextKey::SettingsShortcutsModifiersPlatform
        } else {
            TextKey::SettingsShortcutsModifiersOther
        });
        return Some(
            localizer
                .format_with(
                    TextKey::SettingsShortcutsRefusedKey,
                    &[("modifiers", modifiers.as_str())],
                )
                .into(),
        );
    }
    if send_combo_is_reserved(behavior, combo) {
        return Some(
            localizer
                .format_with(
                    TextKey::SettingsShortcutsRefusedReserved,
                    &[("combo", display_combo(combo).as_str())],
                )
                .into(),
        );
    }
    conflict_owner(keymap, id, combo).map(|owner| {
        let combo = display_combo(combo);
        // Name the page for shortcuts that live on their feature's page.
        let taken = if group(owner) == "Appshots" {
            let page = group_label("Appshots");
            localizer.format_with(
                TextKey::SettingsShortcutsRefusedTakenPage,
                &[
                    ("combo", combo.as_str()),
                    ("owner", owner.label_text().as_str()),
                    ("page", page.as_str()),
                ],
            )
        } else {
            localizer.format_with(
                TextKey::SettingsShortcutsRefusedTaken,
                &[
                    ("combo", combo.as_str()),
                    ("owner", owner.label_text().as_str()),
                ],
            )
        };
        taken.into()
    })
}

/// The shortcut (other than `id`) already bound to `combo`, if any. Pure.
pub fn conflict_owner(keymap: &KeymapConfig, id: ShortcutId, combo: &str) -> Option<ShortcutId> {
    ShortcutId::ALL
        .into_iter()
        .find(|&other| other.available() && other != id && keymap.get(other) == combo)
}

pub fn send_combo_is_reserved(_behavior: ComposerSendBehavior, combo: &str) -> bool {
    combo == "mod-enter"
}

pub fn modifier_send_label(is_macos: bool) -> &'static str {
    if is_macos { "⌘ Enter" } else { "Ctrl Enter" }
}

/// The page's sections, in display order. [`group`] is a total match, so every
/// [`ShortcutId::ALL`] entry lands in exactly one — a shortcut added later
/// extends the match and appears on the page by construction
/// (`every_shortcut_lands_in_a_rendered_group` holds the other half: its group
/// name must be listed here).
const GROUP_ORDER: [&str; 8] = [
    "Appearance",
    "Files",
    "Browser",
    "Panels",
    "Sessions",
    "Projects",
    "Jump to session",
    "Appshots",
];

/// What a group's heading reads.
///
/// The name [`group`] returns is a stable key that `GROUP_ORDER` lists and the
/// tests compare; this is only the heading it renders as, so translating it
/// cannot move a row or empty a card.
fn group_label(name: &str) -> String {
    let localizer = crate::l10n::localizer();
    localizer.text(match name {
        "Appearance" => TextKey::SettingsShortcutsGroupAppearance,
        "Files" => TextKey::SettingsShortcutsGroupFiles,
        "Browser" => TextKey::SettingsShortcutsGroupBrowser,
        "Panels" => TextKey::SettingsShortcutsGroupPanels,
        "Sessions" => TextKey::SettingsShortcutsGroupSessions,
        "Projects" => TextKey::SettingsShortcutsGroupProjects,
        "Jump to session" => TextKey::SettingsShortcutsGroupJump,
        "Appshots" => TextKey::SettingsShortcutsGroupAppshots,
        _ => return name.to_owned(),
    })
}

/// The section a shortcut's row renders under.
fn group(id: ShortcutId) -> &'static str {
    match id {
        ShortcutId::RandomWallpaper => "Appearance",
        ShortcutId::CaptureAppshot => "Appshots",
        ShortcutId::SaveFile => "Files",
        ShortcutId::BrowserReload => "Browser",
        ShortcutId::ToggleSidebar
        | ShortcutId::ToggleChanges
        | ShortcutId::ToggleFiles
        | ShortcutId::ToggleTerminal => "Panels",
        ShortcutId::NewProject => "Projects",
        ShortcutId::OpenModelPicker
        | ShortcutId::NewSession
        | ShortcutId::NextSession
        | ShortcutId::PrevSession
        | ShortcutId::ArchiveSession => "Sessions",
        ShortcutId::JumpSession(_) => "Jump to session",
    }
}

impl Render for ShortcutsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Feature pages (Appshots) rebind their shortcut through the store. Read
        // it back so a later commit here never republishes an older keymap.
        if self.recording.is_none()
            && let Some(store) = cx.try_global::<crate::settings::SettingsStore>()
        {
            self.keymap.clone_from(&store.current.keymap);
        }
        if self.appshots_page {
            if std::mem::take(&mut self.appshots_focus_pending) {
                window.focus(&self.focus, cx);
            }
            return self.render_appshots(window, cx);
        }
        let theme = Theme::of(cx).for_settings_surface();
        let localizer = crate::l10n::localizer();
        let recording = self.recording;
        let escape_stops_active_agent = self.escape_stops_active_agent;
        let send_behavior = self.composer_send_behavior;
        let compact_mode = crate::settings::transcript_compact_mode(cx);
        let customized = self.keymap != KeymapConfig::default()
            || escape_stops_active_agent
            || send_behavior != ComposerSendBehavior::default();
        let modifier_label = modifier_send_label(cfg!(target_os = "macos"));

        let send_behaviors = [
            (ComposerSendBehavior::Enter, "Enter"),
            (ComposerSendBehavior::ModEnter, modifier_label),
        ];
        let send_behavior_control = widgets::select(
            "composer-send-behavior",
            localizer.text(TextKey::SettingsShortcutsSendWith),
            &theme,
            |page: &mut Self| &mut page.send_select,
        )
        .options(
            send_behaviors
                .iter()
                .map(|(_, label)| widgets::SelectOption::new(*label)),
            send_behaviors
                .iter()
                .position(|(behavior, _)| *behavior == send_behavior)
                .unwrap_or_default(),
        )
        .width(128.0)
        .font_family(theme.font_mono.clone())
        .on_select(move |page, ix, _, cx| page.set_composer_send_behavior(send_behaviors[ix].0, cx))
        .render(&self.send_select, cx);

        let languages = rcode_presentation::LanguagePreference::ALL;
        let language_labels: Vec<String> = languages
            .iter()
            .map(|preference| {
                use rcode_presentation::TextKey;
                localizer.text(match preference {
                    rcode_presentation::LanguagePreference::System => {
                        TextKey::SettingsLanguageSystem
                    }
                    rcode_presentation::LanguagePreference::EnUs => TextKey::SettingsLanguageEn,
                    rcode_presentation::LanguagePreference::ZhCn => TextKey::SettingsLanguageZh,
                })
            })
            .collect();
        let language_control = widgets::select(
            "settings-language",
            localizer.text(TextKey::SettingsLanguage),
            &theme,
            |page: &mut Self| &mut page.language_select,
        )
        .options(
            language_labels
                .iter()
                .map(|label| widgets::SelectOption::new(label)),
            languages
                .iter()
                .position(|candidate| *candidate == self.language)
                .unwrap_or_default(),
        )
        .width(160.0)
        .on_select(move |page, ix, _, cx| page.set_language(languages[ix], cx))
        .render(&self.language_select, cx);

        let language_row = widgets::card_row(&theme, true)
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .child(widgets::row_title(
                        &theme,
                        localizer.text(rcode_presentation::TextKey::SettingsLanguage),
                    ))
                    // Stated up front rather than only after a change: the
                    // choice is written now and read on the next launch, and a
                    // user who sees no effect will otherwise assume it failed.
                    .child(widgets::meta_line(
                        &theme,
                        vec![
                            div()
                                .child(
                                    localizer
                                        .text(rcode_presentation::TextKey::SettingsLanguageRestart),
                                )
                                .into_any_element(),
                        ],
                    )),
            )
            .child(language_control);

        let send_behavior_row = widgets::card_row(&theme, true)
            .child(div().flex_1().min_w(px(160.0)).child(widgets::row_title(
                &theme,
                localizer.text(TextKey::SettingsShortcutsSendWith),
            )))
            .child(send_behavior_control);
        let compact_mode_row = widgets::card_row(&theme, false)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(widgets::row_title(
                        &theme,
                        localizer.text(TextKey::SettingsShortcutsCompactMode),
                    ))
                    .child(widgets::meta_line(
                        &theme,
                        vec![
                            div()
                                .child(localizer.text(TextKey::SettingsShortcutsCompactModeDetail))
                                .into_any_element(),
                        ],
                    )),
            )
            .child(
                widgets::toggle_switch(&theme, compact_mode, "transcript-compact-mode")
                    .id("transcript-compact-mode-toggle")
                    .tab_index(0)
                    .role(gpui::Role::Switch)
                    .aria_label(localizer.text(TextKey::SettingsShortcutsCompactMode))
                    .aria_toggled(if compact_mode {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .focus_visible(|s| s.border_2().border_color(theme.accent))
                    .cursor_pointer()
                    .on_click(cx.listener(move |_, _, _, cx| {
                        crate::settings::set_transcript_compact_mode(!compact_mode, cx);
                        cx.notify();
                    })),
            );
        let compact_model_picker = crate::settings::compact_model_picker(cx);
        let compact_model_picker_row = widgets::card_row(&theme, false)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(widgets::row_title(
                        &theme,
                        localizer.text(TextKey::SettingsShortcutsCompactPicker),
                    ))
                    .child(widgets::meta_line(
                        &theme,
                        vec![
                            div()
                                .child(
                                    localizer.text(TextKey::SettingsShortcutsCompactPickerDetail),
                                )
                                .into_any_element(),
                        ],
                    )),
            )
            .child(
                widgets::toggle_switch(&theme, compact_model_picker, "compact-model-picker")
                    .id("compact-model-picker-toggle")
                    .tab_index(0)
                    .role(gpui::Role::Switch)
                    .aria_label(localizer.text(TextKey::SettingsShortcutsCompactPicker))
                    .aria_toggled(if compact_model_picker {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .focus_visible(|s| s.border_2().border_color(theme.accent))
                    .cursor_pointer()
                    .on_click(cx.listener(move |_, _, _, cx| {
                        crate::settings::update(
                            crate::settings::SavePolicy::Debounced,
                            cx,
                            |settings| settings.compact_model_picker = !compact_model_picker,
                        );
                        cx.refresh_windows();
                        cx.notify();
                    })),
            );
        let escape_behavior_row = widgets::card_row(&theme, false)
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.0))
                    .child(widgets::row_title(
                        &theme,
                        localizer.text(TextKey::SettingsShortcutsEscape),
                    ))
                    .child(widgets::meta_line(
                        &theme,
                        vec![
                            div()
                                .child(localizer.text(TextKey::SettingsShortcutsEscapeDetail))
                                .into_any_element(),
                        ],
                    )),
            )
            .child(
                widgets::toggle_switch(
                    &theme,
                    escape_stops_active_agent,
                    "escape-stops-active-agent",
                )
                .id("escape-stops-active-agent-toggle")
                .debug_selector(|| "escape-stops-active-agent-toggle".into())
                .tab_index(0)
                .role(gpui::Role::Switch)
                .aria_label(localizer.text(TextKey::SettingsShortcutsEscapeAria))
                .aria_toggled(if escape_stops_active_agent {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .focus_visible(|s| s.border_2().border_color(theme.accent))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_escape_stops_active_agent(!escape_stops_active_agent, cx);
                })),
            );
        if self.general_page {
            let data_location_row = self.render_data_location_row(&theme, cx);
            let pull_request_destination = crate::settings::current(cx).pull_request_destination;
            let pull_request_destination_control = widgets::select(
                "pull-request-destination",
                localizer.text(TextKey::SettingsLinksPullRequests),
                &theme,
                |page: &mut Self| &mut page.pull_request_destination_select,
            )
            .options(
                PullRequestDestination::ALL
                    .into_iter()
                    .map(|destination| widgets::SelectOption::new(destination.label_text())),
                PullRequestDestination::ALL
                    .into_iter()
                    .position(|destination| destination == pull_request_destination)
                    .unwrap_or_default(),
            )
            .width(160.0)
            .on_select(|_, ix, _, cx| {
                crate::settings::update(SavePolicy::Immediate, cx, |settings| {
                    settings.pull_request_destination = PullRequestDestination::ALL[ix];
                });
                cx.refresh_windows();
            })
            .render(&self.pull_request_destination_select, cx);
            let pull_request_destination_row = widgets::card_row(&theme, true)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(160.0))
                        .child(widgets::row_title(
                            &theme,
                            localizer.text(TextKey::SettingsLinksPullRequests),
                        ))
                        .child(widgets::meta_line(
                            &theme,
                            vec![
                                div()
                                    .child(localizer.text(TextKey::SettingsLinksPullRequestsDetail))
                                    .into_any_element(),
                            ],
                        )),
                )
                .child(pull_request_destination_control);
            let scrollbar = self.render_scrollbar(&theme, cx);
            return div()
                .id("general-settings-page-host")
                .relative()
                .size_full()
                .on_hover(cx.listener(Self::on_scroll_hovered))
                .child(
                    crate::edge_fade::edge_faded(
                        16.0,
                        true,
                        true,
                        div()
                            .id("general-settings-page")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll.scroll)
                            .child(
                                widgets::page_column()
                                    .child(widgets::page_header(
                                        &theme,
                                        &localizer.text(TextKey::SettingsGeneralTitle),
                                        None,
                                    ))
                                    .child(
                                        widgets::section_card(&theme)
                                            .child(language_row)
                                            .child(send_behavior_row)
                                            .child(compact_mode_row)
                                            .child(compact_model_picker_row)
                                            .child(escape_behavior_row),
                                    )
                                    .child(widgets::section(
                                        &theme,
                                        localizer.text(TextKey::SettingsDataLocation),
                                        widgets::section_card(&theme)
                                            .mt_0()
                                            .child(data_location_row),
                                    ))
                                    .child(widgets::section(
                                        &theme,
                                        localizer.text(TextKey::SettingsLinks),
                                        widgets::section_card(&theme)
                                            .mt_0()
                                            .child(pull_request_destination_row),
                                    ))
                                    .child(self.thread_naming.clone())
                                    .child(self.worktrees.clone()),
                            ),
                    )
                    .fade_overflow_y(&self.scroll.scroll),
                )
                .children(scrollbar)
                .children(self.render_language_restart_dialog(window.viewport_size(), cx))
                .children(self.render_data_location_restart_dialog(window.viewport_size(), cx))
                .into_any_element();
        }
        // One block per group, each under its small section label — the flat
        // 16-row table read as one undifferentiated wall. `ix` (the id's
        // position in ALL) keys the interactive elements, so ids stay unique
        // across blocks. The section wrapper owns the spacing, so the block's
        // own top margin is zeroed.
        let mut groups: Vec<gpui::AnyElement> = Vec::new();
        for name in GROUP_ORDER {
            // Feature pages own these: Settings → Appshots.
            if name == "Appshots" {
                continue;
            }
            let mut card = widgets::section_card(&theme).mt_0();
            let ids = ShortcutId::ALL.into_iter().filter(|&id| group(id) == name);
            for (gx, id) in ids.enumerate() {
                let ix = ShortcutId::ALL.iter().position(|&a| a == id).unwrap_or(0);
                card = card.child(self.render_row(id, ix, gx, recording, &theme, cx));
            }
            groups.push(widgets::section(&theme, group_label(name), card).into_any_element());
        }

        // Helper line stays in the muted tone even for a rejected conflict —
        // the message names the specific clash (the original app settings.shortcuts.tsx).
        let helper: SharedString = if recording.is_some() {
            localizer.text(TextKey::SettingsShortcutsPressEscape).into()
        } else if let Some(notice) = self.conflict_notice.clone() {
            notice
        } else {
            localizer.text(TextKey::SettingsShortcutsUnique).into()
        };

        let scrollbar = self.render_scrollbar(&theme, cx);
        div()
            .id("shortcuts-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("shortcuts-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .track_focus(&self.focus)
                        .child(
                            widgets::page_column()
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_start()
                                        .flex_wrap()
                                        .justify_between()
                                        .gap(px(24.0))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w(px(200.0))
                                                .flex()
                                                .flex_col()
                                                .child(widgets::page_header(
                                                    &theme,
                                                    &localizer
                                                        .text(TextKey::SettingsShortcutsTitle),
                                                    None,
                                                ))
                                                .child(
                                                    widgets::page_subtitle(
                                                        &theme,
                                                        localizer.text(
                                                            TextKey::SettingsShortcutsSubtitle,
                                                        ),
                                                    )
                                                    .max_w(px(512.0))
                                                    .line_height(px(20.0)),
                                                ),
                                        )
                                        .child({
                                            // `disabled:opacity-35` when nothing is
                                            // customized or while recording.
                                            let disabled = !customized || recording.is_some();
                                            widgets::ghost_action(&theme)
                                                .id("shortcuts-restore-defaults")
                                                .flex_none()
                                                .when(disabled, |el| el.opacity(0.35))
                                                .when(!disabled, |el| {
                                                    el.on_click(cx.listener(|this, _, _, cx| {
                                                        this.keymap = KeymapConfig::default();
                                                        this.stop_recording();
                                                        this.conflict_notice = None;
                                                        this.commit(cx);
                                                        this.set_escape_stops_active_agent(
                                                            false, cx,
                                                        );
                                                        this.set_composer_send_behavior(
                                                            ComposerSendBehavior::Enter,
                                                            cx,
                                                        );
                                                    }))
                                                })
                                                .child(
                                                    crate::icons::icon(crate::icons::RESTART)
                                                        .size(px(14.0))
                                                        .text_color(theme.text_muted),
                                                )
                                                .child(
                                                    localizer
                                                        .text(TextKey::SettingsShortcutsRestore),
                                                )
                                        }),
                                )
                                .child(div().flex().flex_col().children(groups))
                                .child(
                                    div()
                                        .mt(px(12.0))
                                        .px(px(4.0))
                                        .min_h(px(20.0))
                                        .flex()
                                        .justify_center()
                                        .text_size(crate::typography::ui_rems(12.0))
                                        .text_color(theme.text_muted)
                                        .child(helper),
                                ),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The settings row is the writer for a file the *next* launch reads, so
    /// the two cases that matter are that a pick lands in that file and that a
    /// pick the next launch would reject never gets there.
    #[gpui::test]
    fn a_data_location_pick_is_validated_before_it_is_written(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
        });
        let config = tempfile::tempdir().expect("config root");
        let target = tempfile::tempdir().expect("target");
        let missing = config.path().join("not-there");
        let (page, cx) = cx.add_window_view(|_, cx| {
            let state = cx.new(|_| AppState::new());
            ShortcutsPage::new(
                state,
                KeymapConfig::default(),
                false,
                ComposerSendBehavior::default(),
                false,
                false,
                AppshotDestination::Automatic,
                rcode_presentation::LanguagePreference::default(),
                DataRoots::same(config.path()),
                cx,
            )
        });

        page.update(cx, |page, _| page.show_section(false, true));
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(
            cx.debug_bounds("data-location-change").is_some(),
            "the General page draws the data-location row"
        );

        page.update(cx, |page, cx| page.set_data_location(Some(missing), cx));
        page.update(cx, |page, _| {
            assert!(
                page.data_location_error.is_some(),
                "a missing folder is refused with a reason"
            );
            assert!(
                !page.data_location_restart_confirm,
                "nothing to restart for"
            );
            assert_eq!(page.data_location, None);
        });
        assert_eq!(
            data_location::read(config.path()),
            None,
            "no file was written"
        );

        page.update(cx, |page, cx| {
            page.set_data_location(Some(target.path().to_path_buf()), cx)
        });
        page.update(cx, |page, _| {
            assert!(page.data_location_error.is_none());
            assert!(
                page.data_location_restart_confirm,
                "the change needs a restart"
            );
            assert_eq!(
                page.data_location.as_deref(),
                Some(target.path().to_string_lossy().as_ref())
            );
        });
        assert_eq!(
            data_location::read(config.path()).as_deref(),
            Some(target.path().to_string_lossy().as_ref()),
            "the pointer is what the next launch reads"
        );
    }

    #[gpui::test]
    fn conversation_controls_work_after_moving_out_of_shortcuts(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
        });
        let (page, cx) = cx.add_window_view(|_, cx| {
            let state = cx.new(|_| AppState::new());
            let mut page = ShortcutsPage::new(
                state,
                KeymapConfig::default(),
                false,
                ComposerSendBehavior::Enter,
                false,
                false,
                AppshotDestination::Automatic,
                rcode_presentation::LanguagePreference::default(),
                DataRoots::same("/tmp/rcode-settings-test"),
                cx,
            );
            page.show_section(false, true);
            page
        });
        cx.update(|window, cx| window.draw(cx).clear());
        let send = cx.debug_bounds("composer-send-behavior").unwrap();
        cx.simulate_click(send.center(), gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear());
        let option = cx.debug_bounds("composer-send-behavior-option-1").unwrap();
        cx.simulate_click(option.center(), gpui::Modifiers::default());
        page.update(cx, |page, _| assert!(!page.send_select.is_open()));
        // Let the menu's exit animation finish before clicking beneath it:
        // the reap compares wall-clock instants, so real time must pass too.
        std::thread::sleep(
            crate::motion::MENU_OUT
                .total()
                .mul_f32(crate::motion::speed_scale()),
        );
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("composer-send-behavior-option-1").is_none());
        let escape = cx.debug_bounds("escape-stops-active-agent-toggle").unwrap();
        cx.simulate_click(escape.center(), gpui::Modifiers::default());
        page.update(cx, |page, _| {
            assert_eq!(page.composer_send_behavior, ComposerSendBehavior::ModEnter);
            assert!(page.escape_stops_active_agent);
            page.show_section(false, false);
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("composer-send-behavior").is_none());
        assert!(
            cx.debug_bounds("escape-stops-active-agent-toggle")
                .is_none()
        );
    }

    #[gpui::test]
    fn appshots_setup_can_be_enabled_and_configured_by_keyboard(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
        });
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            let mut page = ShortcutsPage::new(
                state,
                KeymapConfig::default(),
                false,
                ComposerSendBehavior::default(),
                false,
                false,
                AppshotDestination::Automatic,
                rcode_presentation::LanguagePreference::default(),
                DataRoots::same("/tmp/rcode-settings-test"),
                cx,
            );
            page.show_appshots(true);
            page
        });
        window
            .update(cx, |page, w, cx| w.focus(&page.focus, cx))
            .unwrap();
        cx.update_window(window.into(), |_, w, cx| {
            w.draw(cx).clear();
        })
        .unwrap();
        let press = |cx: &mut gpui::TestAppContext, key: &str| {
            cx.update_window(window.into(), |_, w, cx| {
                w.draw(cx).clear();
                let keystroke = gpui::Keystroke::parse(key).unwrap();
                w.dispatch_event(
                    gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                        keystroke: keystroke.clone(),
                        is_held: false,
                        prefer_character_input: false,
                    }),
                    cx,
                );
                w.dispatch_event(
                    gpui::PlatformInput::KeyUp(gpui::KeyUpEvent { keystroke }),
                    cx,
                );
            })
            .unwrap();
        };
        press(cx, "tab");
        press(cx, "space");
        window
            .update(cx, |page, _, _| assert!(page.appshots_enabled))
            .unwrap();
        press(cx, "tab");
        press(cx, "enter");
        window
            .update(cx, |page, _, _| assert!(page.appshot_sound_enabled))
            .unwrap();
        // Skip the shortcut trigger; arrows open the destination dropdown on
        // the current choice and Enter commits the highlighted one.
        for key in ["tab", "tab", "down", "down", "enter"] {
            press(cx, key);
        }
        window
            .update(cx, |page, _, _| {
                assert_eq!(page.appshot_destination, AppshotDestination::LastSession);
                assert!(!page.destination_select.is_open());
            })
            .unwrap();
        for key in ["space", "down", "enter"] {
            press(cx, key);
        }
        window
            .update(cx, |page, _, _| {
                assert_eq!(page.appshot_destination, AppshotDestination::NewSession)
            })
            .unwrap();
        // Escape closes without choosing.
        for key in ["up", "up", "escape"] {
            press(cx, key);
        }
        window
            .update(cx, |page, _, _| {
                assert_eq!(page.appshot_destination, AppshotDestination::NewSession);
                assert!(!page.destination_select.is_open());
            })
            .unwrap();
        for key in ["up", "up", "enter"] {
            press(cx, key);
        }
        window
            .update(cx, |page, _, _| {
                assert_eq!(page.appshot_destination, AppshotDestination::LastSession)
            })
            .unwrap();
    }

    #[gpui::test]
    fn recorder_refuses_bound_actions_before_they_can_run(cx: &mut gpui::TestAppContext) {
        use std::{cell::Cell, rc::Rc};
        let fired = Rc::new(Cell::new(false));
        let observed = fired.clone();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            cx.bind_keys([gpui::KeyBinding::new(
                &crate::settings::platform_combo("mod-n"),
                crate::shell::NewSession,
                None,
            )]);
            cx.on_action(move |_: &crate::shell::NewSession, _| observed.set(true));
        });
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            ShortcutsPage::new(
                state,
                KeymapConfig::default(),
                false,
                ComposerSendBehavior::default(),
                false,
                true,
                AppshotDestination::Automatic,
                rcode_presentation::LanguagePreference::default(),
                DataRoots::same("/tmp/rcode-settings-test"),
                cx,
            )
        });
        window
            .update(cx, |page, window, cx| {
                page.start_recording(ShortcutId::CaptureAppshot, window, cx)
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), &crate::settings::platform_combo("mod-n"));
        window
            .update(cx, |page, _, _| {
                assert!(!fired.get(), "the existing action ran while recording");
                assert!(
                    page.conflict_notice
                        .as_deref()
                        .unwrap()
                        .contains(&ShortcutId::NewSession.label_text())
                );
                assert_eq!(
                    page.keymap.capture_appshot,
                    ShortcutId::CaptureAppshot.default_combo()
                );
                assert!(page.recording.is_none());
                assert!(page.recording_interceptor.is_none());
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), &crate::settings::platform_combo("mod-n"));
        assert!(
            fired.get(),
            "finishing recording must restore normal actions"
        );
    }

    #[test]
    fn recording_outcomes() {
        assert_eq!(
            record_key("escape", false, false, false, false),
            RecordOutcome::Cancelled
        );
        assert_eq!(
            record_key("Escape", true, false, false, false),
            RecordOutcome::Cancelled
        );
        assert_eq!(
            record_key("s", false, false, false, true),
            RecordOutcome::Set("mod-s".into())
        );
        assert_eq!(
            record_key("k", false, true, true, true),
            RecordOutcome::Set("mod-alt-shift-k".into())
        );
        // macOS-only: elsewhere ctrl IS the primary and records as "mod".
        #[cfg(target_os = "macos")]
        assert_eq!(
            record_key("tab", true, false, true, false),
            RecordOutcome::Set("ctrl-shift-tab".into())
        );
        // Bare modifiers stay recording.
        assert_eq!(
            record_key("shift", false, false, true, false),
            RecordOutcome::Ignored
        );
        assert_eq!(
            record_key("ctrl", true, false, false, false),
            RecordOutcome::Ignored
        );
    }

    #[test]
    fn every_shortcut_lands_in_a_rendered_group() {
        // The page renders GROUP_ORDER's cards and nothing else — a group()
        // arm returning a name missing from GROUP_ORDER would silently drop
        // its rows from Settings.
        for id in ShortcutId::ALL {
            assert!(
                GROUP_ORDER.contains(&group(id)),
                "{:?} is grouped under {:?}, which GROUP_ORDER does not render",
                id,
                group(id)
            );
        }
        // And every named group has at least one row — no empty cards.
        for name in GROUP_ORDER {
            assert!(
                ShortcutId::ALL.into_iter().any(|id| group(id) == name),
                "group {:?} would render an empty card",
                name
            );
        }
    }

    #[test]
    fn conflicting_records_are_refused() {
        // The original app parity: a combo bound elsewhere is refused at record time (the
        // helper names the owner) — conflicts never persist into the keymap.
        let keymap = KeymapConfig::default();
        let RecordOutcome::Set(combo) = record_key("r", false, false, false, true) else {
            panic!("expected Set");
        };
        assert_eq!(
            conflict_owner(&keymap, ShortcutId::ToggleSidebar, &combo),
            Some(ShortcutId::ToggleChanges)
        );
        // Re-recording a shortcut's own combo is not a conflict.
        assert_eq!(
            conflict_owner(&keymap, ShortcutId::ToggleChanges, &combo),
            None
        );
        // A free combo conflicts with nothing.
        assert_eq!(
            conflict_owner(&keymap, ShortcutId::ToggleSidebar, "mod-shift-x"),
            None
        );
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn appshot_binding_participates_in_existing_conflict_checks() {
        let mut keymap = KeymapConfig::default();
        assert_eq!(
            conflict_owner(&keymap, ShortcutId::CaptureAppshot, "mod-n"),
            Some(ShortcutId::NewSession)
        );
        keymap.set(ShortcutId::CaptureAppshot, "mod-alt-k".into());
        assert_eq!(
            conflict_owner(&keymap, ShortcutId::NewSession, "mod-alt-k"),
            Some(ShortcutId::CaptureAppshot)
        );
        assert_eq!(
            conflict_owner(&keymap, ShortcutId::CaptureAppshot, "mod-alt-k"),
            None
        );
    }

    #[test]
    fn modifier_send_labels_are_platform_specific() {
        assert_eq!(modifier_send_label(true), "⌘ Enter");
        assert_eq!(modifier_send_label(false), "Ctrl Enter");
    }

    #[test]
    fn modifier_send_is_always_reserved_for_the_composer() {
        assert!(send_combo_is_reserved(
            ComposerSendBehavior::Enter,
            "mod-enter"
        ));
        assert!(send_combo_is_reserved(
            ComposerSendBehavior::ModEnter,
            "mod-enter"
        ));
        assert!(!send_combo_is_reserved(
            ComposerSendBehavior::ModEnter,
            "mod-shift-enter"
        ));
    }
}
