//! Loading placeholders shaped like the content they stand in for, so the
//! board and detail keep their layout while GitHub answers. Dense groups breathe
//! together; only the small prose preview uses a quiet travelling wave. Both use
//! the self-parking shared loader clock and stay still under reduced motion.
use crate::{settings::widgets, theme::Theme};
use gpui::{AnyElement, App, EntityId, div, prelude::*, px, relative};
use rcode_presentation::TextKey;

const SKELETON_CYCLE: crate::motion::MotionSpec =
    crate::motion::MotionSpec::new(2000, crate::motion::FEEDBACK_EASE_OUT);

#[derive(Clone, Copy)]
pub(crate) enum SkeletonMode {
    Lockstep,
    Wave,
}

struct SkeletonSample {
    opacity: f32,
    sweep: Option<f32>,
}

fn skeleton_sample(mode: SkeletonMode, phase: f32, index: usize, reduced: bool) -> SkeletonSample {
    if reduced {
        return SkeletonSample {
            opacity: 0.8,
            sweep: None,
        };
    }
    match mode {
        SkeletonMode::Lockstep => SkeletonSample {
            // A whole dense group has one quiet rhythm, not dozens of chases.
            opacity: 0.6 + 0.4 * crate::motion::pulse_wave(phase),
            sweep: None,
        },
        SkeletonMode::Wave => {
            let phase = (phase - index.min(4) as f32 * 0.045).rem_euclid(1.0);
            SkeletonSample {
                opacity: 0.85,
                // Sweep for 1.3s, then leave 0.7s of quiet in each 2s period.
                sweep: (phase < 0.65).then_some(phase / 0.65),
            }
        }
    }
}

/// Dense loading surfaces share a lockstep breath. Their row index remains in
/// the API because callers also use it to choose deterministic geometry.
pub(crate) fn bar(
    width: impl Into<gpui::Length>,
    height: f32,
    index: usize,
    view: EntityId,
    theme: &Theme,
    cx: &mut App,
) -> gpui::Div {
    bar_with_mode(
        width,
        height,
        index,
        SkeletonMode::Lockstep,
        view,
        theme,
        cx,
    )
}

pub(crate) fn bar_with_mode(
    width: impl Into<gpui::Length>,
    height: f32,
    index: usize,
    mode: SkeletonMode,
    view: EntityId,
    theme: &Theme,
    cx: &mut App,
) -> gpui::Div {
    let phase = crate::motion::pulse_delta_slow(&SKELETON_CYCLE, view, cx);
    let sample = skeleton_sample(mode, phase, index, crate::motion::reduced_motion(cx));
    let mut bar = div()
        .relative()
        .w(width.into())
        // A fixed-width bar never overflows a narrow pane.
        .max_w_full()
        .h(px(height))
        .flex_none()
        .rounded(px(height / 2.0))
        .bg(theme.ink(0.07))
        .opacity(sample.opacity);
    if let Some(progress) = sample.sweep {
        let highlight = theme.ink(0.045);
        bar = bar.child(
            gpui::canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let width = f32::from(bounds.size.width);
                    if width <= 0.0 {
                        return;
                    }
                    let beam = (width * 0.4).clamp(24.0, 96.0);
                    let center = crate::motion::lerp(-beam, width + beam, progress);
                    let intensity = |x: f32| {
                        let distance = ((x - center) / beam).abs().min(1.0);
                        let fade = 1.0 - distance;
                        fade * fade * (3.0 - 2.0 * fade)
                    };
                    // Native two-stop gradients approximate a broad feathered
                    // sweep without allocating an offscreen blur/filter surface.
                    window.with_rounded_clip(bounds, px(height / 2.0), |window| {
                        for stripe in 0..16 {
                            let x0 = width * stripe as f32 / 16.0;
                            let x1 = width * (stripe + 1) as f32 / 16.0;
                            let left = intensity(x0);
                            let right = intensity(x1);
                            if left == 0.0 && right == 0.0 {
                                continue;
                            }
                            let stripe_bounds = gpui::Bounds {
                                origin: gpui::point(bounds.origin.x + px(x0), bounds.origin.y),
                                size: gpui::size(px(x1 - x0), bounds.size.height),
                            };
                            window.paint_quad(gpui::quad(
                                stripe_bounds,
                                px(0.0),
                                gpui::linear_gradient(
                                    105.0,
                                    gpui::linear_color_stop(highlight.opacity(left), 0.0),
                                    gpui::linear_color_stop(highlight.opacity(right), 1.0),
                                ),
                                px(0.0),
                                gpui::transparent_black(),
                                gpui::BorderStyle::default(),
                            ));
                        }
                    });
                },
            )
            .absolute()
            .size_full(),
        );
    }
    bar
}

