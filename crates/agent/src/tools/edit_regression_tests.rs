use super::*;
use tokio_util::sync::CancellationToken;

fn context(path: &std::path::Path) -> ToolContext {
    ToolContext {
        cwd: path.into(),
        cancel: CancellationToken::new(),
        nested: None,
    }
}

async fn edit_text(
    before: &str,
    old: &str,
    new: &str,
    all: bool,
) -> (String, Result<ToolResult, ToolError>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("file.txt"), before).unwrap();
    let result = EditTool
        .call(
            json!({
                "file_path":"file.txt", "old_string":old, "new_string":new, "replace_all":all
            }),
            &context(dir.path()),
        )
        .await;
    (
        std::fs::read_to_string(dir.path().join("file.txt")).unwrap(),
        result,
    )
}

#[tokio::test]
async fn lf_text_from_read_can_edit_a_crlf_file_without_changing_its_format() {
    let dir = tempfile::tempdir().unwrap();
    let before = "\u{feff}前言\r\nold one\r\nold two\r\n末尾";
    std::fs::write(dir.path().join("file.txt"), before).unwrap();
    let read = super::super::read::ReadTool
        .call(json!({"file_path":"file.txt"}), &context(dir.path()))
        .await
        .unwrap();
    assert!(read.output.contains("old one\n"));
    assert!(!read.output.contains('\r'));
    let result = EditTool.call(json!({
        "file_path":"file.txt", "old_string":"old one\nold two", "new_string":"new one\nnew two"
    }), &context(dir.path())).await.unwrap();
    let expected = "\u{feff}前言\r\nnew one\r\nnew two\r\n末尾";
    assert_eq!(
        std::fs::read_to_string(dir.path().join("file.txt")).unwrap(),
        expected
    );
    let diff = result.diff.unwrap();
    assert_eq!(diff.old_text.as_deref(), Some(before));
    assert_eq!(diff.new_text, expected);
}

#[tokio::test]
async fn crlf_input_does_not_add_carriage_returns_to_lf_files() {
    let (after, result) = edit_text("a\nb\nc\n", "a\r\nb", "x\r\ny", false).await;
    result.unwrap();
    assert_eq!(after, "x\ny\nc\n");
}

#[tokio::test]
async fn untouched_mixed_line_endings_and_unicode_remain_byte_identical() {
    let (after, result) =
        edit_text("汉\r\nkeep\nold\r\ntail\n", "old", "new\r\nadded", false).await;
    result.unwrap();
    assert_eq!(after, "汉\r\nkeep\nnew\r\nadded\r\ntail\n");
}

#[tokio::test]
async fn normalized_duplicates_are_ambiguous_even_when_one_is_an_exact_match() {
    let before = "a\nb\nseparator\r\na\r\nb\r\n";
    let (after, result) = edit_text(before, "a\nb", "x\ny", false).await;
    assert!(result.unwrap_err().to_string().contains("appears 2 times"));
    assert_eq!(after, before);
}

#[tokio::test]
async fn replace_all_preserves_each_occurrences_local_line_endings() {
    let (after, result) = edit_text("a\nb\nseparator\r\na\r\nb\r\n", "a\nb", "x\ny", true).await;
    assert!(result.unwrap().output.contains("2 replacements"));
    assert_eq!(after, "x\ny\nseparator\r\nx\r\ny\r\n");
}

#[tokio::test]
async fn replacements_at_newline_boundaries_do_not_leave_stray_cr_bytes() {
    for (before, old, new, expected) in [
        ("α\r\nb\r\nc", "\nb\n", "\nx\n", "α\r\nx\r\nc"),
        ("a\r\nb", "\n", "", "ab"),
        ("a\r\nb", "b", "b\nc", "a\r\nb\r\nc"),
    ] {
        let (after, result) = edit_text(before, old, new, false).await;
        result.unwrap();
        assert_eq!(after, expected);
    }
}

#[tokio::test]
async fn a_repeated_edit_reports_the_new_text_already_present_without_writing() {
    let before = "fn example() {\r\n    new();\r\n}\r\n";
    let (after, result) = edit_text(before, "    old();", "    new();", false).await;
    assert!(result.unwrap_err().to_string().contains("already present"));
    assert_eq!(after, before);
}

#[tokio::test]
async fn an_inexact_modifier_is_not_fuzzily_replaced_and_gets_a_location_hint() {
    let before = "// header\npub(crate) fn absolute() {\n    value()\n}\n";
    let (after, result) = edit_text(
        before,
        "    fn absolute() {\n    value()\n}",
        "replacement",
        false,
    )
    .await;
    assert!(result.unwrap_err().to_string().contains("near line 2"));
    assert_eq!(after, before);
}
