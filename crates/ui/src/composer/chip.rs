//! What a chip looks like: its pill, icon and hover preview.
use super::*;

/// A staged attachment an attachment chip can refer to; an image chip opens
/// its picture on click.
pub(super) struct ChipAttachment {
    pub(super) image: Option<std::sync::Arc<gpui::Image>>,
}

/// Spaces reserve room at the start of every chip for its icon well, which is
/// painted over them, so the text, caret and wrapping all treat the chip as
/// plain text. Non-breaking spaces, because the bundled Geist has no wider or
/// thinner ones.
pub(super) const CHIP_ICON_SLOT: &str = "\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}";
/// Breathing room after a chip's label.
pub(super) const CHIP_TRAILING_PAD: &str = "\u{00A0}\u{00A0}";
/// The bundled face a chip's padding is shaped in. A space's advance follows
/// the font (Geist Mono's is 2.4× Geist's), and the counts above are tuned to
/// Geist, so pinning the padding keeps every chip's icon slot and insets the
/// same whatever the interface font. The label keeps the surrounding font.
pub(crate) const CHIP_PAD_FAMILY: &str = "Geist";
/// The icon sits at this size in its well.
const CHIP_ICON_SIZE: f32 = 14.0;

/// What a chip refers to; it picks the chip's icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChipKind {
    Image,
    Skill,
    Command,
    File,
    Directory,
    /// Another chat, dropped from the sidebar.
    Chat,
}

/// Byte positions are deliberately absent: editing before a reference must
/// not turn the same chip into a fresh entrance. Duplicate targets occupy
/// separate, deterministic occurrence slots within the current draft.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct InputChipIdentity {
    path: String,
    kind: ChipKind,
    attachment: Option<u32>,
    occurrence: usize,
}

fn input_chip_identities(mentions: &[(FileMentionLink, Range<usize>)]) -> Vec<InputChipIdentity> {
    let mut occurrences = HashMap::new();
    mentions.iter().map(|(mention, _)| {
        let target = (mention.path.clone(), mention.kind, mention.attachment);
        let occurrence = occurrences.entry(target).or_insert(0);
        let identity = InputChipIdentity {
            path: mention.path.clone(),
            kind: mention.kind,
            attachment: mention.attachment,
            occurrence: *occurrence,
        };
        *occurrence += 1;
        identity
    }).collect()
}

/// This state lives with the input document, not its rebuilt text element.
/// Settled entries stay present until the reference itself disappears.
#[derive(Default)]
pub(super) struct InputChipMotion {
    revision: u64,
    entries: HashMap<InputChipIdentity, motion::Presence>,
}

impl InputChipMotion {
    pub(super) fn restored(&mut self, revision: u64) {
        self.revision = revision;
        self.entries.clear();
    }

    pub(super) fn sync(
        &mut self,
        mentions: &[(FileMentionLink, Range<usize>)],
        revision: u64,
        now: Instant,
        reduced: bool,
    ) -> Vec<motion::PresenceSample> {
        let identities = input_chip_identities(mentions);
        let live: HashSet<_> = identities.iter().cloned().collect();
        self.entries.retain(|identity, _| live.contains(identity));
        let edited = revision != self.revision;
        self.revision = revision;
        let mut inserted = 0;
        identities.into_iter().map(|identity| {
            let presence = self.entries.entry(identity).or_insert_with(|| {
                if edited && !reduced {
                    let presence = motion::Presence::enter(inserted, now, false);
                    inserted += 1;
                    presence
                } else {
                    motion::Presence::present(now)
                }
            });
            // Switching reduced motion on settles existing entrances too; a
            // quick switch back cannot resume a half-finished decorative fade.
            if reduced {
                *presence = motion::Presence::present(now);
            }
            presence.sample(now, reduced)
        }).collect()
    }
}

