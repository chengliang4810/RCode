//! `Bash` — run a shell command.
//!
//! Commands run with the host process permissions, like the file tools under
//! the application's trust-all policy. The working directory sets the initial
//! location; it does not restrict filesystem access.
//!
//! One model-visible tool, with PowerShell on Windows and Bash on Unix/macOS.
//! Windows scripts travel through a file rather than command-line quoting;
//! stdin stays closed so child commands cannot consume the script.

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

use crate::tool::{Tool, ToolContext, ToolError, ToolResult};

/// Default ceiling on a command.
const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Hard ceiling, whatever the caller asks for.
const MAX_TIMEOUT_SECS: u64 = 600;
/// Combined UTF-8 output kept before truncation, excluding the result header.
const MAX_OUTPUT_BYTES: usize = 100_000;
const READ_CHUNK_BYTES: usize = 8192;
/// An exited launcher must not leave the tool waiting forever for inherited pipes.
const POST_EXIT_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const PROCESS_REAP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
pub(crate) enum CommandShell {
    Bash,
    #[cfg(windows)]
    PowerShell,
}

pub struct BashTool;

pub(crate) fn platform_shell() -> CommandShell {
    if cfg!(windows) {
        #[cfg(windows)]
        return CommandShell::PowerShell;
    }
    CommandShell::Bash
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn description(&self) -> &str {
        #[cfg(windows)]
        return "Run a shell command. On Windows the Bash tool executes PowerShell syntax, not Bash syntax. Uses pwsh when installed, otherwise Windows PowerShell 5.1, without profiles or interactive input. Send PowerShell directly; do not wrap it in another shell. Prefer Read/Write/Edit, Glob/Grep and FolderOperations for ordinary file work. Cmdlet errors and nonzero native exit codes are failures.";
        #[cfg(not(windows))]
        "Run a shell command (bash syntax). Prefer the purpose-built tools: \
         Read/Write/Edit for files, Glob/Grep for searching, FolderOperations \
         for listings — they give structured output. Use Bash for builds, tests, version control and anything \
         without a dedicated tool. Bash pipelines report a failure if any stage fails (pipefail). \
         For an expected nonzero status, handle it explicitly with shell conditionals."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": if cfg!(windows) { "PowerShell script to execute directly on Windows." } else { "Bash command to execute." }
                },
                "timeout": timeout_schema()
            },
            "required": ["command"]
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        execute(input, ctx, true, None).await
    }
}

pub(crate) fn timeout_schema() -> Value {
    json!({
        "type": "integer",
        "minimum": 1,
        "maximum": MAX_TIMEOUT_SECS,
        "description": "Timeout in seconds, not milliseconds. Defaults to 120; values above 600 are capped at 600."
    })
}

pub(crate) fn timeout_seconds(input: &Value) -> Result<u64, ToolError> {
    match input.get("timeout") {
        None => Ok(DEFAULT_TIMEOUT_SECS),
        Some(value) => value
            .as_u64()
            .filter(|seconds| *seconds > 0)
            .map(|seconds| seconds.min(MAX_TIMEOUT_SECS))
            .ok_or_else(|| {
                ToolError::InvalidInput("`timeout` must be a positive integer in seconds".into())
            }),
    }
}

pub(crate) async fn execute(
    input: Value,
    ctx: &ToolContext,
    timed: bool,
    progress: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
) -> Result<ToolResult, ToolError> {
    execute_with_shell(platform_shell(), input, ctx, timed, progress).await
}