/// Deterministic width ladder so rows read as varied text, not slabs.
const WIDTHS: [f32; 6] = [0.62, 0.48, 0.71, 0.55, 0.39, 0.66];

/// The board's grouped list: a group label over a card of PR rows.
pub(crate) fn board(
    layout: crate::pull_requests::PullRequestTableLayout,
    view: EntityId,
    theme: &Theme,
    cx: &mut App,
) -> AnyElement {
    let rows = (0..6)
        .map(|index| {
            let identity = div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(Theme::SPACE_XS))
                .child(
                    div()
                        .h(px(20.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(bar(px(14.0), 14.0, index, view, theme, cx))
                        .child(bar(relative(WIDTHS[index]), 12.0, index, view, theme, cx)),
                )
                .child(
                    div()
                        .pl(px(22.0))
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(Theme::SPACE_SM))
                        .child(bar(px(16.0), 16.0, index, view, theme, cx))
                        .child(bar(px(72.0), 10.0, index, view, theme, cx))
                        .child(bar(px(52.0), 16.0, index, view, theme, cx))
                        .child(bar(px(120.0), 12.0, index, view, theme, cx)),
                );
            let row = crate::pull_requests::table_row_shell(layout, index == 0, index == 5, theme)
                .child(identity);
            if layout == crate::pull_requests::PullRequestTableLayout::Narrow {
                row.child(
                    div()
                        .pl(px(22.0))
                        .h(px(28.0))
                        .flex()
                        .items_center()
                        .gap(px(Theme::SPACE_SM))
                        .child(bar(px(72.0), 10.0, index, view, theme, cx))
                        .child(div().flex_1())
                        .child(bar(px(44.0), 10.0, index, view, theme, cx))
                        .child(
                            div()
                                .size(px(28.0))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(bar(px(14.0), 14.0, index, view, theme, cx)),
                        ),
                )
                .into_any_element()
            } else {
                row.child(
                    div()
                        .w(px(112.0))
                        .flex_none()
                        .flex()
                        .flex_col()
                        .items_end()
                        .gap(px(Theme::SPACE_XS))
                        .child(div().h(px(16.0)).flex().items_center().child(bar(
                            px(44.0),
                            10.0,
                            index,
                            view,
                            theme,
                            cx,
                        )))
                        .child(div().h(px(16.0)).flex().items_center().child(bar(
                            px(72.0),
                            10.0,
                            index,
                            view,
                            theme,
                            cx,
                        ))),
                )
                .child(
                    div()
                        .size(px(28.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(bar(px(14.0), 14.0, index, view, theme, cx)),
                )
                .into_any_element()
            }
        })
        .collect::<Vec<_>>();
    div()
        .id("pull-requests-skeleton")
        .debug_selector(|| "pull-requests-skeleton".into())
        .role(gpui::Role::ProgressIndicator)
        .aria_label(crate::l10n::localizer().text(TextKey::PrSkelLoadingList))
        .w_full()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .min_h(px(24.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .child(bar(px(120.0), 10.0, 0, view, theme, cx)),
        )
        .child(widgets::section_card(theme).mt_0().w_full().children(rows))
        .into_any_element()
}

/// The Summary's card stack below a real header.
pub(crate) fn summary(gap: f32, view: EntityId, theme: &Theme, cx: &mut App) -> AnyElement {
    let localizer = crate::l10n::localizer();
    // Each field's stable id and its label resource: the id must match the
    // loaded page's (and never move with the language), the label must not.
    let fields = [
        ("Status", TextKey::PrSkelStatus),
        ("Review", TextKey::PrSkelReview),
        ("Branch", TextKey::GitBranch),
        ("Changes", TextKey::GitChanges),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (id, key))| {
        let height = if index < 2 { 22.0 } else { 12.0 };
        crate::pull_request_detail::field_row(
            id,
            &localizer.text(key),
            bar(
                px(if index == 2 { 220.0 } else { 96.0 }),
                height,
                index,
                view,
                theme,
                cx,
            )
            .into_any_element(),
            index == 0,
            theme,
        )
    })
    .collect::<Vec<_>>();
    let lines = (0..5)
        .map(|index| {
            bar_with_mode(
                relative(if index == 4 { 0.4 } else { 0.92 }),
                10.0,
                index,
                SkeletonMode::Wave,
                view,
                theme,
                cx,
            )
            .into_any_element()
        })
        .collect::<Vec<_>>();
    div()
        .id("pr-detail-skeleton")
        .debug_selector(|| "pr-detail-skeleton".into())
        .role(gpui::Role::ProgressIndicator)
        .aria_label(localizer.text(TextKey::PrSkelLoadingDetail))
        .mt(px(24.0))
        .flex()
        .flex_col()
        .gap(px(gap))
        .child(widgets::section_card(theme).mt_0().children(fields))
        .child(
            widgets::section_card(theme).mt_0().child(
                widgets::card_row(theme, true)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(widgets::row_title(
                                theme,
                                localizer.text(TextKey::PrSkelCurrentAssessment),
                            ))
                            .child(bar(relative(0.6), 14.0, 4, view, theme, cx))
                            .child(bar(relative(0.4), 14.0, 4, view, theme, cx))
                            .child(bar(px(100.0), 13.0, 4, view, theme, cx)),
                    )
                    .child(
                        div()
                            .size(px(28.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(bar(px(14.0), 14.0, 4, view, theme, cx)),
                    ),
            ),
        )
        .child(
            widgets::section_card(theme)
                .mt_0()
                .px(px(16.0))
                .py(px(10.0))
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(bar(px(14.0), 14.0, 4, view, theme, cx))
                .child(div().flex_1().child(localizer.text(TextKey::PrSkelChecks)))
                .child(bar(px(72.0), 22.0, 4, view, theme, cx)),
        )
        .child(
            widgets::section_card(theme)
                .mt_0()
                .child(
                    widgets::card_row(theme, true)
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(220.0))
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(widgets::row_title(
                                    theme,
                                    localizer.text(TextKey::PrSkelHandoffTitle),
                                ))
                                .child(
                                    div()
                                        .text_size(crate::typography::ui_rems(12.0))
                                        .text_color(theme.text_muted)
                                        .child(localizer.text(TextKey::PrSkelHandoffBody)),
                                ),
                        )
                        .child(bar(px(180.0), 28.0, 5, view, theme, cx)),
                )
                .child(
                    widgets::card_row(theme, false)
                        .child(div().flex_1().min_w(px(120.0)).child(widgets::row_title(
                            theme,
                            localizer.text(TextKey::PrSkelVerification),
                        )))
                        .child(div().flex_1().min_w_0().child(bar(
                            relative(0.8),
                            28.0,
                            6,
                            view,
                            theme,
                            cx,
                        ))),
                )
                .child(crate::pull_request_detail::field_row(
                    "Checkout",
                    &localizer.text(TextKey::PrSkelCheckout),
                    bar(relative(0.7), 12.0, 6, view, theme, cx).into_any_element(),
                    false,
                    theme,
                )),
        )
        .child(
            widgets::section_card(theme)
                .mt_0()
                .p(px(16.0))
                .gap(px(12.0))
                .children(lines),
        )
        .into_any_element()
}

/// Diff lines inside the stream card, under a file header.
pub(crate) fn diff(split: bool, view: EntityId, theme: &Theme, cx: &mut App) -> AnyElement {
    let header = div()
        .h(px(crate::changes::FILE_HEADER_HEIGHT))
        .flex_none()
        .px(px(Theme::SPACE_MD))
        .flex()
        .items_center()
        .gap(px(8.0))
        .bg(theme.ink(0.025))
        .child(bar(px(13.0), 13.0, 0, view, theme, cx))
        .child(bar(px(14.0), 14.0, 0, view, theme, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(bar(relative(0.55), 10.0, 0, view, theme, cx)),
        )
        .child(bar(px(64.0), 10.0, 0, view, theme, cx))
        .child(bar(px(24.0), 24.0, 0, view, theme, cx));
    let cell = |index: usize, cx: &mut App| {
        let gutter = |cx: &mut App| {
            div()
                .w(px(crate::changes::GUTTER_WIDTH))
                .flex_none()
                .pr(px(8.0))
                .flex()
                .justify_end()
                .child(bar(px(12.0), 8.0, index, view, theme, cx))
        };
        div()
            .h(px(crate::changes::diff_line_height(theme)))
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .overflow_hidden()
            .child(div().w(px(crate::changes::ACCENT_BAR_WIDTH)).flex_none())
            .child(gutter(cx))
            .when(!split, |el| el.child(gutter(cx)))
            .child(
                div()
                    .w(px(if split {
                        crate::changes::SPLIT_MARKER_WIDTH
                    } else {
                        crate::changes::MARKER_WIDTH
                    }))
                    .flex_none(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pl(px(if split {
                        crate::changes::SPLIT_CODE_PADDING_LEFT
                    } else {
                        crate::changes::UNIFIED_CODE_PADDING_LEFT
                    }))
                    .child(bar(
                        relative(WIDTHS[index % WIDTHS.len()]),
                        8.0,
                        index,
                        view,
                        theme,
                        cx,
                    )),
            )
    };
    let lines = (0..14)
        .map(|index| {
            div()
                .h(px(crate::changes::diff_line_height(theme)))
                .flex_none()
                .flex()
                .child(cell(index, cx))
                .when(split, |row| {
                    row.child(
                        div()
                            .w(px(crate::changes::SPLIT_DIVIDER_WIDTH))
                            .self_stretch()
                            .flex_none()
                            .bg(theme.border),
                    )
                    .child(cell(index, cx))
                })
                .into_any_element()
        })
        .collect::<Vec<_>>();
    div()
        .id("pr-code-skeleton")
        .debug_selector(|| "pr-code-skeleton".into())
        .role(gpui::Role::ProgressIndicator)
        .aria_label(crate::l10n::localizer().text(TextKey::PrSkelLoadingDiff))
        .size_full()
        .flex()
        .flex_col()
        .child(header)
        .child(
            div()
                .h(px(crate::changes::HUNK_HEADER_HEIGHT))
                .flex_none()
                .px(px(Theme::SPACE_MD))
                .flex()
                .items_center()
                .child(bar(px(180.0), 10.0, 0, view, theme, cx)),
        )
        .child(div().flex().flex_col().children(lines))
        .into_any_element()
}

/// Rows of the changed-file tree.
pub(crate) fn tree(view: EntityId, theme: &Theme, cx: &mut App) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(1.0))
        .children((0..12).map(|index| {
            let nested = index % 4 != 0;
            div()
                .h(px(28.0))
                .flex_none()
                .pl(px(if nested { 22.0 } else { 8.0 }))
                .pr(px(8.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(bar(px(14.0), 14.0, index, view, theme, cx))
                .child(bar(
                    relative(WIDTHS[index % WIDTHS.len()] * 0.7),
                    10.0,
                    index,
                    view,
                    theme,
                    cx,
                ))
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_skeletons_are_phase_locked_and_reduced_motion_has_no_sweep() {
        for phase in [0.0, 0.1, 0.5, 0.9] {
            let first = skeleton_sample(SkeletonMode::Lockstep, phase, 0, false);
            let last = skeleton_sample(SkeletonMode::Lockstep, phase, 20, false);
            assert_eq!(first.opacity, last.opacity);
            assert!(first.sweep.is_none());
            assert!((0.6..=1.0).contains(&first.opacity));
            for mode in [SkeletonMode::Lockstep, SkeletonMode::Wave] {
                let reduced = skeleton_sample(mode, phase, 2, true);
                assert_eq!(reduced.opacity, 0.8);
                assert!(reduced.sweep.is_none());
            }
        }
    }

    #[test]
    fn wave_stagger_is_ninety_ms_and_each_cycle_has_a_quiet_tail() {
        let first = skeleton_sample(SkeletonMode::Wave, 0.3, 0, false);
        let next = skeleton_sample(SkeletonMode::Wave, 0.345, 1, false);
        assert!((first.sweep.unwrap() - next.sweep.unwrap()).abs() < 0.00001);
        assert!(
            skeleton_sample(SkeletonMode::Wave, 0.9, 0, false)
                .sweep
                .is_none()
        );
        assert!(
            skeleton_sample(SkeletonMode::Wave, 0.9, 4, false)
                .sweep
                .is_none()
        );
    }
}
