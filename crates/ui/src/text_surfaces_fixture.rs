//! Offline visual QA over the actual Changes and TerminalPanel entities.
//! Compiled only for `appshots-fixture`; all content is synthetic test data.
use std::any::Any;

use gpui::{AppContext, Context, Entity, IntoElement, Render, Window, div, prelude::*, px};

use crate::{
    changes::Changes,
    state::AppState,
    terminal::panel::TerminalPanel,
    theme::Theme,
    typography::{self, UiTextRole},
};

#[derive(Clone)]
pub struct FixtureOptions {
    pub dark: bool,
    pub ui_font_size: f32,
    pub code_font_size: f32,
    pub terminal_font_size: f32,
    pub code_font: typography::UiFontFamily,
    pub terminal_font: typography::UiFontFamily,
    /// Application rem multiplier, independent of the OS DPI scale.
    pub scale: f32,
    pub split: bool,
    pub wrap: bool,
    pub wide_cursor: bool,
}

impl Default for FixtureOptions {
    fn default() -> Self {
        Self {
            dark: false,
            ui_font_size: 16.0,
            code_font_size: typography::CODE_FONT_SIZE_DEFAULT,
            terminal_font_size: typography::TERMINAL_FONT_SIZE_DEFAULT,
            code_font: typography::UiFontFamily::GeistMono,
            terminal_font: typography::UiFontFamily::GeistMono,
            scale: 1.0,
            split: false,
            wrap: false,
            wide_cursor: false,
        }
    }
}

fn patch() -> String {
    let long_line = format!(
        " \t// long line: {}",
        "中文W123🙂\tpunctuation()[]{}; ".repeat(16)
    );
    // Each context line needs its leading ASCII space. Rust's escaped
    // multiline continuation would remove that marker before parsing.
    [
        "diff --git a/src/中文_geometry.rs b/src/中文_geometry.rs",
        "--- a/src/中文_geometry.rs",
        "+++ b/src/中文_geometry.rs",
        "@@ -9998,8 +9998,9 @@",
        " // 中文 English 0123456789，全角（测试）【示例】；半角 (test) [42].",
        "-const TITLE: &str = \"旧值 old 123\";",
        "+const TITLE: &str = \"新值 English 456：，。！？；\";",
        " fn mixed_width() {",
        "-\tlet emoji = \"🙂\";",
        "+\tlet emoji = \"🙂 👩‍💻 1️⃣Z e\u{301}\";",
        "+\tlet 中文变量 = \"汉字 AB １２３ 123\";",
        " \tprintln!(\"{} {}\", TITLE, emoji);",
        &long_line,
        " \tlet arrows = \"-> => != == │─╭╮╰╯\";",
        " }",
        "",
    ]
    .join("\n")
}

fn terminal_bytes(wide_cursor: bool) -> Vec<u8> {
    let mut bytes = concat!(
        "\x1b[2J\x1b[H",
        "中文 English 0123456789，。！？；：\r\n",
        "A中文Ｂ🙂Z １２３ (test) [42] {x}\r\n",
        "\x1b[1m粗体 Bold 123 中文🙂\x1b[0m / regular 常规\r\n",
        "\x1b[3mItalic e\u{301}\x1b[0m  Emoji 1\u{fe0f}\u{20e3}Z 👩\u{200d}💻\r\n",
        "\x1b[4m下划线 underline\x1b[0m  \x1b[31m红色 red\x1b[0m\r\n",
        "\x1b[44;97m 蓝底白字 selection 对照 \x1b[0m\r\n",
        "│ box ──╭╮╰╯ │ -> => != == -- ++\r\n",
        "tabs:\t中\tAB\t123\r\n",
        "cursor: A中文Ｂ🙂Z|",
        // Cursor rests on the final marker, directly after mixed-width text.
        "\x1b[D"
    )
    .as_bytes()
    .to_vec();
    if wide_cursor {
        // Row 9: "cursor: A" is nine columns, followed by 中's two cells.
        bytes.extend_from_slice(b"\x1b[9;10H");
    }
    bytes
}

pub struct TextSurfacesFixture {
    changes: Entity<Changes>,
    terminal: Entity<TerminalPanel>,
    options: FixtureOptions,
    _lifetime: Box<dyn Any>,
}

