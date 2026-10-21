//! `Bash` — run a shell command.
//!
//! Commands run with the host process permissions, like the file tools under
//! the application's trust-all policy. The working directory sets the initial
//! location; it does not restrict filesystem access.
//!
//! The command runs through the platform's shell — `cmd /C` on Windows, `sh -c`
//! elsewhere — because the model writes shell syntax, not an argv list.

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::tool::{Tool, ToolContext, ToolError, ToolResult};

/// Default ceiling on a command.
const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Hard ceiling, whatever the caller asks for.
const MAX_TIMEOUT_SECS: u64 = 600;
/// Combined output kept before truncation.
const MAX_OUTPUT_CHARS: usize = 100_000;

pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn description(&self) -> &str {
        "Run a shell command (bash syntax). Prefer the purpose-built tools: \
         Read/Write/Edit for files, Glob/Grep for searching, FolderOperations \
         for listings — they give structured output. Use Bash for builds, tests, version control and anything \
         without a dedicated tool."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command line to run."
                },
                "timeout": {
                    "type": "integer",
                    "description": "Seconds to allow before killing it. Defaults to 120, capped at 600."
                }
            },
            "required": ["command"]
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        execute(input, ctx, true, None).await
    }
}

pub(crate) async fn execute(
    input: Value,
    ctx: &ToolContext,
    timed: bool,
    progress: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
) -> Result<ToolResult, ToolError> {
    if ctx.cancel.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    let command = input
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidInput("`command` is required".to_owned()))?;
    if command.trim().is_empty() {
        return Err(ToolError::InvalidInput("`command` is empty".to_owned()));
    }
    let seconds = input
        .get("timeout")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS);

    let mut process = shell_command(command);
    process
        .current_dir(&ctx.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // A GUI app has no console; without this a child would flash a window.
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        // Its own process group, so a signal to the group reaches every
        // descendant that did not start a group of its own. Without this
        // the shell and the tool share the app's group, and signalling it
        // would take the app down with the command.
        process.process_group(0);
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        process.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = process
        .spawn()
        .map_err(|error| ToolError::Execution(format!("cannot start `{command}`: {error}")))?;

    // Take the pipes before waiting so output can be drained as it
    // arrives. `wait_with_output` would buffer everything the command ever
    // wrote — a runaway `yes` or a 10-minute build log accumulates
    // unboundedly and can exhaust the process's memory before the cap is
    // ever applied.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Own the tree, not just the shell. `kill_on_drop` reaches the direct
    // child only; a command that starts its own children (any build, any
    // `sh -c '… &'`) leaves them running after the shell dies, still
    // writing files and holding resources. Verified on Windows: a 1-second
    // timeout returned "it was killed" while the grandchild kept going.
    let mut tree = ProcessTree::new(&mut child);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut stdout_buf = BoundedOutput::new(MAX_OUTPUT_CHARS);
    let mut stderr_buf = BoundedOutput::new(MAX_OUTPUT_CHARS);

    let mut out_lines = stdout.map(BufReader::new).map(|reader| reader.lines());
    let mut err_lines = stderr.map(BufReader::new).map(|reader| reader.lines());
    let mut out_done = out_lines.is_none();
    let mut err_done = err_lines.is_none();

    let status = loop {
        tokio::select! {
            // Reading a line at a time keeps memory bounded by the cap
            // rather than by what the command produced. A single line
            // longer than the cap is truncated by the buffer itself.
            line = async { out_lines.as_mut().unwrap().next_line().await }, if !out_done => {
                match line {
                    Ok(Some(line)) => { stdout_buf.push_line(&line); if let Some(progress) = &progress { progress(format!("{}{}", stdout_buf.text(), stderr_buf.text())); } },
                    Ok(None) | Err(_) => out_done = true,
                }
            }
            line = async { err_lines.as_mut().unwrap().next_line().await }, if !err_done => {
                match line {
                    Ok(Some(line)) => { stderr_buf.push_line(&line); if let Some(progress) = &progress { progress(format!("{}{}", stdout_buf.text(), stderr_buf.text())); } },
                    Ok(None) | Err(_) => err_done = true,
                }
            }
            result = child.wait() => {
                break result.map_err(|error| ToolError::Execution(error.to_string()))?;
            }
            _ = tokio::time::sleep_until(deadline), if timed => {
                // Kill the whole tree before returning: the message says
                // "it was killed", and it needs to be true of everything
                // the command started, not just the top shell.
                tree.kill();
                let _ = child.wait().await;
                return Err(ToolError::Timeout(format!(
                    "`{command}` did not finish within {seconds}s; it and everything it \
                     started were killed"
                )));
            }
            _ = ctx.cancel.cancelled() => {
                tree.kill();
                let _ = child.wait().await;
                return Err(ToolError::Cancelled);
            }
        }
    };

    // Drain whatever is still buffered in the pipes after the process
    // exited, so a short-lived command's output is not lost to the race.
    if let Some(lines) = out_lines.as_mut() {
        while let Ok(Some(line)) = lines.next_line().await {
            stdout_buf.push_line(&line);
        }
    }
    if let Some(lines) = err_lines.as_mut() {
        while let Ok(Some(line)) = lines.next_line().await {
            stderr_buf.push_line(&line);
        }
    }
    // The tree is reaped on drop; `kill` is only for the paths that
    // return early.
    tree.disarm();

    let mut body = String::new();
    if !stdout_buf.text().trim().is_empty() {
        body.push_str(stdout_buf.text().trim_end());
    }
    if !stderr_buf.text().trim().is_empty() {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str("stderr:\n");
        body.push_str(stderr_buf.text().trim_end());
    }
    if body.is_empty() {
        body.push_str("(no output)");
    }

    let is_error = !status.success();
    let status_label = match status.code() {
        Some(code) => format!("exit {code}"),
        None => "killed by a signal".to_owned(),
    };
    let result = ToolResult {
        output: format!("{status_label}\n{body}"),
        is_error,
        diff: None,
    };
    Ok(result)
}

