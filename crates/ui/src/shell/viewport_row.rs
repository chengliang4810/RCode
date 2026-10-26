use gpui::{AnyElement, App, AvailableSpace, IntoElement, Window, canvas, prelude::*, px, size};

pub(super) fn viewport_row(
    height: f32,
    force_visible: bool,
    render: impl 'static + FnOnce(&mut Window, &mut App) -> AnyElement,
) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            let visible = bounds.intersect(&window.content_mask().bounds);
            if !force_visible && (visible.size.width <= px(0.0) || visible.size.height <= px(0.0)) {
                return None;
            }
            let mut child = gpui::div()
                .w(bounds.size.width)
                .h(bounds.size.height)
                .flex()
                .flex_col()
                .child(render(window, cx))
                .into_any_element();
            child.layout_as_root(
                size(
                    AvailableSpace::Definite(bounds.size.width),
                    AvailableSpace::Definite(bounds.size.height),
                ),
                window,
                cx,
            );
            child.prepaint_at(bounds.origin, window, cx);
            Some(child)
        },
        |_, child, window, cx| {
            if let Some(mut child) = child {
                child.paint(window, cx);
            }
        },
    )
    .w_full()
    .h(px(height))
    .flex_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, MouseButton, Render, ScrollHandle, TestAppContext, point};
    use std::{cell::RefCell, rc::Rc};

    struct RowHost {
        rendered: Rc<RefCell<Vec<usize>>>,
        clicked: Rc<RefCell<Vec<usize>>>,
        scroll: ScrollHandle,
        collapsed: bool,
        force_last: bool,
    }

    impl Render for RowHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div().w(px(280.0)).h(px(220.0)).child(
                gpui::div()
                    .id("viewport")
                    .w_full()
                    .h(px(if self.collapsed { 0.0 } else { 220.0 }))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .children((0..500).map(|index| {
                        let rendered = self.rendered.clone();
                        let clicked = self.clicked.clone();
                        viewport_row(40.0, self.force_last && index == 499, move |_, _| {
                            rendered.borrow_mut().push(index);
                            gpui::div()
                                .id(("row", index))
                                .debug_selector(move || format!("viewport-row-{index}"))
                                .h(px(40.0))
                                .on_click(move |_, _, _| clicked.borrow_mut().push(index))
                                .into_any_element()
                        })
                    })),
            )
        }
    }

    #[gpui::test]
    fn renders_visible_rows_and_preserves_scroll_geometry_and_clicks(cx: &mut TestAppContext) {
        let rendered = Rc::new(RefCell::new(Vec::new()));
        let clicked = Rc::new(RefCell::new(Vec::new()));
        let scroll = ScrollHandle::new();
        let (host, cx) = cx.add_window_view(|_, _| RowHost {
            rendered: rendered.clone(),
            clicked: clicked.clone(),
            scroll: scroll.clone(),
            collapsed: false,
            force_last: false,
        });
        assert_eq!(*rendered.borrow(), (0..6).collect::<Vec<_>>());
        let first = cx.debug_bounds("viewport-row-0").unwrap();
        let next = cx.debug_bounds("viewport-row-1").unwrap();
        assert_eq!(first.size.height, px(40.0));
        assert_eq!(first.size.width, px(280.0));
        assert_eq!(next.top() - first.top(), px(42.0));
        cx.simulate_click(first.center(), gpui::Modifiers::default());
        assert_eq!(*clicked.borrow(), vec![0]);
        rendered.borrow_mut().clear();
        scroll.set_offset(point(px(0.0), px(-42.0 * 500.0)));
        host.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert!(rendered.borrow().contains(&499));
        assert!(rendered.borrow().iter().all(|index| *index >= 494));
        let last = cx.debug_bounds("viewport-row-499").unwrap();
        cx.simulate_mouse_down(last.center(), MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_up(last.center(), MouseButton::Left, gpui::Modifiers::default());
        assert_eq!(*clicked.borrow(), vec![0, 499]);
        rendered.borrow_mut().clear();
        host.update(cx, |host, cx| {
            host.collapsed = true;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(rendered.borrow().is_empty());
        rendered.borrow_mut().clear();
        scroll.set_offset(point(px(0.0), px(0.0)));
        host.update(cx, |host, cx| {
            host.collapsed = false;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(rendered.borrow().contains(&0));
        assert!(rendered.borrow().iter().all(|index| *index < 6));
    }

    #[gpui::test]
    fn forced_row_remains_mounted_under_a_collapsed_mask(cx: &mut TestAppContext) {
        let rendered = Rc::new(RefCell::new(Vec::new()));
        let (_, _) = cx.add_window_view(|_, _| RowHost {
            rendered: rendered.clone(),
            clicked: Rc::new(RefCell::new(Vec::new())),
            scroll: ScrollHandle::new(),
            collapsed: true,
            force_last: true,
        });
        assert_eq!(*rendered.borrow(), vec![499]);
    }
}
