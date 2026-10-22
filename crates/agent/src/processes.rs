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
    pub background: bool,
    #[serde(default)]
    pub members: Vec<(u32, String)>,
    #[serde(skip_deserializing)]
    pub recovered: bool,
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
        let registration = Registration {
            registry: self.registry.clone(),
            record: ProcessRecord {
                id: format!("bash_{}", uuid::Uuid::new_v4()),
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
                        #[cfg(windows)]
                        let control = Control::Job(Arc::new(crate::tools::bash::JoinJob::open(
                            &record.job_name,
                        )?));
                        #[cfg(unix)]
                        let control = {
                            let current = group_members(record.pid)?;
                            if !record.members.iter().any(|member| current.contains(member)) {
                                return Err(std::io::Error::from(std::io::ErrorKind::NotFound));
                            }
                            Control::Group {
                                pid: record.pid,
                                members: Mutex::new(current),
                            }
                        };
                        if !control.alive()? {
                            return Err(std::io::Error::from(std::io::ErrorKind::NotFound));
                        }
                        record.recovered = true;
                        record.background = true;
                        registry.state.lock().unwrap().entries.insert(
                            record.id.clone(),
                            Entry {
                                record,
                                control,
                                stop: None,
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
                let Control::Group { members: owned, .. } = &entry.control;
                *owned.lock().unwrap() = members;
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
            if matches!(entry.control.alive(), Ok(false)) {
                self.remove_journal(id);
                false
            } else {
                true
            }
        });
        let mut records: Vec<_> = state
            .entries
            .values()
            .filter(|e| e.record.chat_id == chat_id && e.record.background)
            .map(|e| e.record.clone())
            .collect();
        records.sort_by(|a, b| a.id.cmp(&b.id));
        records
    }
    pub fn stop(&self, chat_id: &str, id: &str) -> std::io::Result<bool> {
        let state = self.state.lock().unwrap();
        let Some(entry) = state
            .entries
            .get(id)
            .filter(|e| e.record.chat_id == chat_id)
        else {
            return Ok(false);
        };
        // Suppress the task notification before cancellation can wake the model.
        if let Some(stop) = &entry.stop {
            stop();
        }
        entry.control.kill()?;
        Ok(true)
    }
    pub fn shutdown(&self) {
        self.state.lock().unwrap().closed = true;
        if let Err(error) = self.stop_all() { tracing::error!(%error, "could not stop command processes at shutdown"); }
    }
    /// Desktop exit can stop services on a local daemon without preventing
    /// that daemon from accepting commands when a new viewport connects.
    pub fn stop_all(&self) -> std::io::Result<()> {
        let state = self.state.lock().unwrap();
        let mut failure = None;
        for entry in state.entries.values() {
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
            .args(["--exact", "processes::tests::crash_fixture", "--ignored", "--nocapture"])
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
}
