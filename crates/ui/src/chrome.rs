//! Product-wide metrics for window chrome and compact pane toolbars.
//! See docs/ui-chrome-standards.md for the two control roles and native exceptions.

use gpui::{Div, Pixels, Point, div, point, prelude::*, px};

pub(crate) const TITLEBAR_HEIGHT: f32 = 38.0;
pub(crate) const TITLEBAR_TOP_PAD: f32 = 4.0;
pub(crate) const TITLEBAR_CENTER_Y: f32 = (TITLEBAR_HEIGHT + TITLEBAR_TOP_PAD) * 0.5;
pub(crate) const TITLEBAR_BUTTON_SIZE: f32 = 28.0;
pub(crate) const TITLEBAR_ICON_SIZE: f32 = 16.0;
pub(crate) const CONTROL_GAP: f32 = 4.0;
pub(crate) const GROUP_GAP: f32 = 8.0;
pub(crate) const CONTROL_RADIUS: f32 = 6.0;
pub(crate) const TITLEBAR_EDGE_INSET: f32 = 6.0;

pub(crate) const COMPACT_BUTTON_SIZE: f32 = 24.0;
pub(crate) const COMPACT_ICON_SIZE: f32 = 14.0;

pub(crate) const MACOS_TRAFFIC_LIGHT_LEFT: f32 = 14.0;
// The current native titlebar uses 14pt traffic-light frames. Keep their
// configured origin tied to the content center rather than a separate offset.
const MACOS_TRAFFIC_LIGHT_HEIGHT: f32 = 14.0;
pub(crate) const MACOS_CLUSTER_START: f32 = 88.0;
pub(crate) const MACOS_FULLSCREEN_CLUSTER_START: f32 = 12.0;

pub(crate) const NAVIGATION_WIDTH: f32 = TITLEBAR_BUTTON_SIZE * 3.0 + GROUP_GAP + CONTROL_GAP;
pub(crate) const NEW_SESSION_SLOT_WIDTH: f32 = GROUP_GAP + TITLEBAR_BUTTON_SIZE;

/// Budget the native controls, navigation, New Session and the sidebar menu.
pub(crate) const fn sidebar_min_width(is_macos: bool, left_captions: usize) -> f32 {
    let cluster_start = if is_macos {
        MACOS_CLUSTER_START
    } else {
        10.0 + left_captions as f32 * (TITLEBAR_BUTTON_SIZE + CONTROL_GAP)
    };
    let required = cluster_start
        + NAVIGATION_WIDTH
        + NEW_SESSION_SLOT_WIDTH
        + GROUP_GAP
        + TITLEBAR_BUTTON_SIZE
        + GROUP_GAP;
    if required > 242.0 { required } else { 242.0 }
}

pub(crate) fn macos_traffic_light_position() -> Point<Pixels> {
    point(
        px(MACOS_TRAFFIC_LIGHT_LEFT),
        px(TITLEBAR_CENTER_Y - MACOS_TRAFFIC_LIGHT_HEIGHT * 0.5),
    )
}

/// Every window-level strip shares this center line. Pane toolbars use their
/// own unpadded row because they live below the native window titlebar.
pub(crate) fn titlebar_row() -> Div {
    div()
        .h(px(TITLEBAR_HEIGHT))
        .flex()
        .flex_row()
        .items_center()
        .pt(px(TITLEBAR_TOP_PAD))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titlebar_controls_and_macos_lights_share_a_center_without_clipping() {
        let native = macos_traffic_light_position();
        assert_eq!(
            native.y + px(MACOS_TRAFFIC_LIGHT_HEIGHT * 0.5),
            px(TITLEBAR_CENTER_Y)
        );
        for size in [TITLEBAR_BUTTON_SIZE, TITLEBAR_ICON_SIZE] {
            let top = TITLEBAR_CENTER_Y - size * 0.5;
            assert!(top >= 0.0 && top + size <= TITLEBAR_HEIGHT);
            assert_eq!(top + size * 0.5, TITLEBAR_CENTER_Y);
        }
        assert!(COMPACT_ICON_SIZE < COMPACT_BUTTON_SIZE);
        assert!(TITLEBAR_ICON_SIZE < TITLEBAR_BUTTON_SIZE);
    }
}
