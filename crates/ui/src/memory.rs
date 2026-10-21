use gpui::{Context, Entity, EventEmitter, SharedString, Subscription, Task, div, prelude::*, px};
use rcode_presentation::TextKey;
use rcode_proto::ProjectMemoryListing;

use crate::files::client::{FilesRequestContext, WorkspaceFilesClient};
use crate::popover::Loadable;
use crate::state::{AppState, EngineHandle};
use crate::{icons, theme::Theme};

pub(crate) enum MemoryEvent {
    Changed,
    OpenFile(String),
}

pub(crate) struct MemoryPanel {
    state: Entity<AppState>,
    chat_id: String,
    context: Option<FilesRequestContext>,
    engine: Option<EngineHandle>,
    revision: u64,
    generation: u64,
    enabled: bool,
    listing: Loadable<ProjectMemoryListing>,
    task: Option<Task<()>>,
    _state: Subscription,
}

impl EventEmitter<MemoryEvent> for MemoryPanel {}

impl MemoryPanel {
    pub(crate) fn new(state: Entity<AppState>, chat_id: String, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |this: &mut Self, _, cx| this.load(false, cx));
        let mut panel = Self {
            state,
            chat_id,
            context: None,
            engine: None,
            revision: u64::MAX,
            generation: 0,
            enabled: false,
            listing: Loadable::Idle,
            task: None,
            _state: subscription,
        };
        panel.load(false, cx);
        panel
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn editor_target(&self, path: &str) -> Option<(String, String)> {
        let Loadable::Ready(listing) = &self.listing else {
            return None;
        };
        let root = listing.directory.as_ref()?;
        let relative = path
            .strip_prefix(root.trim_end_matches('/'))?
            .strip_prefix('/')?;
        Some((root.clone(), relative.to_owned()))
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(true, cx);
    }

    fn load(&mut self, force: bool, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let context = FilesRequestContext::for_chat(state, &self.chat_id);
        let revision = state.memory_settings_revision;
        let engine = state.engine().cloned();
        let same_engine = match (&self.engine, &engine) {
            (Some(a), Some(b)) => a.same_connection(b),
            (None, None) => true,
            _ => false,
        };
        let changed = self.context != context || self.revision != revision || !same_engine;
        if !force && !changed {
            return;
        }
        self.context = context.clone();
        self.engine = engine.clone();
        self.revision = revision;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        if changed {
            self.enabled = false;
        }
        let (Some(context), Some(engine)) = (context, engine) else {
            self.listing = Loadable::Idle;
            self.task = None;
            cx.emit(MemoryEvent::Changed);
            cx.notify();
            return;
        };
        self.listing = Loadable::Loading;
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = WorkspaceFilesClient::new(engine, context)
                .project_memory()
                .await;
            this.update(cx, |panel, cx| {
                if generation != panel.generation {
                    return;
                }
                match result {
                    Ok(listing) => {
                        panel.enabled = listing.enabled;
                        panel.listing = Loadable::Ready(listing);
                    }
                    Err(error) => {
                        panel.listing = Loadable::Error(error.to_string());
                    }
                }
                cx.emit(MemoryEvent::Changed);
                cx.notify();
            })
            .ok();
        }));
        cx.emit(MemoryEvent::Changed);
        cx.notify();
    }
}

impl Render for MemoryPanel {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let localizer = crate::l10n::localizer();
        let mut files = div()
            .id("memory-files")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(6.0));
        match self.listing.clone() {
            Loadable::Ready(listing) => {
                if listing.files.is_empty() {
                    files = files.child(
                        div()
                            .p(px(12.0))
                            .text_color(theme.text_muted)
                            .child(localizer.text(TextKey::MemoryEmpty)),
                    );
                }
                for (index, file) in listing.files.into_iter().enumerate() {
                    let path = file.absolute_path;
                    let label = file.path;
                    files = files.child(
                        div()
                            .id(("memory-file", index))
                            .h(px(32.0))
                            .px(px(8.0))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .rounded(px(5.0))
                            .cursor_pointer()
                            .role(gpui::Role::Button)
                            .tab_index(0)
                            .aria_label(label.clone())
                            .hover(|row| row.bg(crate::theme::ink(0.06)))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(MemoryEvent::OpenFile(path.clone()))
                            }))
                            .child(
                                crate::icons::icon(icons::FILE_MARKDOWN)
                                    .size(px(14.0))
                                    .text_color(theme.text_muted),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(SharedString::from(label)),
                            ),
                    );
                }
                if listing.truncated {
                    files = files.child(
                        div()
                            .p(px(12.0))
                            .text_color(theme.text_muted)
                            .child(localizer.text(TextKey::MemoryListTruncated)),
                    );
                }
            }
            Loadable::Error(error) => {
                files = files.child(div().p(px(12.0)).text_color(theme.text_muted).child(error));
            }
            _ => {
                files = files.child(
                    div()
                        .p(px(12.0))
                        .text_color(theme.text_muted)
                        .child(localizer.text(TextKey::ActionLoading)),
                );
            }
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .text_size(crate::typography::ui_rems(12.0))
            .child(
                crate::surface_chrome::toolbar(&theme).justify_end().child(
                    crate::settings::widgets::text_action(
                        &theme,
                        crate::settings::widgets::ActionTone::Quiet,
                        localizer.text(TextKey::GitRefresh),
                    )
                    .id("memory-refresh")
                    .role(gpui::Role::Button)
                    .tab_index(0)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                ),
            )
            .child(files)
    }
}