impl TextSurfacesFixture {
    pub fn new(options: FixtureOptions, lifetime: Box<dyn Any>, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|_| {
            let mut state = AppState::new();
            state.chats.push(
                serde_json::from_value(serde_json::json!({
                    "id": "text-surfaces",
                    "deviceId": "fixture",
                    "title": "中文 English",
                    "archived": false,
                    "cwd": "/text-surfaces",
                    "checkoutId": "text-surfaces",
                    "createdAt": "2026-10-23T00:00:00Z"
                }))
                .expect("synthetic fixture chat"),
            );
            state.selected_chat = Some("text-surfaces".into());
            state
        });
        let changes = cx.new(|cx| {
            Changes::for_text_fixture(state.clone(), &patch(), options.split, options.wrap, cx)
        });
        let terminal = cx.new(|cx| {
            TerminalPanel::for_text_fixture(state, &terminal_bytes(options.wide_cursor), cx)
        });
        Self {
            changes,
            terminal,
            options,
            _lifetime: lifetime,
        }
    }
}

impl Render for TextSurfacesFixture {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let terminal_height = (theme.terminal_font_size
            * (crate::terminal::view::TERM_LINE_HEIGHT / crate::terminal::view::TERM_FONT_SIZE)
            * 10.0
            + 2.0 * crate::terminal::view::TERM_PADDING
            + crate::terminal::panel::TAB_BAR_HEIGHT)
            .max(360.0);
        let status = format!(
            "DPI {:.0}% · UI {} × {:.2} · Diff {} / setting {:.1}px → paint {:.1}px · Terminal {} / {:.1}px",
            window.scale_factor() * 100.0,
            self.options.ui_font_size,
            self.options.scale,
            theme.font_mono,
            theme.code_font_size,
            crate::changes::diff_text_size(theme),
            theme.font_terminal,
            theme.terminal_font_size,
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .font_family(theme.font_sans.clone())
            .text_color(theme.text)
            .bg(theme.bg)
            .child(
                div()
                    .flex_none()
                    .px(px(12.0))
                    .py(px(8.0))
                    .text_size(UiTextRole::Caption.size())
                    .line_height(UiTextRole::Caption.line_height())
                    .child(status),
            )
            .child(div().flex_1().min_h_0().child(self.changes.clone()))
            .child(
                div()
                    .flex_none()
                    .h(px(terminal_height))
                    .border_t_1()
                    .border_color(theme.border)
                    .child(self.terminal.clone()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::{self, DiffMode, DiffRow, LineKind};

    #[test]
    fn fixture_diff_patch_has_renderable_mixed_width_body() {
        let files = changes::parse_patch(&patch());
        assert_eq!(files.len(), 1);
        let file = &files[0];
        assert_eq!(file.hunks.len(), 1);
        let lines = &file.hunks[0].lines;
        assert_eq!(
            lines.len(),
            11,
            "the fixture must render source rows after its hunk header"
        );
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.kind == LineKind::Add)
                .count(),
            3
        );
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.kind == LineKind::Del)
                .count(),
            2
        );
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.kind == LineKind::Context)
                .count(),
            6
        );
        assert_eq!(lines[0].kind, LineKind::Context);
        assert!(lines[0].text.starts_with("// 中文 English 0123456789"));
        assert_eq!(lines[0].old_no, Some(9998));
        assert_eq!(lines[0].new_no, Some(9998));
        assert!(
            lines
                .iter()
                .any(|line| line.kind == LineKind::Add && line.text.contains("1️⃣Z"))
        );
        assert!(lines.iter().any(|line| line.kind == LineKind::Context
            && line.text.starts_with('\t')
            && line.text.contains("long line:")));
        assert_eq!(file.max_line, 10006);
        for mode in [DiffMode::Unified, DiffMode::Split] {
            let (rows, ranges) = changes::flatten_rows(&files, &[], None, mode, |_| false);
            let source_rows = rows
                .iter()
                .filter(|row| matches!(row, DiffRow::Line { .. } | DiffRow::SplitLine { .. }))
                .count();
            assert!(
                source_rows >= 9,
                "the production list must contain source body rows"
            );
            assert_eq!(ranges, vec![0..rows.len()]);
        }
    }
}