/// What a chip's icon is painted from: a monochrome glyph tinted like the
/// chip, or the file theme's own polychrome artwork, which tells formats and
/// languages apart.
pub(crate) enum ChipIcon {
    Glyph(&'static str),
    FileTheme(SharedString),
}

/// The padding around the label in a chip's display `range`: the side
/// bearing and icon slot before it, and the trailing room after it. Callers
/// shape both in [`CHIP_PAD_FAMILY`].
pub(crate) fn chip_pad_ranges(range: &Range<usize>) -> [Range<usize>; 2] {
    let lead_end = (range.start + MENTION_SIDE_PAD.len() + CHIP_ICON_SLOT.len()).min(range.end);
    let trail_start = range
        .end
        .saturating_sub(CHIP_TRAILING_PAD.len())
        .max(lead_end);
    [range.start..lead_end, trail_start..range.end]
}

/// How far to move a chip's pill down from the middle of its row, which
/// starts at `row_top`, so the pill centers its label's caps. GPUI puts a
/// line's baseline `(ascent - descent) / 2` below the row's middle, with the
/// tallest face's metrics, then rounds it to a device pixel (`scale` per
/// point). Geist's caps sit on the middle (1005 - 295 = 710, its cap height);
/// other faces and the rounding drift them by a pixel or so. Everything is in
/// window coordinates, since the rounding depends on where the row lands.
pub(super) fn label_center_offset(
    row_top: f32,
    line_height: f32,
    ascent: f32,
    descent: f32,
    cap_height: f32,
    scale: f32,
) -> f32 {
    if cap_height <= 0.0 {
        // A face without a cap height: keep the row's middle.
        return 0.0;
    }
    // GPUI's glyph snapping: halves round toward zero.
    let snap = |y: f32| {
        let device = y * scale;
        let rounded = if device.fract().abs() == 0.5 {
            device.trunc()
        } else {
            device.round()
        };
        rounded / scale
    };
    let baseline = snap(row_top + (line_height - ascent - descent) / 2.0 + ascent);
    let offset = baseline - cap_height / 2.0 - (row_top + line_height / 2.0);
    // The pill lands on whole device pixels too; a fraction would only
    // round it away from a label that is already centered.
    snap(offset)
}

/// How far to move a chip's pill down from the middle of its row so it
/// centers the label (see [`paint_chip`]). `label` is the label's first byte
/// within `line`, and `row_top` the row's top in window coordinates. The
/// shift stays within `room`, the pill's inset from its row, so it never
/// reaches into the next row.
pub(crate) fn chip_label_offset(
    line: &gpui::LineLayout,
    label: usize,
    row_top: Pixels,
    line_height: Pixels,
    room: Pixels,
    window: &Window,
) -> Pixels {
    // The face that actually shaped the label, fallbacks included.
    let Some(font_id) = line
        .runs
        .iter()
        .find(|run| run.glyphs.iter().any(|glyph| glyph.index >= label))
        .map(|run| run.font_id)
    else {
        return px(0.0);
    };
    let cap_height = window.text_system().cap_height(font_id, line.font_size);
    let offset = label_center_offset(
        f32::from(row_top),
        f32::from(line_height),
        f32::from(line.ascent),
        f32::from(line.descent),
        f32::from(cap_height),
        window.scale_factor(),
    );
    px(offset.clamp(-f32::from(room), f32::from(room)))
}

/// [`chip_label_offset`] for a chip at byte `label` of a text element's
/// `layout`, whose lines are its text split at newlines.
pub(crate) fn text_chip_label_offset(
    layout: &gpui::TextLayout,
    label: usize,
    row_top: Pixels,
    room: Pixels,
    window: &Window,
) -> Pixels {
    let mut start = 0;
    for line in layout.line_layouts() {
        let end = start + line.unwrapped_layout.len;
        if label < end {
            return chip_label_offset(
                &line.unwrapped_layout,
                label - start,
                row_top,
                layout.line_height(),
                room,
                window,
            );
        }
        start = end + 1;
    }
    px(0.0)
}

/// The non-empty padding ranges of `spans`, in order: what [`chip_text`]
/// shapes in [`CHIP_PAD_FAMILY`].
pub(super) fn chip_pad_overrides(spans: &[SentMentionSpan]) -> Vec<Range<usize>> {
    spans
        .iter()
        .flat_map(|span| chip_pad_ranges(&span.range))
        .filter(|range| !range.is_empty())
        .collect()
}

/// The icon of a chip. `path` names the file or folder for the file kinds.
pub(crate) fn chip_icon(
    kind: ChipKind,
    path: &str,
    appearance: crate::theme::Appearance,
) -> ChipIcon {
    use crate::file_icons::{FileIconIdentity, asset_path};
    match kind {
        ChipKind::Image => ChipIcon::Glyph(crate::icons::GALLERY),
        ChipKind::Skill => ChipIcon::Glyph(crate::icons::MAGIC_STICK_3),
        ChipKind::Command => ChipIcon::Glyph(if path == "/goal" { crate::icons::GOAL } else { crate::icons::COMMAND }),
        ChipKind::Chat => ChipIcon::Glyph(crate::icons::CHAT_ROUND_LINE),
        ChipKind::File => ChipIcon::FileTheme(asset_path(FileIconIdentity::file(path), appearance)),
        ChipKind::Directory => ChipIcon::FileTheme(asset_path(
            FileIconIdentity::directory(path.trim_end_matches('/'), false),
            appearance,
        )),
    }
}

/// Paint a chip like the transcript's file badges: a soft rounded pill whose
/// first row starts with the icon on a small well. Callers center `chip` on
/// the label's cap height: the row's middle moved by [`chip_label_offset`],
/// which is zero for Geist (ascent 1005, descent 295, cap 710 per 1000 units)
/// and corrects for other interface fonts.
pub(crate) fn paint_chip(
    window: &mut Window,
    chip: Bounds<Pixels>,
    icon: Option<&ChipIcon>,
    theme: &Theme,
    cx: &App,
) {
    paint_chip_surface(window, chip, icon, theme, 1.0, 1.0, cx);
}

/// Input-only feedback: text, caret, selection and hitboxes keep their final
/// geometry. Only the pill/well ink and monochrome icon softly reveal; file
/// artwork stays stable because the native image primitive has no local alpha.
pub(super) fn paint_input_chip(
    window: &mut Window,
    chip: Bounds<Pixels>,
    icon: Option<&ChipIcon>,
    theme: &Theme,
    sample: motion::PresenceSample,
    cx: &App,
) {
    let opacity = sample.opacity;
    paint_chip_surface(window, chip, icon, theme, opacity, 0.96 + 0.04 * opacity, cx);
}

fn paint_chip_surface(
    window: &mut Window,
    chip: Bounds<Pixels>,
    icon: Option<&ChipIcon>,
    theme: &Theme,
    opacity: f32,
    scale: f32,
    cx: &App,
) {
    let visual_size = size(chip.size.width * scale, chip.size.height * scale);
    let visual_bounds = Bounds::new(
        chip.origin + point((chip.size.width - visual_size.width) / 2.0, (chip.size.height - visual_size.height) / 2.0),
        visual_size,
    );
    window.paint_quad(quad(
        visual_bounds,
        px(5.0 * scale),
        theme.ink(0.09).opacity(opacity),
        px(0.0),
        gpui::transparent_black(),
        BorderStyle::default(),
    ));
    let Some(icon) = icon else {
        return;
    };
    let well = chip.size.height - px(2.0);
    let well_bounds = Bounds::new(chip.origin + point(px(1.0), px(1.0)), size(well, well));
    window.paint_quad(quad(
        well_bounds,
        px(4.0),
        crate::file_icons::well_bg(theme).opacity(opacity),
        px(0.0),
        gpui::transparent_black(),
        BorderStyle::default(),
    ));
    // The icon follows the chip's size, so chips on a small line stay tidy.
    let side = px(CHIP_ICON_SIZE).min(well - px(4.0));
    let bounds = Bounds::new(
        well_bounds.origin + point((well - side) / 2.0, (well - side) / 2.0),
        size(side, side),
    );
    match icon {
        ChipIcon::Glyph(path) => {
            let _ = window.paint_svg(
                bounds,
                (*path).into(),
                None,
                gpui::TransformationMatrix::default(),
                theme.text_muted.opacity(opacity),
                cx,
            );
        }
        ChipIcon::FileTheme(path) => {
            if let Some(image) = crate::file_icons::raster(path, cx) {
                let _ = window.paint_image(bounds, Default::default(), image, 0, false);
            }
        }
    }
}

/// A sent message's text with its chips painted over their spans, for rows
/// that show one line of it (the queue). `text` is the projected display text
/// and `spans` its chips, as [`sent_mention_display`] returns them.
pub(crate) fn chip_text(
    text: SharedString,
    spans: Vec<SentMentionSpan>,
    theme: &Theme,
) -> gpui::AnyElement {
    if spans.is_empty() {
        return text.into_any_element();
    }
    // Split runs at the padding (a default highlight changes nothing else),
    // so the family override lands on whole runs over the inherited style.
    let pads = chip_pad_overrides(&spans);
    let styled = gpui::StyledText::new(text)
        .with_highlights(
            pads.iter()
                .map(|range| (range.clone(), gpui::HighlightStyle::default())),
        )
        .with_font_family_overrides(
            pads.into_iter()
                .map(|range| (range, SharedString::from(CHIP_PAD_FAMILY))),
        );
    let layout = styled.layout().clone();
    let icons: Vec<ChipIcon> = spans
        .iter()
        .map(|span| chip_icon(span.kind, &span.path, theme.appearance))
        .collect();
    let theme = theme.clone();
    let underlay = gpui::canvas(
        |_, _, _| (),
        move |_, _, window, cx| {
            for (span, icon) in spans.iter().zip(&icons) {
                let label = chip_pad_ranges(&span.range)[0].end;
                for (row, rect) in
                    crate::markdown::render::range_rects(&layout, &span.range, 0.0, 2.0)
                        .into_iter()
                        .enumerate()
                {
                    let row_top = rect.origin.y - px(2.0);
                    let offset = text_chip_label_offset(&layout, label, row_top, px(2.0), window);
                    let rect = Bounds::new(rect.origin + point(px(0.0), offset), rect.size);
                    paint_chip(window, rect, (row == 0).then_some(icon), &theme, cx);
                }
            }
        },
    )
    .absolute()
    .size_full();
    div()
        .relative()
        .child(underlay)
        .child(styled)
        .into_any_element()
}

#[cfg(test)]
mod input_motion_tests {
    use super::*;

