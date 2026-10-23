//! Fixed main prompt, conditional context, and provider cache sections.
mod instructions;
mod settings;
mod snapshot;

use rcode_agent::ToolRegistry;
use rcode_model::prompt_cache::SystemPrompt;
pub use settings::PromptSettings;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub const IDENTITY: &str = "You are RCode, an interactive coding assistant.";
pub const MAIN_SYSTEM_PROMPT: &str = concat!(
    include_str!("prompts/main.md"),
    "\n",
    include_str!("prompts/harness.md"),
    "\n",
    include_str!("prompts/context.md"),
    "\n",
    include_str!("prompts/tasks.md"),
    "\n",
    include_str!("prompts/actions.md"),
    "\n",
    include_str!("prompts/communication.md"),
    "\n",
    include_str!("prompts/formatting.md")
);

#[derive(Default)]
struct PromptSession {
    frozen: Option<snapshot::FrozenPrompt>,
}

#[derive(Default)]
pub struct PromptSessions {
    sessions: std::sync::Mutex<HashMap<String, (Instant, Arc<Mutex<PromptSession>>)>>,
}

pub struct PreparedPrompt {
    pub system: SystemPrompt,
    pub session_id: String,
    pub user_instructions: String,
    pub context: Option<Arc<dyn rcode_agent::context::ContextSource>>,
}

pub struct PromptRequest<'a> {
    pub cwd: &'a Path,
    pub data_dir: Option<&'a Path>,
    pub user_home: Option<&'a Path>,
    pub tools: &'a ToolRegistry,
    pub memory: Option<&'a str>,
    pub mcp_instructions: Option<String>,
    pub settings: &'a PromptSettings,
    pub resume: Option<&'a str>,
}

impl PromptSessions {
    fn session(&self, id: &str) -> Arc<Mutex<PromptSession>> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !sessions.contains_key(id) && sessions.len() >= 128 {
            if let Some(oldest) = sessions
                .iter()
                .filter(|(_, (_, session))| Arc::strong_count(session) == 1)
                .min_by_key(|(_, (used, _))| *used)
                .map(|(id, _)| id.clone())
            {
                sessions.remove(&oldest);
            } else {
                return Arc::new(Mutex::new(PromptSession::default()));
            }
        }
        let entry = sessions.entry(id.to_owned()).or_insert_with(|| {
            (
                Instant::now(),
                Arc::new(Mutex::new(PromptSession::default())),
            )
        });
        entry.0 = Instant::now();
        entry.1.clone()
    }

    pub async fn prepare(&self, request: PromptRequest<'_>) -> Result<PreparedPrompt, String> {
        let PromptRequest {
            cwd,
            data_dir,
            user_home,
            tools,
            memory,
            mcp_instructions,
            settings,
            resume,
        } = request;
        let session_id = resume
            .filter(|id| !id.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let session = self.session(&session_id);
        let mut state = session.lock().await;
        if state.frozen.is_none() {
            let saved = match data_dir {
                Some(dir) => snapshot::load(dir, &session_id).await?,
                None => None,
            };
            let frozen = if let Some(saved) = saved {
                saved
            } else {
                let is_git = git_output(cwd, &["rev-parse", "--is-inside-work-tree"])
                    .await
                    .as_deref()
                    == Some("true");
                let frozen = snapshot::FrozenPrompt {
                    version: 1,
                    cwd: cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf()),
                    identity: IDENTITY.to_owned(),
                    static_sections: vec![
                        include_str!("prompts/main.md").trim().to_owned(),
                        format!(
                            "{}\n\n{}",
                            include_str!("prompts/harness.md").trim(),
                            include_str!("prompts/context.md").trim()
                        ),
                        include_str!("prompts/tasks.md").trim().to_owned(),
                        include_str!("prompts/actions.md").trim().to_owned(),
                        include_str!("prompts/communication.md").trim().to_owned(),
                        include_str!("prompts/formatting.md").trim().to_owned(),
                    ],
                    environment: environment(cwd, tools, &settings.additional_directories, is_git),
                    date: format!(
                        "# Current date\n\nToday's date is {}.",
                        chrono::Local::now().format("%Y-%m-%d")
                    ),
                    instructions: if settings.disable_project_instructions {
                        String::new()
                    } else {
                        instructions::load(cwd, user_home)
                    },
                };
                match data_dir {
                    Some(dir) => snapshot::commit(dir, &session_id, frozen).await?,
                    None => frozen,
                }
            };
            state.frozen = Some(frozen);
        }
        let frozen = state.frozen.as_ref().expect("snapshot loaded");
        let current_cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        if frozen.cwd != current_cwd {
            return Err("the prompt snapshot belongs to another working directory; start a new conversation".into());
        }
        let mut frozen_sections = vec![frozen.environment.clone(), frozen.date.clone()];
        if !settings.disable_project_instructions && !frozen.instructions.is_empty() {
            frozen_sections.push(frozen.instructions.clone());
        }
        let mut dynamic_sections = Vec::new();
        let context = (data_dir.is_some() || tools.all().iter().any(|tool| tool.name().starts_with("mcp__")))
            .then(|| crate::context::RuntimeContext::new(cwd, data_dir, memory.is_some(), tools, mcp_instructions.as_deref()));
        let mut add = |value: Option<String>| {
            if let Some(value) = value.filter(|text| !text.trim().is_empty()) {
                dynamic_sections.push(value);
            }
        };
        // Platform/tool availability may change after a saved session resumes.
        // Keep this outside the immutable conversation snapshot.
        add(platform_shell_instructions(tools));
        if context.is_some() { add(Some(rcode_agent::context::POLICY.to_owned())); }
        // Keep behavioral/unclassified entries in system; factual pointers are
        // refreshed by the context source at each model boundary.
        add(memory.map(str::to_owned));
        // MCP connects/disconnects independently of cached session sections.
        add(mcp_instructions
            .filter(|_| {
                tools
                    .all()
                    .iter()
                    .any(|tool| tool.name().starts_with("mcp__"))
            })
            .map(|text| format!("# MCP server instructions\n\n{text}")));
        let system = SystemPrompt {
            client_header: None,
            identity: frozen.identity.clone(),
            static_sections: frozen.static_sections.clone(),
            frozen_sections,
            dynamic_sections,
            cache_mode: settings.cache_mode,
        };
        let user_instructions = if settings.disable_project_instructions {
            String::new()
        } else {
            frozen.instructions.clone()
        };
        drop(state);
        Ok(PreparedPrompt {
            system,
            session_id,
            user_instructions,
            context,
        })
    }
}