pub(crate) async fn execute_with_shell(
    shell: CommandShell,
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
    if command.contains('\0') {
        return Err(ToolError::InvalidInput(
            "`command` contains a NUL byte".into(),
        ));
    }
    let seconds = timeout_seconds(&input)?;

    // Keep the temporary script alive for the whole process lifetime, including
    // managed background execution, timeout and cancellation cleanup.
    let (mut process, _script) = match shell {
        CommandShell::Bash => shell_command(command)?,
        #[cfg(windows)]
        CommandShell::PowerShell => super::powershell::shell_command(command)?,
    };
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
        // Attach the suspended leader to its Job Object before any user code
        // can start children. Resume only after the ownership is established.
        use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
        process.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
    }

    let mut child = process
        .spawn()
        .map_err(|error| ToolError::Execution(format!("cannot start `{command}`: {error}")))?;

    // Take the pipes before waiting so output can be drained as it
    // arrives. `wait_with_output` would buffer everything the command ever
    // wrote — a runaway `yes` or a 10-minute build log accumulates
    // unboundedly and can exhaust the process's memory before the cap is
    // ever applied.
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();

    // Own the tree, not just the shell. `kill_on_drop` reaches the direct
    // child only; a command that starts its own children (any build, any
    // `sh -c '… &'`) leaves them running after the shell dies, still
    // writing files and holding resources. Verified on Windows: a 1-second
    // timeout returned "it was killed" while the grandchild kept going.
    let mut tree = match ProcessTree::new(&mut child) {
        Ok(tree) => tree,
        Err(error) => {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(PROCESS_REAP_TIMEOUT, child.wait()).await;
            return Err(ToolError::Execution(format!(
                "cannot own the command's process tree: {error}"
            )));
        }
    };

    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut remaining = MAX_OUTPUT_BYTES;
    let mut stdout_buf = BoundedOutput::default();
    let mut stderr_buf = BoundedOutput::default();
    let mut out_chunk = [0; READ_CHUNK_BYTES];
    let mut err_chunk = [0; READ_CHUNK_BYTES];
    let mut out_done = stdout.is_none();
    let mut err_done = stderr.is_none();
    let mut exit_status = None;
    let mut drain_deadline = None;

    let status = loop {
        if let Some(status) = exit_status {
            if out_done && err_done {
                break status;
            }
        }
        tokio::select! {
            // Cancellation/deadlines remain active after wait() completes,
            // even if descendants keep a pipe open or continuously write.
            biased;
            _ = ctx.cancel.cancelled() => {
                stop_process_tree(&mut child, &mut tree).await;
                return Err(ToolError::Cancelled);
            }
            _ = tokio::time::sleep_until(deadline), if timed => {
                stop_process_tree(&mut child, &mut tree).await;
                return Err(ToolError::Timeout(format!(
                    "`{command}` did not finish within {seconds}s; it and everything it \
                     started were killed"
                )));
            }
            _ = tokio::time::sleep_until(drain_deadline.unwrap_or(deadline)), if drain_deadline.is_some() => {
                if tree.managed.is_some() {
                    // The launcher is gone but a registered service still owns
                    // inherited pipe handles. Retain OS ownership in the
                    // registry and drain without holding up the tool result.
                    let out = stdout.take();
                    let err = stderr.take();
                    tokio::spawn(async move {
                        let drain_out = async move {
                            if let Some(mut pipe) = out { let _ = tokio::io::copy(&mut pipe, &mut tokio::io::sink()).await; }
                        };
                        let drain_err = async move {
                            if let Some(mut pipe) = err { let _ = tokio::io::copy(&mut pipe, &mut tokio::io::sink()).await; }
                        };
                        tokio::join!(drain_out, drain_err);
                    });
                    break exit_status.expect("drain deadline follows launcher exit");
                }
                stop_process_tree(&mut child, &mut tree).await;
                return Err(ToolError::Execution(
                    "command exited but its inherited output pipes did not close within 2s; \
                     its remaining child processes were killed".to_owned()
                ));
            }
            result = child.wait(), if exit_status.is_none() => {
                match result {
                    Ok(status) => {
                        exit_status = Some(status);
                        drain_deadline = Some(tokio::time::Instant::now() + POST_EXIT_DRAIN_TIMEOUT);
                    }
                    Err(error) => {
                        stop_process_tree(&mut child, &mut tree).await;
                        return Err(ToolError::Execution(error.to_string()));
                    }
                }
            }
            (is_stdout, result) = async {
                // Keep control signals prioritized above, but let the two
                // readable streams make fair progress even under continuous output.
                tokio::select! {
                    result = async { stdout.as_mut().unwrap().read(&mut out_chunk).await }, if !out_done => (true, result),
                    result = async { stderr.as_mut().unwrap().read(&mut err_chunk).await }, if !err_done => (false, result),
                }
            }, if !out_done || !err_done => {
                let (buffer, done, chunk, name) = if is_stdout {
                    (&mut stdout_buf, &mut out_done, &out_chunk, "stdout")
                } else {
                    (&mut stderr_buf, &mut err_done, &err_chunk, "stderr")
                };
                let changed = match result {
                    Ok(0) => { *done = true; buffer.finish(&mut remaining) }
                    Ok(count) => buffer.push_bytes(&chunk[..count], &mut remaining),
                    Err(error) => {
                        stop_process_tree(&mut child, &mut tree).await;
                        return Err(ToolError::Execution(format!("reading command {name}: {error}")));
                    }
                };
                if changed {
                    if let Some(progress) = &progress {
                        progress(output_text(&stdout_buf, &stderr_buf));
                    }
                }
            }
        }
    };

    // Both streams reached EOF. A launcher which explicitly redirected its
    // descendants' output may leave them running on successful completion.
    tree.disarm();

    let mut body = output_text(&stdout_buf, &stderr_buf);
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

