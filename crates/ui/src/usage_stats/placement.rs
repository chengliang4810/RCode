//! Position statistics from the trigger rectangle, independently of the pointer.
use gpui::{
    AnyElement, App, Bounds, Element, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Point, Position, Size, Style, Window, point, px,
};
use std::{cell::Cell, rc::Rc};

pub(crate) type PanelBounds = Rc<Cell<Option<Bounds<Pixels>>>>;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Align {
    Start,
    End,
}

fn origin(
    trigger: Bounds<Pixels>,
    size: Size<Pixels>,
    viewport: Size<Pixels>,
    align: Align,
) -> Point<Pixels> {
    let gap = px(8.0);
    let margin = px(8.0);
    let above = trigger.top() - gap - size.height;
    let y = if above >= margin {
        above
    } else {
        trigger.bottom() + gap
    };
    let x = match align {
        Align::Start => trigger.left(),
        Align::End => trigger.right() - size.width,
    };
    point(
        x.clamp(margin, (viewport.width - margin - size.width).max(margin)),
        y,
    )
}

pub(crate) struct Panel {
    content: AnyElement,
    trigger: PanelBounds,
    bounds: PanelBounds,
    align: Align,
}

impl Panel {
    pub(crate) fn new(
        content: AnyElement,
        trigger: PanelBounds,
        bounds: PanelBounds,
        align: Align,
    ) -> Self {
        Self {
            content,
            trigger,
            bounds,
            align,
        }
    }
}

impl IntoElement for Panel {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Panel {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, LayoutId) {
        let child = self.content.request_layout(window, cx);
        let layout = window.request_layout(
            Style {
                position: Position::Absolute,
                ..Default::default()
            },
            [child],
            cx,
        );
        (layout, child)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(trigger) = self.trigger.get() else {
            return;
        };
        let size = window.layout_bounds(*child).size;
        let origin = origin(trigger, size, window.viewport_size(), self.align);
        self.bounds.set(Some(Bounds::new(origin, size)));
        window.with_element_offset(origin - bounds.origin, |window| {
            self.content.prepaint(window, cx)
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut LayoutId,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.bounds.get().is_some() {
            self.content.paint(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    #[test]
    fn placements_keep_a_gap_and_fit_both_sides_and_horizontal_edges() {
        let viewport = size(px(600.0), px(400.0));
        let card = size(px(300.0), px(160.0));
        for x in [8.0, 300.0, 450.0] {
            for y in [8.0, 180.0, 370.0] {
                let trigger = Bounds::new(point(px(x), px(y)), size(px(140.0), px(22.0)));
                let panel = Bounds::new(origin(trigger, card, viewport, Align::End), card);
                assert!(panel.left() >= px(8.0) && panel.right() <= viewport.width - px(8.0));
                assert!(panel.top() >= px(8.0) && panel.bottom() <= viewport.height - px(8.0));
                assert!(
                    panel.bottom() <= trigger.top() - px(8.0)
                        || panel.top() >= trigger.bottom() + px(8.0)
                );
            }
        }
    }
}