/// Output held for the model, capped as it arrives.
///
/// The cap has to apply *during* collection, not after: a command that prints
/// for ten minutes produces as much as it likes in that time, and buffering it
/// all before trimming means the process's memory is bounded by the command
/// rather than by the cap. That is the difference between a slow command and
/// an out-of-memory crash.
struct BoundedOutput {
    text: String,
    /// The limit, in characters.
    cap: usize,
    /// How much was dropped, so the model can tell the tail is missing.
    dropped: usize,
}

impl BoundedOutput {
    fn new(cap: usize) -> Self {
        Self {
            text: String::new(),
            cap,
            dropped: 0,
        }
    }

    fn push_line(&mut self, line: &str) {
        if self.text.len() >= self.cap {
            // Already full: count the line and stop growing. Counting rather
            // than measuring bytes keeps this O(1) per line.
            self.dropped += 1;
            return;
        }
        self.text.push_str(line);
        self.text.push('\n');
    }

    /// The collected text, with a note when anything was dropped.
    fn text(&self) -> String {
        if self.dropped == 0 {
            return self.text.clone();
        }
        format!(
            "{}\n… ({} more lines dropped)",
            self.text.trim_end(),
            self.dropped
        )
    }
}

/// Owns everything a command started, so stopping the command stops all of it.
///
/// `kill_on_drop` reaches only the direct child; a shell that forks — every
/// build, `sh -c 'thing &'`, a Makefile — leaves descendants behind that keep
/// writing the files the run has moved on from.
///
/// Unix: the shell is started in its own process group, and the signal goes to
/// the group, which reaches everything that did not create a group of its own.
/// Windows: the child joins a Job Object with kill-on-close; terminating the
/// job takes the whole tree.
struct ProcessTree {
    /// Set once the command finished cleanly, so `Drop` does not kill a job
    /// that already exited.
    disarmed: bool,
    #[cfg(unix)]
    group: Option<i32>,
    #[cfg(windows)]
    job: Option<JoinJob>,
}

impl ProcessTree {
    #[cfg(unix)]
    fn new(child: &mut tokio::process::Child) -> Self {
        // The process group was created at spawn (`process_group(0)`), so the
        // child's pid is also the group id, and a negative pid addresses it.
        Self {
            disarmed: false,
            group: child.id().map(|pid| -(pid as i32)),
        }
    }

    #[cfg(windows)]
    fn new(child: &mut tokio::process::Child) -> Self {
        // Assigned immediately after spawn. A process already running could in
        // principle start a child before the assignment lands; closing that gap
        // needs the thread handle, which tokio does not expose, and a command
        // that forks within microseconds of starting is not the case this
        // defends against — a build's children come seconds later.
        let job = child
            .raw_handle()
            .ok_or_else(|| std::io::Error::other("the child has already exited"))
            .and_then(|handle| JoinJob::create().and_then(|job| job.assign(handle).map(|()| job)))
            .map_err(|error| {
                // Not fatal: the command still runs, and `kill_on_drop` still
                // reaches the shell. Losing the tree is worth a log line, not a
                // failed command.
                tracing::warn!(%error, "bash: could not attach the command to a job object");
                error
            })
            .ok();
        Self {
            disarmed: false,
            job,
        }
    }