    fn projected(text: &str) -> TextProjection {
        TextProjection::rich(text, None, None)
    }

    fn later(now: Instant, millis: u64) -> Instant {
        now + Duration::from_millis(millis).mul_f32(motion::speed_scale())
    }

    #[test]
    fn input_chip_identity_survives_prefix_edits_and_unrelated_targets() {
        let link = local_file_link("src/café.rs", false);
        let original = projected(&format!("{link} {link}"));
        let shifted = projected(&format!("前缀 🙂 {} {link} {link}", local_file_link("Cargo.toml", false)));
        assert_eq!(original.mentions.len(), 2);
        assert_eq!(shifted.mentions.len(), 3);
        assert_ne!(original.mentions[0].0.range, shifted.mentions[1].0.range);
        assert_eq!(input_chip_identities(&original.mentions), input_chip_identities(&shifted.mentions)[1..]);
        let ids = input_chip_identities(&original.mentions);
        assert_ne!(ids[0], ids[1], "repeated references retain separate occurrence slots");
        let mut changed_kind = original.mentions.clone();
        changed_kind[0].0.kind = ChipKind::Directory;
        assert_ne!(ids[0], input_chip_identities(&changed_kind)[0]);
        let mut attachment = original.mentions.clone();
        attachment[0].0.attachment = Some(7);
        assert_ne!(ids[0], input_chip_identities(&attachment)[0]);
    }

