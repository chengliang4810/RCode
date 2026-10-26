//! [`frosted`] — the frosted-glass float: wraps a popover/dialog card so its
//! ENTIRE subtree paints inside one scene layer (a single draw order) with a
//! backdrop blur painted first.
//!
//! The single layer order is the point: with per-primitive bounds-tree
//! ordering, a hover repaint elsewhere could reassign the card's quads BELOW
//! the blur — washes, dividers, and borders intermittently got snapshotted and
//! blurred away (user reports). Inside one layer the blur/content relationship
//! is structural: blur first, then shadow, tint, border, rows, text.

use gpui::{
    AnyElement, App, Bounds, Corners, Element, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window, px,
};

use crate::theme::Theme;

/// Shared backdrop-blur sigma for floating menus, popovers and palettes.
/// Keep these on the same surface as Cmd+K and the composer.
pub const MENU_BLUR: f32 = 16.0;

/// Coverage for controls over an in-app wallpaper. Only the fill is translucent;
/// glyphs, icons and focus rings retain their theme colors and opacity.
#[derive(Clone, Copy)]
pub(crate) enum WallpaperSurface {
    Card,
    Input,
    Toolbar,
}

impl WallpaperSurface {
    pub(crate) fn background(
        self,
        theme: &Theme,
        enabled: bool,
        fallback: gpui::Hsla,
    ) -> gpui::Hsla {
        if !enabled {
            return fallback;
        }
        let dark = matches!(theme.appearance, crate::theme::Appearance::Dark);
        let mut tint = crate::theme::flatten(theme.input_bg, theme.bg);
        tint.a = match (self, dark) {
            (Self::Card, false) => 0.38,
            (Self::Card, true) => 0.55,
            (Self::Input, false) => 0.52,
            (Self::Input, true) => 0.68,
            (Self::Toolbar, false) => 0.20,
            (Self::Toolbar, true) => 0.35,
        };
        tint
    }
}

/// Frost `child` (a popover card): backdrop-blurred on glass, pass-through on
/// opaque platforms. `corner_radius` must match the card's rounding.
pub fn frosted(corner_radius: f32, blur_radius: f32, child: impl IntoElement) -> Frosted {
    Frosted {
        corner_radius,
        blur_radius,
        wallpaper: false,
        child: child.into_any_element(),
    }
}

pub struct Frosted {
    corner_radius: f32,
    blur_radius: f32,
    wallpaper: bool,
    child: AnyElement,
}

impl Frosted {
    /// In-app wallpaper needs local blur even when window glass is disabled.
    /// This does not alter the window material or other routes and popovers.
    pub(crate) fn with_wallpaper(mut self, enabled: bool) -> Self {
        self.wallpaper = enabled;
        self
    }
}

impl Element for Frosted {
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
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.wallpaper || Theme::of(cx).is_frost() {
            window.paint_layer(bounds, |window| {
                window.paint_backdrop_blur(
                    bounds,
                    Corners::all(px(self.corner_radius)),
                    px(self.blur_radius),
                );
                self.child.paint(window, cx);
            });
        } else {
            self.child.paint(window, cx);
        }
    }
}

impl IntoElement for Frosted {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Paint `child` in its own scene layer, giving it a fresh draw order above
/// everything painted so far in the enclosing layer.
///
/// Needed for overlays INSIDE a frosted card: the card's single layer means
/// every primitive shares one draw order, and equal orders render grouped by
/// primitive kind (quads, then icons, then images) — so a close button's
/// circle painted "after" a thumbnail still shows up UNDER the image. A
/// nested layer restores the intended stacking.
pub fn layered(child: impl IntoElement) -> Layered {
    Layered {
        child: child.into_any_element(),
    }
}

pub struct Layered {
    child: AnyElement,
}

impl Element for Layered {
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
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.paint_layer(bounds, |window| self.child.paint(window, cx));
    }
}

impl IntoElement for Layered {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[cfg(test)]
mod wallpaper_tests {
    use super::*;
    use gpui::{Context, Render, div, prelude::*};

    #[test]
    fn wallpaper_fills_keep_the_original_material_when_disabled() {
        for theme in [Theme::light(), Theme::default()] {
            let fallback = theme.composer_surface_bg();
            for role in [
                WallpaperSurface::Card,
                WallpaperSurface::Input,
                WallpaperSurface::Toolbar,
            ] {
                assert_eq!(role.background(&theme, false, fallback), fallback);
            }
            let card = WallpaperSurface::Card.background(&theme, true, fallback);
            let input = WallpaperSurface::Input.background(&theme, true, fallback);
            let toolbar = WallpaperSurface::Toolbar.background(&theme, true, fallback);
            assert!(0.0 < toolbar.a && toolbar.a < card.a && card.a < input.a && input.a < 1.0);
            assert_eq!((card.h, card.s, card.l), (input.h, input.s, input.l));
        }
    }

    #[gpui::test]
    fn local_wallpaper_blur_preserves_layout_and_clicks(cx: &mut gpui::TestAppContext) {
        struct Fixture {
            wallpaper: bool,
            clicks: usize,
        }
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let button = div()
                    .id("wallpaper-control")
                    .debug_selector(|| "wallpaper-control".into())
                    .w(px(160.0))
                    .h(px(40.0))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.clicks += 1;
                        cx.notify();
                    }));
                div()
                    .size_full()
                    .child(frosted(6.0, MENU_BLUR, button).with_wallpaper(self.wallpaper))
            }
        }
        cx.update(|cx| cx.set_global(Theme::light()));
        let (fixture, cx) = cx.add_window_view(|_, _| Fixture {
            wallpaper: false,
            clicks: 0,
        });
        cx.update(|window, cx| window.draw(cx).clear());
        let original = cx.debug_bounds("wallpaper-control").unwrap();
        for enabled in [true, false, true] {
            fixture.update(cx, |fixture, cx| {
                fixture.wallpaper = enabled;
                cx.notify();
            });
            cx.update(|window, cx| window.draw(cx).clear());
            assert_eq!(cx.debug_bounds("wallpaper-control"), Some(original));
            cx.simulate_click(original.center(), gpui::Modifiers::default());
        }
        fixture.read_with(cx, |fixture, _| assert_eq!(fixture.clicks, 3));
    }
}