pub(crate) fn platform_shell_instructions(tools: &ToolRegistry) -> Option<String> {
    #[cfg(windows)]
    if tools.get("Bash").is_some() {
        let program = rcode_agent::tools::powershell_path();
        let syntax = if program
            .file_stem()
            .is_some_and(|name| name.eq_ignore_ascii_case("pwsh"))
        {
            "Selected engine: pwsh. PowerShell 7+ supports `&&` and `||` for conditional command chains. Its default file encoding is UTF-8."
        } else {
            "Selected engine: Windows PowerShell 5.1. Do not use `&&`, `||`, ternary `?:` or null-coalescing `??`; use `A; if ($?) { B }` for conditional execution. Specify `-Encoding utf8` when writing text with PowerShell cmdlets."
        };
        return Some(format!(
            "{}\n\n{syntax}",
            include_str!("prompts/windows.md").trim()
        ));
    }
    let _ = tools;
    None
}

fn environment(cwd: &Path, tools: &ToolRegistry, additional: &[PathBuf], is_git: bool) -> String {
    let shell = if tools.get("Bash").is_some() {
        rcode_agent::tools::command_shell_program()
    } else {
        "No shell tool is available".into()
    };
    format!(
        "# Environment\n\n- Primary working directory: {}\n- Is a git repository: {is_git}\n- Additional working directories: {}\n- Platform: {} ({})\n- Shell used by Bash: {shell}",
        serde_json::json!(cwd.to_string_lossy()),
        serde_json::json!(additional),
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

async fn git_output(cwd: &Path, args: &[&str]) -> Option<String> {
    use tokio::io::AsyncReadExt;
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut command = tokio::process::Command::new("git");
        command
            .args(args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        let mut child = command.spawn().ok()?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()?
            .take(16 * 1024)
            .read_to_end(&mut bytes)
            .await
            .ok()?;
        if bytes.len() >= 16 * 1024 {
            let _ = child.start_kill();
            let _ = child.wait().await;
        } else if !child.wait().await.ok()?.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&bytes).trim().to_owned())
    })
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod snapshot_tests;
#[cfg(test)]
mod tests;
