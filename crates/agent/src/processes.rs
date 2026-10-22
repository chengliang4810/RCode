//! Local command ownership, independent of model turns. The journal contains
//! identities of trees we created, never arbitrary PIDs supplied by the UI.
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessRecord {
    pub id: String,
    pub chat_id: String,
    pub command: String,
    pub cwd: PathBuf,
    pub pid: u32,
    pub job_name: String,
    #[serde(default)]
    pub output_path: Option<PathBuf>,
    #[serde(default = "running_status")]
    pub status: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub rotated: bool,
    #[serde(default)]
    pub background: bool,
    #[serde(default)]
    pub members: Vec<(u32, String)>,
    #[serde(skip_deserializing)]
    pub recovered: bool,
}

fn running_status() -> String {
    "running".into()
}

pub(crate) struct Registration {
    pub registry: Arc<ProcessRegistry>,
    pub record: ProcessRecord,
    pub stop: Arc<dyn Fn() + Send + Sync>,
}
tokio::task_local! { pub(crate) static REGISTRATION: Registration; }

pub(crate) struct TrackedShell {
    pub tool: Arc<dyn crate::Tool>,
    pub chat: String,
    pub registry: Arc<ProcessRegistry>,
}
#[async_trait::async_trait]
impl crate::Tool for TrackedShell {
    fn name(&self) -> &str {
        self.tool.name()
    }
    fn description(&self) -> &str {
        self.tool.description()
    }
    fn input_schema(&self) -> serde_json::Value {
        self.tool.input_schema()
    }
    async fn call(
        &self,
        input: serde_json::Value,
        ctx: &crate::ToolContext,
    ) -> Result<crate::ToolResult, crate::ToolError> {
        let cancel = ctx.cancel.child_token();
        let stop = cancel.clone();
        let owned = crate::ToolContext {
            cwd: ctx.cwd.clone(),
            cancel,
            nested: ctx.nested.clone(),
        };
        let id = format!("bash_{}", uuid::Uuid::new_v4());
        let output_path = self
            .registry
            .output_path(&id)
            .map_err(|e| crate::ToolError::Execution(e.to_string()))?;
        let registration = Registration {
            registry: self.registry.clone(),
            record: ProcessRecord {
                id,
                output_path: Some(output_path),
                status: running_status(),
                exit_code: None,
                rotated: false,
                chat_id: self.chat.clone(),
                command: input["command"].as_str().unwrap_or_default().to_owned(),
                cwd: ctx.cwd.clone(),
                pid: 0,
                job_name: format!("Local\\RCode.Command.{}", uuid::Uuid::new_v4()),
                background: false,
                members: vec![],
                recovered: false,
            },
            stop: Arc::new(move || stop.cancel()),
        };
        REGISTRATION
            .scope(registration, self.tool.call(input, &owned))
            .await
    }
}