    /// Kill the tree now.
    fn kill(&mut self) {
        #[cfg(unix)]
        if let Some(group) = self.group.take() {
            // SAFETY: `kill` on a process group this call created.
            unsafe { libc::kill(group, libc::SIGKILL) };
        }
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            job.terminate();
        }
    }

    /// The command exited on its own; nothing to kill.
    fn disarm(&mut self) {
        self.disarmed = true;
        #[cfg(unix)]
        {
            self.group = None;
        }
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            // A successful launcher may intentionally leave a server running.
            // Timeouts/cancellation retain kill-on-close through the kill path.
            if let Err(error) = job.release_descendants() {
                tracing::warn!(%error, "could not release background command descendants");
            }
        }
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        // A dropped tool call is a cancelled run: the caller will not get to
        // call `kill`, so the tree must not outlive this value.
        if !self.disarmed {
            self.kill();
        }
    }
}

/// A Windows Job Object that kills its members when it closes.
///
/// The property that matters is `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: closing
/// the handle terminates every process in the job, and a process cannot leave
/// once it is in (barring a breakaway flag nothing here sets). That is what
/// makes the tree — including grandchildren the shell never tracked —
/// reachable from one handle.
#[cfg(windows)]
struct JoinJob {
    handle: std::os::windows::io::OwnedHandle,
}

#[cfg(windows)]
use std::os::windows::io::AsRawHandle as _;

