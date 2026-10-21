//! Personal plugin marketplace; source resolution and download live in the engine.

use super::widgets;

const PAGE_SIZE: usize = 20;
use crate::{
    composer::{ComposerInput, ComposerInputEvent},
    icons, popover,
    state::AppState,
    theme::Theme,
};
use gpui::{
    AnyElement, Context, Entity, SharedString, Subscription, Task, Window, div, prelude::*, px,
};
use rcode_plugins::marketplace::{Item, Listing, SOURCES};
use rcode_presentation::TextKey;
use rcode_rpc::methods;

pub struct MarketplacePage {
    state: Entity<AppState>,
    source: usize,
    page: usize,
    search: Entity<ComposerInput>,
    items: Vec<Item>,
    loading: bool,
    loaded: bool,
    load_generation: u64,
    cached: bool,
    installing: Option<String>,
    error: Option<SharedString>,
    scroll: widgets::PageScroll,
    _load: Option<Task<()>>,
    _install: Option<Task<()>>,
    _state: Subscription,
    _search: Subscription,
}

impl MarketplacePage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            ComposerInput::new(
                crate::l10n::localizer().text(TextKey::SettingsMarketplaceSearch),
                cx,
            )
            .with_single_line()
            .with_tab_stop()
        });
        let search_events = cx.subscribe(&search, |page: &mut Self, _, event, cx| {
            if matches!(event, ComposerInputEvent::Edited) {
                page.page = 0;
                page.scroll.scroll.set_offset(gpui::Point::default());
                cx.notify();
            }
        });
        let observe = cx.observe(&state, |page: &mut Self, _, cx| {
            if !page.loaded && !page.loading && page.error.is_none() {
                page.reload(false, cx);
            }
        });
        let mut page = Self {
            state,
            source: 0,
            page: 0,
            search,
            items: Vec::new(),
            loading: false,
            loaded: false,
            load_generation: 0,
            cached: false,
            installing: None,
            error: None,
            scroll: widgets::PageScroll::default(),
            _load: None,
            _install: None,
            _state: observe,
            _search: search_events,
        };
        page.reload(false, cx);
        page
    }

    pub fn has_pending_install(&self) -> bool {
        self.installing.is_some()
    }

    fn select_source(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.installing.is_some() || index == self.source {
            return;
        }
        self._load = None;
        self.source = index;
        self.page = 0;
        self.scroll.scroll.set_offset(gpui::Point::default());
        self.items.clear();
        self.loaded = false;
        self.loading = false;
        self.cached = false;
        self.error = None;
        self.search
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        self.reload(false, cx);
    }

    fn reload(&mut self, force: bool, cx: &mut Context<Self>) {
        if (self.loading && !force) || self.installing.is_some() {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.loading = true;
        self.error = None;
        self._load = None;
        let source = SOURCES[self.source].id;
        self.load_generation = self.load_generation.wrapping_add(1);
        let generation = self.load_generation;
        self._load = Some(cx.spawn(async move |page, cx| {
            let mut refresh = force;
            loop {
                let result = engine
                    .client()
                    .call(
                        methods::LIST_PLUGIN_MARKETPLACE,
                        serde_json::json!({"source": source, "refresh": refresh}),
                    )
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|v| serde_json::from_value::<Listing>(v).map_err(|e| e.to_string()));
                let keep_refreshing = page
                    .update(cx, |page, cx| {
                        if page.load_generation != generation {
                            return false;
                        }
                        let mut keep_refreshing = false;
                        match result {
                            Ok(listing) => {
                                keep_refreshing = !refresh && listing.cached && listing.stale;
                                page.items = listing.plugins;
                                // Normal cache hits need no warning. Only a failed refresh does.
                                page.cached = refresh && listing.cached;
                                page.loaded = true;
                            }
                            Err(error) => {
                                page.error = Some(
                                    crate::l10n::localizer()
                                        .format_with(
                                            TextKey::SettingsMarketplaceLoadFailed,
                                            &[("error", error.as_str())],
                                        )
                                        .into(),
                                )
                            }
                        }
                        page.loading = keep_refreshing;
                        cx.notify();
                        keep_refreshing
                    })
                    .unwrap_or(false);
                if !keep_refreshing {
                    break;
                }
                refresh = true;
            }
        }));
        cx.notify();
    }

    fn install(&mut self, name: String, cx: &mut Context<Self>) {
        if self.installing.is_some() || !self.loaded {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        // A background list reply must not overwrite the installed state below.
        self._load = None;
        self.load_generation = self.load_generation.wrapping_add(1);
        self.loading = false;
        self.installing = Some(name.clone());
        self.error = None;
        let source = SOURCES[self.source].id;
        self._install = Some(cx.spawn(async move |page, cx| {
            let result = engine
                .client()
                .call(
                    methods::INSTALL_MARKETPLACE_PLUGIN,
                    serde_json::json!({"source": source, "name": name}),
                )
                .await;
            page.update(cx, |page, cx| {
                page.installing = None;
                match result {
                    Ok(_) => {
                        if let Some(item) = page.items.iter_mut().find(|item| item.name == name) {
                            item.installed = true;
                        }
                    }
                    Err(error) => {
                        page.error = Some(
                            crate::l10n::localizer()
                                .format_with(
                                    TextKey::SettingsMarketplaceInstallFailed,
                                    &[("error", error.to_string().as_str())],
                                )
                                .into(),
                        )
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn change_page(&mut self, page: usize, cx: &mut Context<Self>) {
        self.page = page;
        self.scroll.scroll.set_offset(gpui::Point::default());
        cx.notify();
    }

    fn render_item(
        &mut self,
        index: usize,
        first: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let item = &self.items[index];
        let name = item.name.clone();
        let localizer = crate::l10n::localizer();
        let busy = self.installing.as_deref() == Some(&name);
        let disabled =
            item.installed || item.unsupported || self.installing.is_some() || !self.loaded;
        let label = if item.installed {
            TextKey::SettingsMarketplaceInstalled
        } else if item.unsupported {
            TextKey::SettingsMarketplaceUnsupported
        } else if busy {
            TextKey::SettingsMarketplaceInstalling
        } else {
            TextKey::ActionInstall
        };
        let mut text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(widgets::row_title(theme, name.clone()));
        if !item.description.is_empty() {
            text = text.child(
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(item.description.clone())),
            );
        }
        if let Some(author) = &item.author {
            text = text.child(
                div()
                    .text_size(crate::typography::ui_rems(11.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(author.clone())),
            );
        }
        let accent = theme.accent;
        let action = widgets::text_action(theme, widgets::ActionTone::Quiet, localizer.text(label))
            .id(("marketplace-install", index))
            .debug_selector(move || format!("marketplace-install-{index}"))
            .role(gpui::Role::Button)
            .tab_index(0)
            .aria_label(localizer.format_with(
                TextKey::SettingsMarketplaceInstallAria,
                &[("name", name.as_str())],
            ))
            .focus_visible(move |style| style.border_2().border_color(accent))
            .when(disabled, |button| button.opacity(0.5))
            .when(!disabled, |button| {
                button.on_click(cx.listener(move |page, _, _, cx| page.install(name.clone(), cx)))
            });
        widgets::card_row(theme, first)
            .child(
                icons::icon(icons::PUZZLE)
                    .size(px(16.0))
                    .text_color(theme.text_muted),
            )
            .child(text)
            .child(action)
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }
}

impl popover::ScrollRailHost for MarketplacePage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }
    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for MarketplacePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let localizer = crate::l10n::localizer();
        let query = self.search.read(cx).text().trim().to_lowercase();
        let indices: Vec<_> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                query.is_empty()
                    || format!(
                        "{} {} {}",
                        item.name,
                        item.description,
                        item.author.as_deref().unwrap_or_default()
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        let page_count = indices.len().div_ceil(PAGE_SIZE).max(1);
        self.page = self.page.min(page_count - 1);
        let page = self.page;
        let pager = if page_count > 1 {
            let previous = widgets::text_action(
                &theme,
                widgets::ActionTone::Quiet,
                localizer.text(TextKey::SettingsMarketplacePrevious),
            )
            .id("marketplace-previous")
            .debug_selector(|| "marketplace-previous".into())
            .role(gpui::Role::Button)
            .tab_index(0)
            .when(page == 0, |button| button.opacity(0.4))
            .when(page > 0, |button| {
                button.on_click(cx.listener(move |this, _, _, cx| this.change_page(page - 1, cx)))
            });
            let next = widgets::text_action(
                &theme,
                widgets::ActionTone::Quiet,
                localizer.text(TextKey::SettingsMarketplaceNext),
            )
            .id("marketplace-next")
            .debug_selector(|| "marketplace-next".into())
            .role(gpui::Role::Button)
            .tab_index(0)
            .when(page + 1 == page_count, |button| button.opacity(0.4))
            .when(page + 1 < page_count, |button| {
                button.on_click(cx.listener(move |this, _, _, cx| this.change_page(page + 1, cx)))
            });
            let current = (page + 1).to_string();
            let total = page_count.to_string();
            Some(
                div()
                    .flex_none()
                    .border_t_1()
                    .border_color(theme.border)
                    .py(px(10.0))
                    .child(
                        widgets::page_column().pt(px(0.0)).pb(px(0.0)).child(
                            div()
                                .flex()
                                .items_center()
                                .justify_end()
                                .gap(px(12.0))
                                .child(previous)
                                .child(
                                    div()
                                        .text_size(crate::typography::ui_rems(12.0))
                                        .text_color(theme.text_muted)
                                        .child(localizer.format_with(
                                            TextKey::SettingsMarketplacePage,
                                            &[
                                                ("page", current.as_str()),
                                                ("pages", total.as_str()),
                                            ],
                                        )),
                                )
                                .child(next),
                        ),
                    ),
            )
        } else {
            None
        };
        let refresh_disabled = (self.loading && !self.loaded) || self.installing.is_some();
        let refresh = widgets::text_action(
            &theme,
            widgets::ActionTone::Quiet,
            localizer.text(TextKey::SettingsMarketplaceRefresh),
        )
        .id("marketplace-refresh")
        .role(gpui::Role::Button)
        .tab_index(0)
        .when(refresh_disabled, |button| button.opacity(0.5))
        .when(!refresh_disabled, |button| {
            button.on_click(cx.listener(|page, _, _, cx| page.reload(true, cx)))
        });
        let mut sources = div()
            .id("marketplace-sources")
            .role(gpui::Role::TabList)
            .flex()
            .items_center()
            .gap(px(8.0))
            .mb(px(12.0));
        for (index, source) in SOURCES.iter().enumerate() {
            let selected = self.source == index;
            let label = if source.id == "official" {
                localizer.text(TextKey::SettingsMarketplaceOfficial)
            } else {
                source.name.to_owned()
            };
            let accent = theme.accent;
            sources = sources.child(
                widgets::text_action(
                    &theme,
                    if selected {
                        widgets::ActionTone::Solid
                    } else {
                        widgets::ActionTone::Quiet
                    },
                    label,
                )
                .id(("marketplace-source", index))
                .role(gpui::Role::Tab)
                .aria_selected(selected)
                .tab_index(0)
                .focus_visible(move |style| style.border_2().border_color(accent))
                .when(self.installing.is_none(), |button| {
                    button
                        .on_click(cx.listener(move |page, _, _, cx| page.select_source(index, cx)))
                }),
            );
        }
        let mut column = widgets::page_column()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(widgets::page_header(
                        &theme,
                        &localizer.text(TextKey::SettingsMarketplace),
                        Some(self.items.len()),
                    ))
                    .child(refresh),
            )
            .child(sources)
            .child(
                div()
                    .p(px(10.0))
                    .border_1()
                    .border_color(theme.border)
                    .rounded(px(8.0))
                    .child(self.search.clone()),
            );
        if self.cached {
            column = column.child(
                div()
                    .mt(px(8.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(localizer.text(TextKey::SettingsMarketplaceCached)),
            );
        }
        if let Some(error) = self.error.clone() {
            column = column.child(widgets::error_strip(&theme, error));
        }
        let body: AnyElement = if self.loading && self.items.is_empty() {
            div()
                .mt(px(24.0))
                .text_color(theme.text_muted)
                .child(localizer.text(TextKey::ActionLoading))
                .into_any_element()
        } else if indices.is_empty() && self.loaded {
            div()
                .mt(px(24.0))
                .text_color(theme.text_muted)
                .child(localizer.text(TextKey::SettingsMarketplaceEmpty))
                .into_any_element()
        } else {
            let mut card = widgets::section_card(&theme).mt(px(16.0));
            for (visible, index) in indices
                .into_iter()
                .skip(page * PAGE_SIZE)
                .take(PAGE_SIZE)
                .enumerate()
            {
                card = card.child(self.render_item(index, visible == 0, &theme, cx));
            }
            card.into_any_element()
        };
        column = column.child(body);
        let scrollbar = popover::rail(self, "marketplace-scrollbar", &theme, cx);
        div()
            .id("marketplace-page-host")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        crate::edge_fade::edge_faded(
                            16.0,
                            true,
                            true,
                            div()
                                .id("marketplace-page")
                                .size_full()
                                .overflow_y_scroll()
                                .track_scroll(&self.scroll.scroll)
                                .child(column),
                        )
                        .fade_overflow_y(&self.scroll.scroll),
                    )
                    .children(scrollbar),
            )
            .children(pager)
    }
}

#[cfg(test)]
#[path = "marketplace_tests.rs"]
mod tests;
