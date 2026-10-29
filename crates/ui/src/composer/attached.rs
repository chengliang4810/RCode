//! Clip hidden strips without changing a panel's intrinsic content layout.
use gpui::{
    AnyElement, App, Bounds, ContentMask, Element, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, Styled, Window, div, prelude::*, px,
};

/// The margin reserves only the visible height. The element clips both paint
/// and hitboxes: painting another glass surface on top cannot hide this strip.
pub(crate) fn clipped_composer_panel(child: impl IntoElement, hidden: f32) -> gpui::Div {
    div().mb(px(-hidden)).child(BottomClip {
        child: child.into_any_element(),
        hidden: px(hidden),
    })
}

struct BottomClip {
    child: AnyElement,
    hidden: Pixels,
}

impl BottomClip {
    fn mask(&self, bounds: Bounds<Pixels>, window: &Window) -> ContentMask<Pixels> {
        // Preserve the parent's other edges so shadows and overlays above or
        // beside the panel retain their original clipping behavior.
        let mut mask = window.content_mask();
        let bottom = (bounds.bottom() - self.hidden).min(mask.bounds.bottom());
        mask.bounds.size.height = (bottom - mask.bounds.top()).max(px(0.0));
        mask
    }
}

impl Element for BottomClip {
    type RequestLayoutState = ();
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
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(self.mask(bounds, window)), |window| {
            self.child.prepaint(window, cx)
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(self.mask(bounds, window)), |window| {
            self.child.paint(window, cx)
        });
    }
}

impl IntoElement for BottomClip {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
