use super::*;
use crate::tools::BashTool;
use crate::{Tool, ToolContext, ToolError};
use serde_json::json;
use std::process::Stdio;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

fn context(cwd: &Path) -> ToolContext {
    ToolContext {
        cwd: cwd.into(),
        cancel: CancellationToken::new(),
        nested: None,
    }
}

fn engines() -> Vec<PathBuf> {
    let mut paths = vec![program().to_owned()];
    let desktop = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    if desktop.is_file() && !paths.contains(&desktop) {
        paths.push(desktop);
    }
    paths
}

async fn output(engine: &Path, cwd: &Path, script: &str) -> std::process::Output {
    let (mut command, path) = command_for(engine, script).unwrap();
    let path = path.unwrap();
    assert!(path.exists());
    command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .creation_flags(0x08000000);
    let output = tokio::time::timeout(Duration::from_secs(15), command.output())
        .await
        .unwrap()
        .unwrap();
    let saved = path.to_path_buf();
    drop(path);
    assert!(!saved.exists());
    output
}

#[tokio::test]
async fn both_editions_preserve_unicode_quotes_paths_and_here_strings() {
    let dir = tempfile::Builder::new()
        .prefix("rcode PS 中文 ")
        .tempdir()
        .unwrap();
    let body = "D:\\projects\\带 空格\\file.txt\n'quote' \"double\" $env:PATH ` literal 🦀";
    for engine in engines() {
        let result = output(
            &engine,
            dir.path(),
            &format!("$value = @'\n{body}\n'@\n[Console]::Write($value)"),
        )
        .await;
        assert!(
            result.status.success(),
            "{}: {:?}",
            engine.display(),
            result
        );
        assert_eq!(String::from_utf8(result.stdout).unwrap(), body);
    }
}

#[tokio::test]
async fn both_editions_report_native_failure_through_a_cmdlet_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    for engine in engines() {
        let result = output(
            &engine,
            dir.path(),
            "cmd.exe /d /c exit 23 | Select-Object -Last 5",
        )
        .await;
        assert_eq!(
            result.status.code(),
            Some(23),
            "{}: {:?}",
            engine.display(),
            result
        );
    }
}

#[tokio::test]
async fn both_editions_fail_on_cmdlet_errors_even_after_a_successful_native_command() {
    let dir = tempfile::tempdir().unwrap();
    for engine in engines() {
        let result = output(&engine, dir.path(), "cmd.exe /d /c exit 0\nGet-Item -LiteralPath './missing-rcode-fixture'\n[Console]::Write('SHOULD_NOT_RUN')").await;
        assert!(!result.status.success(), "{}", engine.display());
        assert!(!String::from_utf8_lossy(&result.stdout).contains("SHOULD_NOT_RUN"));
        assert!(String::from_utf8_lossy(&result.stderr).contains("missing-rcode-fixture"));
    }
}

#[tokio::test]
async fn both_editions_keep_successful_native_stderr_as_success() {
    let dir = tempfile::tempdir().unwrap();
    for engine in engines() {
        for redirect in ["", " 2>&1 | Select-Object -Last 5"] {
            let result = output(
                &engine,
                dir.path(),
                &format!("cmd.exe /d /c 'echo warning 1>&2 & exit 0'{redirect}"),
            )
            .await;
            assert!(
                result.status.success(),
                "{}: {:?}",
                engine.display(),
                result
            );
            assert!(
                String::from_utf8_lossy(&result.stderr).contains("warning")
                    || String::from_utf8_lossy(&result.stdout).contains("warning")
            );
        }
    }
}

#[tokio::test]
async fn large_scripts_and_stdin_eof_work_through_the_tool() {
    let dir = tempfile::tempdir().unwrap();
    let body = "a\\b ' \" 中文\n".repeat(5000);
    let script = format!(
        "$value = @'\n{body}\n'@\nif ([Console]::In.ReadToEnd().Length -ne 0) {{ throw 'stdin was not EOF' }}\n[Console]::Write($value.Length)"
    );
    let result = BashTool
        .call(json!({"command":script}), &context(dir.path()))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output);
    assert!(
        result
            .output
            .ends_with(&body.encode_utf16().count().to_string()),
        "{}",
        result.output
    );
}

#[tokio::test]
async fn the_working_directory_and_explicit_exit_code_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let result = BashTool
        .call(
            json!({"command":"[Console]::Write((Get-Location).Path); exit 17"}),
            &context(dir.path()),
        )
        .await
        .unwrap();
    assert!(result.is_error, "{}", result.output);
    assert!(result.output.starts_with("exit 17\n"), "{}", result.output);
    // Windows CI can expose TEMP through an 8.3 alias, while PowerShell
    // reports its expanded path. Compare directory identity, not spelling.
    let reported = result.output.strip_prefix("exit 17\n").unwrap().trim();
    assert_eq!(
        std::fs::canonicalize(reported).unwrap(),
        std::fs::canonicalize(dir.path()).unwrap(),
        "{}",
        result.output,
    );
}

#[tokio::test]
async fn timeout_stops_a_powershell_script() {
    let dir = tempfile::tempdir().unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        BashTool.call(
            json!({"command":"Start-Sleep -Seconds 30", "timeout":1}),
            &context(dir.path()),
        ),
    )
    .await
    .unwrap();
    assert!(matches!(result, Err(ToolError::Timeout(_))), "{result:?}");
}

#[tokio::test]
async fn cancellation_stops_powershell_and_removes_its_script_file() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let cancel = ctx.cancel.clone();
    let marker = dir.path().join("script-path.txt");
    let task = tokio::spawn(async move {
        BashTool.call(json!({"command":"[IO.File]::WriteAllText((Join-Path $PWD 'script-path.txt'), $PSCommandPath); Start-Sleep -Seconds 30"}), &ctx).await
    });
    let script = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Ok(path) = std::fs::read_to_string(&marker) {
                if !path.is_empty() {
                    break path;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(Path::new(&script).is_file());
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(ToolError::Cancelled)));
    assert!(!Path::new(&script).exists());
}