async fn stop_process_tree(child: &mut tokio::process::Child, tree: &mut ProcessTree) {
    tree.kill();
    // Also cover failures before the leader could join its process group/job.
    let _ = child.start_kill();
    let _ = tokio::time::timeout(PROCESS_REAP_TIMEOUT, child.wait()).await;
}

fn output_text(stdout: &BoundedOutput, stderr: &BoundedOutput) -> String {
    let mut text = stdout.text.trim_end().to_owned();
    if !stderr.text.trim().is_empty() {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str("stderr:\n");
        text.push_str(stderr.text.trim_end());
    }
    if stdout.truncated || stderr.truncated {
        text.push_str("\n… (output truncated)");
    }
    text
}

/// Output held for the model, capped as it arrives.
///
/// The cap has to apply *during* collection, not after: a command that prints
/// for ten minutes produces as much as it likes in that time, and buffering it
/// all before trimming means the process's memory is bounded by the command
/// rather than by the cap. That is the difference between a slow command and
/// an out-of-memory crash.
#[derive(Default)]
struct BoundedOutput {
    text: String,
    /// At most three bytes of an incomplete UTF-8 character across reads.
    pending: Vec<u8>,
    truncated: bool,
}

impl BoundedOutput {
    fn push_text(&mut self, text: &str, remaining: &mut usize) -> bool {
        let mut end = text.len().min(*remaining);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.text.push_str(&text[..end]);
        if end < text.len() {
            // Keep a prefix, never part of a character. Once full, neither
            // stream allocates for new output; the pipe still gets drained.
            let changed = end > 0 || !self.truncated;
            self.truncated = true;
            *remaining = 0;
            changed
        } else {
            *remaining -= end;
            end > 0
        }
    }

    fn push_bytes(&mut self, bytes: &[u8], remaining: &mut usize) -> bool {
        let mut changed = false;
        for chunk in bytes.chunks(READ_CHUNK_BYTES) {
            if *remaining == 0 {
                self.pending.clear();
                changed |= !self.truncated;
                self.truncated = true;
                break;
            }
            let mut joined = std::mem::take(&mut self.pending);
            joined.extend_from_slice(chunk);
            let mut undecoded = joined.as_slice();
            while !undecoded.is_empty() {
                match std::str::from_utf8(undecoded) {
                    Ok(text) => {
                        changed |= self.push_text(text, remaining);
                        break;
                    }
                    Err(error) => {
                        let valid = error.valid_up_to();
                        changed |= self.push_text(
                            std::str::from_utf8(&undecoded[..valid])
                                .expect("validated UTF-8 prefix"),
                            remaining,
                        );
                        if let Some(invalid) = error.error_len() {
                            changed |= self.push_text("\u{fffd}", remaining);
                            undecoded = &undecoded[valid + invalid..];
                        } else {
                            self.pending.extend_from_slice(&undecoded[valid..]);
                            break;
                        }
                    }
                }
                if *remaining == 0 {
                    if !undecoded.is_empty() {
                        changed |= !self.truncated;
                        self.truncated = true;
                    }
                    break;
                }
            }
        }
        changed
    }

