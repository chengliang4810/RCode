//! Personalization: memory and the device's global custom instructions.

use gpui::{Context, Entity, Focusable, Subscription, Task, Window, div, prelude::*, px};
use rcode_engine::custom_instructions::InstructionsDocument;
use rcode_presentation::TextKey;
use rcode_rpc::methods;

use super::{memory::MemoryCard, widgets};
use crate::{composer::ComposerInput, popover, state::AppState, theme::Theme};

pub struct PersonalizationPage {
    state: Entity<AppState>,
    memory: Entity<MemoryCard>,
    input: Entity<ComposerInput>,
    stored: Option<InstructionsDocument>,
    busy: bool,
    save_requested: bool,
    error: Option<String>,
    scroll: widgets::PageScroll,
    _task: Option<Task<()>>,
    _blur: Option<Subscription>,
    _state: Subscription,
}

impl PersonalizationPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let memory = cx.new(|cx| MemoryCard::new(state.clone(), cx));
        let input = cx.new(|cx| {
            ComposerInput::with_context(
                crate::l10n::localizer().text(TextKey::SettingsPersonalizationInstructionsPlaceholder),
                "SettingsTextarea",
                cx,
            )
            .with_viewport_height(260.0)
            .with_text_metrics(13.0, 20.0)
            .with_tab_stop()
        });
        let state_sub = cx.observe(&state, |page: &mut Self, _, cx| {
            if page.stored.is_none() && !page.busy && page.error.is_none() {
                page.load(cx);
            }
        });
        let mut page = Self {
            state,
            memory,
            input,
            stored: None,
            busy: false,
            save_requested: false,
            error: None,
            scroll: widgets::PageScroll::default(),
            _task: None,
            _blur: None,
            _state: state_sub,
        };
        page.load(cx);
        page
    }

    pub fn has_pending_save(&self, cx: &gpui::App) -> bool {
        self.busy
            || self
                .stored
                .as_ref()
                .is_some_and(|stored| self.input.read(cx).text() != stored.content)
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.busy = true;
        self.error = None;
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::GET_CUSTOM_INSTRUCTIONS, serde_json::json!({}))
                .await
                .map_err(|e| e.to_string())
                .and_then(|value| {
                    serde_json::from_value::<InstructionsDocument>(value).map_err(|e| e.to_string())
                });
            this.update(cx, |page, cx| {
                page.busy = false;
                match result {
                    Ok(document) => {
                        page.input
                            .update(cx, |input, cx| input.set_text(document.content.clone(), cx));
                        page.stored = Some(document);
                        page.error = None;
                    }
                    Err(error) => {
                        page.error = Some(crate::l10n::localizer().format_with(
                            TextKey::SettingsPersonalizationLoadFailed,
                            &[("error", error.as_str())],
                        ))
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Called on input blur and before navigating away. Writes are serialized;
    /// typing during a save never replaces the newer editor text with its reply.
    pub fn flush_pending(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            self.save_requested = true;
            return;
        }
        let Some(stored) = &self.stored else {
            return;
        };
        let content = self.input.read(cx).text().to_owned();
        if content == stored.content {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = serde_json::json!({"content": content, "expectedRevision": stored.revision});
        self.busy = true;
        self.save_requested = false;
        self.error = None;
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::SAVE_CUSTOM_INSTRUCTIONS, params)
                .await
                .map_err(|e| e.to_string())
                .and_then(|value| {
                    serde_json::from_value::<InstructionsDocument>(value).map_err(|e| e.to_string())
                });
            this.update(cx, |page, cx| {
                page.busy = false;
                let success = result.is_ok();
                match result {
                    Ok(document) => {
                        page.stored = Some(document);
                        page.error = None;
                    }
                    Err(error) => {
                        page.error = Some(crate::l10n::localizer().format_with(
                            TextKey::SettingsPersonalizationSaveFailed,
                            &[("error", error.as_str())],
                        ))
                    }
                }
                if success && page.save_requested {
                    page.save_requested = false;
                    // The current task must finish before installing its successor.
                    let page = cx.entity().downgrade();
                    cx.defer(move |cx| {
                        page.update(cx, |page, cx| page.flush_pending(cx)).ok();
                    });
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
}

impl popover::ScrollRailHost for PersonalizationPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }
    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for PersonalizationPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self._blur.is_none() {
            self._blur = Some(
                cx.on_blur(&self.input.focus_handle(cx), window, |page, _, cx| {
                    page.flush_pending(cx)
                }),
            );
        }
        let theme = Theme::of(cx).for_settings_surface();
        let localizer = crate::l10n::localizer();
        let mut editor = div().flex().flex_col().gap(px(8.0));
        if self.stored.is_some() {
            editor = editor.child(
                div()
                    .id("personalization-instructions")
                    .debug_selector(|| "personalization-instructions".into())
                    .p(px(12.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(theme.border)
                    .child(self.input.clone()),
            );
        } else if self.error.is_none() {
            editor = editor.child(
                div()
                    .text_color(theme.text_muted)
                    .child(localizer.text(TextKey::ActionLoading)),
            );
        }
        if let Some(error) = self.error.clone() {
            editor = editor.child(div().text_color(theme.text_muted).child(error));
            editor = editor.child(
                widgets::text_action(
                    &theme,
                    widgets::ActionTone::Quiet,
                    localizer.text(TextKey::ActionRetry),
                )
                .id("personalization-retry")
                .role(gpui::Role::Button)
                .tab_index(0)
                .on_click(cx.listener(|page, _, _, cx| {
                    if page.stored.is_some() {
                        page.flush_pending(cx);
                    } else {
                        page.load(cx);
                    }
                })),
            );
            if self.stored.is_some() && !self.busy {
                editor = editor.child(
                    widgets::text_action(
                        &theme,
                        widgets::ActionTone::Quiet,
                        localizer.text(TextKey::SettingsPersonalizationReload),
                    )
                    .id("personalization-reload")
                    .role(gpui::Role::Button)
                    .tab_index(0)
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.stored = None;
                        page.load(cx);
                    })),
                );
            }
        }
        let scrollbar = popover::rail(self, "personalization-scrollbar", &theme, cx);
        div()
            .id("personalization-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("personalization-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(
                            widgets::page_column()
                                .child(widgets::page_header(
                                    &theme,
                                    &localizer.text(TextKey::ShellSettingsNavPersonalization),
                                    None,
                                ))
                                .child(self.memory.clone())
                                .child(widgets::section(
                                    &theme,
                                    localizer.text(TextKey::SettingsPersonalizationInstructions),
                                    editor,
                                )),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
    }
}

#[cfg(test)]
#[path = "personalization_tests.rs"]
mod tests;
