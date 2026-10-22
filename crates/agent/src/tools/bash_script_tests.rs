use super::*;
use tokio_util::sync::CancellationToken;

// Exercise the POSIX backend explicitly, even though the public Bash tool now
// selects PowerShell on Windows. The platform tool has its own PS fixtures.
struct PosixTool;
impl PosixTool {
    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        execute_with_shell(CommandShell::Bash, input, ctx, true, None).await
    }
}

fn context(path: &std::path::Path) -> ToolContext {
    ToolContext {
        cwd: path.into(),
        cancel: CancellationToken::new(),
        nested: None,
    }
}

#[tokio::test]
async fn quoted_heredoc_preserves_windows_paths_quotes_and_unicode() {
    if !matches!(shell(), Shell::Posix(_)) {
        return;
    }
    let dir = tempfile::Builder::new()
        .prefix("rcode script 空格 ")
        .tempdir()
        .unwrap();
    // These are literal script bytes, including doubled and trailing slashes,
    // both quote kinds and text which must never undergo shell expansion.
    let body = r#"println!("ancestors of D:\\a\\b = {:?}", Path::new("D:\\a\\b"));
"/// separator, `C:\\x` or `C:/x`.\r\n"
single ' double " trailing \\
$HOME $(printf SHOULD_NOT_EXPAND) `echo SHOULD_NOT_EXPAND`
路径 🦀
"#;
    let command = format!("cat <<'RCODE_EOF' > payload.txt\n{body}RCODE_EOF\n");
    let result = PosixTool
        .call(json!({"command":command}), &context(dir.path()))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output);
    assert_eq!(
        std::fs::read(dir.path().join("payload.txt")).unwrap(),
        body.as_bytes()
    );
}

#[tokio::test]
async fn a_large_script_does_not_hit_the_windows_command_line_limit() {
    if !matches!(shell(), Shell::Posix(_)) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let body = "a\\b ' \" 字符\n".repeat(6000);
    let command = format!("cat <<'RCODE_EOF' > large.txt\n{body}RCODE_EOF\n");
    let result = PosixTool
        .call(json!({"command":command}), &context(dir.path()))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output);
    assert_eq!(
        std::fs::read(dir.path().join("large.txt")).unwrap(),
        body.as_bytes()
    );
}

#[tokio::test]
async fn commands_reading_stdin_see_eof_without_consuming_following_script_lines() {
    if !matches!(shell(), Shell::Posix(_)) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let result = PosixTool
        .call(
            json!({
                "command":"cat > input.txt\nprintf after > after.txt",
                "timeout":5
            }),
            &context(dir.path()),
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output);
    assert_eq!(std::fs::read(dir.path().join("input.txt")).unwrap(), b"");
    assert_eq!(
        std::fs::read(dir.path().join("after.txt")).unwrap(),
        b"after"
    );
}

#[tokio::test]
async fn a_failing_pipeline_preserves_status_and_does_not_run_the_success_branch() {
    if !matches!(shell(), Shell::Posix(_)) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let result = PosixTool.call(json!({
        "command":"(printf compiler-failed; exit 23) | cat && printf WRONG > should-not-exist"
    }), &context(dir.path())).await.unwrap();
    assert!(result.is_error, "{}", result.output);
    assert!(result.output.starts_with("exit 23\n"), "{}", result.output);
    assert!(result.output.contains("compiler-failed"));
    assert!(!dir.path().join("should-not-exist").exists());
}

#[tokio::test]
async fn pipefail_still_allows_explicit_recovery_and_does_not_enable_errexit() {
    if !matches!(shell(), Shell::Posix(_)) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let result = PosixTool
        .call(
            json!({
                "command":"(exit 7) | cat || printf recovered; false; printf continued"
            }),
            &context(dir.path()),
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output);
    assert!(result.output.contains("recoveredcontinued"));
}

#[test]
fn timeout_is_capped_in_seconds_and_invalid_values_are_rejected() {
    for (input, expected) in [
        (json!({}), 120),
        (json!({"timeout":1}), 1),
        (json!({"timeout":600}), 600),
        (json!({"timeout":900}), 600),
        (json!({"timeout":u64::MAX}), 600),
    ] {
        assert_eq!(timeout_seconds(&input).unwrap(), expected);
    }
    for value in [
        json!(0),
        json!(-1),
        json!(1.5),
        json!("900"),
        json!(null),
        json!(true),
    ] {
        assert!(matches!(
            timeout_seconds(&json!({"timeout":value})),
            Err(ToolError::InvalidInput(_))
        ));
    }
}

#[cfg(windows)]
#[tokio::test]
async fn script_file_is_kept_until_exit_then_removed() {
    if !matches!(shell(), Shell::Posix(_)) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let result = PosixTool
        .call(
            json!({
                "command":"test -f \"$0\" && printf '%s' \"$0\""
            }),
            &context(dir.path()),
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output);
    let path = result.output.strip_prefix("exit 0\n").unwrap();
    assert!(path.contains("rcode-bash-"), "{path}");
    assert!(
        !std::path::Path::new(path).exists(),
        "script leaked: {path}"
    );
}

#[tokio::test]
async fn nul_in_a_command_is_rejected_before_execution() {
    let dir = tempfile::tempdir().unwrap();
    let error = PosixTool
        .call(
            json!({"command":"echo before\0echo after"}),
            &context(dir.path()),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("NUL"));
}
