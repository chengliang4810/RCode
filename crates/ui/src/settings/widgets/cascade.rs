//! Two-level choices using the same floating submenu as model context-window settings.
use super::*;

#[derive(Clone)]
pub struct CascadeItem {
    pub label: SharedString,
    pub value: Option<String>,
    pub children: Vec<CascadeItem>,
}
impl CascadeItem {
    pub fn choice(value: impl Into<String>, label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            value: Some(value.into()),
            children: vec![],
        }
    }
    pub fn group(label: impl Into<SharedString>, children: Vec<Self>) -> Self {
        Self {
            label: label.into(),
            value: None,
            children,
        }
    }
    fn contains(&self, selected: &str) -> bool {
        self.value.as_deref() == Some(selected)
            || self
                .children
                .iter()
                .any(|child| child.value.as_deref() == Some(selected))
    }
}

#[derive(Default)]
pub struct CascadeState {
    menu: popover::Popup<()>,
    active: usize,
    child: Option<usize>,
    child_active: usize,
    child_bounds: Option<gpui::Bounds<Pixels>>,
    left: bool,
    hover: popover::HoverIntent<usize>,
}
impl CascadeState {
    fn open(&mut self, items: &[CascadeItem], selected: &str) {
        self.active = items
            .iter()
            .position(|item| item.contains(selected))
            .unwrap_or(0);
        self.child = None;
        self.child_bounds = None;
        self.hover.reset();
        self.menu.open(());
    }
    fn open_child(&mut self, index: usize, items: &[CascadeItem], selected: &str) {
        self.active = index;
        let next = items
            .get(index)
            .filter(|item| !item.children.is_empty())
            .map(|_| index);
        if self.child != next {
            self.child = next;
            self.child_bounds = None;
            self.child_active = items
                .get(index)
                .and_then(|item| {
                    item.children
                        .iter()
                        .position(|child| child.contains(selected))
                })
                .unwrap_or(0);
        }
    }
    fn value(&self, items: &[CascadeItem]) -> Option<String> {
        let item = if let Some(index) = self.child {
            items.get(index)?.children.get(self.child_active)?
        } else {
            items.get(self.active)?
        };
        item.value.clone()
    }
}
type Reach<V> = Rc<dyn Fn(&mut V) -> &mut CascadeState>;
type Handler<V> = Rc<dyn Fn(&mut V, &str, &mut Window, &mut Context<V>)>;

