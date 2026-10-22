use super::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

const FIXTURE_ROOT: &str = "RCODE_BASH_REGRESSION_ROOT";
const FIXTURE_PREFIX: &str = "tools::bash::regression_tests";

fn context(directory: &Path) -> ToolContext {
    ToolContext {
        cwd: directory.to_path_buf(),
        cancel: CancellationToken::new(),
        nested: None,
    }
}

fn fixture_command(directory: &Path, name: &str) -> String {
    let executable = std::env::current_exe().unwrap();
    let arguments =
        format!("--exact {FIXTURE_PREFIX}::{name} --ignored --nocapture --test-threads=1");
    if cfg!(windows) {
        return format!(
            "$env:{FIXTURE_ROOT} = '{}'; & '{}' {arguments}",
            directory.to_string_lossy().replace('\'', "''"),
            executable.to_string_lossy().replace('\'', "''")
        );
    }
    match shell() {
        Shell::Posix(_) => {
            fn quote(path: &Path) -> String {
                let text = path.to_string_lossy();
                #[cfg(windows)]
                let text = text.replace('\\', "/");
                format!("'{}'", text.replace('\'', "'\\''"))
            }
            format!(
                "{FIXTURE_ROOT}={} {} {arguments}",
                quote(directory),
                quote(&executable)
            )
        }
        Shell::Cmd => format!(
            "set \"{FIXTURE_ROOT}={}\" && \"{}\" {arguments}",
            directory.display(),
            executable.display()
        ),
    }
}

fn fixture_root() -> Option<PathBuf> {
    std::env::var_os(FIXTURE_ROOT).map(PathBuf::from)
}

// These helpers run as separate native processes: no Python/PowerShell
// dependency, and the same cases exercise POSIX shells and the cmd fallback.
#[test]
#[ignore = "subprocess helper, invoked explicitly by the Bash regression tests"]
fn inherited_pipe_holder_fixture() {
    let Some(root) = fixture_root() else {
        return;
    };
    std::fs::write(root.join("holder.pid"), std::process::id().to_string()).unwrap();
    println!("STDOUT_READY");
    eprintln!("STDERR_READY");
    std::fs::write(root.join("holder.ready"), "ready").unwrap();
    std::thread::sleep(Duration::from_secs(5));
    // Killing only the already-exited leader would still allow this write.
    std::fs::write(root.join("holder.survived"), "survived").unwrap();
}

#[test]
#[ignore = "subprocess helper, invoked explicitly by the Bash regression tests"]
fn inherited_pipe_launcher_fixture() {
    let Some(root) = fixture_root() else {
        return;
    };
    let _child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("{FIXTURE_PREFIX}::inherited_pipe_holder_fixture"),
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(FIXTURE_ROOT, &root)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !root.join("holder.ready").exists() {
        assert!(std::time::Instant::now() < deadline, "holder did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    std::fs::write(root.join("launcher.exiting"), "exiting").unwrap();
    // Do not wait for the child: the launcher and wrapping shell exit while
    // their child still holds BOTH output pipe write handles.
}

#[test]
#[ignore = "subprocess helper, invoked explicitly by the Bash regression tests"]
fn large_output_fixture() {
    let Some(root) = fixture_root() else {
        return;
    };
    let mode = std::fs::read_to_string(root.join("output.mode")).unwrap();
    if mode == "unicode" {
        std::io::stdout()
            .write_all("汉€🦀".repeat(200_000).as_bytes())
            .unwrap();
    } else {
        std::io::stdout().write_all(&vec![b'x'; 2_000_000]).unwrap();
        if mode == "both" {
            std::io::stderr().write_all(&vec![b'y'; 2_000_000]).unwrap();
        }
    }
    std::fs::write(root.join("output.completed"), "drained").unwrap();
}

async fn wait_for_launcher_exit(root: &Path) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !root.join("launcher.exiting").exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "launcher did not exit"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Only the helper's libtest footer remains before the shell exits.
    tokio::time::sleep(Duration::from_millis(100)).await;
}

async fn assert_holder_was_killed(root: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::Foundation::{ERROR_INVALID_PARAMETER, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
        };
        let pid: u32 = std::fs::read_to_string(root.join("holder.pid"))
            .unwrap()
            .parse()
            .unwrap();
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(ERROR_INVALID_PARAMETER as i32),
                "cannot verify descendant process cleanup"
            );
        } else {
            // Retain the process object while waiting instead of polling a reusable PID.
            let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            loop {
                if unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } == WAIT_OBJECT_0 {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "job descendant survived"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
    #[cfg(not(windows))]
    {
        // kill(pid, 0) would count orphan zombies as live writers. Observe the
        // scheduled side effect past its five-second due time instead.
        tokio::time::sleep(Duration::from_millis(5200)).await;
    }
    assert!(
        !root.join("holder.survived").exists(),
        "a descendant wrote after its process tree was stopped"
    );
}

#[tokio::test]
async fn original_deadline_applies_after_the_shell_exits_with_inherited_pipes() {
    let directory = tempfile::tempdir().unwrap();
    let ctx = context(directory.path());
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        execute(
            json!({
                "command": fixture_command(directory.path(), "inherited_pipe_launcher_fixture"),
                "timeout": 2,
            }),
            &ctx,
            true,
            None,
        ),
    )
    .await
    .expect("the tool remained stuck draining inherited pipes");
    assert!(directory.path().join("launcher.exiting").exists());
    assert!(matches!(result, Err(ToolError::Timeout(_))), "{result:?}");
    assert_holder_was_killed(directory.path()).await;
}

