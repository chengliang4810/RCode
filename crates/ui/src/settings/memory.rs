//! Settings → Personalization → Memory: whether the agent remembers across sessions.
//!
//! Off omits automatic-memory guidance and the project's MEMORY.md index.
//! Memory is maintained with ordinary file tools under the trust-all policy.
//!
//! The switch is a plain on/off, and its effect is the *next* message: the
//! harness re-reads `agent-settings.json` at the start of every run, so a
//! disabled memory leaves the prompt without a restart.

use gpui::{Context, Entity, SharedString, Subscription, Task, div, prelude::*};
use rcode_harness::settings::AgentSettings;
use rcode_rpc::methods;

use rcode_presentation::TextKey;

use crate::icons;
use crate::popover::Loadable;
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

pub struct MemoryCard {
    state: Entity<AppState>,
    settings: Loadable<AgentSettings>,
    error: Option<String>,
    task: Option<Task<()>>,
    _state: Subscription,
}

impl MemoryCard {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        // Settings can open before the engine connects: load once it does.
        let state_sub = cx.observe(&state, |this: &mut Self, _, cx| {
            if matches!(this.settings, Loadable::Idle) {
                this.call(None, cx);
            }
        });
        let mut card = Self {
            state,
            settings: Loadable::Idle,
            error: None,
            task: None,
            _state: state_sub,
        };
        card.call(None, cx);
        card
    }

    /// `GetAgentSettings`, or `SetAgentSettings` when saving; either reply is
    /// the device's authoritative settings.
    fn call(&mut self, save: Option<AgentSettings>, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let (method, params) = match save {
            Some(settings) => (
                methods::SET_AGENT_SETTINGS,
                serde_json::to_value(settings).unwrap_or_default(),
            ),
            None => (methods::GET_AGENT_SETTINGS, serde_json::json!({})),
        };
        if matches!(self.settings, Loadable::Idle) {
            self.settings = Loadable::Loading;
        }
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(method, params)
                .await
                .map_err(|e| e.to_string())
                .and_then(|v| {
                    serde_json::from_value::<AgentSettings>(v).map_err(|e| e.to_string())
                });
            this.update(cx, |card, cx| {
                match result {
                    Ok(settings) => {
                        card.settings = Loadable::Ready(settings);
                        card.error = None;
                        card.state.update(cx, |state, cx| {
                            state.memory_settings_revision =
                                state.memory_settings_revision.wrapping_add(1);
                            cx.notify();
                        });
                    }
                    // A failed save keeps the last known value on screen and
                    // surfaces the reason, rather than blanking the switch.
                    Err(error) if matches!(card.settings, Loadable::Ready(_)) => {
                        card.error = Some(error);
                    }
                    Err(error) => card.settings = Loadable::Error(error),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
}

impl Render for MemoryCard {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::toast::take_error(&mut self.error, cx);
        let theme = Theme::of(cx).for_settings_surface();
        let localizer = crate::l10n::localizer();

        let control: gpui::AnyElement = match self.settings.clone() {
            Loadable::Ready(settings) => div()
                .flex_none()
                .child(widgets::toggle_switch(&theme, settings.memory, "memory"))
                .id("memory-toggle")
                .tab_index(0)
                .role(gpui::Role::Switch)
                .aria_label(localizer.text(TextKey::SettingsMemorySection))
                .on_click(cx.listener(move |card, _, _, cx| {
                    let mut next = settings.clone();
                    next.memory = !next.memory;
                    card.call(Some(next), cx);
                }))
                .into_any_element(),
            Loadable::Error(error) => div()
                .text_color(theme.text_muted)
                .child(error.clone())
                .into_any_element(),
            Loadable::Idle | Loadable::Loading => div()
                .text_color(theme.text_muted)
                .child(localizer.text(TextKey::ActionLoading))
                .into_any_element(),
        };

        let description = div()
            .child(SharedString::from(
                localizer.text(TextKey::SettingsMemoryDetail),
            ))
            .into_any_element();

        widgets::section(
            &theme,
            localizer.text(TextKey::SettingsMemorySection),
            widgets::section_card(&theme).mt_0().child(
                widgets::card_row(&theme, false)
                    .child(widgets::row_tile(&theme, icons::FILE_MARKDOWN))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(widgets::row_title(
                                &theme,
                                localizer.text(TextKey::SettingsMemory),
                            ))
                            .child(widgets::meta_line(&theme, vec![description])),
                    )
                    .child(control),
            ),
        )
    }
}
