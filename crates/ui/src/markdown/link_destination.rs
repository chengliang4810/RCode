//! Destination disclosure is local text only. Never fetch link previews.
use crate::theme::Theme;
use gpui::{AnyElement, App, SharedString, Window, div, prelude::*, px};
fn viewport_limits(width: f32, height: f32) -> (f32, f32) {
    (
        (width - 24.).clamp(1., 360.),
        (height - 80.).clamp(1., 160.),
    )
}
/// Wrap by measured graphemes: GPUI's word wrapping can otherwise leave a
/// URL scheme alone on the first line. This never replaces the destination.
fn wrapped_destination(url: &str, width: f32, x_for_index: impl Fn(usize) -> f32) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    let mut shown = String::new();
    let mut start = 0;
    let mut origin = x_for_index(0);
    for (at, grapheme) in url.grapheme_indices(true) {
        if at > start && (x_for_index(at + grapheme.len()) - origin).abs() > width {
            shown.push_str(&url[start..at]);
            shown.push('\n');
            start = at;
            origin = x_for_index(at);
        }
    }
    shown.push_str(&url[start..]);
    shown
}
pub fn destination_card(
    url: &str,
    bounds: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    window: &Window,
    _cx: &App,
) -> AnyElement {
    let theme = Theme::dark();
    let (max_width, height) = viewport_limits(
        window.viewport_size().width.into(),
        window.viewport_size().height.into(),
    );
    let text: SharedString = url.to_owned().into();
    let shaped = window.text_system().shape_line(
        text.clone(),
        px(11.),
        &[window.text_style().to_run(text.len())],
        None,
    );
    let width = (f32::from(shaped.width()).ceil() + 22.).min(max_width);
    let text = wrapped_destination(url, (width - 22.).max(1.), |at| {
        shaped.x_for_index(at).into()
    });
    div()
        .id("web-destination-card")
        .relative()
        .child(
            gpui::canvas(|_, _, _| (), move |rect, _, _, _| bounds.set(Some(rect)))
                .absolute()
                .size_full(),
        )
        .w(px(width))
        .max_w(px(width))
        .px(px(10.))
        .py(px(6.))
        .rounded(px(12.))
        .bg(theme.surface_raised)
        .shadow_sm()
        .text_size(px(11.))
        .line_height(px(14.))
        .text_color(theme.text)
        .text_center()
        .role(gpui::Role::Tooltip)
        .flex()
        .flex_col()
        .child(
            div()
                .id("web-destination-scroll")
                .max_h(px(height))
                .overflow_y_scroll()
                .child(text),
        )
        .into_any_element()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disclosure_fits_small_viewports_and_keeps_the_complete_destination() {
        for (w, h) in [(320., 240.), (1000., 800.), (80., 80.)] {
            let (width, height) = viewport_limits(w, h);
            assert!(width < w && height < h);
        }
        let url = format!("https://example.com/{}?q=🙂", "á界".repeat(5000));
        let shown = wrapped_destination(&url, 300., |at| at as f32);
        assert_eq!(shown.replace('\n', ""), url);
        assert!(shown.lines().all(|line| line.len() <= 300));
        let short = "https://example.com";
        assert_eq!(wrapped_destination(short, 300., |at| at as f32), short);
    }
}
