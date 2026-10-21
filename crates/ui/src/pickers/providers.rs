//! The conversation model picker: configured provider identities, then their models.
use super::*;
use rcode_harness::provider_store::ProviderSettingsView;

#[derive(Default)]
pub(super) struct ProviderMenu {
    pub view: Option<ProviderSettingsView>,
    loading: bool,
    attempted: bool,
    cache: std::cell::RefCell<Option<(ModelRowsKey, String, std::sync::Arc<Vec<ProviderGroup>>)>>,
    pub open: Option<String>,
    active: usize,
    pub bounds: Option<gpui::Bounds<gpui::Pixels>>,
    left: bool,
    hover: popover::HoverIntent<usize>,
    scroll: gpui::ScrollHandle,
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        picker: Entity<Pickers>,
        left: bool,
        _dir: tempfile::TempDir,
    }
    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().relative().child(
                div()
                    .absolute()
                    .bottom(px(60.0))
                    .when(self.left, |el| el.right(px(16.0)))
                    .when(!self.left, |el| el.left(px(32.0)))
                    .child(self.picker.clone()),
            )
        }
    }
    fn setup(
        cx: &mut gpui::TestAppContext,
        compact: bool,
        left: bool,
    ) -> (gpui::WindowHandle<Fixture>, Entity<Pickers>) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
            crate::app_menus::init(cx);
            motion::set_reduced_motion(cx, true);
            let mut settings = crate::settings::UiSettings::default();
            settings.compact_model_picker = compact;
            crate::settings::init(settings, dir.path(), cx);
        });
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            let picker = cx.new(|cx| {
                let mut picker = Pickers::new(state, cx);
                picker.data_dir = None; picker.defaults = ComposerDefaults::default();
                picker.config.harness = Some(HarnessId::Native); picker.config.model = Some("a1".into());
                picker.harnesses = Loadable::Ready(vec![HarnessDescriptor { id: HarnessId::Native,
                    name: "RCode".into(), installed: true, enabled: Some(true), reasoning_levels: vec![],
                    steering_mode: rcode_proto::SteeringMode::StepBoundary, supports_steering: true }]);
                picker.models.insert(HarnessId::Native, Loadable::Ready(["a1", "a2", "b1"].map(|id| Model {
                    id: id.into(), label: "Shared supplier · Same model".into(), description: Some(id.into()),
                    reasoning_levels: vec![ReasoningLevel::Low, ReasoningLevel::High],
                    options: rcode_harness::context_window::model_options(Some(1_000_000)),
                }).into()));
                picker.provider_menu.view = Some(serde_json::from_value(serde_json::json!({
                    "revision":"fixture", "providers":[
                        {"id":"first","label":"Shared supplier","protocol":"openai-chat","credential":{"kind":"none"},
                         "models":[{"id":"a1","model":"Same model","inputFormat":"text-and-images"},{"id":"a2","model":"Other model","inputFormat":"text"}]},
                        {"id":"second","label":"Shared supplier","protocol":"openai-chat","credential":{"kind":"none"},
                         "models":[{"id":"b1","model":"Same model"}]},
                        {"id":"empty","protocol":"openai-chat","credential":{"kind":"none"},"models":[]}
                    ]
                })).unwrap());
                picker.provider_menu.attempted = true;
                picker
            });
            Fixture { picker, left, _dir: dir }
        });
        let picker = window
            .read_with(cx, |fixture, _| fixture.picker.clone())
            .unwrap();
        (window, picker)
    }
    fn redraw(cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
            window.draw(cx).clear();
        });
    }
    #[gpui::test]
    fn conversation_chip_opens_provider_children_and_selects_on_both_sides(
        cx: &mut gpui::TestAppContext,
    ) {
        for compact in [false, true] {
            for left in [false, true] {
                let (window, picker) = setup(cx, compact, left);
                let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
                redraw(&mut visual);
                let trigger = visual.debug_bounds("picker-model").unwrap().center();
                visual.simulate_click(trigger, Default::default());
                redraw(&mut visual);
                let provider_row = visual.debug_bounds("conversation-provider-0").unwrap();
                let selected_provider = visual.debug_bounds("provider-selected-0").unwrap();
                let provider_label = visual.debug_bounds("provider-label-0").unwrap();
                let gap = selected_provider.left() - provider_label.right();
                assert!(gap >= px(4.0) && gap <= px(8.0));
                assert!(visual.debug_bounds("provider-selected-1").is_none());
                visual.update(|window, cx| {
                    picker.read_with(cx, |picker, cx| {
                        let search = picker.search.read(cx).focus_handle(cx);
                        assert!(!picker.focus.contains(&search, window));
                    });
                });
                visual.simulate_mouse_move(provider_row.center(), None, Default::default());
                redraw(&mut visual);
                let selected_model = visual.debug_bounds("model-selected-0").unwrap();
                let model_row = visual
                    .debug_bounds("conversation-provider-model-0")
                    .unwrap();
                assert!(selected_model.center().x > model_row.right() - px(30.0));
                assert!(visual.debug_bounds("model-selected-1").is_none());
                assert!(visual.debug_bounds("model-vision-0").is_some());
                assert!(visual.debug_bounds("model-vision-1").is_none());
                let provider = visual.debug_bounds("conversation-provider-1").unwrap();
                visual.simulate_mouse_move(provider.center(), None, Default::default());
                redraw(&mut visual);
                picker.read_with(&visual, |picker, cx| {
                    assert_eq!(picker.provider_menu.open.as_deref(), Some("native:second"));
                    assert_eq!(picker.provider_menu.left, left);
                    assert_eq!(
                        picker.provider_groups(cx).len(),
                        2,
                        "empty providers are omitted and same names keep distinct IDs"
                    );
                });
                assert!(
                    visual
                        .debug_bounds("conversation-provider-model-0")
                        .is_none()
                );
                let child = visual
                    .debug_bounds("conversation-provider-model-2")
                    .unwrap()
                    .center();
                visual.simulate_click(child, Default::default());
                redraw(&mut visual);
                picker.read_with(&visual, |picker, cx| {
                    assert_eq!(picker.resolved(cx).model.as_deref(), Some("b1"));
                    assert!(
                        picker.provider_menu.open.is_none(),
                        "child click must not reopen its provider ancestor"
                    );
                    assert!(
                        picker
                            .setting_groups(cx)
                            .iter()
                            .any(|group| group.id == ModelSetting::Reasoning)
                    );
                    assert!(
                        picker
                            .setting_groups(cx)
                            .iter()
                            .any(|group| group.id == ModelSetting::Option("contextWindow".into()))
                    );
                    if compact {
                        assert!(!picker.compact_model_list);
                        assert!(picker.compact_effort(cx).is_some());
                    }
                });
                visual.simulate_click(gpui::point(px(8.0), px(8.0)), Default::default());
                redraw(&mut visual);
                picker.read_with(&visual, |picker, _| assert!(!picker.is_open()));
                // This test checks selection across opens, independently of
                // the exit animation's suppression of immediate re-clicks.
                picker.update(&mut visual, |picker, cx| {
                    picker.open = Default::default();
                    cx.notify();
                });
                redraw(&mut visual);
                let trigger = visual.debug_bounds("picker-model").unwrap().center();
                visual.simulate_click(trigger, Default::default());
                redraw(&mut visual);
                picker.read_with(&visual, |picker, _| assert!(picker.is_open()));
                if compact {
                    picker.update(&mut visual, |picker, cx| picker.show_compact_models(cx));
                    redraw(&mut visual);
                }
                assert!(visual.debug_bounds("provider-selected-0").is_none());
                assert!(visual.debug_bounds("provider-selected-1").is_some());
                visual.simulate_keystrokes("right");
                redraw(&mut visual);
                picker.read_with(&visual, |picker, cx| {
                    assert_eq!(
                        picker.provider_menu.open.as_deref(),
                        Some("native:second"),
                        "compact={compact}, left={left}"
                    );
                    assert!(picker.row_is_selected(&picker.model_rows(cx)[2], cx));
                });
                assert!(
                    visual
                        .debug_bounds("conversation-provider-model-2")
                        .is_some(),
                    "child missing: compact={compact}, left={left}"
                );
                assert!(
                    visual.debug_bounds("model-selected-2").is_some(),
                    "check missing: compact={compact}, left={left}"
                );
                visual.simulate_click(gpui::point(px(8.0), px(8.0)), Default::default());
                redraw(&mut visual);
            }
        }
    }
    #[gpui::test]
    fn keyboard_returns_to_providers_then_selects_another_model(cx: &mut gpui::TestAppContext) {
        let (window, picker) = setup(cx, true, false);
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        redraw(&mut visual);
        let trigger = visual.debug_bounds("picker-model").unwrap().center();
        visual.simulate_click(trigger, Default::default());
        redraw(&mut visual);
        visual.simulate_keystrokes("down right");
        redraw(&mut visual);
        picker.read_with(&visual, |picker, _| {
            assert_eq!(picker.provider_menu.open.as_deref(), Some("native:second"))
        });
        visual.simulate_keystrokes("escape");
        picker.read_with(&visual, |picker, _| {
            assert!(picker.is_open());
            assert!(picker.compact_model_list);
            assert!(picker.provider_menu.open.is_none());
        });
        visual.simulate_keystrokes("up right down enter");
        redraw(&mut visual);
        picker.read_with(&visual, |picker, cx| {
            assert_eq!(picker.resolved(cx).model.as_deref(), Some("a2"));
            assert!(!picker.compact_model_list);
            assert!(picker.provider_menu.open.is_none());
        });
    }
    #[gpui::test]
    fn retired_virtual_selections_fall_back_to_a_real_model_without_adding_choices(
        cx: &mut gpui::TestAppContext,
    ) {
        for compact in [false, true] {
            let (_window, picker) = setup(cx, compact, false);
            picker.update(cx, |picker, cx| {
                for legacy in ["route:haiku", "route:sonnet", "route:opus", "route:fable"] {
                    picker.config.model = Some(legacy.into());
                    assert_eq!(picker.resolved(cx).model.as_deref(), Some("a1"));
                    let groups = picker.provider_groups(cx);
                    assert_eq!(groups.len(), 2);
                    assert!(groups.iter().all(|group| group.key.starts_with("native:")));
                }
            });
        }
    }
    #[gpui::test]
    fn composer_effort_opens_directly_and_preserves_the_model(cx: &mut gpui::TestAppContext) {
        for compact in [false, true] {
            let (window, picker) = setup(cx, compact, false);
            let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
            redraw(&mut visual);
            picker.read_with(&visual, |picker, cx| {
                assert_eq!(picker.model_chip_label(picker.selected_model_label(cx).unwrap().into(), cx).as_ref(), "Same model");
            });
            let trigger = visual.debug_bounds("picker-effort").unwrap().center();
            visual.simulate_click(trigger, Default::default());
            redraw(&mut visual);
            let (index, level) = picker.read_with(&visual, |picker, cx| {
                assert_eq!(picker.open_kind(), Some(PickerKind::Effort));
                assert!(picker.provider_menu.open.is_none());
                let index = 1 - picker.compact_effort(cx).unwrap().selected;
                (index, [ReasoningLevel::Low, ReasoningLevel::High][index])
            });
            assert!(visual.debug_bounds("conversation-provider-list").is_none());
            assert!(visual.debug_bounds("compact-select-model").is_none());
            let fast = visual.debug_bounds("compact-fast-preview").unwrap();
            let options = picker.read_with(&visual, |picker, cx| picker.resolved(cx).model_options.clone());
            visual.simulate_click(fast.center(), Default::default());
            visual.simulate_keystrokes("f");
            redraw(&mut visual);
            picker.read_with(&visual, |picker, cx| {
                assert!(picker.is_open());
                assert_eq!(picker.resolved(cx).model_options, options);
            });
            let slider = visual.debug_bounds("compact-effort-slider").unwrap();
            assert!(f32::from(slider.size.height) > 0.0);
            let choice = gpui::point(
                if index == 0 { slider.left() + px(22.0) } else { slider.right() - px(22.0) },
                slider.center().y,
            );
            let start = gpui::point(
                if index == 0 { slider.right() - px(22.0) } else { slider.left() + px(22.0) },
                slider.center().y,
            );
            visual.simulate_mouse_down(start, gpui::MouseButton::Left, Default::default());
            redraw(&mut visual);
            visual.simulate_mouse_move(choice, Some(gpui::MouseButton::Left), Default::default());
            redraw(&mut visual);
            visual.simulate_mouse_up(choice, gpui::MouseButton::Left, Default::default());
            redraw(&mut visual);
            picker.read_with(&visual, |picker, cx| {
                assert_eq!(picker.resolved(cx).model.as_deref(), Some("a1"));
                assert_eq!(picker.resolved(cx).reasoning, Some(level));
                assert!(picker.is_open(), "the slider stays open for further adjustment");
            });
            let model = visual.debug_bounds("picker-model").unwrap().center();
            visual.simulate_click(model, Default::default());
            redraw(&mut visual);
            picker.read_with(&visual, |picker, _| assert_eq!(picker.open_kind(), Some(PickerKind::HarnessModel), "compact={compact}"));
            assert!(visual.debug_bounds("conversation-provider-list").is_some());
            let effort = visual.debug_bounds("picker-effort").unwrap().center();
            visual.simulate_click(effort, Default::default());
            redraw(&mut visual);
            picker.read_with(&visual, |picker, _| assert_eq!(picker.open_kind(), Some(PickerKind::Effort)));
            assert!(visual.debug_bounds("conversation-provider-list").is_none());
            assert!(visual.debug_bounds("compact-effort-slider").is_some());
            visual.simulate_keystrokes("escape");
            redraw(&mut visual);
            picker.update(&mut visual, |picker, cx| {
                picker.open = Default::default();
                let Loadable::Ready(models) = picker.models.get_mut(&HarnessId::Native).unwrap() else { unreachable!() };
                models[0].reasoning_levels.clear();
                models[0].options.clear();
                picker.catalog_rev += 1;
                cx.notify();
            });
            redraw(&mut visual);
            assert!(visual.debug_bounds("picker-effort").is_none());
        }
    }

    #[gpui::test]
    fn direct_effort_supports_option_ladders_and_keyboard(cx: &mut gpui::TestAppContext) {
        let (window, picker) = setup(cx, true, false);
        picker.update(cx, |picker, cx| {
            let Loadable::Ready(models) = picker.models.get_mut(&HarnessId::Native).unwrap() else { unreachable!() };
            models[0].reasoning_levels.clear();
            models[0].options = serde_json::from_value(serde_json::json!([{
                "id":"reasoning_effort", "label":"Effort", "defaultChoice":"low",
                "choices":[{"id":"low","label":"Low"},{"id":"high","label":"High"}]
            }])).unwrap();
            picker.catalog_rev += 1;
            cx.notify();
        });
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        redraw(&mut visual);
        let trigger = visual.debug_bounds("picker-effort").unwrap().center();
        visual.simulate_click(trigger, Default::default());
        redraw(&mut visual);
        assert!(visual.debug_bounds("compact-effort-slider").is_some());
        visual.simulate_keystrokes("end");
        redraw(&mut visual);
        picker.read_with(&visual, |picker, cx| {
            assert_eq!(picker.resolved(cx).model_options["reasoning_effort"], "high");
            assert!(picker.is_open());
        });
        visual.simulate_keystrokes("home right escape");
        redraw(&mut visual);
        picker.read_with(&visual, |picker, cx| {
            let config = picker.resolved(cx);
            assert_eq!(config.model.as_deref(), Some("a1"));
            assert_eq!(config.model_options["reasoning_effort"], "high");
            assert!(!picker.is_open());
        });
    }

    #[gpui::test]
    fn provider_children_follow_saved_model_order_before_the_model_catalog_refreshes(
        cx: &mut gpui::TestAppContext,
    ) {
        for compact in [false, true] {
            let (_window, picker) = setup(cx, compact, false);
            picker.update(cx, |picker, cx| {
                let before = picker.provider_groups(cx);
                assert_eq!(
                    before[0]
                        .rows
                        .iter()
                        .map(|(index, _)| *index)
                        .collect::<Vec<_>>(),
                    [0, 1]
                );
                let view = picker.provider_menu.view.as_mut().unwrap();
                view.providers[0].models.reverse();
                view.revision = "reordered".into();
                let after = picker.provider_groups(cx);
                assert_eq!(
                    after[0]
                        .rows
                        .iter()
                        .map(|(index, _)| *index)
                        .collect::<Vec<_>>(),
                    [1, 0]
                );
                assert_eq!(after[0].rows[0].1.as_ref(), "Other model");
                assert_eq!(after[1].rows[0].0, 2);
                assert!(!std::sync::Arc::ptr_eq(&before, &after));
            });
        }
    }
    #[gpui::test]
    fn provider_group_cache_ignores_hidden_search_and_preserves_exact_membership(
        cx: &mut gpui::TestAppContext,
    ) {
        let (_window, picker) = setup(cx, true, false);
        picker.update(cx, |picker, cx| {
            let first = picker.provider_groups(cx);
            assert_eq!(
                first[0]
                    .rows
                    .iter()
                    .map(|(index, _)| *index)
                    .collect::<Vec<_>>(),
                [0, 1]
            );
            assert_eq!(first[1].rows[0].0, 2);
            picker.active = 2;
            assert!(
                std::sync::Arc::ptr_eq(&first, &picker.provider_groups(cx)),
                "hover and keyboard highlights reuse the cached groups"
            );
            picker
                .search
                .update(cx, |input, cx| input.set_text("b1", cx));
            let filtered = picker.provider_groups(cx);
            assert!(std::sync::Arc::ptr_eq(&first, &filtered));
            assert_eq!(filtered.len(), 2);
            assert_eq!(filtered[0].rows.len(), 2);
            assert_eq!(filtered[1].key, "native:second");
            assert_eq!(filtered[1].rows.len(), 1);
        });
    }
    #[gpui::test]
    fn standard_picker_keyboard_keeps_context_choices_available(cx: &mut gpui::TestAppContext) {
        let (window, picker) = setup(cx, false, false);
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        redraw(&mut visual);
        let trigger = visual.debug_bounds("picker-model").unwrap().center();
        visual.simulate_click(trigger, Default::default());
        redraw(&mut visual);
        visual.simulate_keystrokes("end right");
        redraw(&mut visual);
        picker.read_with(&visual, |picker, _| {
            assert_eq!(
                picker.setting_menu,
                Some(ModelSetting::Option("contextWindow".into()))
            )
        });
        visual.simulate_keystrokes("down enter");
        redraw(&mut visual);
        picker.read_with(&visual, |picker, cx| {
            assert_eq!(picker.resolved(cx).model_options["contextWindow"], "500k");
            assert!(picker.setting_menu.is_none());
        });
    }
}
#[derive(Clone)]
struct ProviderGroup {
    key: String,
    label: SharedString,
    rows: Vec<(usize, SharedString)>,
}
impl ProviderMenu {
    pub fn reset_highlight(&mut self) {
        self.active = 0;
    }
    pub fn close(&mut self) {
        self.open = None;
        self.bounds = None;
        self.hover.reset();
    }
}