pub fn cascade<V: 'static>(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    theme: &Theme,
    reach: impl Fn(&mut V) -> &mut CascadeState + 'static,
) -> Cascade<V> {
    Cascade {
        id: id.into(),
        label: label.into(),
        theme: theme.clone(),
        reach: Rc::new(reach),
        items: vec![],
        selected: String::new(),
        placeholder: SharedString::default(),
        width: 230.0,
        enabled: true,
        on_select: None,
        show_group: true,
    }
}
pub struct Cascade<V: 'static> {
    id: SharedString,
    label: SharedString,
    theme: Theme,
    reach: Reach<V>,
    items: Vec<CascadeItem>,
    selected: String,
    placeholder: SharedString,
    width: f32,
    enabled: bool,
    on_select: Option<Handler<V>>,
    show_group: bool,
}
fn close<V: 'static>(view: &mut V, reach: &Reach<V>, cx: &mut Context<V>) {
    let state = reach(view);
    state.hover.reset();
    if state.menu.begin_close() {
        let reach = reach.clone();
        popover::reap_popup(cx, move |view: &mut V| &mut reach(view).menu);
    }
    cx.notify();
}
fn commit<V: 'static>(
    view: &mut V,
    value: &str,
    reach: &Reach<V>,
    handler: &Option<Handler<V>>,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    close(view, reach, cx);
    if let Some(handler) = handler {
        handler(view, value, window, cx);
    }
}
impl<V: 'static> Cascade<V> {
    pub fn show_group(mut self, show: bool) -> Self {
        self.show_group = show;
        self
    }
    pub fn options(mut self, items: Vec<CascadeItem>, selected: &str) -> Self {
        self.items = items;
        self.selected = selected.to_owned();
        self
    }
    pub fn placeholder(mut self, label: impl Into<SharedString>) -> Self {
        self.placeholder = label.into();
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn on_select(
        mut self,
        handler: impl Fn(&mut V, &str, &mut Window, &mut Context<V>) + 'static,
    ) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }
    pub fn render(self, state: &CascadeState, cx: &mut Context<V>) -> gpui::Stateful<gpui::Div> {
        let Self {
            id,
            label,
            theme,
            reach,
            items,
            selected,
            placeholder,
            width,
            enabled,
            on_select,
            show_group,
        } = self;
        let items = Rc::new(items);
        let selected: Rc<str> = selected.into();
        let value = items
            .iter()
            .find_map(|item| {
                if item.value.as_deref() == Some(&*selected) {
                    return Some(item.label.clone());
                }
                item.children
                    .iter()
                    .find(|child| child.value.as_deref() == Some(&*selected))
                    .map(|child| {
                        if show_group {
                            format!("{} / {}", item.label, child.label).into()
                        } else {
                            child.label.clone()
                        }
                    })
            })
            .unwrap_or(placeholder);
        let mut trigger = select_trigger(&theme, id.clone(), state.menu.is_open())
            .debug_selector({
                let id = id.clone();
                move || id.to_string()
            })
            .w(px(width))
            .aria_label(format!("{label}: {value}"))
            .when(!enabled, |el| el.opacity(0.4))
            .child(select_value(value))
            .child(select_chevron(&theme, state.menu.is_open()));
        if !enabled {
            return trigger;
        }
        trigger = trigger
            .on_mouse_down(gpui::MouseButton::Left, {
                let reach = reach.clone();
                cx.listener(move |view, _, _, _| reach(view).menu.note_trigger_press())
            })
            .on_click({
                let reach = reach.clone();
                let items = items.clone();
                let selected = selected.clone();
                let handler = on_select.clone();
                cx.listener(move |view, event: &gpui::ClickEvent, window, cx| {
                    let state = reach(view);
                    if event.is_keyboard() && state.menu.is_open() {
                        if let Some(value) = state.value(&items) {
                            commit(view, &value, &reach, &handler, window, cx);
                        } else {
                            state.open_child(state.active, &items, &selected);
                            cx.notify();
                        }
                    } else if state.menu.take_press_was_open() || state.menu.is_open() {
                        close(view, &reach, cx);
                    } else {
                        state.open(&items, &selected);
                        cx.notify();
                    }
                })
            })
            .on_key_down({
                let reach = reach.clone();
                let items = items.clone();
                let selected = selected.clone();
                cx.listener(move |view, event: &gpui::KeyDownEvent, _, cx| {
                    let key = event.keystroke.key.as_str();
                    let state = reach(view);
                    if !state.menu.is_open() {
                        if key == "up" || key == "down" {
                            state.open(&items, &selected);
                        } else {
                            return;
                        }
                    } else {
                        state.hover.reset();
                        let count = state
                            .child
                            .and_then(|index| items.get(index))
                            .map_or(items.len(), |item| item.children.len());
                        let active = if state.child.is_some() {
                            &mut state.child_active
                        } else {
                            &mut state.active
                        };
                        match key {
                            "up" | "down" => {
                                *active = popover::menu_step(
                                    Some(*active),
                                    count,
                                    if key == "up" { -1 } else { 1 },
                                )
                                .unwrap_or(0)
                            }
                            "home" => *active = 0,
                            "end" => *active = count.saturating_sub(1),
                            "right" => state.open_child(state.active, &items, &selected),
                            "left" | "escape" if state.child.is_some() => {
                                state.child = None;
                                state.child_bounds = None;
                            }
                            "escape" => {
                                cx.stop_propagation();
                                close(view, &reach, cx);
                                return;
                            }
                            _ => return,
                        }
                    }
                    cx.stop_propagation();
                    cx.notify();
                })
            });
        if state.menu.get().is_none() {
            return trigger;
        }
        let mut rows = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let branch = !item.children.is_empty();
            let row_id: SharedString = format!("{id}-option-{index}").into();
            let mut row = select_row(
                &theme,
                item.contains(&selected),
                index == state.active,
                row_id.clone(),
            )
            .id(row_id.clone())
            .debug_selector(move || row_id.to_string())
            .role(if branch {
                gpui::Role::MenuItem
            } else {
                gpui::Role::MenuItemRadio
            })
            .aria_label(item.label.clone())
            .aria_expanded(branch && state.child == Some(index))
            .aria_toggled(if item.contains(&selected) {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .on_click({
                let reach = reach.clone();
                let items = items.clone();
                let selected = selected.clone();
                let handler = on_select.clone();
                cx.listener(move |view, _, window, cx| {
                    cx.stop_propagation();
                    if !reach(view).menu.is_open() {
                        return;
                    }
                    reach(view).hover.reset();
                    if let Some(value) = &items[index].value {
                        commit(view, value, &reach, &handler, window, cx);
                    } else {
                        reach(view).open_child(index, &items, &selected);
                        cx.notify();
                    }
                })
            })
            .on_mouse_move({
                let reach = reach.clone();
                let items = items.clone();
                let selected = selected.clone();
                cx.listener(move |view, event: &gpui::MouseMoveEvent, _, cx| {
                    let state = reach(view);
                    if !state.menu.is_open() {
                        return;
                    }
                    let action = if state.hover.pending() == Some(&index) {
                        state.hover.moved(
                            state.child.as_ref(),
                            &index,
                            event.position,
                            state.child_bounds,
                            state.left,
                        )
                    } else {
                        state.hover.enter(
                            state.child.as_ref(),
                            &index,
                            event.position,
                            state.child_bounds,
                            state.left,
                        )
                    };
                    match action {
                        popover::HoverAction::Open => {
                            state.open_child(index, &items, &selected);
                            cx.notify();
                        }
                        popover::HoverAction::Defer => {
                            let reach = reach.clone();
                            let items = items.clone();
                            let selected = selected.clone();
                            state.hover.defer(cx, move |view, cx| {
                                let state = reach(view);
                                if state.menu.is_open() && state.hover.pending() == Some(&index) {
                                    state.open_child(index, &items, &selected);
                                    state.hover.cancel();
                                    cx.notify();
                                }
                            });
                        }
                        _ => {}
                    }
                })
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(item.label.clone()),
            );
            if branch {
                row = row
                    .relative()
                    .child(select_check(&theme, item.contains(&selected)))
                    .child(
                        crate::icons::icon(crate::icons::ALT_ARROW_RIGHT)
                            .size(px(12.0))
                            .text_color(theme.text_muted),
                    );
                let entity = cx.entity().downgrade();
                let measure_reach = reach.clone();
                row = row.child(
                    gpui::canvas(
                        move |bounds, window, cx| {
                            let limits = dropdown_limits(window.viewport_size(), sidebar_width(cx));
                            let left = bounds.right() + px(264.0) > limits.right()
                                && bounds.left() - px(264.0) >= limits.left();
                            let _ = entity.update(cx, |view, cx| {
                                let state = measure_reach(view);
                                if state.left != left {
                                    state.left = left;
                                    cx.notify();
                                }
                            });
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                );
                if state.child == Some(index) {
                    let mut children = Vec::new();
                    for (child_index, child) in item.children.iter().enumerate() {
                        let child_id: SharedString =
                            format!("{id}-child-{index}-{child_index}").into();
                        let chosen = child.value.as_deref() == Some(&*selected);
                        let child_value = child.value.clone().unwrap_or_default();
                        children.push(
                            select_row(
                                &theme,
                                chosen,
                                state.child_active == child_index,
                                child_id.clone(),
                            )
                            .id(child_id.clone())
                            .debug_selector(move || child_id.to_string())
                            .role(gpui::Role::MenuItemRadio)
                            .aria_label(child.label.clone())
                            .aria_toggled(if chosen {
                                gpui::Toggled::True
                            } else {
                                gpui::Toggled::False
                            })
                            .on_mouse_move({
                                let reach = reach.clone();
                                cx.listener(move |view, _, _, cx| {
                                    let state = reach(view);
                                    if !state.menu.is_open() {
                                        return;
                                    }
                                    state.hover.cancel();
                                    if state.child_active != child_index {
                                        state.child_active = child_index;
                                        cx.notify();
                                    }
                                })
                            })
                            .on_click({
                                let reach = reach.clone();
                                let handler = on_select.clone();
                                cx.listener(move |view, _, window, cx| {
                                    cx.stop_propagation();
                                    if !reach(view).menu.is_open() {
                                        return;
                                    }
                                    commit(view, &child_value, &reach, &handler, window, cx);
                                })
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(child.label.clone()),
                            )
                            .child(select_check(&theme, chosen))
                            .into_any_element(),
                        );
                    }
                    let entity = cx.entity().downgrade();
                    let measure_reach = reach.clone();
                    let menu = select_menu(&theme)
                        .w(px(252.0))
                        .relative()
                        .child(dropdown_rows(
                            format!("{id}-children"),
                            children,
                            SELECT_HEIGHT,
                            8.0,
                        ))
                        .child(
                            gpui::canvas(
                                move |bounds, _, cx| {
                                    let _ = entity.update(cx, |view, _| {
                                        measure_reach(view).child_bounds = Some(bounds)
                                    });
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .inset_0(),
                        );
                    row = row.child(popover::nested_menu(
                        format!("{id}-child-menu"),
                        menu.into_any_element(),
                        state.left,
                    ));
                }
            } else {
                row = row.child(select_check(&theme, item.contains(&selected)));
            }
            rows.push(row.into_any_element());
        }
        let menu = select_menu(&theme)
            .w(px(252.0))
            .on_mouse_down_out({
                let reach = reach.clone();
                cx.listener(move |view, event: &gpui::MouseDownEvent, _, cx| {
                    if !reach(view)
                        .child_bounds
                        .is_some_and(|bounds| bounds.contains(&event.position))
                    {
                        close(view, &reach, cx);
                    }
                })
            })
            .child(dropdown_rows(
                format!("{id}-list"),
                rows,
                SELECT_HEIGHT,
                8.0,
            ));
        trigger.child(SettingsDropdown {
            id: format!("{id}-menu").into(),
            content: menu,
            closing: state.menu.closing_since(),
            trigger_height: SELECT_HEIGHT,
            align: popover::MenuAlign::End,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn items() -> Vec<CascadeItem> {
        vec![
            CascadeItem::choice("inherit", "Inherit"),
            CascadeItem::group(
                "Provider A",
                vec![
                    CascadeItem::choice("a-1", "Same model"),
                    CascadeItem::choice("a-2", "Other model"),
                ],
            ),
            CascadeItem::group("Provider B", vec![CascadeItem::choice("b-1", "Same model")]),
        ]
    }
    struct Fixture {
        state: CascadeState,
        selected: String,
        left: bool,
        commits: usize,
    }
    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let picker = cascade("cascade-test", "Model", Theme::of(cx), |view: &mut Self| {
                &mut view.state
            })
            .options(items(), &self.selected)
            .on_select(|view, value, _, cx| {
                view.selected = value.to_owned();
                view.commits += 1;
                cx.notify();
            })
            .render(&self.state, cx);
            div().size_full().relative().child(
                div()
                    .absolute()
                    .top(px(120.0))
                    .when(self.left, |el| el.right(px(16.0)))
                    .when(!self.left, |el| el.left(px(300.0)))
                    .child(picker),
            )
        }
    }
    fn redraw(cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
            window.draw(cx).clear();
        });
    }
    #[gpui::test]
    fn provider_children_select_on_either_side_without_outside_dismissal(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
        });
        for left in [false, true] {
            let window = cx.add_window(|_, _| Fixture {
                state: Default::default(),
                selected: "a-1".into(),
                left,
                commits: 0,
            });
            let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
            redraw(&mut visual);
            let trigger = visual.debug_bounds("cascade-test").unwrap().center();
            visual.simulate_click(trigger, Default::default());
            redraw(&mut visual);
            let provider = visual.debug_bounds("cascade-test-option-1").unwrap();
            visual.simulate_event(gpui::MouseMoveEvent {
                position: provider.center(),
                ..Default::default()
            });
            redraw(&mut visual);
            let child = visual.debug_bounds("cascade-test-child-1-1").unwrap();
            window
                .read_with(&visual, |view, _| {
                    assert_eq!(view.state.child, Some(1));
                    assert_eq!(view.state.left, left);
                    let bounds = view.state.child_bounds.unwrap();
                    if left {
                        assert!(bounds.right() < provider.left());
                    } else {
                        assert!(bounds.left() > provider.right());
                    }
                })
                .unwrap();
            visual.simulate_click(child.center(), Default::default());
            redraw(&mut visual);
            window
                .read_with(&visual, |view, _| {
                    assert_eq!(view.selected, "a-2");
                    assert_eq!(view.commits, 1);
                    assert!(!view.state.menu.is_open());
                })
                .unwrap();
            // A second supplier with the same display name must deliver its own identity.
            // The shared popup exit animation is covered separately; finish it for this fresh press.
            window
                .update(&mut visual, |view, _, cx| {
                    view.state.menu = Default::default();
                    cx.notify();
                })
                .unwrap();
            redraw(&mut visual);
            visual.simulate_click(trigger, Default::default());
            redraw(&mut visual);
            let provider_b = visual
                .debug_bounds("cascade-test-option-2")
                .unwrap()
                .center();
            visual.simulate_click(provider_b, Default::default());
            redraw(&mut visual);
            let child_b = visual
                .debug_bounds("cascade-test-child-2-0")
                .unwrap()
                .center();
            visual.simulate_click(child_b, Default::default());
            redraw(&mut visual);
            window
                .read_with(&visual, |view, _| assert_eq!(view.selected, "b-1"))
                .unwrap();
            window
                .update(&mut visual, |view, _, cx| {
                    view.state.menu = Default::default();
                    cx.notify();
                })
                .unwrap();
            redraw(&mut visual);
            visual.simulate_click(trigger, Default::default());
            redraw(&mut visual);
            visual.simulate_click(gpui::point(px(12.0), px(12.0)), Default::default());
            redraw(&mut visual);
            window
                .read_with(&visual, |view, _| {
                    assert!(!view.state.menu.is_open());
                    assert_eq!(view.commits, 2);
                })
                .unwrap();
        }
    }
    #[gpui::test]
    fn keyboard_enters_and_leaves_provider_before_committing_model(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::dark());
        });
        let window = cx.add_window(|_, _| Fixture {
            state: Default::default(),
            selected: "a-1".into(),
            left: false,
            commits: 0,
        });
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        redraw(&mut visual);
        let trigger = visual.debug_bounds("cascade-test").unwrap().center();
        visual.simulate_click(trigger, Default::default());
        visual.simulate_keystrokes("right down");
        redraw(&mut visual);
        window
            .read_with(&visual, |view, _| {
                assert_eq!(view.state.child, Some(1));
                assert_eq!(view.state.child_active, 1);
                assert_eq!(view.commits, 0);
            })
            .unwrap();
        visual.simulate_keystrokes("escape");
        window
            .read_with(&visual, |view, _| {
                assert!(view.state.menu.is_open());
                assert!(view.state.child.is_none());
            })
            .unwrap();
        visual.simulate_keystrokes("right down enter");
        // GPUI's button activation commits on key-up; simulate_keystrokes sends only key-down.
        visual.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
        });
        redraw(&mut visual);
        window
            .read_with(&visual, |view, _| {
                assert_eq!(view.selected, "a-2");
                assert_eq!(view.commits, 1);
            })
            .unwrap();
    }
}