#[cfg(windows)]
impl JoinJob {
    fn create() -> std::io::Result<Self> {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        // SAFETY: null attributes create an unnamed, non-inheritable job.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: the call returned a fresh, owned handle.
        let handle = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) };

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: the job handle is live and the buffer is the exact type the
        // information class expects.
        let ok = unsafe {
            SetInformationJobObject(
                handle.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { handle })
    }

    fn assign(&self, process: std::os::windows::io::RawHandle) -> std::io::Result<()> {
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        // SAFETY: both handles are live — the job is owned here, the process
        // is the caller's freshly spawned child.
        let ok = unsafe { AssignProcessToJobObject(self.handle.as_raw_handle(), process) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn release_descendants(&self) -> std::io::Result<()> {
        use windows_sys::Win32::System::JobObjects::{
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };
        let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        // SAFETY: the owned job handle is live and the buffer matches this information class.
        let ok = unsafe {
            SetInformationJobObject(
                self.handle.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Kill every process in the job.
    fn terminate(&self) {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        // SAFETY: the job handle is live; a zero exit code is conventional.
        unsafe { TerminateJobObject(self.handle.as_raw_handle(), 1) };
    }
}

/// The shell a command runs through, resolved once per process.
///
/// The tool is named Bash and models write bash syntax — pipelines, `;`
/// separators, `$?` — so on Windows it prefers a real POSIX shell (Git for
/// Windows puts one on PATH) and only falls back to `cmd` when there is none.
/// Measured on the first live round: with `cmd /C` alone, `ls /; echo ok`
/// fails outright ("cannot access '/;'") and the model burns call after call
/// rediscovering that; with bash, the same commands just run.
#[derive(Clone, Copy)]
pub(crate) enum Shell {
    Posix(&'static str),
    Cmd,
}

pub(crate) fn shell() -> Shell {
    static SHELL: std::sync::OnceLock<Shell> = std::sync::OnceLock::new();
    *SHELL.get_or_init(|| {
        if let Some(found) = find_posix_shell() {
            Shell::Posix(found)
        } else {
            tracing::warn!(
                "no bash found on PATH; falling back to cmd, where bash syntax will fail"
            );
            Shell::Cmd
        }
    })
}

/// The first bash-ish executable on PATH, as a static name if we can help it.
///
/// Returns the common names rather than a full path when they resolve, so the
/// spawned command reads clearly in logs.
#[cfg(windows)]
fn find_posix_shell() -> Option<&'static str> {
    // Git's standard installation adds only cmd/ to PATH, not bin/bash.exe.
    // Resolve beside git.exe before considering the Windows WSL launcher.
    let mut candidates = Vec::new();
    if let Some(git) = which_static("git") {
        if let Some(root) = git.parent().and_then(|parent| parent.parent()) {
            candidates.extend([root.join("bin/bash.exe"), root.join("usr/bin/bash.exe")]);
        }
    }
    for variable in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(variable) {
            let root = std::path::PathBuf::from(root);
            candidates.extend([
                root.join("Git/bin/bash.exe"),
                root.join("Programs/Git/bin/bash.exe"),
            ]);
        }
    }
    for name in ["bash", "sh"] {
        if let Some(path) = which_static(name) {
            candidates.push(path);
        }
    }
    candidates
        .into_iter()
        .find(|path| {
            path.is_file()
                && !path
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("windows\\system32")
        })
        .map(|path| &*Box::leak(path.to_string_lossy().into_owned().into_boxed_str()))
}

#[cfg(not(windows))]
fn find_posix_shell() -> Option<&'static str> {
    Some(if std::path::Path::new("/bin/bash").is_file() {
        "/bin/bash"
    } else {
        "sh"
    })
}

/// `which`, without a dependency: walk PATH ourselves looking for `name.exe`.
#[cfg(windows)]
fn which_static(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    let extensions = std::env::var_os("PATHEXT")
        .map(|ext| {
            ext.to_string_lossy()
                .split(';')
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_else(|| vec![".exe".to_owned()]);
    for dir in std::env::split_paths(&path) {
        for extension in &extensions {
            let candidate = dir.join(format!("{name}{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn shell_command(command: &str) -> tokio::process::Command {
    match shell() {
        Shell::Posix(program) => {
            let mut process = tokio::process::Command::new(program);
            process.arg("-c").arg(command);
            process
        }
        Shell::Cmd => {
            let mut process = tokio::process::Command::new("cmd");
            process.arg("/C").arg(command);
            process
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    fn context(cwd: &std::path::Path) -> ToolContext {
        ToolContext {
            cwd: cwd.to_path_buf(),
            cancel: CancellationToken::new(),
            nested: None,
        }
    }

    fn echo_command(text: &str) -> String {
        #[cfg(windows)]
        {
            format!("echo {text}")
        }
        #[cfg(not(windows))]
        {
            format!("echo {text}")
        }
    }

    #[tokio::test]
    async fn a_successful_command_reports_its_output() {
        let dir = tempfile::tempdir().unwrap();
        let result = BashTool
            .call(
                json!({ "command": echo_command("hello") }),
                &context(dir.path()),
            )
            .await
            .expect("runs");

        assert!(!result.is_error, "output was: {}", result.output);
        assert!(result.output.contains("hello"));
        assert!(result.output.contains("exit 0"));
    }

    #[tokio::test]
    async fn a_failing_command_is_an_error_result_not_a_tool_error() {
        let dir = tempfile::tempdir().unwrap();
        // A command that exits non-zero: the model should see the exit code, so
        // this is an error *result*, not a failed call.
        let command = "exit 3";
        let result = BashTool
            .call(json!({ "command": command }), &context(dir.path()))
            .await
            .expect("runs");

        assert!(result.is_error);
        assert!(result.output.contains("exit 3"));
    }

    #[tokio::test]
    async fn an_empty_command_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let error = BashTool
            .call(json!({ "command": "   " }), &context(dir.path()))
            .await
            .expect_err("empty");
        assert!(matches!(error, ToolError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn a_cancelled_context_stops_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let ctx = ToolContext {
            cwd: dir.path().to_path_buf(),
            cancel,
            nested: None,
        };

        let outcome = BashTool
            .call(json!({ "command": echo_command("hi") }), &ctx)
            .await;
        // Either the race picked cancellation, or the command finished first —
        // both are acceptable; what matters is it does not hang.
        match outcome {
            Err(ToolError::Cancelled) => {}
            Ok(_) => {}
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn bash_syntax_works_when_a_posix_shell_is_selected() {
        // The defect the first live round found: `ls /; echo ok` sent through
        // cmd fails ("cannot access '/;'"), so the model burns calls guessing
        // shell syntax. The name Bash promises this syntax, and this test
        // makes that promise checkable.
        if !matches!(shell(), Shell::Posix(_)) {
            return; // No POSIX shell available on this machine.
        }
        let dir = tempfile::tempdir().unwrap();
        let result = BashTool
            .call(
                json!({ "command": "echo first; echo second; echo status=$?" }),
                &context(dir.path()),
            )
            .await
            .expect("runs");
        assert!(!result.is_error, "output: {}", result.output);
        assert!(result.output.contains("first"));
        assert!(result.output.contains("second"));
        assert!(result.output.contains("status=0"));
    }

    #[tokio::test]
    async fn the_working_directory_is_the_command_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("marker.txt"), "x").unwrap();

        let command = match shell() {
            Shell::Posix(_) => "ls",
            Shell::Cmd => "dir /b",
        };
        let result = BashTool
            .call(json!({ "command": command }), &context(dir.path()))
            .await
            .expect("runs");

        assert!(
            result.output.contains("marker.txt"),
            "output: {}",
            result.output
        );
    }

    // === Process-tree ownership (F04) ===

    /// The command that starts a child which outlives it, then waits.
    ///
    /// The child sleeps briefly and then writes a file. If the tree is killed
    /// when the shell is killed, the file never appears; if only the shell dies,
    /// the file shows up a moment later.
    fn forking_command(marker: &std::path::Path) -> String {
        let marker = marker.display();
        match shell() {
            Shell::Posix(_) => {
                // A subshell in the background: it is not the shell the tool
                // spawned, and killing that shell alone would leave it running.
                format!("( sleep 2; echo CHILD_SURVIVED > '{marker}' ) & wait")
            }
            Shell::Cmd => {
                // `start /b` detaches a second process without a window.
                format!(
                    "start /b cmd /c \"ping -n 3 127.0.0.1 >nul & echo CHILD_SURVIVED> \"{marker}\"\""
                )
            }
        }
    }

    #[tokio::test]
    async fn a_timeout_kills_the_children_the_command_started() {
        // The bug this guards: `kill_on_drop` reaches only the shell, so a
        // timed-out command's grandchildren kept running — still writing files
        // and holding resources after the tool said "it was killed".
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("survived.txt");

        let error = BashTool
            .call(
                json!({ "command": forking_command(&marker), "timeout": 1 }),
                &context(dir.path()),
            )
            .await
            .expect_err("the command should time out");
        assert!(error.to_string().contains("did not finish"), "got: {error}");

        // Wait past when the child would have written, then check it did not.
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(
            !marker.exists(),
            "a child of the timed-out command survived and wrote {}",
            marker.display()
        );
    }

    #[tokio::test]
    async fn cancelling_kills_the_children_too() {
        // Stopping a run must stop what it started, by the same reasoning.
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("survived.txt");
        let cancel = CancellationToken::new();
        let ctx = ToolContext {
            cwd: dir.path().to_path_buf(),
            cancel: cancel.clone(),
            nested: None,
        };

        let command = forking_command(&marker);
        let handle = tokio::spawn(async move {
            BashTool
                .call(json!({ "command": command, "timeout": 30 }), &ctx)
                .await
        });
        // Let the shell start its child before cancelling.
        tokio::time::sleep(Duration::from_millis(400)).await;
        cancel.cancel();

        let error = handle
            .await
            .expect("the task finished")
            .expect_err("cancelled");
        assert!(matches!(error, ToolError::Cancelled), "got: {error}");

        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(
            !marker.exists(),
            "a child of the cancelled command survived and wrote {}",
            marker.display()
        );
    }

    #[tokio::test]
    async fn a_command_that_finishes_normally_is_not_killed_late() {
        // The guard must not take down a command that succeeded: the tree is
        // disarmed once the process exits.
        let dir = tempfile::tempdir().unwrap();
        let result = BashTool
            .call(
                json!({ "command": echo_command("done") }),
                &context(dir.path()),
            )
            .await
            .expect("runs");
        assert!(!result.is_error, "output: {}", result.output);
        assert!(result.output.contains("done"));
    }

    // === Bounded output (F15) ===

    #[tokio::test]
    async fn a_high_output_command_is_capped_while_it_runs() {
        // The cap must apply during collection, not after: the previous
        // implementation buffered everything and truncated at the end, so a
        // long command could exhaust memory before the cap ever applied.
        let dir = tempfile::tempdir().unwrap();
        let command = match shell() {
            Shell::Posix(_) => "seq 1 500000".to_owned(),
            Shell::Cmd => "for /l %i in (1,1,500000) do @echo %i".to_owned(),
        };

        let result = BashTool
            .call(json!({ "command": command }), &context(dir.path()))
            .await
            .expect("runs");

        // The output is capped near MAX_OUTPUT_CHARS and says what it dropped.
        assert!(
            result.output.chars().count() < MAX_OUTPUT_CHARS + 1_000,
            "output was not capped: {} chars",
            result.output.chars().count()
        );
        assert!(
            result.output.contains("more lines dropped") || result.output.contains("truncated"),
            "a capped output must say so: {}",
            &result.output[result.output.len().saturating_sub(200)..]
        );
    }
}