    fn finish(&mut self, remaining: &mut usize) -> bool {
        if self.pending.is_empty() {
            false
        } else {
            self.pending.clear();
            self.push_text("\u{fffd}", remaining)
        }
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
    managed: Option<(std::sync::Arc<crate::processes::ProcessRegistry>, String)>,
    /// Set once the command finished cleanly, so `Drop` does not kill a job
    /// that already exited.
    disarmed: bool,
    #[cfg(unix)]
    group: Option<i32>,
    #[cfg(windows)]
    job: Option<std::sync::Arc<JoinJob>>,
}

impl ProcessTree {
    #[cfg(unix)]
    fn new(child: &mut tokio::process::Child) -> std::io::Result<Self> {
        // The process group was created at spawn (`process_group(0)`), so the
        // child's pid is also the group id, and a negative pid addresses it.
        let mut tree = Self {
            managed: None,
            disarmed: false,
            group: child.id().map(|pid| -(pid as i32)),
        };
        if let Ok(result) = crate::processes::REGISTRATION.try_with(|registration| {
            let mut record = registration.record.clone();
            record.pid = child.id().ok_or_else(|| std::io::Error::other("missing child pid"))?;
            let control = crate::processes::Control::Group { pid: record.pid, members: std::sync::Mutex::new(crate::processes::group_members(record.pid)?) };
            registration.registry.register(record.clone(), control, registration.stop.clone())?;
            Ok::<_, std::io::Error>((registration.registry.clone(), record.id))
        }) { tree.managed = Some(result?); }
        Ok(tree)
    }

    #[cfg(windows)]
    fn new(child: &mut tokio::process::Child) -> std::io::Result<Self> {
        let process = child
            .raw_handle()
            .ok_or_else(|| std::io::Error::other("the suspended child has no process handle"))?;
        let pid = child
            .id()
            .ok_or_else(|| std::io::Error::other("the suspended child has no process ID"))?;
        let name = crate::processes::REGISTRATION.try_with(|registration| registration.record.job_name.clone()).ok();
        let job = std::sync::Arc::new(JoinJob::create_named(name.as_deref())?);
        job.assign(process)?;
        // Persist ownership before user code can create descendants. Leave
        // kill-on-close armed until the journal is safely on disk.
        let managed = crate::processes::REGISTRATION.try_with(|registration| {
            let mut record = registration.record.clone();
            record.pid = pid;
            registration.registry.register(record.clone(), crate::processes::Control::Job(job.clone()), registration.stop.clone())?;
            job.retain_in_members()?;
            job.release_descendants()?;
            Ok::<_, std::io::Error>((registration.registry.clone(), record.id))
        }).ok().transpose()?;
        resume_suspended_leader(pid)?;
        Ok(Self {
            managed,
            disarmed: false,
            job: Some(job),
        })
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
        if let Some((registry, id)) = &self.managed {
            #[cfg(windows)]
            if let Some(job) = &self.job {
                if let Err(error) = job.retain_in_members() {
                    tracing::warn!(%error, "could not retain recovery handle in command descendants");
                }
            }
            registry.refresh(id);
            return;
        }
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

/// Tokio exposes the suspended process handle but not its primary thread.
/// Resolve that one thread without allowing the command to execute first.
#[cfg(windows)]
fn resume_suspended_leader(pid: u32) -> std::io::Result<()> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::{ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessIdOfThread, OpenThread, ResumeThread, THREAD_QUERY_LIMITED_INFORMATION,
        THREAD_SUSPEND_RESUME,
    };

    // SAFETY: a fresh, non-inherited thread snapshot is owned by this function.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    if unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut primary = None;
    loop {
        if entry.th32OwnerProcessID == pid {
            if primary.is_some() {
                return Err(std::io::Error::other(
                    "suspended command has multiple threads",
                ));
            }
            let handle = unsafe {
                OpenThread(
                    THREAD_QUERY_LIMITED_INFORMATION | THREAD_SUSPEND_RESUME,
                    0,
                    entry.th32ThreadID,
                )
            };
            if handle.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let thread = unsafe { OwnedHandle::from_raw_handle(handle) };
            if unsafe { GetProcessIdOfThread(thread.as_raw_handle()) } != pid {
                return Err(std::io::Error::other(
                    "suspended command thread identity changed",
                ));
            }
            primary = Some(thread);
        }
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        if unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_NO_MORE_FILES as i32) {
                return Err(error);
            }
            break;
        }
    }
    let primary = primary
        .ok_or_else(|| std::io::Error::other("suspended command primary thread is missing"))?;
    match unsafe { ResumeThread(primary.as_raw_handle()) } {
        1 => Ok(()),
        u32::MAX => Err(std::io::Error::last_os_error()),
        _ => Err(std::io::Error::other(
            "command primary thread was not suspended once",
        )),
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
pub(crate) struct JoinJob {
    handle: std::os::windows::io::OwnedHandle,
}

#[cfg(windows)]
use std::os::windows::io::AsRawHandle as _;

#[cfg(windows)]
impl JoinJob {
    fn create_named(name: Option<&str>) -> std::io::Result<Self> {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        // SAFETY: null attributes create an unnamed, non-inheritable job.
        let name = name.map(|name| name.encode_utf16().chain(Some(0)).collect::<Vec<_>>());
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), name.as_ref().map_or(std::ptr::null(), |name| name.as_ptr())) };
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