    #[test]
    fn input_chip_entrance_does_not_restart_while_editing_or_after_settling() {
        let link = local_file_link("src/main.rs", false);
        let original = projected(&link);
        let shifted = projected(&format!("中文 prefix {link}"));
        let now = Instant::now();
        let mut motion_state = InputChipMotion::default();
        assert!(motion_state.sync(&original.mentions, 1, now, false)[0].active);
        let sampled = later(now, 120);
        let before = motion_state.sync(&original.mentions, 1, sampled, false)[0];
        let after = motion_state.sync(&shifted.mentions, 2, sampled, false)[0];
        assert_eq!(before.opacity, after.opacity, "prefix edit must not restart a visible entrance");
        let settled_at = later(now, 2000);
        let settled = motion_state.sync(&shifted.mentions, 2, settled_at, false)[0];
        assert_eq!(settled.opacity, 1.0);
        assert!(!settled.active);
        assert!(!motion_state.sync(&original.mentions, 3, later(now, 2010), false)[0].active);
        assert_eq!(motion_state.entries.len(), 1, "settled identity must remain retained");
        motion_state.sync(&[], 4, later(now, 2020), false);
        assert!(motion_state.entries.is_empty());
        assert!(motion_state.sync(&original.mentions, 5, later(now, 2030), false)[0].active);
    }

    #[test]
    fn restored_drafts_and_reduced_motion_never_replay_chip_ink() {
        let projection = projected(&local_file_link("src/lib.rs", false));
        let now = Instant::now();
        let mut motion_state = InputChipMotion::default();
        motion_state.restored(20);
        assert!(!motion_state.sync(&projection.mentions, 20, now, false)[0].active);
        assert!(!motion_state.sync(&projection.mentions, 21, now, false)[0].active);
        motion_state.sync(&[], 22, now, false);
        assert!(motion_state.sync(&projection.mentions, 23, now, false)[0].active);
        assert!(!motion_state.sync(&projection.mentions, 23, later(now, 70), true)[0].active);
        let restored = motion_state.sync(&projection.mentions, 24, later(now, 80), false)[0];
        assert!(!restored.active);
        assert_eq!(restored.opacity, 1.0);
    }
}
