//! One window-sized crop, clipped to the chosen columns. Opening a pane changes
//! the clip, never the image scale or focal point.
use crate::new_thread_background_effects::ArtworkFrame;
use crate::settings::WallpaperScope;
use gpui::{AnyElement, IntoElement, div, prelude::*, px};

fn horizontal_extent(
    scope: WallpaperScope,
    width: f32,
    sidebar: f32,
    right: f32,
) -> Option<(f32, f32)> {
    let width = width.max(0.0);
    let left = match scope {
        WallpaperScope::NewThread => return None,
        WallpaperScope::Conversation => sidebar.clamp(0.0, width),
        _ => 0.0,
    };
    let end = match scope {
        WallpaperScope::Workspace => width,
        _ => (width - right.max(0.0)).clamp(left, width),
    };
    Some((left, end - left))
}

pub(super) fn render(
    frame: ArtworkFrame,
    scope: WallpaperScope,
    width: f32,
    height: f32,
    sidebar: f32,
    right: f32,
    corner: f32,
) -> AnyElement {
    let Some((left, visible_width)) = horizontal_extent(scope, width, sidebar, right) else {
        return gpui::Empty.into_any_element();
    };
    div()
        .id("workspace-wallpaper")
        .debug_selector(|| "workspace-wallpaper".into())
        .absolute()
        .top_0()
        .left(px(left))
        .w(px(visible_width))
        .h(px(height))
        .overflow_hidden()
        .children(
            [
                (frame.previous, frame.previous_adjustment, 1.0 - frame.mix),
                (frame.current, frame.current_adjustment, frame.mix),
            ]
            .into_iter()
            .filter_map(|(image, adjustment, mix)| {
                let image = image?;
                Some(
                    div()
                        .absolute()
                        .top_0()
                        .left(px(-left))
                        .w(px(width))
                        .h(px(height))
                        // Keep prose and chrome readable with every wallpaper/theme.
                        .opacity(0.32 * mix)
                        .child(
                            gpui::canvas(
                                |_, _, _| {},
                                move |bounds, _, window, _| {
                                    crate::new_thread_background_mask::paint_adjusted(
                                        image,
                                        bounds,
                                        adjustment,
                                        gpui::Corners::all(px(corner)),
                                        window,
                                    );
                                },
                            )
                            .absolute()
                            .inset_0(),
                        ),
                )
            }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_clip_to_the_visible_columns() {
        assert_eq!(
            horizontal_extent(WallpaperScope::NewThread, 1440.0, 256.0, 400.0),
            None
        );
        assert_eq!(
            horizontal_extent(WallpaperScope::Conversation, 1440.0, 256.0, 400.0),
            Some((256.0, 784.0))
        );
        assert_eq!(
            horizontal_extent(WallpaperScope::ConversationAndSidebar, 1440.0, 256.0, 400.0),
            Some((0.0, 1040.0))
        );
        assert_eq!(
            horizontal_extent(WallpaperScope::Workspace, 1440.0, 256.0, 400.0),
            Some((0.0, 1440.0))
        );
        for sidebar in [0.0, 128.0, 256.0] {
            for right in [0.0, 400.0, 686.0] {
                let (left, width) =
                    horizontal_extent(WallpaperScope::Conversation, 1440.0, sidebar, right)
                        .unwrap();
                assert_eq!(left, sidebar);
                assert_eq!(left + width, 1440.0 - right);
            }
        }
        assert_eq!(
            horizontal_extent(WallpaperScope::Conversation, 200.0, 256.0, 400.0),
            Some((200.0, 0.0))
        );
    }
}