    pub(crate) fn open(name: &str) -> std::io::Result<Self> {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::System::JobObjects::OpenJobObjectW;
        use windows_sys::Win32::System::SystemServices::{JOB_OBJECT_QUERY, JOB_OBJECT_TERMINATE};
        if !name.strip_prefix("Local\\RCode.Command.").is_some_and(|id| uuid::Uuid::parse_str(id).is_ok()) {
            return Err(std::io::Error::other("invalid command job name"));
        }
        let name: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY | JOB_OBJECT_TERMINATE, 0, name.as_ptr()) };
        if handle.is_null() { return Err(std::io::Error::last_os_error()); }
        Ok(Self { handle: unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) } })
    }

    pub(crate) fn active(&self) -> std::io::Result<bool> {
        use windows_sys::Win32::System::JobObjects::{QueryInformationJobObject, JobObjectBasicAccountingInformation, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION};
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        if unsafe { QueryInformationJobObject(self.handle.as_raw_handle(), JobObjectBasicAccountingInformation, (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(), std::mem::size_of_val(&info) as u32, std::ptr::null_mut()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(info.ActiveProcesses > 0)
    }

    pub(crate) fn stop(&self) -> std::io::Result<()> {
        if unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.handle.as_raw_handle(), 1) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Keep the named job reachable after the host is forcibly terminated.
    /// Windows removes a job's name when its last handle closes even when its
    /// processes survive. Each live member receives an inheritable reference;
    /// the OS closes that reference when the member exits. Manual/normal-exit
    /// termination still targets the entire job, including these references.
    fn retain_in_members(&self) -> std::io::Result<()> {
        use std::os::windows::io::{FromRawHandle, OwnedHandle};
        use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, ERROR_MORE_DATA};
        use windows_sys::Win32::System::JobObjects::{IsProcessInJob, QueryInformationJobObject, JobObjectBasicProcessIdList, JOBOBJECT_BASIC_PROCESS_ID_LIST};
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcess, PROCESS_DUP_HANDLE, PROCESS_QUERY_LIMITED_INFORMATION};
        let mut storage = vec![0usize; 64];
        loop {
            let ok = unsafe { QueryInformationJobObject(self.handle.as_raw_handle(), JobObjectBasicProcessIdList, storage.as_mut_ptr().cast(), (storage.len() * std::mem::size_of::<usize>()) as u32, std::ptr::null_mut()) };
            if ok != 0 { break; }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_MORE_DATA as i32) || storage.len() >= 1_048_576 { return Err(error); }
            storage.resize(storage.len() * 2, 0);
        }
        let info = unsafe { &*storage.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>() };
        let pids = unsafe { std::slice::from_raw_parts(info.ProcessIdList.as_ptr(), info.NumberOfProcessIdsInList as usize) };
        for pid in pids {
            let handle = unsafe { OpenProcess(PROCESS_DUP_HANDLE | PROCESS_QUERY_LIMITED_INFORMATION, 0, *pid as u32) };
            if handle.is_null() { continue; } // The member may have just exited.
            let process = unsafe { OwnedHandle::from_raw_handle(handle) };
            let mut member = 0;
            if unsafe { IsProcessInJob(process.as_raw_handle(), self.handle.as_raw_handle(), &mut member) } == 0 || member == 0 { continue; }
            let mut remote = std::ptr::null_mut();
            if unsafe { DuplicateHandle(GetCurrentProcess(), self.handle.as_raw_handle(), process.as_raw_handle(), &mut remote, 0, 1, DUPLICATE_SAME_ACCESS) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            // `remote` belongs to the member process, not this process.
        }
        Ok(())
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
pub(super) fn which_static(name: &str) -> Option<std::path::PathBuf> {
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

fn shell_command(
    command: &str,
) -> Result<(tokio::process::Command, Option<tempfile::TempPath>), ToolError> {
    match shell() {
        Shell::Posix(program) => {
            let mut process = tokio::process::Command::new(program);
            // Probe in a subshell: older /bin/sh implementations reject
            // pipefail as a fatal option error. Do not enable errexit; callers
            // still control conditionals, explicit recovery and sequencing.
            let script =
                format!("if (set -o pipefail) 2>/dev/null; then set -o pipefail; fi\n{command}");
            #[cfg(windows)]
            {
                use std::io::Write;
                // Rust/Windows argv quoting and MSYS parsing disagree about
                // backslashes in -c payloads, even inside quoted heredocs.
                // A UTF-8 file avoids that translation and command-line limits
                // without exposing the script to child processes via stdin.
                let mut file = tempfile::Builder::new()
                    .prefix("rcode-bash-")
                    .suffix(".sh")
                    .tempfile()
                    .map_err(|error| {
                        ToolError::Execution(format!("cannot create shell script: {error}"))
                    })?;
                file.write_all(script.as_bytes()).map_err(|error| {
                    ToolError::Execution(format!("cannot write shell script: {error}"))
                })?;
                let path = file.into_temp_path();
                process.arg(path.to_string_lossy().replace('\\', "/"));
                Ok((process, Some(path)))
            }
            #[cfg(not(windows))]
            {
                process.arg("-c").arg(script);
                Ok((process, None))
            }
        }
        Shell::Cmd => {
            let mut process = tokio::process::Command::new("cmd");
            process.arg("/C").arg(command);
            Ok((process, None))
        }
    }
}

#[cfg(test)]
#[path = "bash_script_tests.rs"]
mod script_tests;

#[cfg(test)]
#[path = "bash_regression_tests.rs"]
mod regression_tests;

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
        if !matches!(platform_shell(), CommandShell::Bash) || !matches!(shell(), Shell::Posix(_)) {
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
        if cfg!(windows) {
            return format!(
                "Start-Job -ScriptBlock {{ param($path); Start-Sleep -Seconds 2; [IO.File]::WriteAllText($path, 'CHILD_SURVIVED') }} -ArgumentList '{}' | Wait-Job | Receive-Job",
                marker.to_string().replace('\'', "''")
            );
        }
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
        let command = if cfg!(windows) {
            "[Console]::Write(('x' * 200000))".to_owned()
        } else {
            match shell() {
                Shell::Posix(_) => "seq 1 500000".to_owned(),
                Shell::Cmd => "for /l %i in (1,1,500000) do @echo %i".to_owned(),
            }
        };

        let result = BashTool
            .call(json!({ "command": command }), &context(dir.path()))
            .await
            .expect("runs");

        // The output is capped near MAX_OUTPUT_BYTES and says what it dropped.
        assert!(
            result.output.len() < MAX_OUTPUT_BYTES + 1_000,
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