pub(crate) enum Control {
    Finished,
    #[cfg(windows)]
    Job(Arc<crate::tools::bash::JoinJob>),
    #[cfg(unix)]
    Group {
        pid: u32,
        members: Mutex<Vec<(u32, String)>>,
    },
}
impl Control {
    fn alive(&self) -> std::io::Result<bool> {
        match self {
            Self::Finished => Ok(false),
            #[cfg(windows)]
            Self::Job(job) => job.active(),
            #[cfg(unix)]
            Self::Group { pid, members } => {
                let current = group_members(*pid)?;
                let mut previous = members.lock().unwrap();
                let alive = previous.iter().any(|member| current.contains(member));
                if alive {
                    *previous = current;
                }
                Ok(alive)
            }
        }
    }
    fn kill(&self) -> std::io::Result<()> {
        match self {
            Self::Finished => Ok(()),
            #[cfg(windows)]
            Self::Job(job) => job.stop(),
            #[cfg(unix)]
            Self::Group { pid, .. } => {
                if !self.alive()? {
                    return Ok(());
                }
                if unsafe { libc::kill(-(*pid as i32), libc::SIGKILL) } == 0 {
                    Ok(())
                } else {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() == Some(libc::ESRCH) {
                        Ok(())
                    } else {
                        Err(error)
                    }
                }
            }
        }
    }
}
struct Entry {
    record: ProcessRecord,
    control: Control,
    stop: Option<Arc<dyn Fn() + Send + Sync>>,
    launcher_pending: bool,
}
#[derive(Default)]
struct State {
    entries: HashMap<String, Entry>,
    promotions: HashSet<String>,
    closed: bool,
}
#[derive(Default)]
pub struct ProcessRegistry {
    directory: Option<PathBuf>,
    state: Mutex<State>,
}
impl ProcessRegistry {
    pub fn start_maintenance(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    let Some(registry) = weak.upgrade() else {
                        break;
                    };
                    if registry.state.lock().unwrap().closed {
                        break;
                    }
                    let _ = tokio::task::spawn_blocking(move || registry.list("")).await;
                }
            });
        }
    }
    pub fn open(root: Option<&std::path::Path>) -> Self {
        let registry = Self {
            directory: root.map(|p| p.join("command-processes")),
            state: Mutex::default(),
        };
        if let Some(directory) = &registry.directory {
            if let Ok(files) = std::fs::read_dir(directory) {
                for file in files.flatten() {
                    if file.path().extension().and_then(|s| s.to_str()) != Some("json") {
                        continue;
                    }
                    let result = (|| -> std::io::Result<()> {
                        let mut record: ProcessRecord =
                            serde_json::from_slice(&std::fs::read(file.path())?)?;
                        if file.file_name().to_string_lossy() != format!("{}.json", record.id)
                            || !valid_id(&record.id)
                        {
                            return Err(std::io::Error::other("invalid process journal identity"));
                        }
                        let recovered = recover_control(&record);
                        let control = match recovered {
                            Ok(control) if control.alive()? => {
                                record.recovered = true;
                                record.background = true;
                                control
                            }
                            Ok(_) => Control::Finished,
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                Control::Finished
                            }
                            Err(error) => return Err(error),
                        };
                        if matches!(control, Control::Finished) {
                            if record
                                .output_path
                                .as_ref()
                                .is_none_or(|path| !path.exists())
                            {
                                return Err(std::io::Error::from(std::io::ErrorKind::NotFound));
                            }
                            settle_record(&mut record);
                            registry.persist(&record)?;
                            return Ok(()); // Terminal output stays on disk, not in the live registry.
                        }
                        registry.state.lock().unwrap().entries.insert(
                            record.id.clone(),
                            Entry {
                                record,
                                control,
                                stop: None,
                                launcher_pending: false,
                            },
                        );
                        Ok(())
                    })();
                    if let Err(error) = result {
                        if error.kind() == std::io::ErrorKind::NotFound {
                            let _ = std::fs::remove_file(file.path());
                        } else {
                            tracing::warn!(%error, "could not recover command process");
                        }
                    }
                }
            }
        }
        registry
    }

    pub fn output_path(&self, id: &str) -> std::io::Result<PathBuf> {
        if !valid_id(id) {
            return Err(std::io::Error::other("invalid process id"));
        }
        let directory = self
            .directory
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("rcode-command-processes"))
            .join("output");
        std::fs::create_dir_all(&directory)?;
        Ok(directory.join(format!("{id}.log")))
    }
    pub fn read_output(&self, chat: &str, id: &str) -> std::io::Result<serde_json::Value> {
        let mut state = self.state.lock().unwrap();
        if !state.entries.contains_key(id) {
            if !valid_id(id) {
                return Err(std::io::Error::from(std::io::ErrorKind::NotFound));
            }
            let directory = self
                .directory
                .as_ref()
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
            let mut record: ProcessRecord =
                serde_json::from_slice(&std::fs::read(directory.join(format!("{id}.json")))?)?;
            if record.id != id || record.chat_id != chat {
                return Err(std::io::Error::from(std::io::ErrorKind::NotFound));
            }
            settle_record(&mut record);
            state.entries.insert(
                id.to_owned(),
                Entry {
                    record,
                    control: Control::Finished,
                    stop: None,
                    launcher_pending: false,
                },
            );
        }
        let entry = state
            .entries
            .get_mut(id)
            .filter(|entry| entry.record.chat_id == chat)
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
        let previous_status = entry.record.status.clone();
        if !entry.control.alive()? && !entry.launcher_pending {
            settle_record(&mut entry.record);
        }
        let path = entry
            .record
            .output_path
            .clone()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
        let rotated = crate::process_output::rotate(&path)?;
        entry.record.rotated |= rotated;
        if rotated || entry.record.status != previous_status {
            self.persist(&entry.record)?;
        }
        let record = entry.record.clone();
        drop(state);
        let (output, truncated) = crate::process_output::preview(&path)?;
        Ok(
            serde_json::json!({"id":id,"status":record.status,"exitCode":record.exit_code,"output":output,"truncated":truncated,"rotated":record.rotated,"outputPath":path}),
        )
    }
    pub(crate) fn finish(&self, id: &str, code: Option<i32>) {
        if let Some(entry) = self.state.lock().unwrap().entries.get_mut(id) {
            entry.launcher_pending = false;
            entry.record.exit_code = code;
            if matches!(entry.control.alive(), Ok(false)) {
                settle_record(&mut entry.record);
            }
            let _ = self.persist(&entry.record);
        }
    }

    pub(crate) fn register(
        &self,
        mut record: ProcessRecord,
        control: Control,
        stop: Arc<dyn Fn() + Send + Sync>,
    ) -> std::io::Result<()> {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return Err(std::io::Error::other(
                "command process manager is shutting down",
            ));
        }
        record.background |= state.promotions.remove(&record.id);
        #[cfg(unix)]
        let record = {
            let mut record = record;
            record.members = group_members(record.pid)?;
            record
        };
        self.persist(&record)?;
        state.entries.insert(
            record.id.clone(),
            Entry {
                record,
                control,
                stop: Some(stop),
                launcher_pending: true,
            },
        );
        Ok(())
    }
    fn persist(&self, record: &ProcessRecord) -> std::io::Result<()> {
        if let Some(directory) = &self.directory {
            std::fs::create_dir_all(directory)?;
            let mut temp = tempfile::NamedTempFile::new_in(directory)?;
            serde_json::to_writer(&mut temp, record)?;
            temp.as_file().sync_all()?;
            temp.persist(directory.join(format!("{}.json", record.id)))
                .map_err(|e| e.error)?;
        }
        Ok(())
    }
    pub(crate) fn refresh(&self, id: &str) {
        let mut state = self.state.lock().unwrap();
        if let Some(entry) = state.entries.get_mut(id) {
            entry.record.background = true;
            #[cfg(unix)]
            if let Ok(members) = group_members(entry.record.pid) {
                entry.record.members = members.clone();
                if let Control::Group { members: owned, .. } = &entry.control {
                    *owned.lock().unwrap() = members;
                }
            }
            if let Err(error) = self.persist(&entry.record) {
                tracing::warn!(%error, "could not refresh command journal");
            }
        }
    }
    pub(crate) fn background(&self, id: &str) {
        let mut state = self.state.lock().unwrap();
        if let Some(entry) = state.entries.get_mut(id) {
            entry.record.background = true;
            if let Err(error) = self.persist(&entry.record) {
                tracing::warn!(%error, "could not persist background promotion");
            }
        } else if !state.closed {
            state.promotions.insert(id.to_owned());
        }
    }
    fn remove_journal(&self, id: &str) {
        if let Some(directory) = &self.directory {
            let _ = std::fs::remove_file(directory.join(format!("{id}.json")));
        }
    }
    pub fn list(&self, chat_id: &str) -> Vec<ProcessRecord> {
        let mut state = self.state.lock().unwrap();
        state.entries.retain(|id, entry| {
            let previous = entry.record.status.clone();
            if let Some(path) = &entry.record.output_path
                && !matches!(entry.control, Control::Finished)
            {
                if let Ok(true) = crate::process_output::rotate(path) {
                    entry.record.rotated = true;
                    let _ = self.persist(&entry.record);
                }
            }
            if matches!(entry.control.alive(), Ok(false)) && !entry.launcher_pending {
                settle_record(&mut entry.record);
                entry.control = Control::Finished;
                if entry.record.output_path.is_some() {
                    if entry.record.status != previous {
                        let _ = self.persist(&entry.record);
                    }
                    self.directory.is_none()
                } else {
                    self.remove_journal(id);
                    false
                }
            } else {
                true
            }
        });
        let mut records: Vec<_> = state
            .entries
            .values()
            .filter(|e| {
                e.record.chat_id == chat_id
                    && e.record.background
                    && matches!(e.record.status.as_str(), "running" | "stopping")
            })
            .map(|e| e.record.clone())
            .collect();
        records.sort_by(|a, b| a.id.cmp(&b.id));
        records
    }
    pub fn stop(&self, chat_id: &str, id: &str) -> std::io::Result<bool> {
        let mut state = self.state.lock().unwrap();
        let Some(entry) = state
            .entries
            .get_mut(id)
            .filter(|e| e.record.chat_id == chat_id)
        else {
            return Ok(false);
        };
        if !entry.control.alive()? {
            return Ok(false);
        }
        entry.record.status = "stopping".into();
        if let Err(error) = self.persist(&entry.record) {
            tracing::warn!(%error, "could not persist process stop");
        }
        // Suppress the task notification before cancellation can wake the model.
        if let Some(stop) = &entry.stop {
            stop();
        }
        entry.control.kill()?;
        Ok(true)
    }
    pub fn shutdown(&self) {
        self.state.lock().unwrap().closed = true;
        if let Err(error) = self.stop_all() {
            tracing::error!(%error, "could not stop command processes at shutdown");
        }
    }
    /// Desktop exit can stop services on a local daemon without preventing
    /// that daemon from accepting commands when a new viewport connects.
    pub fn stop_all(&self) -> std::io::Result<()> {
        let mut state = self.state.lock().unwrap();
        let mut failure = None;
        for entry in state.entries.values_mut() {
            if matches!(entry.control.alive(), Ok(false)) {
                continue;
            }
            entry.record.status = "stopping".into();
            let _ = self.persist(&entry.record);
            if let Some(stop) = &entry.stop {
                stop();
            }
            if let Err(error) = entry.control.kill() {
                failure = Some(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
}
impl Drop for ProcessRegistry {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn settle_record(record: &mut ProcessRecord) {
    if !matches!(record.status.as_str(), "running" | "stopping") {
        return;
    }
    record.status = if record.status == "stopping" {
        "stopped"
    } else {
        match record.exit_code {
            Some(0) => "completed",
            Some(_) => "failed",
            None => "exited",
        }
    }
    .into();
}
fn recover_control(record: &ProcessRecord) -> std::io::Result<Control> {
    #[cfg(windows)]
    {
        Ok(Control::Job(Arc::new(crate::tools::bash::JoinJob::open(
            &record.job_name,
        )?)))
    }
    #[cfg(unix)]
    {
        let current = group_members(record.pid)?;
        if !record.members.iter().any(|member| current.contains(member)) {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound));
        }
        Ok(Control::Group {
            pid: record.pid,
            members: Mutex::new(current),
        })
    }
}
fn valid_id(id: &str) -> bool {
    id.strip_prefix("bash_")
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
}

// A recovered Unix process group is accepted only with a matching member and
// creation time. This prevents a stale group ID from targeting a reused PID.
#[cfg(unix)]
pub(crate) fn group_members(group: u32) -> std::io::Result<Vec<(u32, String)>> {
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,pgid=,lstart=,stat="])
        .env("LC_ALL", "C")
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(
            "cannot inspect command process group",
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let parts: Vec<_> = line.split_whitespace().collect();
            if parts.len() < 8
                || parts[1].parse::<u32>().ok()? != group
                || parts[7].starts_with('Z')
            {
                return None;
            }
            Some((parts[0].parse().ok()?, parts[2..7].join(" ")))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Tool, ToolContext};
    use tokio_util::sync::CancellationToken;

    fn registration(registry: Arc<ProcessRegistry>, cwd: PathBuf) -> Registration {
        Registration {
            registry,
            record: ProcessRecord {
                id: format!("bash_{}", uuid::Uuid::new_v4()),
                chat_id: "chat".into(),
                output_path: None,
                status: running_status(),
                exit_code: None,
                rotated: false,
                command: "fixture server".into(),
                cwd,
                pid: 0,
                job_name: format!("Local\\RCode.Command.{}", uuid::Uuid::new_v4()),
                background: true,
                members: vec![],
                recovered: false,
            },
            stop: Arc::new(|| {}),
        }
    }

    async fn wait_until(mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            while !condition() {
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            }
        })
        .await
        .expect("process fixture timed out");
    }

    #[cfg(windows)]
    fn detached_command() -> &'static str {
        "Start-Process powershell.exe -ArgumentList '-NoProfile -NonInteractive -Command Start-Sleep 120' -WindowStyle Hidden -RedirectStandardOutput out.log -RedirectStandardError err.log"
    }
    #[cfg(unix)]
    fn detached_command() -> &'static str {
        "sleep 120 </dev/null >out.log 2>err.log &"
    }

    async fn launch_detached(registry: Arc<ProcessRegistry>, cwd: &std::path::Path) {
        let ctx = ToolContext {
            cwd: cwd.into(),
            cancel: CancellationToken::new(),
            nested: None,
        };
        let result = REGISTRATION
            .scope(
                registration(registry, cwd.into()),
                crate::tools::BashTool
                    .call(serde_json::json!({"command":detached_command()}), &ctx),
            )
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output);
    }

    #[tokio::test]
    async fn detached_launcher_remains_visible_and_stops_entire_tree() {
        let root = tempfile::tempdir().unwrap();
        let registry = Arc::new(ProcessRegistry::open(Some(root.path())));
        launch_detached(registry.clone(), root.path()).await;
        let rows = registry.list("chat");
        assert_eq!(rows.len(), 1, "service survives its launcher");
        assert!(registry.list("another-chat").is_empty());
        assert!(!registry.stop("another-chat", &rows[0].id).unwrap());
        assert_eq!(registry.list("chat").len(), 1);
        assert!(registry.stop("chat", &rows[0].id).unwrap());
        wait_until(|| registry.list("chat").is_empty()).await;
        assert_eq!(
            std::fs::read_dir(root.path().join("command-processes"))
                .unwrap()
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn shutdown_cleans_detached_services_and_rejects_late_launches() {
        let root = tempfile::tempdir().unwrap();
        let registry = Arc::new(ProcessRegistry::open(Some(root.path())));
        launch_detached(registry.clone(), root.path()).await;
        assert_eq!(registry.list("chat").len(), 1);
        registry.shutdown();
        wait_until(|| registry.list("chat").is_empty()).await;
        let ctx = ToolContext {
            cwd: root.path().into(),
            cancel: CancellationToken::new(),
            nested: None,
        };
        let result = REGISTRATION
            .scope(
                registration(registry.clone(), root.path().into()),
                crate::tools::BashTool.call(serde_json::json!({"command":"echo late"}), &ctx),
            )
            .await;
        assert!(result.is_err());
        assert!(registry.list("chat").is_empty());
    }

    #[tokio::test]
    async fn desktop_exit_stops_daemon_services_but_allows_future_commands() {
        let root = tempfile::tempdir().unwrap();
        let registry = Arc::new(ProcessRegistry::open(Some(root.path())));
        launch_detached(registry.clone(), root.path()).await;
        registry.stop_all().unwrap();
        wait_until(|| registry.list("chat").is_empty()).await;
        launch_detached(registry.clone(), root.path()).await;
        assert_eq!(registry.list("chat").len(), 1);
        registry.shutdown();
        wait_until(|| registry.list("chat").is_empty()).await;
    }

    #[tokio::test]
    #[ignore = "subprocess helper for forced-host-exit recovery"]
    async fn crash_fixture() {
        let Some(root) = std::env::var_os("RCODE_COMMAND_CRASH_FIXTURE") else {
            return;
        };
        let root = PathBuf::from(root);
        let registry = Arc::new(ProcessRegistry::open(Some(&root)));
        launch_detached(registry.clone(), &root).await;
        assert_eq!(registry.list("chat").len(), 1);
        std::fs::write(root.join("ready"), "ready").unwrap();
        std::future::pending::<()>().await;
    }

    #[tokio::test]
    async fn forced_host_exit_recovers_service_without_a_model_turn() {
        let root = tempfile::tempdir().unwrap();
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "processes::tests::crash_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("RCODE_COMMAND_CRASH_FIXTURE", root.path())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        let mut host = command.spawn().unwrap();
        wait_until(|| root.path().join("ready").exists()).await;
        host.kill().await.unwrap();
        let registry = ProcessRegistry::open(Some(root.path()));
        let rows = registry.list("chat");
        assert_eq!(
            rows.len(),
            1,
            "recover by durable OS identity after host dies"
        );
        assert!(rows[0].recovered);
        assert_eq!(serde_json::to_value(&rows[0]).unwrap()["recovered"], true);
        registry.stop("chat", &rows[0].id).unwrap();
        wait_until(|| registry.list("chat").is_empty()).await;
    }

    #[test]
    fn stale_journal_is_pruned_without_signalling_any_pid() {
        let root = tempfile::tempdir().unwrap();
        let registry = ProcessRegistry::open(Some(root.path()));
        let mut record = registration(Arc::default(), root.path().into()).record;
        record.pid = std::process::id(); // Deliberately a real, unrelated PID.
        record.members = vec![(record.pid, "wrong creation time".into())];
        registry.persist(&record).unwrap();
        let recovered = ProcessRegistry::open(Some(root.path()));
        assert!(recovered.list("chat").is_empty());
        assert!(
            !root
                .path()
                .join("command-processes")
                .join(format!("{}.json", record.id))
                .exists()
        );
    }

    #[tokio::test]
    async fn file_output_is_bounded_in_preview_and_retained_after_completion() {
        let root = tempfile::tempdir().unwrap();
        let registry = Arc::new(ProcessRegistry::open(Some(root.path())));
        let mut registration = registration(registry.clone(), root.path().into());
        let id = registration.record.id.clone();
        let path = registry.output_path(&id).unwrap();
        registration.record.output_path = Some(path.clone());
        let ctx = ToolContext {
            cwd: root.path().into(),
            cancel: CancellationToken::new(),
            nested: None,
        };
        let command = if cfg!(windows) {
            "[Console]::Write(('x' * 120000)); [Console]::Error.WriteLine('最新错误'); [Console]::WriteLine('FINAL_MARKER')"
        } else {
            "head -c 120000 /dev/zero | tr '\\0' x; echo 最新错误 >&2; echo FINAL_MARKER"
        };
        let result = REGISTRATION
            .scope(
                registration,
                crate::tools::BashTool.call(serde_json::json!({"command":command}), &ctx),
            )
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output);
        let preview = registry.read_output("chat", &id).unwrap();
        assert_eq!(preview["status"], "completed");
        assert_eq!(preview["truncated"], true);
        assert!(
            preview["output"]
                .as_str()
                .unwrap()
                .ends_with("FINAL_MARKER\n")
                || preview["output"]
                    .as_str()
                    .unwrap()
                    .ends_with("FINAL_MARKER\r\n")
        );
        assert!(preview["output"].as_str().unwrap().contains("最新错误"));
        assert!(std::fs::metadata(&path).unwrap().len() > 100000);
        assert!(registry.read_output("other-chat", &id).is_err());
        assert!(registry.list("chat").is_empty());
        drop(registry);
        let recovered = ProcessRegistry::open(Some(root.path()));
        assert_eq!(
            recovered.read_output("chat", &id).unwrap()["output"],
            preview["output"]
        );
    }

    #[test]
    #[ignore = "writer subprocess for the durable output fixture"]
    fn log_writer_fixture() {
        use std::io::Write;
        for index in 0..600 {
            println!("durable-line-{index}");
            std::io::stdout().flush().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    #[test]
    #[ignore = "launcher subprocess for the durable output fixture"]
    fn log_launcher_fixture() {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "processes::tests::log_writer_fixture",
            "--ignored",
            "--nocapture",
        ]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        }
        command.spawn().unwrap();
    }

    #[tokio::test]
    async fn detached_service_keeps_logging_after_its_launcher_exits() {
        let root = tempfile::tempdir().unwrap();
        let registry = Arc::new(ProcessRegistry::open(Some(root.path())));
        let mut registration = registration(registry.clone(), root.path().into());
        let id = registration.record.id.clone();
        registration.record.output_path = Some(registry.output_path(&id).unwrap());
        let exe = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .replace('\'', "''");
        let command = format!(
            "{}'{exe}' --exact processes::tests::log_launcher_fixture --ignored --nocapture",
            if cfg!(windows) { "& " } else { "" }
        );
        let ctx = ToolContext {
            cwd: root.path().into(),
            cancel: CancellationToken::new(),
            nested: None,
        };
        let result = REGISTRATION
            .scope(
                registration,
                crate::tools::BashTool.call(serde_json::json!({"command":command}), &ctx),
            )
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output);
        assert_eq!(registry.list("chat").len(), 1);
        let first = registry.read_output("chat", &id).unwrap();
        assert_eq!(first["status"], "running");
        wait_until(|| registry.read_output("chat", &id).unwrap()["output"] != first["output"])
            .await;
        registry.stop("chat", &id).unwrap();
        wait_until(|| registry.list("chat").is_empty()).await;
        assert_eq!(
            registry.read_output("chat", &id).unwrap()["status"],
            "stopped"
        );
    }

    #[tokio::test]
    #[ignore = "host subprocess for the durable output fixture"]
    async fn log_host_fixture() {
        let root = PathBuf::from(std::env::var_os("RCODE_LOG_FIXTURE").unwrap());
        let registry = Arc::new(ProcessRegistry::open(Some(&root)));
        let mut registration = registration(registry.clone(), root.clone());
        let id = registration.record.id.clone();
        registration.record.output_path = Some(registry.output_path(&id).unwrap());
        let exe = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .replace('\'', "''");
        let command = if cfg!(windows) {
            format!("& '{exe}' --exact processes::tests::log_writer_fixture --ignored --nocapture")
        } else {
            format!("'{exe}' --exact processes::tests::log_writer_fixture --ignored --nocapture")
        };
        let ctx = ToolContext {
            cwd: root.clone(),
            cancel: CancellationToken::new(),
            nested: None,
        };
        tokio::spawn(async move {
            REGISTRATION
                .scope(
                    registration,
                    crate::tools::BashTool.call(serde_json::json!({"command":command}), &ctx),
                )
                .await
        });
        wait_until(|| {
            registry
                .read_output("chat", &id)
                .ok()
                .is_some_and(|v| v["output"].as_str().unwrap().contains("durable-line"))
        })
        .await;
        std::fs::write(root.join("ready-log"), id).unwrap();
        std::future::pending::<()>().await;
    }

    #[tokio::test]
    async fn logs_keep_growing_after_forced_host_exit_and_can_be_recovered() {
        let root = tempfile::tempdir().unwrap();
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "processes::tests::log_host_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("RCODE_LOG_FIXTURE", root.path())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        let mut host = command.spawn().unwrap();
        wait_until(|| root.path().join("ready-log").exists()).await;
        let id = std::fs::read_to_string(root.path().join("ready-log")).unwrap();
        host.kill().await.unwrap();
        let recovered = ProcessRegistry::open(Some(root.path()));
        assert_eq!(recovered.list("chat").len(), 1);
        let first = recovered.read_output("chat", &id).unwrap()["output"].clone();
        wait_until(|| recovered.read_output("chat", &id).unwrap()["output"] != first).await;
        recovered.stop("chat", &id).unwrap();
        wait_until(|| recovered.list("chat").is_empty()).await;
        let final_output = recovered.read_output("chat", &id).unwrap();
        assert_eq!(final_output["status"], "stopped");
        assert!(
            final_output["output"]
                .as_str()
                .unwrap()
                .contains("durable-line")
        );
    }
}