impl Pickers {
    fn model_supports_vision(&self, row: &ModelRowData) -> bool {
        row.harness == HarnessId::Native
            && self.provider_menu.view.as_ref().is_some_and(|view| {
                view.providers
                    .iter()
                    .flat_map(|provider| &provider.models)
                    .any(|model| model.id == row.model.id && model.input_format.accepts_images())
            })
    }

    pub(super) fn render_model_name(
        &self,
        index: usize,
        row: &ModelRowData,
        cx: &App,
    ) -> gpui::Div {
        let theme = Theme::of(cx).for_popup();
        let vision = self.model_supports_vision(row);
        let hint = crate::l10n::localizer().text(TextKey::ProvidersModelInputVisionBadge);
        div()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(4.0))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(SharedString::from(row.model.label.clone())),
            )
            .when(vision, |el| {
                el.child(
                    div()
                        .id(("model-vision", index))
                        .debug_selector(move || format!("model-vision-{index}"))
                        .flex_none()
                        .size(px(13.0))
                        .aria_label(hint.clone())
                        .tooltip(crate::settings::widgets::text_tooltip(hint))
                        .child(
                            crate::icons::icon(crate::icons::EYE)
                                .size(px(13.0))
                                .text_color(theme.text_muted),
                        ),
                )
            })
    }

    pub(super) fn ensure_model_providers(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.provider_menu.loading || (!force && self.provider_menu.attempted) {
            return;
        }
        let Some(engine) = self.engine(cx) else {
            return;
        };
        let target = self.space_target(cx);
        let generation = self.target_generation;
        self.provider_menu.loading = true;
        self.provider_menu.attempted = true;
        cx.spawn(async move |this, cx| {
            let mut params = serde_json::json!({});
            if let Some(target) = target {
                params["targetDeviceId"] = target.into();
            }
            let result = engine
                .client()
                .call(methods::GET_PROVIDER_SETTINGS, params)
                .await
                .ok()
                .and_then(|value| serde_json::from_value::<ProviderSettingsView>(value).ok());
            this.update(cx, |this, cx| {
                if this.target_generation != generation {
                    return;
                }
                this.provider_menu.loading = false;
                if let Some(view) = result {
                    let changed = this
                        .provider_menu
                        .view
                        .as_ref()
                        .is_none_or(|old| old.revision != view.revision);
                    this.provider_menu.view = Some(view);
                    if changed {
                        this.prime_provider_menu(cx);
                    }

                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
    pub(super) fn provider_mode(&self, cx: &App) -> bool {
        self.title.is_none()
            && self.effective_harness(cx) == Some(HarnessId::Native)
            && self.provider_menu.view.is_some()
            && self
                .models
                .get(&HarnessId::Native)
                .and_then(Loadable::ready)
                .is_some()
    }
    pub(super) fn prime_provider_menu(&mut self, cx: &mut Context<Self>) {
        self.provider_menu.close();
        let selected = self.selected_model_index(cx);
        self.provider_menu.active = self
            .provider_groups(cx)
            .iter()
            .position(|group| group.rows.iter().any(|(index, _)| *index == selected))
            .unwrap_or(0);
    }
    fn provider_groups(&self, cx: &App) -> std::sync::Arc<Vec<ProviderGroup>> {
        let key = ModelRowsKey {
            query: String::new(),
            rail: self.model_rail,
            effective: self.effective_harness(cx),
            locked: self.harness_locked(cx),
            catalog_rev: self.catalog_rev,
            selected: self.effective_model_id(cx).map(str::to_owned),
        };
        let revision = self
            .provider_menu
            .view
            .as_ref()
            .map(|view| view.revision.as_str())
            .unwrap_or_default();
        if let Some((old_key, old_revision, groups)) = self.provider_menu.cache.borrow().as_ref()
            && old_key == &key
            && old_revision == revision
        {
            return groups.clone();
        }
        let mut identities = HashMap::new();
        let mut model_order = HashMap::new();
        if let Some(view) = &self.provider_menu.view {
            for provider in &view.providers {
                for (position, model) in provider.models.iter().enumerate() {
                    model_order.insert(model.id.as_str(), position);
                    identities.insert(
                        model.id.as_str(),
                        (
                            format!("native:{}", provider.id),
                            provider.label.as_deref().unwrap_or(&provider.id),
                            model.label.as_deref().unwrap_or(&model.model),
                        ),
                    );
                }
            }
        }
        let rows = self.model_rows(cx);
        let mut groups: Vec<ProviderGroup> = Vec::new();
        let mut indices = HashMap::new();
        for (index, row) in rows.iter().enumerate() {
            let (key, label, model_label) = if row.harness == HarnessId::Native {
                identities
                    .get(row.model.id.as_str())
                    .map(|(key, label, model)| {
                        (key.clone(), (*label).to_owned(), (*model).to_owned())
                    })
                    .unwrap_or_else(|| {
                        (
                            "native:unavailable".into(),
                            row.harness_name.to_string(),
                            row.model.label.clone(),
                        )
                    })
            } else {
                (
                    format!("harness:{:?}", row.harness),
                    row.harness_name.to_string(),
                    row.model.label.clone(),
                )
            };
            let group = *indices.entry(key.clone()).or_insert_with(|| {
                groups.push(ProviderGroup {
                    key,
                    label: label.into(),
                    rows: Vec::new(),
                });
                groups.len() - 1
            });
            groups[group].rows.push((index, model_label.into()));
        }
        // The provider settings revision can arrive before the refreshed model catalog.
        // Keep each provider's children in the order the user saved in either case.
        for group in &mut groups {
            if group.key.starts_with("native:") {
                group.rows.sort_by_key(|(index, _)| {
                    model_order
                        .get(rows[*index].model.id.as_str())
                        .copied()
                        .unwrap_or(usize::MAX)
                });
            }
        }
        let groups = std::sync::Arc::new(groups);
        *self.provider_menu.cache.borrow_mut() = Some((key, revision.to_owned(), groups.clone()));
        groups
    }
    pub(super) fn displayed_model_count(&self, cx: &App) -> usize {
        if self.provider_mode(cx) {
            self.provider_groups(cx).len()
        } else {
            self.model_rows_len(cx)
        }
    }
    fn open_provider(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(group) = self.provider_groups(cx).get(index).cloned() else {
            return;
        };
        if self.provider_menu.open.as_deref() != Some(&group.key) {
            self.provider_menu.active = index;
            self.provider_menu.open = Some(group.key);
            self.provider_menu.bounds = None;
            let selected = self.selected_model_index(cx);
            let local = group
                .rows
                .iter()
                .position(|(i, _)| *i == selected)
                .unwrap_or(0);
            self.active = group.rows[local].0;
            self.model_scroll
                .scroll_to_item(local, gpui::ScrollStrategy::Nearest);
            cx.notify();
        }
    }
    pub(super) fn activate_provider_choice(&mut self, cx: &mut Context<Self>) {
        let groups = self.provider_groups(cx);
        if let Some(group) = groups
            .iter()
            .find(|group| self.provider_menu.open.as_deref() == Some(&group.key))
        {
            if group.rows.iter().any(|(index, _)| *index == self.active) {
                self.activate_model_index(self.active, cx);
                self.provider_menu.close();
            }
        } else {
            if !self.compact_model_picker(cx)
                && let Some(index) = self.provider_menu.active.checked_sub(groups.len())
                && let Some(group) = self.setting_groups(cx).get(index)
            {
                self.open_setting(SettingScope::Tray, group.id.clone(), cx);
                return;
            }
            self.open_provider(
                self.provider_menu
                    .active
                    .min(groups.len().saturating_sub(1)),
                cx,
            );
        }
    }
    pub(super) fn provider_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if !self.provider_mode(cx) || (self.compact_model_picker(cx) && !self.compact_model_list) {
            return false;
        }
        let groups = self.provider_groups(cx);
        let child = groups
            .iter()
            .position(|group| self.provider_menu.open.as_deref() == Some(&group.key));
        self.provider_menu.hover.cancel();
        match event.keystroke.key.as_str() {
            "up" | "down" => {
                let delta = if event.keystroke.key == "up" { -1 } else { 1 };
                if let Some(index) = child {
                    let group = &groups[index];
                    let active = group
                        .rows
                        .iter()
                        .position(|(index, _)| *index == self.active);
                    if let Some(local) = popover::menu_step(active, group.rows.len(), delta) {
                        self.active = group.rows[local].0;
                        self.model_scroll
                            .scroll_to_item(local, gpui::ScrollStrategy::Nearest);
                    }
                } else if let Some(index) = popover::menu_step(
                    Some(self.provider_menu.active),
                    groups.len()
                        + if self.compact_model_picker(cx) {
                            0
                        } else {
                            self.setting_groups(cx).len()
                        },
                    delta,
                ) {
                    self.provider_menu.active = index;
                    if index < groups.len() {
                        self.provider_menu.scroll.scroll_to_item(index);
                    } else {
                        self.active = self.model_rows_len(cx) + index - groups.len();
                    }
                }
            }
            "home" | "end" => {
                if let Some(index) = child {
                    let group = &groups[index];
                    let local = if event.keystroke.key == "home" {
                        0
                    } else {
                        group.rows.len().saturating_sub(1)
                    };
                    self.active = group.rows[local].0;
                    self.model_scroll
                        .scroll_to_item(local, gpui::ScrollStrategy::Nearest);
                } else {
                    let count = groups.len()
                        + if self.compact_model_picker(cx) {
                            0
                        } else {
                            self.setting_groups(cx).len()
                        };
                    self.provider_menu.active = if event.keystroke.key == "home" {
                        0
                    } else {
                        count.saturating_sub(1)
                    };
                    if self.provider_menu.active >= groups.len() {
                        self.active =
                            self.model_rows_len(cx) + self.provider_menu.active - groups.len();
                    } else {
                        self.provider_menu
                            .scroll
                            .scroll_to_item(self.provider_menu.active);
                    }
                }
            }
            "right" if child.is_none() => self.activate_provider_choice(cx),
            "enter" | "space" => self.activate_provider_choice(cx),
            "escape" | "left" if child.is_some() => self.provider_menu.close(),
            "escape" if self.compact_model_picker(cx) => self.show_compact_panel(cx),
            "escape" => self.animate_close(cx),
            _ => return false,
        }
        cx.stop_propagation();
        cx.notify();
        true
    }
    fn hover_provider(
        &mut self,
        index: usize,
        pointer: gpui::Point<gpui::Pixels>,
        moved: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.is_open() {
            return;
        }
        let groups = self.provider_groups(cx);
        let current = groups
            .iter()
            .position(|group| self.provider_menu.open.as_deref() == Some(&group.key));
        let state = &mut self.provider_menu;
        let action = if moved && (current == Some(index) || state.hover.pending() == Some(&index)) {
            state
                .hover
                .moved(current.as_ref(), &index, pointer, state.bounds, state.left)
        } else {
            state
                .hover
                .enter(current.as_ref(), &index, pointer, state.bounds, state.left)
        };
        match action {
            popover::HoverAction::Open => self.open_provider(index, cx),
            popover::HoverAction::Defer => state.hover.defer(cx, move |this, cx| {
                if this.is_open() && this.provider_menu.hover.pending() == Some(&index) {
                    this.provider_menu.hover.cancel();
                    this.open_provider(index, cx);
                }
            }),
            _ => {}
        }
    }
    pub(super) fn provider_menu_height(&self, cx: &App) -> f32 {
        let providers = self.provider_groups(cx).len().max(1).min(8) as f32;
        let settings = if self.compact_model_picker(cx) {
            0.0
        } else {
            model_menu_budgets(self.menu_geometry().height, self.setting_groups(cx).len()).1
        };
        providers * 32.0 + popover::CARD_INSET * 2.0 + 2.0 + settings
    }

    pub(super) fn render_provider_popover(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).for_popup();
        let groups = self.provider_groups(cx);
        let rows = self.model_rows(cx);
        if self
            .provider_menu
            .open
            .as_ref()
            .is_some_and(|key| !groups.iter().any(|group| &group.key == key))
        {
            self.provider_menu.close();
        }
        let mut menu = div().h_full().flex().flex_col();
        let mut list = div()
            .id("conversation-provider-list")
            .debug_selector(|| "conversation-provider-list".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.provider_menu.scroll)
            .px(px(popover::CARD_INSET))
            .py(px(popover::CARD_INSET));
        for (index, group) in groups.iter().enumerate() {
            let open = self.provider_menu.open.as_deref() == Some(&group.key);
            let selected = group
                .rows
                .iter()
                .any(|(i, _)| self.row_is_selected(&rows[*i], cx));
            let id: SharedString = format!("conversation-provider-{index}").into();
            let entity = cx.entity().downgrade();
            let mut row = popover::menu_row(
                &theme,
                open || self.provider_menu.active == index,
                id.clone(),
            )
            .id(id.clone())
            .debug_selector(move || id.to_string())
            .relative()
            .h(px(32.0))
            .py_0()
            .role(gpui::Role::MenuItem)
            .aria_label(group.label.clone())
            .aria_expanded(open)
            .aria_selected(selected)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.provider_menu.hover.cancel();
                this.open_provider(index, cx);
                cx.stop_propagation();
            }))
            .on_mouse_move(
                cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                    this.hover_provider(index, event.position, true, cx)
                }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .debug_selector(move || format!("provider-label-{index}"))
                            .child(group.label.clone()),
                    )
                    .when(selected, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .size(px(14.0))
                                .debug_selector(move || format!("provider-selected-{index}"))
                                .child(
                                    crate::icons::icon(crate::icons::CHECK)
                                        .size(px(14.0))
                                        .text_color(theme.text),
                                ),
                        )
                    }),
            )
            .child(
                crate::icons::icon(crate::icons::ALT_ARROW_RIGHT)
                    .size(px(12.0))
                    .text_color(theme.text_muted),
            )
            .child(
                gpui::canvas(
                    move |bounds, window, cx| {
                        let left = bounds.right() + px(300.0) > window.viewport_size().width;
                        let _ = entity.update(cx, |this, cx| {
                            if this.provider_menu.left != left {
                                this.provider_menu.left = left;
                                cx.notify();
                            }
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            );
            if open {
                let members = std::sync::Arc::new(group.rows.clone());
                let data = rows.clone();
                let entity = cx.entity();
                let models = gpui::uniform_list(
                    "conversation-provider-models",
                    members.len(),
                    move |range, _, app| {
                        entity.update(app, |this, cx| {
                            range
                                .filter_map(|local| {
                                    let (index, label) = members.get(local)?;
                                    let mut row = data.get(*index)?.clone();
                                    if row.harness == HarnessId::Native {
                                        row.model.label = label.to_string();
                                        row.ambiguous = false;
                                    }
                                    let id: SharedString =
                                        format!("conversation-provider-model-{index}").into();
                                    Some(
                                        div()
                                            .id(id.clone())
                                            .debug_selector(move || id.to_string())
                                            .on_click(
                                                cx.listener(|_, _, _, cx| cx.stop_propagation()),
                                            )
                                            .child(this.render_model_row(*index, &row, cx))
                                            .into_any_element(),
                                    )
                                })
                                .collect::<Vec<_>>()
                        })
                    },
                )
                .h_full()
                .track_scroll(&self.model_scroll);
                let entity = cx.entity().downgrade();
                let height = (group.rows.len() as f32
                    * if self.compact_model_picker(cx) {
                        34.0
                    } else {
                        54.0
                    }
                    + 8.0)
                    .min(320.0)
                    .min((self.menu_geometry().height - 8.0).max(40.0));
                let child = popover::popover_card(&theme)
                    .w(px(288.0))
                    .h(px(height))
                    .relative()
                    .on_mouse_move(cx.listener(|this, _, _, _| this.provider_menu.hover.cancel()))
                    .child(models)
                    .child(
                        gpui::canvas(
                            move |bounds, _, cx| {
                                let _ = entity
                                    .update(cx, |this, _| this.provider_menu.bounds = Some(bounds));
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    );
                row = row.child(popover::nested_menu(
                    format!("provider-model-menu-{index}"),
                    child.into_any_element(),
                    self.provider_menu.left,
                ));
            }
            list = list.child(row);
        }
        if groups.is_empty() {
            list = list.child(
                div()
                    .p(px(12.0))
                    .text_color(theme.text_muted)
                    .child(crate::l10n::localizer().text(TextKey::PickersModelsEmpty)),
            );
        }
        menu = menu.child(list);
        if !self.compact_model_picker(cx) {
            menu = menu.child(self.render_traits_sections(cx));
        }
        menu.into_any_element()
    }
}