#[tokio::test]
async fn cancellation_applies_while_both_inherited_pipes_are_still_open() {
    let directory = tempfile::tempdir().unwrap();
    let ctx = context(directory.path());
    let cancel = ctx.cancel.clone();
    let command = fixture_command(directory.path(), "inherited_pipe_launcher_fixture");
    let handle =
        tokio::spawn(
            async move { execute(json!({ "command": command }), &ctx, false, None).await },
        );
    wait_for_launcher_exit(directory.path()).await;
    assert!(
        !handle.is_finished(),
        "the fixture did not leave inherited pipes open"
    );
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("cancellation was ignored while draining pipes")
        .unwrap();
    assert!(matches!(result, Err(ToolError::Cancelled)), "{result:?}");
    assert_holder_was_killed(directory.path()).await;
}

#[tokio::test]
async fn an_exited_launcher_has_a_bounded_drain_even_for_untimed_tasks() {
    let directory = tempfile::tempdir().unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        execute(
            json!({ "command": fixture_command(directory.path(), "inherited_pipe_launcher_fixture") }),
            &context(directory.path()),
            false,
            None,
        ),
    )
    .await
    .expect("untimed task remained stuck draining inherited pipes");
    assert!(
        matches!(result, Err(ToolError::Execution(ref message)) if message.contains("inherited output pipes")),
        "{result:?}"
    );
    assert_holder_was_killed(directory.path()).await;
}

async fn verify_large_output(mode: &str) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("output.mode"), mode).unwrap();
    let largest_progress = Arc::new(AtomicUsize::new(0));
    let notifications_after_truncation = Arc::new(AtomicUsize::new(0));
    let last_was_truncated = Arc::new(Mutex::new(false));
    let progress = {
        let largest = largest_progress.clone();
        let repeated = notifications_after_truncation.clone();
        let last = last_was_truncated.clone();
        Arc::new(move |text: String| {
            largest.fetch_max(text.len(), Ordering::Relaxed);
            let mut was_truncated = last.lock().unwrap();
            if *was_truncated {
                repeated.fetch_add(1, Ordering::Relaxed);
            }
            *was_truncated = text.contains("output truncated");
        })
    };
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        execute(
            json!({ "command": fixture_command(directory.path(), "large_output_fixture") }),
            &context(directory.path()),
            true,
            Some(progress),
        ),
    )
    .await
    .expect("overflowed pipes were not drained")
    .expect("the fixture finished");
    assert!(!result.is_error, "{}", result.output);
    assert!(result.output.len() <= MAX_OUTPUT_BYTES + 128);
    assert!(largest_progress.load(Ordering::Relaxed) <= MAX_OUTPUT_BYTES + 128);
    assert!(result.output.contains("output truncated"));
    assert!(directory.path().join("output.completed").exists());
    if mode == "unicode" {
        assert!(
            !result.output.contains('\u{fffd}'),
            "UTF-8 was split at a read/cap boundary"
        );
    }
    if mode != "both" {
        assert_eq!(notifications_after_truncation.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn two_megabyte_single_line_is_bounded_in_result_and_progress() {
    verify_large_output("stdout").await;
}

#[tokio::test]
async fn stdout_and_stderr_share_one_budget_and_keep_draining_after_it_is_full() {
    verify_large_output("both").await;
}

#[tokio::test]
async fn multibyte_single_line_remains_valid_across_read_and_cap_boundaries() {
    verify_large_output("unicode").await;
}

#[test]
fn utf8_decoder_preserves_characters_across_every_split() {
    let text = "A汉€🦀Z";
    for split in 0..=text.len() {
        let mut output = BoundedOutput::default();
        let mut remaining = 128;
        output.push_bytes(&text.as_bytes()[..split], &mut remaining);
        output.push_bytes(&text.as_bytes()[split..], &mut remaining);
        output.finish(&mut remaining);
        assert_eq!(output.text, text, "byte split at {split}");
        assert!(!output.truncated);
        assert!(output.pending.is_empty());
    }
}

#[test]
fn shared_budget_never_keeps_a_partial_character() {
    let mut stdout = BoundedOutput::default();
    let mut stderr = BoundedOutput::default();
    let mut remaining = 7;
    stdout.push_bytes("汉abc".as_bytes(), &mut remaining);
    stderr.push_bytes("🦀tail".as_bytes(), &mut remaining);
    assert_eq!(stdout.text, "汉abc");
    assert!(stderr.text.is_empty());
    assert!(stderr.truncated);
    assert_eq!(remaining, 0);
}

#[test]
fn invalid_and_incomplete_utf8_is_replaced_without_exceeding_the_budget() {
    let mut output = BoundedOutput::default();
    let mut remaining = 6;
    output.push_bytes(&[0xff, 0xe2, 0x82], &mut remaining);
    assert_eq!(output.text, "\u{fffd}");
    output.finish(&mut remaining);
    assert_eq!(output.text, "\u{fffd}\u{fffd}");
    assert_eq!(remaining, 0);
    assert!(output.push_bytes(b"x", &mut remaining));
    assert!(output.truncated);
    assert!(!output.push_bytes(b"more", &mut remaining));
    assert_eq!(output.text.len(), 6);
}
