//! EngineRpc — the engine-side `RpcService`: sessions + docs + the workspace-doc
//! entity surface.
//!
//! Methods (ARCHITECTURE.md):
//! - `ListHarnesses` → `[HarnessDescriptor]`
//! - `ListModels {harness}` → `[Model]`
//! - `QueueCommand {chatId, command}` → `{commandId}` (durable doc command)
//! - `WatchDocMessages {chatId}` → stream of joined `SessionMessageEntry[]`,
//!   re-emitted on every doc change
//! - `WatchChats` / `WatchDevices` → streams of the workspace doc's entity rows
//! - `WatchSessions` → stream of `Session[]`: this engine's live statuses merged with
//!   remote devices' workspace session rows
//! - `Mutate {op, …}` → `{ok}` — workspace entity mutations (createChat, renameChat,
//!   setChatArchived, deleteChat, renameDevice, markChatSeen)
//! - `EngineInfo` → `{deviceId, workspaceScope}` — this runtime's fixed identity
//!   and data boundary (never forwarded)
//! - `LocalDevice` → `{deviceId}` — legacy engine identity (never forwarded)
//! - Repos (§3.5): `ListRepos`, `AddRepo {path}`, `CloneRepo {url}`,
//!   `CreateRepo {name}`, `ListBranches {repoPath}` (default branch first),
//!   `ListFolders {path?}`, `CreateWorktree {repoPath, branch}`, `DeleteWorktree
//!   {repoPath, worktreePath}`; `WatchCheckoutDiffs` → stream of `CheckoutDiff[]`
//! - Workspace files: lazy directory listing, recursive path search, bounded text
//!   reads, hash-guarded writes, and a checkout-scoped filesystem change stream.
//! - Terminals (§3.4): `OpenTerminal {chatId, cols, rows, cwd?}` → `TerminalSession`,
//!   `SubscribeTerminal {terminalId, afterSeq?}` → stream of `TerminalEvent`
//!   (replay then live tail), `WriteTerminal {terminalId, data}`, `ResizeTerminal`,
//!   `CloseTerminal`. M5 is single-user local: per-user owner checks land with
//!   real multi-account auth in M6.
//! - Sessions (§2 DataRpc): `WatchDocMessages`, `Mutate`, `QueueCommand`.
//! - Uploads (§3.7): `UploadChunk {uploadId, data, seq?}`,
//!   `UploadCommit {uploadId, fileName}` → `{path}`,
//!   `ReadAttachmentChunk {path, offset}` → `{name, mimeType, data, nextOffset,
//!   done}` (path-jailed to the uploads dir + workspace-known chat cwds).
//!
//! ## Device-addressed routing (`targetDeviceId`, ARCHITECTURE.md)
//!
//! ControlRpc methods are relay-forwardable: params may carry `targetDeviceId`. When it
//! names another device, the call is forwarded verbatim over that device's relay DO via
//! the [`LinkCache`] — the remote engine sees its own id and handles locally, so the
//! forward can never loop. Streaming methods are proxied by re-subscribing remotely and
//! piping items. To make another method device-addressable, nothing per-method is needed
//! beyond listing it in [`forwardable`] (and [`is_stream_method`] if it streams);
//! handlers stay transport-agnostic. This includes the workspace file surface,
//! whose checkout always lives on the routed target device.

use async_trait::async_trait;
use base64::Engine as _;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration;
use tokio::sync::watch;

use rcode_doc::{MessagePart, SessionCommandPayload};
use rcode_proto::{
    AgentEvent, ChatConfig, CreateWorktreeOutcome, EngineInfo, HarnessId, Space, ToolCall,
};
use rcode_rpc::{
    LinkCache, RpcError, RpcReply, RpcService, capability_errors, methods, parse_params,
};

use crate::auth::Auth;
use crate::change_requests::CheckoutChangeRequests;
use crate::diff_sync::CheckoutDiffSync;
use crate::doc_host::DocHost;
use crate::registry::{HarnessRegistry, TitleSettings};
use crate::repos::{Repos, session_home_dir};
use crate::sessions::SessionsEngine;
use crate::source_control::{ChangeRequestError, GitHubCli};
use crate::terminals::Terminals;
use crate::uploads::Uploads;
use crate::workspace_host::WorkspaceHost;

const FILE_SEARCH_RPC_TIMEOUT: Duration = Duration::from_secs(6);
const FILE_SEARCH_FEATURED_PATHS: usize = 32;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatParams {
    chat_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListModelsParams {
    harness: HarnessId,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetHarnessEnabledParams {
    harness: HarnessId,
    enabled: bool,
}

/// The parameters of every skills write, in one shape.
///
/// The four operations share almost every field — which project, which skill,
/// and the values — so one struct beats four that differ by one optional. A
/// field an operation does not use is simply absent, which is how the JSON
/// wire already reads.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillWriteParams {
    /// Settings catalog excludes project configuration.
    #[serde(default)]
    personal: bool,
    /// Kept for wire symmetry with `ListSkills`; the root is resolved the same
    /// way either op needs it.
    #[serde(default)]
    chat_id: Option<String>,
    #[serde(default)]
    space_id: Option<String>,
    /// The working directory the skills live under.
    #[serde(default)]
    path: Option<String>,
    /// Which skill, by name — create, delete and update.
    #[serde(default)]
    name: String,
    /// Which skill, by its directory — the enable toggle. Named separately from
    /// `path` (the working directory) because the two are different things and
    /// one field doing both would silently mean whichever the caller assumed.
    #[serde(default)]
    skill_path: Option<String>,
    /// Which managed root the op targets: the project's `.agents/skills` or the
    /// user's `~/.rcode/skills`. A scope is a *name* for one of those two
    /// roots, never a path — the root is derived on this side, so a request
    /// cannot aim a write anywhere the skill scan does not look. Absent means
    /// the project scope, which is what every caller before scopes meant.
    #[serde(default)]
    scope: Option<rcode_proto::invocation::SkillScope>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    new_name: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

/// The parameters of every subagent-definition write, mirroring
/// [`SkillWriteParams`]: the same root resolution, the same scope
/// addressing, and the reply is always the fresh list.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubagentWriteParams {
    /// Settings catalog excludes project configuration.
    #[serde(default)]
    personal: bool,
    #[serde(default)]
    chat_id: Option<String>,
    #[serde(default)]
    space_id: Option<String>,
    #[serde(default)]
    path: Option<String>,
    /// Which definition, by frontmatter name — create, delete and update.
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    /// The child's system prompt. An absent (or empty) one on update
    /// keeps what the file already carries.
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    new_name: Option<String>,
    /// The child's tool allowlist; empty means everything the parent has.
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    disallowed_tools: Vec<String>,
    /// `inherit` or an explicit provider/model pair — refused if it is
    /// not one of those, so a typo cannot park an override that only
    /// fails silently at run time.
    #[serde(default)]
    model: Option<String>,
    /// Which managed root the op targets. A name, never a path.
    #[serde(default)]
    scope: Option<rcode_proto::invocation::SkillScope>,
    /// Which definition, by its file path — the enable and model toggles.
    /// Named separately from `path` (the working directory) because the
    /// two are different things and one field doing both would silently
    /// mean whichever the caller assumed.
    #[serde(default)]
    definition_path: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

/// Add, replace or remove one server in `mcp.json`.
///
/// Rewrites only the named entry: the file is the user's, other tools read it,
/// and clobbering an entry this call did not mean to touch is the kind of loss
/// that has no undo. A missing file is created; its parent directory too.
fn write_mcp_entry(
    path: &std::path::Path,
    name: &str,
    server: Option<serde_json::Value>,
    create_only: bool,
) -> Result<(), String> {
    if let Some(entry) = server.as_ref() {
        rcode_mcp::config::validate_server(name, entry).map_err(|error| error.to_string())?;
    } else if create_only {
        return Err("a new MCP server needs a configuration".into());
    }
    let mut root: serde_json::Value = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|error| format!("{} is not valid JSON: {error}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            serde_json::json!({ "mcpServers": {} })
        }
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    if !root.is_object() {
        return Err(format!("{} is not a JSON object", path.display()));
    }
    let servers = root
        .as_object_mut()
        .expect("checked above")
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    let Some(servers) = servers.as_object_mut() else {
        return Err(format!("{}: `mcpServers` is not an object", path.display()));
    };
    if create_only && servers.contains_key(name) {
        return Err(format!("MCP server `{name}` already exists"));
    }
    match server {
        Some(entry) if entry.is_null() => {
            servers.remove(name);
        }
        Some(entry) => {
            servers.insert(name.to_owned(), entry);
        }
        None => {
            servers.remove(name);
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(&root).map_err(|error| error.to_string())?;
    // Write beside the name, then rename: an interrupted save must not leave a
    // truncated file that reads as "no servers".
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, json).map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, path).map_err(|error| error.to_string())
}

async fn update_harness_enabled(
    registry: &HarnessRegistry,
    harness: HarnessId,
    enabled: bool,
) -> Result<(), RpcError> {
    registry
        .set_enabled(harness, enabled)
        .map_err(RpcError::Failed)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueCommandParams {
    chat_id: String,
    command: SessionCommandPayload,
    /// Queued attachments (bytes already committed locally as `pending://`
    /// refs) the engine delivers to a remote host AFTER the command is
    /// durably queued — never as a gate in front of it.
    #[serde(default)]
    transfers: Vec<crate::uploads::AttachmentTransfer>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelayCommandParams {
    chat_id: String,
    /// The full command entry, client-minted id included — the exactly-once
    /// key the host claims in its processed ledger before executing.
    entry: rcode_doc::SessionCommandEntry,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueMessageParams {
    chat_id: String,
    text: String,
    /// Direct viewers keep this id through queue display and promotion.
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    issued_by: Option<String>,
    #[serde(default)]
    attachments: Vec<String>,
    /// Keep this row visible during the current turn even when the harness
    /// supports mid-turn steering.
    #[serde(default)]
    hold_for_turn_end: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueuedMessageParams {
    chat_id: String,
    id: String,
    /// Present for UpdateQueuedMessage only; empty text deletes the row.
    #[serde(default)]
    text: String,
    /// Present for MoveQueuedMessage only.
    #[serde(default)]
    to_index: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeginQueuedMessageEditParams {
    chat_id: String,
    id: String,
    editor_device_id: String,
    editor_instance_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenewQueuedMessageEditParams {
    chat_id: String,
    id: String,
    lease_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum FinishQueuedMessageEditAction {
    Commit,
    Cancel,
    Discard,
    ReleaseUnchanged,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FinishQueuedMessageEditParams {
    chat_id: String,
    id: String,
    lease_id: String,
    action: FinishQueuedMessageEditAction,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    expected_text_hash: Option<String>,
    #[serde(default)]
    attachments: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoPathParams {
    /// `repoPath` per §3.5 (the §2.1 shorthand `repo` is accepted as an alias).
    #[serde(alias = "repo")]
    repo_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckoutChangeRequestParams {
    cwd: String,
    #[serde(default)]
    branch: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwitchRefParams {
    /// The checkout to switch — a session's cwd (main folder or worktree).
    repo_path: String,
    ref_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateWorktreeParams {
    #[serde(alias = "repo")]
    repo_path: String,
    branch: String,
    #[serde(default)]
    space_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteWorktreeParams {
    #[serde(alias = "repo")]
    repo_path: String,
    #[serde(alias = "path")]
    worktree_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscardWorkingTreeParams {
    chat_id: String,
    checkout_id: String,
    expected_checksum: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListFoldersParams {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileSearchParams {
    query: String,
    #[serde(default)]
    chat_id: Option<String>,
    #[serde(default)]
    space_id: Option<String>,
    /// Existing linked worktree selected for a new chat. The engine accepts it
    /// only after verifying it against the space repository's worktree list.
    #[serde(default)]
    path: Option<String>,
}

fn tool_file_path(call: &ToolCall) -> Option<&str> {
    match call {
        ToolCall::ReadFile { path }
        | ToolCall::WriteFile { path, .. }
        | ToolCall::EditFile { path, .. } => Some(path),
        ToolCall::ApplyPatch { path } | ToolCall::Search { path, .. } => path.as_deref(),
        ToolCall::Exec { .. }
        | ToolCall::Glob { .. }
        | ToolCall::WebFetch { .. }
        | ToolCall::WebSearch { .. }
        | ToolCall::Todo { .. }
        | ToolCall::Mcp { .. }
        | ToolCall::Unknown { .. } => None,
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenTerminalParams {
    chat_id: String,
    cols: u16,
    rows: u16,
    /// Explicit working directory (new-chat canvas: the selected project
    /// folder, or `~`). When omitted, the chat row's cwd is used, then the
    /// space named by a `space-canvas:{spaceId}` chat id.
    #[serde(default)]
    cwd: Option<String>,
}

/// Matches the UI canvas panel key (`AppState::panel_session_key`).
const CANVAS_TERMINAL_PREFIX: &str = "space-canvas:";

/// A cwd the user (or a project-less chat) meant as "host home", not a folder.
fn meaningful_cwd(cwd: Option<String>) -> Option<String> {
    cwd.filter(|cwd| {
        let trimmed = cwd.trim();
        !trimmed.is_empty() && trimmed != "~"
    })
}

/// Space id encoded in a new-chat canvas terminal key, if any.
fn canvas_space_id(chat_id: &str) -> Option<&str> {
    chat_id
        .strip_prefix(CANVAS_TERMINAL_PREFIX)
        .filter(|id| !id.is_empty())
}

/// Resolve the PTY cwd: a real explicit path wins, then the chat row, then
/// the project folder named by `space-canvas:{spaceId}`, then `~`.
/// The portable `~` marker is a fallback, not an override — otherwise a
/// canvas OpenTerminal that still says `~` (spaces watch not landed in the
/// UI) would ignore the selected project encoded in `chatId`.
fn resolve_open_terminal_cwd(
    explicit: Option<String>,
    chat_cwd: Option<String>,
    space_cwd: Option<String>,
    scratch_root: &std::path::Path,
) -> Result<String, String> {
    let raw = meaningful_cwd(explicit)
        .or_else(|| meaningful_cwd(chat_cwd))
        .or_else(|| meaningful_cwd(space_cwd))
        .unwrap_or_else(|| "~".to_string());
    crate::repos::resolve_chat_cwd(&raw, scratch_root)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TerminalIdParams {
    terminal_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubscribeTerminalParams {
    terminal_id: String,
    #[serde(default)]
    after_seq: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WriteTerminalParams {
    terminal_id: String,
    /// Base64 input bytes (plain UTF-8 accepted leniently).
    data: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResizeTerminalParams {
    terminal_id: String,
    cols: u16,
    rows: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadChunkParams {
    upload_id: String,
    /// Base64 payload chunk.
    data: String,
    #[serde(default)]
    seq: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadCommitParams {
    upload_id: String,
    file_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadAttachmentChunkParams {
    path: String,
    #[serde(default)]
    offset: u64,
}

const _: () = assert!(rcode_proto::VISUAL_MAX_BYTES == rcode_visual::MAX_SCRIPT_BYTES);
const _: () = assert!(rcode_proto::VISUAL_MAX_ASSETS == rcode_visual::assets::MAX_ASSETS);
const _: () = assert!(rcode_proto::VISUAL_MAX_ASSET_BYTES == rcode_visual::assets::MAX_ASSET_BYTES);
const _: () = assert!(
    rcode_proto::VISUAL_MAX_TOTAL_ASSET_BYTES == rcode_visual::assets::MAX_TOTAL_ASSET_BYTES
);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishVisualParams {
    chat_id: String,
    title: String,
    script: String,
    /// Files the script uses, base64 by name.
    #[serde(default)]
    assets: std::collections::BTreeMap<String, String>,
}

fn no_turn_for_visual() -> RpcError {
    RpcError::Failed(
        "This chat has no running turn on this device; show_visual only works from inside a turn."
            .into(),
    )
}

/// Decode and bound a visual's assets.
fn decode_visual_assets(
    assets: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<(String, Vec<u8>)>, RpcError> {
    use base64::Engine as _;
    use rcode_visual::assets::{MAX_ASSET_BYTES, MAX_ASSETS, MAX_TOTAL_ASSET_BYTES, valid_name};
    if assets.len() > MAX_ASSETS {
        return Err(RpcError::BadParams(format!(
            "a visual takes at most {MAX_ASSETS} assets"
        )));
    }
    let mut total = 0;
    let mut out = Vec::with_capacity(assets.len());
    let mut seen = std::collections::HashSet::new();
    for (name, data) in assets {
        if !valid_name(name) {
            return Err(RpcError::BadParams(format!(
                "asset name '{name}' must be a plain file name"
            )));
        }
        // Stored as files, and file systems often ignore case.
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(RpcError::BadParams(format!(
                "asset names must differ by more than case ('{name}')"
            )));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| RpcError::BadParams(format!("asset '{name}' is not base64: {e}")))?;
        total += bytes.len();
        if bytes.len() > MAX_ASSET_BYTES || total > MAX_TOTAL_ASSET_BYTES {
            return Err(RpcError::BadParams(format!(
                "asset '{name}' is too large ({} MB each, {} MB together)",
                MAX_ASSET_BYTES / (1024 * 1024),
                MAX_TOTAL_ASSET_BYTES / (1024 * 1024)
            )));
        }
        out.push((name.clone(), bytes));
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchToolBlobParams {
    /// Doc-resident sidecar ref (`{chatId}/{partId}` or `…​.diff`).
    blob_ref: String,
}

/// The Mutate surface (ARCHITECTURE.md DataRpc), tagged by `op`.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum MutateParams {
    #[serde(rename_all = "camelCase")]
    CreateChat {
        chat_id: String,
        /// The project the chat is created in — fixes host device + base cwd.
        /// `None` mints a project-less chat: `deviceId` picks the host and the
        /// cwd defaults to `~` (expanded on the host at run time).
        #[serde(default)]
        space_id: Option<String>,
        /// Host device for a project-less chat; ignored when `spaceId` is set.
        #[serde(default)]
        device_id: Option<String>,
        #[serde(default)]
        config: Option<ChatConfig>,
        /// The picked ref, named on the row from the first frame (the footer
        /// read "Select ref" until the diff reconciler stamped it).
        #[serde(default)]
        branch: Option<String>,
        /// Cwd override (isolated-worktree path); default = the space's folder.
        #[serde(default)]
        cwd: Option<String>,
        /// The chat whose agent is creating this one (RCode MCP); recorded
        /// on the row as `parentChatId` for orchestration trees.
        #[serde(default)]
        parent_chat_id: Option<String>,
    },
    /// Create a space (device + folder pair). Idempotent by id; a live
    /// duplicate `(deviceId, path)` no-ops. `gitDetected` is seeded from the
    /// picker's FolderEntry — the owning device's SpacesSync re-verifies.
    #[serde(rename_all = "camelCase")]
    CreateSpace {
        space_id: String,
        device_id: String,
        path: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        git_detected: bool,
    },
    /// LWW display-name set; `name: None` clears back to basename(path).
    #[serde(rename_all = "camelCase")]
    RenameSpace {
        space_id: String,
        #[serde(default)]
        name: Option<String>,
    },
    /// Hard delete: cascades to every chat (and session row) in the space.
    /// Live runs hosted here are interrupted best-effort.
    #[serde(rename_all = "camelCase")]
    DeleteSpace { space_id: String },
    #[serde(rename_all = "camelCase")]
    RenameChat { chat_id: String, title: String },
    /// Set the chat's checkout branch label — the sidebar's
    /// "project · branch" sub-line.
    #[serde(rename_all = "camelCase")]
    SetChatBranch { chat_id: String, branch: String },
    /// Retarget a chat onto another folder — mid-session switch to an
    /// EXISTING worktree (the picked ref's checkout). Next run starts a
    /// fresh harness conversation there (resume is cwd-scoped).
    #[serde(rename_all = "camelCase")]
    SetChatCwd { chat_id: String, cwd: String },
    /// Backdate a chat's activity timestamps (epoch ms) — the sidebar's
    /// relative-time column. Used by tooling/seeds; the doc fold sets these on
    /// real message traffic.
    #[serde(rename_all = "camelCase")]
    SetChatActivity {
        chat_id: String,
        #[serde(default)]
        last_message_at: Option<i64>,
        #[serde(default)]
        created_at: Option<i64>,
    },
    /// Re-home a chat to another device (tooling/seeds; device migration later).
    #[serde(rename_all = "camelCase")]
    SetChatHost { chat_id: String, device_id: String },
    #[serde(rename_all = "camelCase")]
    SetChatArchived { chat_id: String, archived: bool },
    /// Change one pin without replacing another device's edits.
    #[serde(rename_all = "camelCase")]
    ChangeSidebarPin {
        change: rcode_proto::SidebarPinChange,
    },
    /// Full-config replace on the chat row (the original app `SetChatConfig`): the
    /// composer's mid-session model / reasoning / options changes, LWW-synced
    /// so they survive restarts and reach every device.
    #[serde(rename_all = "camelCase")]
    SetChatConfig { chat_id: String, config: ChatConfig },
    /// Tombstone: removes the chats-map row; the session doc remains.
    #[serde(rename_all = "camelCase")]
    DeleteChat { chat_id: String },
    #[serde(rename_all = "camelCase")]
    RenameDevice { device_id: String, name: String },
    /// Synced seen marker (LWW + monotonic guard): clears the "completed"
    /// badge on every device. `at` is epoch ms; default = now.
    #[serde(rename_all = "camelCase")]
    MarkChatSeen {
        chat_id: String,
        #[serde(default)]
        at: Option<i64>,
    },
}

#[derive(Clone)]
pub struct EngineRpc {
    remote_access: Option<std::sync::Arc<crate::remote_access::RemoteAccess>>,
    remote_hosts: Option<std::sync::Arc<crate::remote_hosts::RemoteHosts>>,
    sessions: SessionsEngine,
    doc_host: DocHost,
    workspace: WorkspaceHost,
    registry: std::sync::Arc<HarnessRegistry>,
    repos: Repos,
    workspace_files: crate::WorkspaceFiles,
    terminals: Terminals,
    previews: Option<rcode_preview::PreviewService>,
    change_requests: CheckoutChangeRequests,
    github: GitHubCli,
    diff_sync: CheckoutDiffSync,
    uploads: Uploads,
    auth: Option<Auth>,
    links: Option<std::sync::Arc<LinkCache>>,
    updater: Option<rcode_update::Updater>,
    engine_info: EngineInfo,
    /// The installation's config root, where capability settings live. The
    /// harness reads `agent-settings.json` from here, so the settings RPCs
    /// must write here too — the data root may have been moved elsewhere.
    config_root: std::path::PathBuf,
}

impl EngineRpc {
    #[allow(clippy::too_many_arguments)] // engine assembly seam, not a public API
    pub fn new(
        sessions: SessionsEngine,
        doc_host: DocHost,
        workspace: WorkspaceHost,
        registry: std::sync::Arc<HarnessRegistry>,
        repos: Repos,
        workspace_files: crate::WorkspaceFiles,
        terminals: Terminals,
        change_requests: CheckoutChangeRequests,
        diff_sync: CheckoutDiffSync,
        uploads: Uploads,
        config_root: std::path::PathBuf,
    ) -> Self {
        let engine_info = EngineInfo {
            device_id: doc_host.device_id().to_string(),
            cursor_sdk_version: None,
            capabilities: rcode_proto::capabilities::current(),
        };
        let github = GitHubCli::new().with_store(repos.data_dir());
        Self {
            remote_access: None,
            remote_hosts: None,
            sessions,
            doc_host,
            workspace,
            registry,
            repos,
            workspace_files,
            terminals,
            previews: None,
            change_requests,
            github,
            diff_sync,
            uploads,
            auth: None,
            links: None,
            updater: None,
            engine_info,
            config_root,
        }
    }

    pub fn with_previews(mut self, previews: rcode_preview::PreviewService) -> Self {
        self.previews = Some(previews);
        self
    }

    pub fn with_remote_access(mut self, access: std::sync::Arc<crate::remote_access::RemoteAccess>) -> Self {
        self.remote_access = Some(access);
        self
    }

    pub fn without_remote_access(mut self) -> Self {
        self.remote_access = None;
        self.remote_hosts = None;
        self
    }

    pub fn with_remote_hosts(mut self, hosts: std::sync::Arc<crate::remote_hosts::RemoteHosts>) -> Self {
        self.remote_hosts = Some(hosts);
        self
    }

    fn hosts(&self) -> Result<&std::sync::Arc<crate::remote_hosts::RemoteHosts>, RpcError> {
        self.remote_hosts.as_ref().ok_or_else(|| RpcError::Failed("remote host management is local only".into()))
    }

    fn catalog_stream<T>(&self, method: &'static str, local: watch::Receiver<T>) -> BoxStream<'static, serde_json::Value>
    where T: serde::Serialize + Clone + Send + Sync + 'static {
        let Some(hosts) = self.remote_hosts.clone() else { return watch_stream(local) };
        let changed = hosts.changes();
        futures::stream::unfold((local, changed, hosts, false), move |(mut local, mut changed, hosts, emitted)| async move {
            if emitted {
                tokio::select! {
                    result = local.changed() => { result.ok()?; },
                    result = changed.changed() => { result.ok()?; },
                }
            }
            changed.borrow_and_update();
            let rows = serde_json::to_value(&*local.borrow_and_update()).ok()?;
            Some((hosts.merge(method, rows), (local, changed, hosts, true)))
        }).boxed()
    }

    async fn forward_direct(&self, client: std::sync::Arc<rcode_rpc::RpcClient>, method: &str, mut params: serde_json::Value) -> Result<RpcReply, RpcError> {
        if let Some(params) = params.as_object_mut() { params.remove("targetDeviceId"); }
        if is_stream_method(method) || method.starts_with("Watch") {
            let rx = if matches!(method, methods::WATCH_CHECKOUT_CHANGE_REQUEST | methods::WATCH_WORKSPACE_GIT_STATUS) {
                client.subscribe_checked(method, params).await?
            } else { client.subscribe_scoped(method, params).await? };
            Ok(RpcReply::Stream(futures::stream::unfold((rx, client), |(mut rx, client)| async move {
                rx.recv().await.map(|value| (value, (rx, client)))
            }).boxed()))
        } else {
            let value = tokio::time::timeout(forward_deadline(method), client.call(method, params)).await
                .map_err(|_| RpcError::Transport("remote request timed out; it may already have been accepted".into()))??;
            Ok(RpcReply::Value(value))
        }
    }

    /// Attach the auth service (AuthStatus + AuthRpc mutations).
    pub fn with_auth(mut self, auth: Auth) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Attach the peer link cache — enables `targetDeviceId` relay forwarding.
    pub fn with_links(mut self, links: std::sync::Arc<LinkCache>) -> Self {
        self.links = Some(links);
        self
    }

    /// Attach the release checker (UpdateStatus stream + ApplyUpdate).
    pub fn with_updater(mut self, updater: rcode_update::Updater) -> Self {
        self.updater = Some(updater);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_github(mut self, github: GitHubCli) -> Self {
        self.github = github;
        self
    }

    fn auth(&self) -> Result<&Auth, RpcError> {
        self.auth
            .as_ref()
            .ok_or_else(|| RpcError::Failed("auth unavailable".into()))
    }

    fn updater(&self) -> Result<&rcode_update::Updater, RpcError> {
        self.updater
            .as_ref()
            .ok_or_else(|| RpcError::Failed("updates unavailable".into()))
    }

    /// The project space a worktree request names, when it names one that
    /// this device owns. Remote spaces are rejected: their worktrees are
    /// created by the device that holds the checkout.
    fn local_project_space(&self, space_id: &str) -> Result<Space, RpcError> {
        let space = self
            .workspace
            .space(space_id)
            .map_err(|err| RpcError::Failed(err.to_string()))?
            .ok_or_else(|| RpcError::Failed("Project space not found".into()))?;
        if space.device_id != self.doc_host.device_id() {
            return Err(RpcError::Failed(
                "Project space belongs to another device".into(),
            ));
        }
        Ok(space)
    }

    /// Resolve a mention-search root from synced workspace rows. A client may
    /// name an existing linked worktree for a new chat, but it is verified
    /// against the space repository before any filesystem walk begins.
    async fn file_search_root(&self, p: &FileSearchParams) -> Result<std::path::PathBuf, RpcError> {
        let target = rcode_proto::WorkspaceTarget {
            memory: false,
            chat_id: p.chat_id.clone(),
            space_id: p.space_id.clone(),
            checkout_path: p.path.clone(),
        };
        self.workspace_files
            .resolve_target(&target)
            .await
            .map(|workspace| workspace.root)
            .map_err(Into::into)
    }

    /// Catalogs also serve projectless sessions, which have no workspace space.
    /// Keep workspace validation for project targets and use only a known local
    /// chat's persisted cwd (or home) for a projectless conversation.
    async fn catalog_root(&self, p: &FileSearchParams) -> Result<std::path::PathBuf, RpcError> {
        if p.space_id.is_none() && p.path.is_none() {
            let Some(chat_id) = &p.chat_id else {
                return session_home_dir().map_err(|error| RpcError::Failed(error.to_string()));
            };
            let chat = self
                .workspace
                .chat(chat_id)
                .map_err(|e| RpcError::Failed(e.to_string()))?
                .ok_or_else(|| RpcError::BadParams("chat not found".into()))?;
            if chat.device_id != self.doc_host.device_id() {
                return Err(RpcError::BadParams("chat belongs to another device".into()));
            }
            if chat.space_id.is_none() {
                let cwd = chat.cwd.as_deref().unwrap_or("~");
                return crate::repos::resolve_chat_cwd(cwd, &self.repos.scratch_root())
                    .map(std::path::PathBuf::from)
                    .map_err(RpcError::Failed);
            }
        }
        self.file_search_root(p).await
    }

    /// The device's provider configuration store.
    ///
    /// Reached through the harness rather than opened separately: the store has
    /// to be the *same* one the running agent reads, or a save would move a
    /// revision nothing else is looking at.
    fn provider_store(
        &self,
    ) -> Result<std::sync::Arc<rcode_harness::provider_store::ProviderStore>, RpcError> {
        self.registry
            .resolve(HarnessId::Native)
            .map_err(|error| RpcError::Failed(error.to_string()))?
            .provider_store()
            .ok_or_else(|| {
                RpcError::Failed("the native agent has no provider configuration".to_owned())
            })
    }

    /// Where a skill lives, resolved the same way the listing resolves it.
    ///
    /// Deliberately the *same* function `ListSkills` uses: the two ends of a
    /// create-then-list round trip must agree on the directory, and a second
    /// resolution path is how they would drift.
    async fn skill_root(&self, p: &SkillWriteParams) -> Result<std::path::PathBuf, RpcError> {
        if p.personal {
            if p.scope.is_none() || p.scope == Some(rcode_proto::invocation::SkillScope::Project) {
                return Err(RpcError::BadParams(
                    "personal settings require a personal or plugin scope".into(),
                ));
            }
            return Ok(self.config_root.clone());
        }
        self.catalog_root(&FileSearchParams {
            query: String::new(),
            chat_id: p.chat_id.clone(),
            space_id: p.space_id.clone(),
            path: p.path.clone(),
        })
        .await
    }

    /// The skill list after a write, read from the same source the list RPC
    /// reads from.
    ///
    /// The harness knows the enable map, so asking it (rather than re-deriving
    /// the flags here) keeps one answer to "is this skill on".
    async fn reply_skills(&self, p: &SkillWriteParams) -> Result<RpcReply, RpcError> {
        let harness = self
            .registry
            .resolve(HarnessId::Native)
            .map_err(|error| RpcError::Failed(error.to_string()))?;
        let root = self.skill_root(p).await?;
        let skills = if p.personal {
            harness.personal_skills().await
        } else {
            harness.skills(&root).await
        }
        .map_err(|error| RpcError::Failed(error.to_string()))?
        .unwrap_or_default();
        RpcReply::value(&skills)
    }

    /// Where a subagent definition lives, resolved exactly the way the
    /// listing resolves it — the safe engine chat/space context, never an
    /// arbitrary absolute path.
    async fn subagent_root(&self, p: &SubagentWriteParams) -> Result<std::path::PathBuf, RpcError> {
        if p.personal {
            if (p.scope.is_none()
                && !p.definition_path.as_deref().is_some_and(|path| {
                    path.starts_with(rcode_harness::native_subagents::BUILTIN_PREFIX)
                }))
                || p.scope == Some(rcode_proto::invocation::SkillScope::Project)
            {
                return Err(RpcError::BadParams(
                    "personal settings require a personal or plugin scope".into(),
                ));
            }
            return Ok(self.config_root.clone());
        }
        self.catalog_root(&FileSearchParams {
            query: String::new(),
            chat_id: p.chat_id.clone(),
            space_id: p.space_id.clone(),
            path: p.path.clone(),
        })
        .await
    }

    /// The definition list after a write, read from the same source the
    /// list RPC reads from — one answer to "which agents exist and which
    /// are on".
    async fn reply_subagents(&self, p: &SubagentWriteParams) -> Result<RpcReply, RpcError> {
        let harness = self
            .registry
            .resolve(HarnessId::Native)
            .map_err(|error| RpcError::Failed(error.to_string()))?;
        let root = self.subagent_root(p).await?;
        let definitions = if p.personal {
            harness.personal_subagents().await
        } else {
            harness.subagents(&root).await
        }
        .map_err(|error| RpcError::Failed(error.to_string()))?
        .unwrap_or_default();
        RpcReply::value(&definitions)
    }

    /// Accept only a checkout already named by a local chat or contained in a
    /// local space. Remote clients must not turn this RPC into an arbitrary path probe.
    async fn change_request_root(&self, cwd: &str) -> Result<std::path::PathBuf, RpcError> {
        let requested = std::path::PathBuf::from(cwd);
        let local_device = self.doc_host.device_id();
        let mut chats_rx = self.workspace.watch_chats();
        let mut spaces_rx = self.workspace.watch_spaces();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(250);
        loop {
            let chats = chats_rx.borrow_and_update().clone();
            if chats.iter().any(|chat| {
                chat.device_id == local_device
                    && chat.cwd.as_deref().map(std::path::Path::new) == Some(requested.as_path())
            }) {
                return Ok(requested);
            }

            let spaces = spaces_rx.borrow_and_update().clone();
            for space in spaces
                .iter()
                .filter(|space| space.device_id == local_device)
            {
                if let Some(checkout) = self
                    .repos
                    .workspace_checkout(std::path::Path::new(&space.path), &requested)
                    .await
                {
                    return Ok(checkout);
                }
            }

            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::select! {
                _ = chats_rx.changed() => {}
                _ = spaces_rx.changed() => {}
                _ = tokio::time::sleep_until(deadline) => break,
            }
        }
        Err(RpcError::BadParams(
            "cwd is not a known checkout on this device".into(),
        ))
    }

    /// Most-recent-first paths the current chat actually touched, followed by
    /// files still changed in its checkout. The search worker validates and
    /// normalizes them against the resolved root before using them as ranking
    /// hints, so stale or out-of-workspace tool paths simply disappear.
    fn featured_file_paths(&self, chat_id: &str) -> Vec<String> {
        let mut paths = Vec::new();
        let mut seen = HashSet::new();
        if let Ok(handle) = self.doc_host.open(chat_id)
            && let Ok(entries) = handle.doc().read_entries()
        {
            for entry in entries.into_iter().rev() {
                for part in entry.parts.into_iter().rev() {
                    if let MessagePart::Tool { call, .. } = part
                        && let Some(path) = tool_file_path(&call)
                        && !path.trim().is_empty()
                        && seen.insert(path.to_string())
                    {
                        paths.push(path.to_string());
                        if paths.len() == FILE_SEARCH_FEATURED_PATHS {
                            break;
                        }
                    }
                }
                if paths.len() == FILE_SEARCH_FEATURED_PATHS {
                    break;
                }
            }
        }

        if let Ok(Some(chat)) = self.workspace.chat(chat_id) {
            let diffs = self.diff_sync.watch_diffs().borrow().clone();
            let diff = chat
                .checkout_id
                .as_deref()
                .and_then(|id| diffs.iter().find(|diff| diff.checkout_id == id))
                .or_else(|| {
                    chat.cwd
                        .as_deref()
                        .and_then(|cwd| diffs.iter().find(|diff| diff.cwd == cwd))
                });
            if let Some(diff) = diff {
                for file in &diff.files {
                    if paths.len() == FILE_SEARCH_FEATURED_PATHS {
                        break;
                    }
                    if seen.insert(file.path.clone()) {
                        paths.push(file.path.clone());
                    }
                }
            }
        }
        paths
    }

    /// Forward a device-addressed call over the target device's relay. On transport
    /// failure the cached link is invalidated so the next call re-dials.
    async fn forward(
        &self,
        target: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<RpcReply, RpcError> {
        let Some(links) = &self.links else {
            return Err(RpcError::Failed(format!(
                "cannot reach device {target}: remote routing unavailable (offline)"
            )));
        };
        let client = links.client(target).await?;
        if is_stream_method(method) {
            // Streams are unbounded by design (a quiet WATCH_* is healthy);
            // only unary calls below get the reply deadline.
            if matches!(
                method,
                methods::WATCH_CHECKOUT_CHANGE_REQUEST | methods::WATCH_WORKSPACE_GIT_STATUS
            ) {
                let rx = match client.subscribe_checked(method, params).await {
                    Ok(rx) => rx,
                    Err(err) => {
                        if should_invalidate_link(&err) {
                            links.invalidate(target);
                        }
                        return Err(err);
                    }
                };
                let stream = futures::stream::unfold((rx, client), |(mut rx, client)| async move {
                    rx.recv().await.map(|item| (item, (rx, client)))
                });
                return Ok(RpcReply::Stream(stream.boxed()));
            }
            let rx = match client.subscribe_scoped(method, params).await {
                Ok(rx) => rx,
                Err(err) => {
                    if should_invalidate_link(&err) {
                        links.invalidate(target);
                    }
                    return Err(err);
                }
            };
            // Pipe remote items; the held client keeps the link's RpcClient alive for
            // the stream's lifetime. A remote error just ends the stream (the relay
            // link-down path fails pending calls; stream receivers close).
            let stream = futures::stream::unfold((rx, client), |(mut rx, client)| async move {
                rx.recv().await.map(|item| (item, (rx, client)))
            });
            return Ok(RpcReply::Stream(stream.boxed()));
        }
        let deadline = forward_deadline(method);
        match tokio::time::timeout(deadline, client.call(method, params)).await {
            Ok(Ok(value)) => Ok(RpcReply::Value(value)),
            Ok(Err(err)) => {
                if should_invalidate_link(&err) {
                    links.invalidate(target);
                }
                Err(err)
            }
            Err(_) => {
                // No reply inside the deadline. The link may be a zombie — the
                // relay's auto-pong keeps a dead host socket looking alive
                // (ws3 auto-pong incident) — so drop it; the next call re-dials.
                // NOTE: the remote may still complete the forwarded work; the
                // caller sees a retryable failure instead of hanging forever
                // (the "Sending…" wedge, 2026-08-18).
                links.invalidate(target);
                Err(RpcError::Transport(format!(
                    "no reply from device {target} for {method} within {}s",
                    deadline.as_secs()
                )))
            }
        }
    }

    async fn mutate(&self, params: MutateParams) -> Result<(), RpcError> {
        let failed = |e: crate::EngineError| RpcError::Failed(e.to_string());
        match params {
            MutateParams::CreateChat {
                chat_id,
                space_id,
                device_id,
                config,
                branch,
                cwd,
                parent_chat_id,
            } => {
                self.doc_host.ensure_not_deleted(&chat_id).map_err(failed)?;
                self.workspace
                    .create_chat_with_parent(
                        &chat_id,
                        space_id.as_deref(),
                        device_id.as_deref(),
                        config,
                        cwd,
                        parent_chat_id,
                    )
                    .map_err(failed)?;
                if let Some(branch) = branch.as_deref().filter(|b| !b.is_empty()) {
                    self.workspace
                        .set_chat_branch(&chat_id, branch)
                        .map_err(failed)?;
                }
                Ok(())
            }
            MutateParams::CreateSpace {
                space_id,
                device_id,
                path,
                name,
                git_detected,
            } => self
                .workspace
                .create_space(&space_id, &device_id, &path, name, git_detected)
                .map_err(failed),
            MutateParams::RenameSpace { space_id, name } => self
                .workspace
                .rename_space(&space_id, name.as_deref())
                .map_err(failed)
                .map(drop),
            MutateParams::DeleteSpace { space_id } => {
                let chat_ids = self.workspace.begin_space_deletion(&space_id).map_err(failed)?;
                // Mark the entire project before awaiting any one runtime.
                for chat_id in &chat_ids {
                    self.sessions.begin_chat_deletion(chat_id);
                    self.doc_host.begin_chat_deletion(chat_id);
                }
                let mut settled = Vec::new();
                for chat_id in &chat_ids {
                    settled.push(self.sessions.settle_chat_deletion(chat_id).await.map_err(failed)?);
                }
                for chat_id in &chat_ids {
                    self.doc_host.ensure_chat_settled(chat_id).map_err(failed)?;
                }
                let deleted = self.workspace.delete_space(&space_id).map_err(failed)?;
                for chat_id in deleted.chat_ids {
                    // The shared workspace gate admitted no local fork after
                    // the first listing. Peer-added rows cannot start locally
                    // while their project is closing either.
                    self.sessions.begin_chat_deletion(&chat_id);
                    self.doc_host.begin_chat_deletion(&chat_id);
                    self.doc_host.purge_chat(&chat_id).map_err(failed)?;
                    self.sessions.forget_deleted_chat(&chat_id);
                }
                Ok(())
            }
            MutateParams::RenameChat { chat_id, title } => self
                .workspace
                .rename_chat(&chat_id, &title)
                .map_err(failed)
                .map(drop),
            MutateParams::SetChatBranch { chat_id, branch } => self
                .workspace
                .set_chat_branch(&chat_id, &branch)
                .map_err(failed)
                .map(drop),
            MutateParams::SetChatCwd { chat_id, cwd } => self
                .workspace
                .set_chat_cwd(&chat_id, &cwd)
                .map_err(failed)
                .map(drop),
            MutateParams::SetChatActivity {
                chat_id,
                last_message_at,
                created_at,
            } => self
                .workspace
                .set_chat_activity(&chat_id, last_message_at, created_at)
                .map_err(failed)
                .map(drop),
            MutateParams::SetChatHost { chat_id, device_id } => self
                .workspace
                .set_chat_host(&chat_id, &device_id)
                .map_err(failed)
                .map(drop),
            MutateParams::SetChatArchived { chat_id, archived } => self
                .workspace
                .set_chat_archived(&chat_id, archived)
                .map_err(failed)
                .map(drop),
            MutateParams::ChangeSidebarPin { change } => {
                self.workspace.change_sidebar_pin(&change).map_err(failed)
            }
            MutateParams::SetChatConfig { chat_id, config } => self
                .workspace
                .set_chat_config(&chat_id, &config)
                .map_err(failed)
                .map(drop),
            MutateParams::DeleteChat { chat_id } => {
                self.workspace.begin_chat_deletion(&chat_id);
                self.sessions.begin_chat_deletion(&chat_id);
                self.doc_host.begin_chat_deletion(&chat_id);
                let _settled = self.sessions.settle_chat_deletion(&chat_id).await.map_err(failed)?;
                self.doc_host.ensure_chat_settled(&chat_id).map_err(failed)?;
                self.workspace.delete_chat(&chat_id).map_err(failed)?;
                self.doc_host.purge_chat(&chat_id).map_err(failed)?;
                self.sessions.forget_deleted_chat(&chat_id);
                Ok(())
            }
            MutateParams::RenameDevice { device_id, name } => self
                .workspace
                .rename_device(&device_id, &name)
                .map_err(failed)
                .map(drop),
            MutateParams::MarkChatSeen { chat_id, at } => {
                let at = at
                    .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
                    .unwrap_or_else(chrono::Utc::now);
                self.workspace
                    .mark_chat_seen(&chat_id, at)
                    .map_err(failed)
                    .map(drop)
            }
        }
    }
}

/// An RPC rejection is scoped to the requested capability. Only a broken
/// transport means the shared device link itself cannot carry other calls.
fn should_invalidate_link(error: &RpcError) -> bool {
    matches!(error, RpcError::Closed | RpcError::Transport(_))
}

fn change_request_rpc_error(error: ChangeRequestError) -> RpcError {
    let code = match error {
        ChangeRequestError::CliUnavailable => capability_errors::PULL_REQUESTS_CLI_UNAVAILABLE,
        ChangeRequestError::Authentication => capability_errors::PULL_REQUESTS_AUTHENTICATION,
        ChangeRequestError::RateLimited => capability_errors::PULL_REQUESTS_RATE_LIMITED,
        ChangeRequestError::Timeout => capability_errors::PULL_REQUESTS_TIMEOUT,
        ChangeRequestError::Decode => capability_errors::PULL_REQUESTS_DECODE,
        ChangeRequestError::RepositoryUnavailable
        | ChangeRequestError::UnsupportedRepository
        | ChangeRequestError::CommandFailed => capability_errors::PULL_REQUESTS_COMMAND_FAILED,
    };
    RpcError::Capability(code.into())
}

/// Reply deadline for a relay-forwarded unary call. The relay is WebSocket
/// frames through a DO: a dropped frame (host socket replaced mid-call, DO
/// restart) loses the reply SILENTLY — the DO's auto-pong keeps the client
/// socket looking healthy — and an unbounded await wedged callers forever
/// (the composer's permanent "Sending…", 2026-08-18). Network-bound git and
/// update methods get a long leash; worktree creation checks out a full tree;
/// everything else is interactive and must fail fast.
fn forward_deadline(method: &str) -> std::time::Duration {
    use std::time::Duration;
    match method {
        methods::CLONE_REPO | methods::FETCH_ALL | methods::APPLY_UPDATE => {
            Duration::from_secs(15 * 60)
        }
        methods::CREATE_WORKTREE => Duration::from_secs(120),
        // Allow the adapter discovery budget plus relay and shutdown overhead.
        methods::LIST_MODELS | methods::LIST_COMMANDS => Duration::from_secs(100),
        methods::LIST_PLUGIN_MARKETPLACE => Duration::from_secs(75),
        methods::INSTALL_MARKETPLACE_PLUGIN => Duration::from_secs(600),
        _ => Duration::from_secs(30),
    }
}

/// ControlRpc methods that honor `targetDeviceId` (ARCHITECTURE.md). Extend this
/// list (plus [`is_stream_method`] for streams) to make more of the surface
/// device-addressable — the handlers themselves need no changes.
fn forwardable(method: &str) -> bool {
    matches!(
        method,
        methods::FORK_SIDE_CHAT
            | methods::LIST_HARNESSES
            | methods::SET_HARNESS_ENABLED
            | methods::GET_TITLE_SETTINGS
            | methods::SET_TITLE_SETTINGS
            | methods::GET_AGENT_SETTINGS
            | methods::LIST_PROJECT_MEMORY
            | methods::SET_AGENT_SETTINGS
            | methods::LIST_MODELS
            | methods::LIST_SKILLS
            | methods::CREATE_SKILL
            | methods::DELETE_SKILL
            | methods::SET_SKILL_ENABLED
            | methods::UPDATE_SKILL
            | methods::LIST_SUBAGENTS
            | methods::CREATE_SUBAGENT
            | methods::DELETE_SUBAGENT
            | methods::UPDATE_SUBAGENT
            | methods::SET_SUBAGENT_ENABLED
            | methods::SET_SUBAGENT_MODEL
            | methods::LIST_PLUGINS
            | methods::LIST_PLUGIN_MARKETPLACE
            | methods::INSTALL_MARKETPLACE_PLUGIN
            | methods::SET_PLUGIN_ENABLED
            | methods::UNINSTALL_PLUGIN
            | methods::BACKGROUND_SUBAGENT
            | methods::LIST_COMMAND_PROCESSES
            | methods::READ_COMMAND_OUTPUT
            | methods::STOP_COMMAND_PROCESS
            | methods::CANCEL_SUBAGENT
            | methods::UPDATE_GOAL
            | methods::LIST_MCP_SERVERS
            | methods::SET_MCP_SERVER
            | methods::LIST_COMMANDS
            | methods::QUEUE_COMMAND
            | methods::WATCH_DOC_MESSAGES
            | methods::WATCH_SESSION_STATE
            | methods::FETCH_TOOL_BLOB
            // The queue lives on the chat doc, and only its host may send from
            // it — same addressing as the command ledger next door.
            | methods::WATCH_QUEUE
            | methods::QUEUE_MESSAGE
            | methods::UPDATE_QUEUED_MESSAGE
            | methods::BEGIN_QUEUED_MESSAGE_EDIT
            | methods::RENEW_QUEUED_MESSAGE_EDIT
            | methods::FINISH_QUEUED_MESSAGE_EDIT
            | methods::MOVE_QUEUED_MESSAGE
            | methods::REMOVE_QUEUED_MESSAGE
            | methods::SEND_QUEUED_MESSAGE_NOW
            | methods::STEER_QUEUED_MESSAGE_NOW
            // Repos/worktrees/folders are device-local filesystem state.
            | methods::LIST_REPOS
            | methods::ADD_REPO
            | methods::CLONE_REPO
            | methods::CREATE_REPO
            | methods::LIST_BRANCHES
            | methods::LIST_REFS
            | methods::LIST_GIT_HISTORY
            | methods::SEARCH_GIT_HISTORY
            | methods::RESOLVE_GIT_AVATARS
            | methods::FETCH_ALL
            | methods::SWITCH_REF
            | methods::LIST_FOLDERS
            | methods::GET_WORKTREE_SETTINGS
            | methods::SET_WORKTREE_SETTINGS
            | methods::LIST_DRIVES
            | methods::SEARCH_HOME_FOLDERS
            | methods::RELEASE_HOME_FOLDER_SEARCH
            | methods::SEARCH_FILES
            | methods::LIST_WORKSPACE_DIRECTORY
            | methods::SEARCH_WORKSPACE_FILES
            | methods::WARM_WORKSPACE_SEARCH
            | methods::SEARCH_WORKSPACE_CONTENT
            | methods::READ_WORKSPACE_IMAGE
            | methods::READ_WORKSPACE_FILE
            | methods::DELETE_WORKSPACE_ENTRY
            | methods::MOVE_WORKSPACE_ENTRY
            | methods::WRITE_WORKSPACE_FILE
            | methods::WATCH_WORKSPACE_FILES
            | methods::CREATE_WORKTREE
            | methods::DELETE_WORKTREE
            // Checkout diffs are produced on the device holding the checkout.
            | methods::WATCH_CHECKOUT_DIFFS
            | methods::WATCH_WORKSPACE_GIT_STATUS
            | methods::WATCH_CHECKOUT_CHANGE_REQUEST
            | methods::LIST_CHANGE_REQUEST_PAGE
            | methods::GET_CHANGE_REQUEST_REPOSITORY
            | methods::GET_CHANGE_REQUEST
            | methods::GET_CHANGE_REQUEST_DIFF
            | methods::POST_CHANGE_REQUEST_COMMENT
            | methods::GET_CHECKOUT_DIFF
            | methods::DISCARD_WORKING_TREE
            | methods::GET_CHECKOUT_FILE_DIFF_TEXT
            // Terminals live on the chat's host device.
            | methods::OPEN_TERMINAL
            | methods::SUBSCRIBE_TERMINAL
            | methods::WRITE_TERMINAL
            | methods::RESIZE_TERMINAL
            | methods::CLOSE_TERMINAL
            // Uploads/attachments target the chat's host device (the agent reads
            // the committed file from that device's disk).
            | methods::UPLOAD_CHUNK
            | methods::UPLOAD_COMMIT
            | methods::READ_ATTACHMENT_CHUNK
            // Updates report/apply on the device whose binary they concern.
            | methods::UPDATE_STATUS
            | methods::APPLY_UPDATE
    )
}

/// Forwardable methods whose reply is a stream (proxied item-by-item).
fn is_stream_method(method: &str) -> bool {
    matches!(
        method,
        methods::WATCH_DOC_MESSAGES
            | methods::WATCH_SESSION_STATE
            | methods::WATCH_QUEUE
            | methods::SUBSCRIBE_TERMINAL
            | methods::WATCH_CHECKOUT_DIFFS
            | methods::WATCH_WORKSPACE_GIT_STATUS
            | methods::WATCH_CHECKOUT_CHANGE_REQUEST
            | methods::WATCH_WORKSPACE_FILES
            | methods::UPDATE_STATUS
    )
}

/// A watch receiver as a stream: current value first, then every change.
fn watch_stream<T>(rx: watch::Receiver<T>) -> BoxStream<'static, serde_json::Value>
where
    T: serde::Serialize + Clone + Send + Sync + 'static,
{
    futures::stream::unfold((rx, false), |(mut rx, emitted)| async move {
        if emitted {
            rx.changed().await.ok()?;
        }
        let value = {
            let borrowed = rx.borrow_and_update();
            serde_json::to_value(&*borrowed).ok()?
        };
        Some((value, (rx, true)))
    })
    .boxed()
}

/// The transcript watch as delta frames (`rcode_doc::transcript_delta`): a
/// full `reset` first, then only changed entries per commit — the whole-Vec
/// serialization here was the per-tick cost that scaled with transcript size.
fn doc_messages_stream(
    rx: watch::Receiver<crate::doc_host::TranscriptSnapshot>,
    doc: std::sync::Arc<rcode_doc::SessionDoc>,
) -> BoxStream<'static, serde_json::Value> {
    use rcode_doc::transcript_delta::{TranscriptFrame, diff_transcript};
    futures::stream::unfold(
        (
            rx,
            None::<crate::doc_host::TranscriptSnapshot>,
            doc,
            None,
            rcode_doc::TranscriptBaseline::default(),
        ),
        |(mut rx, mut prev, doc, mut previous_usage, mut opening_baseline)| async move {
            loop {
                if prev.is_some() {
                    rx.changed().await.ok()?;
                }
                // Watchers retain the immutable published snapshot. Each
                // connection used to deep-copy the entire transcript here.
                let current = rx.borrow_and_update().clone();
                let frame = match prev.as_ref() {
                    None => TranscriptFrame::reset(&current.entries),
                    Some(prev) => diff_transcript(&prev.entries, &current.entries),
                };
                let replay_baseline = match prev.as_ref() {
                    None => {
                        opening_baseline = rcode_doc::TranscriptBaseline::capture(&current.entries);
                        Some(opening_baseline.clone())
                    }
                    Some(prev)
                        if !std::sync::Arc::ptr_eq(
                            &prev.replay_baseline,
                            &current.replay_baseline,
                        ) =>
                    {
                        // The tracker only observes changes after attach; its
                        // baseline omits unchanged cached parts. Preserve this
                        // subscription's opening cutoff without capturing live
                        // appends or sharing another viewer's later cutoff.
                        // Ordinary live updates never rebuild this metadata.
                        let mut baseline = (*current.replay_baseline).clone();
                        for (entry, parts) in &opening_baseline.entries {
                            let merged = baseline.entries.entry(entry.clone()).or_default();
                            for (part, &len) in parts {
                                let cutoff = merged.entry(part.clone()).or_default();
                                *cutoff = (*cutoff).max(len);
                            }
                        }
                        Some(baseline)
                    }
                    _ => None,
                };
                prev = Some(current);
                // No-op commits (a second watcher attaching, command-only
                // changes) produce empty deltas — skip the frame entirely.
                let usage = doc.context_usage();
                let goal = doc.goal();
                let state = (usage, goal.clone());
                if frame.is_empty_delta() && previous_usage.as_ref() == Some(&state) && replay_baseline.is_none() {
                    continue;
                }
                previous_usage = Some(state);
                let value = serde_json::to_value(rcode_doc::TranscriptUpdate {
                    frame,
                    context_usage: usage,
                    goal,
                    replay_baseline,
                })
                .ok()?;
                return Some((value, (rx, prev, doc, previous_usage, opening_baseline)));
            }
        },
    )
    .boxed()
}

/// First paint reads only the local tail; full history is deferred until the
/// next stream poll. No network dependency or persisted truncation.
async fn opening_doc_messages_stream(
    host: crate::doc_host::DocHost,
    chat_id: String,
) -> Result<BoxStream<'static, serde_json::Value>, RpcError> {
    let (handle, preview) = tokio::task::spawn_blocking(move || {
        let handle = host.open(&chat_id)?;
        let entries = handle.doc().read_opening_tail(128)?;
        let mut preview = serde_json::to_value(rcode_doc::TranscriptUpdate {
            frame: rcode_doc::TranscriptFrame::reset(&entries),
            context_usage: handle.doc().context_usage(),
            goal: handle.doc().goal(),
            replay_baseline: Some(rcode_doc::TranscriptBaseline::capture(&entries)),
        })
        .map_err(|e| crate::EngineError::Other(e.to_string()))?;
        preview["historyPending"] = serde_json::Value::Bool(true);
        Ok::<_, crate::EngineError>((handle, preview))
    })
    .await
    .map_err(|e| RpcError::Failed(e.to_string()))?
    .map_err(|e| RpcError::Failed(e.to_string()))?;
    // Do not build the full mirror before yielding the preview.
    // The next poll attaches normally and begins with a complete
    // authoritative reset; subsequent frames use normal deltas.
    let full = futures::stream::once(async move {
        match tokio::task::spawn_blocking(move || (handle.watch_messages(), handle.doc_arc())).await
        {
            Ok((rx, doc)) => doc_messages_stream(rx, doc),
            Err(error) => {
                tracing::warn!(%error, "transcript opening failed");
                futures::stream::empty().boxed()
            }
        }
    })
    .flatten();
    Ok(futures::stream::once(async move { preview })
        .chain(full)
        .boxed())
}

/// Auth RPC surface. Local-only: the device is always signed in as its fixed
/// local identity, so `AuthStatus` streams a constant signed-in state and the
/// former sign-in / sign-out / org mutation methods are no longer part of the
/// surface (`handles` returns false for them, so they fall through to
/// `UnknownMethod`).
#[derive(Clone)]
pub struct AuthRpc {
    auth: Auth,
}

impl AuthRpc {
    pub fn new(auth: Auth) -> Self {
        Self { auth }
    }

    pub fn handles(method: &str) -> bool {
        matches!(method, methods::AUTH_STATUS)
    }
}

#[async_trait]
impl RpcService for AuthRpc {
    async fn handle(&self, method: &str, _params: serde_json::Value) -> Result<RpcReply, RpcError> {
        match method {
            methods::AUTH_STATUS => Ok(RpcReply::Stream(watch_stream(self.auth.watch_state()))),
            _ => Err(RpcError::UnknownMethod(method.to_string())),
        }
    }
}

#[async_trait]
impl RpcService for EngineRpc {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<RpcReply, RpcError> {
        if let Some(hosts) = &self.remote_hosts
            && let Some(target) = hosts.target_for(method, &params)
            && let Some(client) = hosts.client(&target)
        {
            return self.forward_direct(client?, method, params).await;
        }
        // Device-addressed routing: forward calls that target another device over its
        // relay. The target compares the id to its own, so forwards cannot loop.
        if forwardable(method)
            && let Some(target) = params.get("targetDeviceId").and_then(|v| v.as_str())
            && target != self.doc_host.device_id()
        {
            let target = target.to_string();
            return self.forward(&target, method, params).await;
        }
        if AuthRpc::handles(method) {
            return AuthRpc::new(self.auth()?.clone())
                .handle(method, params)
                .await;
        }
        match method {
            "ListRemoteHosts" => RpcReply::value(&self.hosts()?.list()),
            "SaveRemoteHost" => RpcReply::value(&self.hosts()?.save(parse_params(params)?).await?),
            "GetRemoteHostApiKey" => {
                let id = params["id"].as_str().ok_or_else(|| RpcError::BadParams("missing host id".into()))?;
                RpcReply::value(&serde_json::json!({"apiKey": self.hosts()?.api_key(id)?}))
            }
            "RemoveRemoteHost" => {
                let id = params["id"].as_str().ok_or_else(|| RpcError::BadParams("missing host id".into()))?;
                self.hosts()?.remove(id).await?;
                RpcReply::value(&serde_json::json!({"ok":true}))
            }
            "GetRemoteAccess" => RpcReply::value(&self.remote_access.as_ref().ok_or_else(|| RpcError::Failed("external access management is local only".into()))?.view()),
            "GetRemoteAccessApiKey" => RpcReply::value(&serde_json::json!({"apiKey":self.remote_access.as_ref().ok_or_else(|| RpcError::Failed("external access management is local only".into()))?.api_key()})),
            "SetRemoteAccess" => {
                let access = self.remote_access.as_ref().ok_or_else(|| RpcError::Failed("external access management is local only".into()))?;
                let edit = parse_params(params)?;
                RpcReply::value(&access.set(edit).await?)
            }
            methods::ENGINE_INFO => RpcReply::value(&self.engine_info),
            methods::ENGINE_READY => RpcReply::value(&serde_json::json!({ "ready": true })),
            methods::GET_TITLE_SETTINGS => RpcReply::value(&self.registry.title_settings()),
            methods::SET_TITLE_SETTINGS => {
                let settings: TitleSettings = parse_params(params)?;
                self.registry
                    .set_title_settings(settings)
                    .map_err(RpcError::Failed)?;
                // Re-read rather than echoing the request: the registry is the
                // authority, and it may have refused or normalized the value.
                RpcReply::value(&self.registry.title_settings())
            }
            methods::GET_PROVIDER_SETTINGS => {
                let store = self.provider_store()?;
                RpcReply::value(&store.view())
            }
            methods::GET_PROVIDER_API_KEY => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    provider_id: String,
                }
                let p: Params = parse_params(params)?;
                let store = self.provider_store()?;
                let environment = std::env::vars().collect();
                RpcReply::value(&serde_json::json!({
                    "apiKey": store.api_key(&p.provider_id, &environment)
                }))
            }
            methods::SAVE_PROVIDER_SETTINGS => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    /// The revision the draft was written against. A mismatch
                    /// is a conflict, not an overwrite.
                    expected_revision: String,
                    draft: rcode_harness::provider_store::ProviderDraft,
                }
                let p: Params = parse_params(params)?;
                let store = self.provider_store()?;
                let view =
                    store
                        .save(p.draft, &p.expected_revision)
                        .map_err(|error| match error {
                            // A conflict is a state the UI handles (keep the
                            // draft, offer to reload), so it is reported as a
                            // distinct error rather than a generic failure.
                            rcode_harness::provider_store::SaveError::Conflict { current } => {
                                RpcError::Failed(format!(
                                    "{}:{current}",
                                    rcode_harness::provider_store::CONFLICT_CODE
                                ))
                            }
                            other => RpcError::Failed(other.to_string()),
                        })?;
                RpcReply::value(&view)
            }
            methods::FETCH_PROVIDER_MODELS => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    provider_id: String,
                    expected_revision: String,
                }
                let p: Params = parse_params(params)?;
                let snapshot = self.provider_store()?.snapshot();
                if snapshot.revision != p.expected_revision {
                    return Err(RpcError::Failed(format!(
                        "{}:{}",
                        rcode_harness::provider_store::CONFLICT_CODE,
                        snapshot.revision
                    )));
                }
                let provider = snapshot
                    .config
                    .providers
                    .iter()
                    .find(|provider| provider.id == p.provider_id)
                    .ok_or_else(|| RpcError::Failed("provider no longer exists".into()))?;
                let models = crate::provider_models::fetch(provider)
                    .await
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&models)
            }
            methods::GET_AGENT_SETTINGS => {
                let settings = rcode_harness::settings::AgentSettings::load(&self.config_root);
                RpcReply::value(&settings)
            }
            methods::GET_CUSTOM_INSTRUCTIONS => {
                let document = crate::custom_instructions::read(&self.config_root)
                    .map_err(RpcError::Failed)?;
                RpcReply::value(&document)
            }
            methods::SAVE_CUSTOM_INSTRUCTIONS => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    content: String,
                    expected_revision: Option<String>,
                }
                let p: Params = parse_params(params)?;
                let document = crate::custom_instructions::save(
                    &self.config_root,
                    p.content,
                    p.expected_revision,
                )
                .map_err(RpcError::Failed)?;
                RpcReply::value(&document)
            }
            methods::SET_AGENT_SETTINGS => {
                let settings: rcode_harness::settings::AgentSettings = parse_params(params)?;
                settings
                    .save(&self.config_root)
                    .map_err(|error| RpcError::Failed(format!("agent-settings.json: {error}")))?;
                // Re-read rather than echoing the request: the file is the
                // authority, and a write that landed differently shows up here.
                let stored = rcode_harness::settings::AgentSettings::load(&self.config_root);
                RpcReply::value(&stored)
            }
            methods::LIST_HARNESSES => RpcReply::value(&self.registry.descriptors()),
            methods::SET_HARNESS_ENABLED => {
                let p: SetHarnessEnabledParams = parse_params(params)?;
                update_harness_enabled(&self.registry, p.harness, p.enabled).await?;
                // Fresh catalog in the reply: the page repaints from it in one
                // round trip, and a refused/raced toggle self-corrects.
                RpcReply::value(&self.registry.descriptors())
            }
            methods::LIST_MODELS => {
                let p: ListModelsParams = parse_params(params)?;
                let harness = self
                    .registry
                    .resolve(p.harness)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                let models = harness
                    .models()
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&models)
            }
            methods::LIST_CUSTOM_COMMANDS => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    #[serde(default)] personal: bool,
                    #[serde(default)] chat_id: Option<String>,
                    #[serde(default)] space_id: Option<String>,
                    #[serde(default)] path: Option<String>,
                }
                let p: Params = parse_params(params)?;
                let root = if p.personal { None } else {
                    Some(self.catalog_root(&FileSearchParams { query: String::new(), chat_id: p.chat_id, space_id: p.space_id, path: p.path }).await?)
                };
                let harness = self.registry.resolve(HarnessId::Native).map_err(|e| RpcError::Failed(e.to_string()))?;
                let commands = harness.custom_commands(root.as_deref()).await.map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&commands)
            }
            methods::MUTATE_CUSTOM_COMMAND => {
                let mutation: rcode_proto::custom_commands::CommandMutation = parse_params(params)?;
                let harness = self.registry.resolve(HarnessId::Native).map_err(|e| RpcError::Failed(e.to_string()))?;
                harness.mutate_custom_command(mutation).await.map_err(|e| RpcError::Failed(e.to_string()))?;
                let commands = harness.custom_commands(None).await.map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&commands)
            }
            methods::LIST_SKILLS => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    #[serde(default)]
                    personal: bool,
                    harness: HarnessId,
                    #[serde(default)]
                    chat_id: Option<String>,
                    #[serde(default)]
                    space_id: Option<String>,
                    #[serde(default)]
                    path: Option<String>,
                }
                let p: Params = parse_params(params)?;
                if p.personal {
                    let harness = self
                        .registry
                        .resolve(p.harness)
                        .map_err(|error| RpcError::Failed(error.to_string()))?;
                    let rows = harness
                        .personal_skills()
                        .await
                        .map_err(|error| RpcError::Failed(error.to_string()))?;
                    return RpcReply::value(&rows);
                }
                let root = self
                    .catalog_root(&FileSearchParams {
                        query: String::new(),
                        chat_id: p.chat_id,
                        space_id: p.space_id,
                        path: p.path,
                    })
                    .await?;
                let skills = self
                    .registry
                    .discover_skills(p.harness, &root)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&skills)
            }
            methods::CREATE_SKILL => {
                let p: SkillWriteParams = parse_params(params)?;
                let root = self.skill_root(&p).await?;
                self.registry
                    .resolve(HarnessId::Native)
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .create_skill(
                        &root,
                        p.scope.unwrap_or_default(),
                        &p.name,
                        &p.description,
                        &p.body.clone().unwrap_or_default(),
                    )
                    .map_err(RpcError::Failed)?;
                // Reply with the fresh list so the page repaints in one round
                // trip, the same shape the enable toggle uses.
                self.reply_skills(&p).await
            }
            methods::DELETE_SKILL => {
                let p: SkillWriteParams = parse_params(params)?;
                let root = self.skill_root(&p).await?;
                self.registry
                    .resolve(HarnessId::Native)
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .delete_skill(&root, p.scope.unwrap_or_default(), &p.name)
                    .map_err(RpcError::Failed)?;
                self.reply_skills(&p).await
            }
            methods::UPDATE_SKILL => {
                let p: SkillWriteParams = parse_params(params)?;
                let root = self.skill_root(&p).await?;
                let new_name = p.new_name.clone().unwrap_or_else(|| p.name.clone());
                self.registry
                    .resolve(HarnessId::Native)
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .update_skill(
                        &root,
                        p.scope.unwrap_or_default(),
                        &p.name,
                        &new_name,
                        &p.description,
                        // An absent body keeps the skill's instruction; a
                        // present one replaces it. The wire makes no
                        // distinction between "absent" and "empty", and an
                        // empty body is not an edit anyone means.
                        p.body.as_deref().filter(|body| !body.trim().is_empty()),
                    )
                    .map_err(RpcError::Failed)?;
                self.reply_skills(&p).await
            }
            methods::SET_SKILL_ENABLED => {
                let p: SkillWriteParams = parse_params(params)?;
                if p.personal {
                    self.skill_root(&p).await?;
                }
                let enabled = p.enabled.unwrap_or(true);
                let mut settings = rcode_harness::settings::AgentSettings::load(&self.config_root);
                // Keyed by the skill's directory, which the caller passes back
                // exactly as the list gave it — so the two ends agree on which
                // skill a toggle means without re-deriving the path.
                let Some(skill_path) = p.skill_path.clone() else {
                    return Err(RpcError::Failed(
                        "SetSkillEnabled needs the skill's path".to_owned(),
                    ));
                };
                // A caller that names a scope must name a skill the harness
                // actually advertises *in that scope* at this root. Checked
                // against the listing itself rather than re-derived paths, so
                // the toggle and the list can never disagree about layout —
                // and a toggle can only ever name a skill this application
                // manages, not an arbitrary file path.
                if let Some(scope) = p.scope {
                    let harness = self
                        .registry
                        .resolve(HarnessId::Native)
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                    let root = self.skill_root(&p).await?;
                    let listed = if p.personal {
                        harness.personal_skills().await
                    } else {
                        harness.skills(&root).await
                    }
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .unwrap_or_default();
                    let known = listed.iter().any(|skill| {
                        skill.path == skill_path && skill.scope.is_none_or(|listed| listed == scope)
                    });
                    if !known {
                        return Err(RpcError::Failed(format!(
                            "`{skill_path}` is not a {} skill this device manages",
                            scope.label()
                        )));
                    }
                }
                if enabled {
                    settings.disabled_skills.remove(&skill_path);
                } else {
                    settings.disabled_skills.insert(skill_path);
                }
                settings
                    .save(&self.config_root)
                    .map_err(|error| RpcError::Failed(format!("agent-settings.json: {error}")))?;
                self.reply_skills(&p).await
            }
            methods::UPDATE_GOAL => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params { chat_id: String, action: String, #[serde(default)] goal_id: Option<String>, #[serde(default)] objective: Option<String>, #[serde(default)] revision: Option<u64> }
                let p: Params = parse_params(params)?;
                let goal = self.sessions.update_goal_if_current(&p.chat_id, &p.action, p.goal_id.as_deref(), p.objective.as_deref(), p.revision).await
                    .map_err(|error| RpcError::Failed(error.to_string()))?;
                RpcReply::value(&serde_json::json!({"goal":goal}))
            }
            methods::BACKGROUND_SUBAGENT => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params { chat_id: String, parent_tool_use_id: String }
                let p: Params = parse_params(params)?;
                let backgrounded = self.registry.resolve(HarnessId::Native).map_err(|e| RpcError::Failed(e.to_string()))?
                    .background_subagent(&p.chat_id, &p.parent_tool_use_id);
                RpcReply::value(&serde_json::json!({"backgrounded":backgrounded}))
            }
            methods::STOP_ALL_COMMAND_PROCESSES => {
                self.registry.resolve(HarnessId::Native).map_err(|e| RpcError::Failed(e.to_string()))?
                    .stop_all_command_processes().map_err(RpcError::Failed)?;
                RpcReply::value(&serde_json::json!({"stopped":true}))
            }
            methods::LIST_COMMAND_PROCESSES | methods::STOP_COMMAND_PROCESS | methods::READ_COMMAND_OUTPUT => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params { chat_id: String, #[serde(default)] id: Option<String> }
                let p: Params = parse_params(params)?;
                let harness = self.registry.resolve(HarnessId::Native).map_err(|e| RpcError::Failed(e.to_string()))?;
                if method == methods::READ_COMMAND_OUTPUT {
                    let id = p.id.ok_or_else(|| RpcError::BadParams("missing process id".into()))?;
                    let value = tokio::task::spawn_blocking(move || harness.read_command_output(&p.chat_id, &id))
                        .await.map_err(|e| RpcError::Failed(e.to_string()))?.map_err(RpcError::Failed)?;
                    RpcReply::value(&value)
                } else if method == methods::LIST_COMMAND_PROCESSES {
                    let value = tokio::task::spawn_blocking(move || harness.list_command_processes(&p.chat_id))
                        .await.map_err(|e| RpcError::Failed(e.to_string()))?;
                    RpcReply::value(&value)
                } else {
                    let id = p.id.ok_or_else(|| RpcError::Failed("missing process id".into()))?;
                    let stopped = harness.stop_command_process(&p.chat_id, &id).map_err(RpcError::Failed)?;
                    RpcReply::value(&serde_json::json!({"stopped": stopped}))
                }
            }
            methods::CANCEL_SUBAGENT => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    /// Kept for wire symmetry; the id below is what identifies
                    /// the subagent.
                    #[serde(default)]
                    #[allow(dead_code)]
                    chat_id: Option<String>,
                    parent_tool_use_id: String,
                }
                let p: Params = parse_params(params)?;
                // The native driver is the only one that runs subagents in
                // this process, so it is the one that can stop one. A driver
                // without subagents answers `false` through the trait default.
                let stopped = self
                    .registry
                    .resolve(HarnessId::Native)
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .cancel_subagent(&p.parent_tool_use_id);
                RpcReply::value(&serde_json::json!({ "stopped": stopped }))
            }
            methods::LIST_PLUGINS => RpcReply::value(&rcode_plugins::discover(&self.config_root)),
            methods::LIST_PLUGIN_MARKETPLACE => {
                #[derive(Deserialize)]
                struct Params {
                    #[serde(default)]
                    refresh: bool,
                    #[serde(default = "default_marketplace_source")]
                    source: String,
                }
                fn default_marketplace_source() -> String { "official".into() }
                let p: Params = parse_params(params)?;
                let listing = rcode_plugins::marketplace::list(&self.config_root, &p.source, p.refresh).await
                    .map_err(|error| RpcError::Failed(error.to_string()))?;
                RpcReply::value(&listing)
            }
            methods::INSTALL_MARKETPLACE_PLUGIN => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Params { source: String, name: String }
                let p: Params = parse_params(params)?;
                rcode_plugins::marketplace::install(&self.config_root, &p.source, &p.name).await
                    .map_err(|error| RpcError::Failed(error.to_string()))?;
                RpcReply::value(&rcode_plugins::discover(&self.config_root))
            }
            methods::SET_PLUGIN_ENABLED => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    name: String,
                    enabled: bool,
                }
                let p: Params = parse_params(params)?;
                rcode_plugins::set_enabled(&self.config_root, &p.name, p.enabled)
                    .map_err(|error| RpcError::Failed(error.to_string()))?;
                RpcReply::value(&rcode_plugins::discover(&self.config_root))
            }
            methods::UNINSTALL_PLUGIN => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    name: String,
                }
                let p: Params = parse_params(params)?;
                rcode_plugins::uninstall(&self.config_root, &p.name)
                    .map_err(|error| RpcError::Failed(error.to_string()))?;
                RpcReply::value(&rcode_plugins::discover(&self.config_root))
            }
            methods::LIST_MCP_SERVERS => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    #[serde(default)]
                    personal: bool,
                    #[serde(default)]
                    chat_id: Option<String>,
                    #[serde(default)]
                    space_id: Option<String>,
                    #[serde(default)]
                    path: Option<String>,
                }
                let p: Params = parse_params(params)?;
                let root = if p.personal {
                    self.config_root.clone()
                } else {
                    self.catalog_root(&FileSearchParams {
                        query: String::new(),
                        chat_id: p.chat_id,
                        space_id: p.space_id,
                        path: p.path,
                    })
                    .await?
                };
                // The listing is the configuration, not a connection: probing
                // every server to render a settings page would start processes
                // the user may have switched off.
                match if p.personal {
                    rcode_mcp::config::load_personal(&self.config_root)
                } else {
                    rcode_mcp::load(&root, &self.config_root)
                } {
                    Ok(servers) => RpcReply::value(
                        &servers
                            .iter()
                            .map(|server| rcode_mcp::ServerReport {
                                name: server.name.clone(),
                                source: server.source,
                                transport: server.transport.describe(),
                                enabled: server.enabled,
                                // "Configured" is what this call can promise;
                                // the run connects.
                                connected: false,
                                problem: None,
                                tool_count: 0,
                            })
                            .collect::<Vec<_>>(),
                    ),
                    Err(error) => return Err(RpcError::Failed(error.to_string())),
                }
            }
            methods::SET_MCP_SERVER => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    #[serde(default)]
                    create_only: bool,
                    #[serde(default)]
                    personal: bool,
                    #[serde(default)]
                    chat_id: Option<String>,
                    #[serde(default)]
                    space_id: Option<String>,
                    #[serde(default)]
                    path: Option<String>,
                    name: String,
                    /// Null removes the entry.
                    #[serde(default)]
                    server: Option<serde_json::Value>,
                    /// Which file to write: the project's or the user's.
                    #[serde(default)]
                    scope: Option<rcode_proto::invocation::SkillScope>,
                }
                let p: Params = parse_params(params)?;
                let project_root = if p.personal {
                    self.config_root.clone()
                } else {
                    self.catalog_root(&FileSearchParams {
                        query: String::new(),
                        chat_id: p.chat_id,
                        space_id: p.space_id,
                        path: p.path,
                    })
                    .await?
                };
                let scope = p.scope.unwrap_or_default();
                if p.personal && scope != rcode_proto::invocation::SkillScope::User {
                    return Err(RpcError::BadParams(
                        "personal MCP settings require user scope".into(),
                    ));
                }
                let path = match scope {
                    rcode_proto::invocation::SkillScope::Project => project_root
                        .join(rcode_mcp::config::PROJECT_DIR)
                        .join(rcode_mcp::config::FILE_NAME),
                    rcode_proto::invocation::SkillScope::User => {
                        self.config_root.join(rcode_mcp::config::FILE_NAME)
                    }
                    // A plugin's servers live in the plugin's own file. Writing
                    // one here would be undone by the next install, so the
                    // request is refused rather than aimed somewhere surprising.
                    rcode_proto::invocation::SkillScope::Plugin => {
                        return Err(RpcError::Failed(
                            "plugin-provided MCP servers are configured in the plugin".to_owned(),
                        ));
                    }
                };
                write_mcp_entry(&path, &p.name, p.server, p.create_only)
                    .map_err(|error| RpcError::Failed(error.to_string()))?;
                // Reply with the fresh listing, the same one-round-trip shape
                // every other settings write uses.
                let servers = if p.personal {
                    rcode_mcp::config::load_personal(&self.config_root)
                } else {
                    rcode_mcp::load(&project_root, &self.config_root)
                }
                .map_err(|error| RpcError::Failed(error.to_string()))?;
                RpcReply::value(
                    &servers
                        .iter()
                        .map(|server| rcode_mcp::ServerReport {
                            name: server.name.clone(),
                            source: server.source,
                            transport: server.transport.describe(),
                            enabled: server.enabled,
                            connected: false,
                            problem: None,
                            tool_count: 0,
                        })
                        .collect::<Vec<_>>(),
                )
            }
            methods::LIST_SUBAGENTS => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    #[serde(default)]
                    personal: bool,
                    #[serde(default)]
                    chat_id: Option<String>,
                    #[serde(default)]
                    space_id: Option<String>,
                    #[serde(default)]
                    path: Option<String>,
                }
                let p: Params = parse_params(params)?;
                if p.personal {
                    let harness = self
                        .registry
                        .resolve(HarnessId::Native)
                        .map_err(|error| RpcError::Failed(error.to_string()))?;
                    let rows = harness
                        .personal_subagents()
                        .await
                        .map_err(|error| RpcError::Failed(error.to_string()))?;
                    return RpcReply::value(&rows);
                }
                let root = self
                    .catalog_root(&FileSearchParams {
                        query: String::new(),
                        chat_id: p.chat_id,
                        space_id: p.space_id,
                        path: p.path,
                    })
                    .await?;
                let definitions = self
                    .registry
                    .resolve(HarnessId::Native)
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .subagents(&root)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .unwrap_or_default();
                RpcReply::value(&definitions)
            }
            methods::CREATE_SUBAGENT => {
                let p: SubagentWriteParams = parse_params(params)?;
                let root = self.subagent_root(&p).await?;
                self.registry
                    .resolve(HarnessId::Native)
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .create_subagent(
                        &root,
                        p.scope.unwrap_or_default(),
                        &p.name,
                        &p.description,
                        &p.system_prompt.clone().unwrap_or_default(),
                        &p.tools,
                        &p.disallowed_tools,
                        // Absent is the file's own "inherit" default.
                        &p.model.clone().unwrap_or_else(|| "inherit".to_owned()),
                    )
                    .map_err(RpcError::Failed)?;
                self.reply_subagents(&p).await
            }
            methods::DELETE_SUBAGENT => {
                let p: SubagentWriteParams = parse_params(params)?;
                let root = self.subagent_root(&p).await?;
                self.registry
                    .resolve(HarnessId::Native)
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .delete_subagent(&root, p.scope.unwrap_or_default(), &p.name)
                    .map_err(RpcError::Failed)?;
                self.reply_subagents(&p).await
            }
            methods::UPDATE_SUBAGENT => {
                let p: SubagentWriteParams = parse_params(params)?;
                let root = self.subagent_root(&p).await?;
                let new_name = p.new_name.clone().unwrap_or_else(|| p.name.clone());
                self.registry
                    .resolve(HarnessId::Native)
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .update_subagent(
                        &root,
                        p.scope.unwrap_or_default(),
                        &p.name,
                        &new_name,
                        &p.description,
                        // An absent body keeps the definition's prompt; an
                        // empty one is not an edit anyone means.
                        p.system_prompt
                            .as_deref()
                            .filter(|prompt| !prompt.trim().is_empty()),
                        &p.tools,
                        &p.disallowed_tools,
                        &p.model.clone().unwrap_or_else(|| "inherit".to_owned()),
                    )
                    .map_err(RpcError::Failed)?;
                self.reply_subagents(&p).await
            }
            methods::SET_SUBAGENT_ENABLED | methods::SET_SUBAGENT_MODEL => {
                let p: SubagentWriteParams = parse_params(params)?;
                if p.personal {
                    self.subagent_root(&p).await?;
                }
                let mut settings = rcode_harness::settings::AgentSettings::load(&self.config_root);
                // Keyed by the definition's path, which the caller passes
                // back exactly as the list gave it — so the two ends agree on
                // which definition a toggle means without re-deriving the
                // path.
                let Some(definition_path) = p.definition_path.clone() else {
                    return Err(RpcError::Failed(
                        "the subagent toggles need the definition's path".to_owned(),
                    ));
                };
                // A caller that names a scope must name a definition the
                // harness actually advertises *in that scope* at this root —
                // checked against the listing itself, so a toggle can only
                // ever name a definition this application manages, never an
                // arbitrary file path.
                if p.scope.is_some() || p.personal {
                    let harness = self
                        .registry
                        .resolve(HarnessId::Native)
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                    let root = self.subagent_root(&p).await?;
                    let listed = if p.personal {
                        harness.personal_subagents().await
                    } else {
                        harness.subagents(&root).await
                    }
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .unwrap_or_default();
                    let known = listed.iter().any(|definition| {
                        definition.path == definition_path
                            && p.scope.is_none_or(|scope| definition.scope == Some(scope))
                    });
                    if !known {
                        return Err(RpcError::Failed(format!(
                            "`{definition_path}` is not a {} subagent this device manages",
                            p.scope.map(|scope| scope.label()).unwrap_or("built-in")
                        )));
                    }
                }
                if method == methods::SET_SUBAGENT_ENABLED
                    && definition_path.starts_with(rcode_harness::native_subagents::BUILTIN_PREFIX)
                {
                    return Err(RpcError::BadParams(
                        "built-in subagents do not have an enable switch".into(),
                    ));
                }
                if method == methods::SET_SUBAGENT_ENABLED {
                    let enabled = p.enabled.unwrap_or(true);
                    if enabled {
                        settings.disabled_subagents.remove(&definition_path);
                    } else {
                        settings.disabled_subagents.insert(definition_path);
                    }
                } else {
                    // Validated here, at the toggle: an alias a typo broke
                    // must be refused now, not discovered as a silent
                    // fallback at run time. `inherit` (the absent spelling)
                    // removes any override.
                    let raw = p.model.clone().unwrap_or_else(|| "inherit".to_owned());
                    rcode_harness::native_subagents::parse_model_alias(&raw)
                        .map_err(RpcError::Failed)?;
                    settings.subagent_models.insert(definition_path, raw);
                }
                settings
                    .save(&self.config_root)
                    .map_err(|error| RpcError::Failed(format!("agent-settings.json: {error}")))?;
                self.reply_subagents(&p).await
            }
            methods::LIST_COMMANDS => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Params {
                    harness: HarnessId,
                    #[serde(default)]
                    chat_id: Option<String>,
                    #[serde(default)]
                    space_id: Option<String>,
                    #[serde(default)]
                    path: Option<String>,
                }
                let p: Params = parse_params(params)?;
                let root = self
                    .catalog_root(&FileSearchParams {
                        query: String::new(),
                        chat_id: p.chat_id,
                        space_id: p.space_id,
                        path: p.path,
                    })
                    .await?;
                let commands = self
                    .registry
                    .discover_commands(p.harness, &root)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&commands)
            }
            methods::QUEUE_COMMAND => {
                let p: QueueCommandParams = parse_params(params)?;
                let command_id = self
                    .doc_host
                    .queue_command_with_transfers(&p.chat_id, p.command, p.transfers)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "commandId": command_id }))
            }
            methods::RETRY_DELIVERY => {
                let p: ChatParams = parse_params(params)?;
                self.doc_host
                    .retry_delivery(&p.chat_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({}))
            }
            methods::RELAY_COMMAND => {
                let p: RelayCommandParams = parse_params(params)?;
                let outcome = self
                    .doc_host
                    .ingest_relayed_command(&p.chat_id, p.entry)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "outcome": outcome }))
            }
            methods::FORK_SIDE_CHAT => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct ForkParams {
                    chat_id: String,
                    source_chat_id: String,
                    /// Where the fork hangs in the tree. Defaults to the
                    /// source; a side chat's own fork button passes the
                    /// side chat's parent so the copy lists as a sibling.
                    #[serde(default)]
                    parent_chat_id: Option<String>,
                }
                let p: ForkParams = parse_params(params)?;
                let parent_chat_id = p
                    .parent_chat_id
                    .clone()
                    .filter(|id| !id.trim().is_empty())
                    .unwrap_or_else(|| p.source_chat_id.clone());
                let failed = |e: crate::EngineError| RpcError::Failed(e.to_string());
                let source = self
                    .workspace
                    .chat(&p.source_chat_id)
                    .map_err(failed)?
                    .ok_or_else(|| RpcError::Failed("Source chat no longer exists".into()))?;
                if source.device_id != self.doc_host.device_id() {
                    return Err(RpcError::Failed(
                        "Fork must be created on the source device".into(),
                    ));
                }
                if let Some(existing) = self
                    .workspace
                    .chat(&p.chat_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                {
                    if existing.parent_chat_id.as_deref() == Some(parent_chat_id.as_str()) {
                        return RpcReply::value(&existing);
                    }
                    return Err(RpcError::Failed("Chat id already exists".into()));
                }
                let source_doc = self.doc_host.open(&p.source_chat_id).map_err(failed)?;
                let entries = source_doc
                    .doc()
                    .read_entries()
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                let boundary = entries
                    .iter()
                    .rposition(|entry| {
                        entry.role == rcode_doc::MessageRole::Assistant
                            && entry.status == Some(rcode_doc::MessageStatus::Complete)
                    })
                    .ok_or_else(|| {
                        RpcError::Failed(
                            "Wait for a completed response before starting a side chat".into(),
                        )
                    })?;
                let mut chat = source.clone();
                chat.id = p.chat_id;
                chat.parent_chat_id = Some(parent_chat_id);
                chat.title = None; // First side-chat turn receives its own generated title.
                chat.archived = false;
                chat.created_at = chrono::Utc::now();
                chat.last_message_at = None;
                chat.last_message_preview = None;
                chat.last_seen_at = None;
                chat.harness_session_id = None;
                chat.harness_session_cwd = None;
                chat.room_gen = Some(2);
                let admission = self.workspace.admit_fork(&chat, &source.id).map_err(failed)?;
                // Missing rows open on chat2. Persist history before publishing
                // the registry row so a crash cannot leave a discoverable empty fork.
                let target = self.doc_host.open(&chat.id).map_err(failed)?;
                let existing = target
                    .doc()
                    .read_entries()
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                for entry in &entries[..=boundary] {
                    let mut entry = entry.clone();
                    // Historical approvals belong to the source runtime; they
                    // must never block or send answers from the new composer.
                    for part in &mut entry.parts {
                        if let rcode_doc::MessagePart::Input { resolved, .. } = part {
                            *resolved = true;
                        }
                    }
                    if !existing.iter().any(|e| e.id == entry.id) {
                        target.doc().push_message(&entry)
                            .map_err(|e| RpcError::Failed(e.to_string()))?;
                    }
                    let invocation = entry.parts.iter().filter_map(|part| match part {
                        rcode_doc::MessagePart::Text { text, .. } => Some(text.as_str()), _ => None,
                    }).collect::<Vec<_>>().join("\n");
                    if let Some(prompt) = source_doc.doc().command_prompt(&entry.id, &invocation) {
                        target.doc().remember_command_prompt(&entry.id, &invocation, &prompt)
                            .map_err(|e| RpcError::Failed(e.to_string()))?;
                    }
                }
                // The seam: everything above came from the source. Its own
                // entry (system role, complete) so the copied history and the
                // fork's first turn never share a row.
                let marker_id = format!("fork:{}", chat.id);
                if !existing.iter().any(|e| e.id == marker_id) {
                    let source_title = source
                        .title
                        .clone()
                        .or_else(|| source.last_message_preview.clone())
                        .unwrap_or_else(|| "New session".into());
                    target
                        .doc()
                        .push_message(&rcode_doc::SessionMessageEntry {
                            duration_ms: None,
                            id: marker_id.clone(),
                            role: rcode_doc::MessageRole::System,
                            parts: vec![rcode_doc::MessagePart::Fork {
                                id: marker_id,
                                source_chat_id: source.id.clone(),
                                source_title,
                            }],
                            created_at: chrono::Utc::now().timestamp_millis(),
                            device_id: self.doc_host.device_id().to_owned(),
                            status: Some(rcode_doc::MessageStatus::Complete),
                            continuation_of: None,
                            excluded: false,
                        })
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                }
                self.doc_host.persist_fork(&target).map_err(failed)?;
                admission.publish(&chat).map_err(failed)?;
                RpcReply::value(&chat)
            }
            methods::FOCUS_CHAT => {
                let p: ChatParams = parse_params(params)?;
                self.doc_host
                    .focus_chat(&p.chat_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({}))
            }
            "WatchSessionReplica" => {
                let p: ChatParams = parse_params(params)?;
                let handle = self.doc_host.open(&p.chat_id).map_err(|error| RpcError::Failed(error.to_string()))?;
                Ok(RpcReply::Stream(crate::session_replica::stream(handle)))
            }
            methods::WATCH_SESSION_STATE => {
                let p: ChatParams = parse_params(params)?;
                let handle = self.doc_host.open(&p.chat_id).map_err(|error| RpcError::Failed(error.to_string()))?;
                Ok(RpcReply::Stream(crate::session_state::stream(handle)))
            }
            methods::WATCH_DOC_MESSAGES => {
                // Opt-in: older viewports retain the full-reset contract.
                let opening_tail = params
                    .get("openingTail")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let p: ChatParams = parse_params(params.clone())?;
                // An open transcript keeps its chat's checkout diff live (and
                // discardable) even when the chat is archived; the guard lives
                // exactly as long as the stream.
                let interest = self.diff_sync.retain_chat(&p.chat_id);
                let stream = if params.get("windowed").and_then(|value|value.as_bool()).unwrap_or(false) {
                    let start = params.get("windowStart").filter(|value|!value.is_null()).cloned()
                        .map(serde_json::from_value::<rcode_doc::TranscriptCursor>).transpose()
                        .map_err(|error|RpcError::Failed(error.to_string()))?;
                    let older=params.get("older").and_then(|value|value.as_bool()).unwrap_or(false);
                    crate::transcript_window::watch(self.doc_host.clone(),p.chat_id,start,older).await?
                } else if opening_tail {
                    opening_doc_messages_stream(self.doc_host.clone(), p.chat_id).await?
                } else {
                    let handle = self
                        .doc_host
                        .open(&p.chat_id)
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                    doc_messages_stream(handle.watch_messages(), handle.doc_arc())
                };
                Ok(RpcReply::Stream(
                    stream
                        .map(move |frame| {
                            let _held = &interest;
                            frame
                        })
                        .boxed(),
                ))
            }
            methods::WATCH_QUEUE => {
                let p: ChatParams = parse_params(params)?;
                let handle = self
                    .doc_host
                    .open(&p.chat_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                let rx = handle.watch_queue();
                Ok(RpcReply::Stream(
                    futures::stream::unfold((rx, true), |(mut rx, first)| async move {
                        if !first {
                            rx.changed().await.ok()?;
                        }
                        let items = rx.borrow_and_update().clone();
                        let value = serde_json::json!({ "items": items });
                        Some((value, (rx, false)))
                    })
                    .boxed(),
                ))
            }
            methods::QUEUE_MESSAGE => {
                let p: QueueMessageParams = parse_params(params)?;
                let id = if let Some(id) = p.id {
                    self.doc_host.queue_message_with_identity(
                        &p.chat_id, &id, p.issued_by.as_deref(), &p.text,
                        p.attachments, p.hold_for_turn_end,
                    ).await
                } else {
                    self.doc_host.queue_message_with_behavior(
                        &p.chat_id,
                        &p.text,
                        p.attachments,
                        p.hold_for_turn_end,
                    )
                }.map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "id": id }))
            }
            methods::UPDATE_QUEUED_MESSAGE => {
                let p: QueuedMessageParams = parse_params(params)?;
                let changed = self
                    .doc_host
                    .update_queued_message(&p.chat_id, &p.id, &p.text)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "changed": changed }))
            }
            methods::BEGIN_QUEUED_MESSAGE_EDIT => {
                let p: BeginQueuedMessageEditParams = parse_params(params)?;
                let outcome = self
                    .doc_host
                    .begin_queued_message_edit(
                        &p.chat_id,
                        &p.id,
                        &p.editor_device_id,
                        &p.editor_instance_id,
                    )
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(
                    &serde_json::to_value(outcome).map_err(|e| RpcError::Failed(e.to_string()))?,
                )
            }
            methods::RENEW_QUEUED_MESSAGE_EDIT => {
                let p: RenewQueuedMessageEditParams = parse_params(params)?;
                let outcome = self
                    .doc_host
                    .renew_queued_message_edit(&p.chat_id, &p.id, &p.lease_id)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(
                    &serde_json::to_value(outcome).map_err(|e| RpcError::Failed(e.to_string()))?,
                )
            }
            methods::FINISH_QUEUED_MESSAGE_EDIT => {
                let p: FinishQueuedMessageEditParams = parse_params(params)?;
                let action = match p.action {
                    FinishQueuedMessageEditAction::Commit => {
                        crate::doc_host::FinishQueueEditAction::Commit
                    }
                    FinishQueuedMessageEditAction::Cancel => {
                        crate::doc_host::FinishQueueEditAction::Cancel
                    }
                    FinishQueuedMessageEditAction::Discard => {
                        crate::doc_host::FinishQueueEditAction::Discard
                    }
                    FinishQueuedMessageEditAction::ReleaseUnchanged => {
                        crate::doc_host::FinishQueueEditAction::ReleaseUnchanged
                    }
                };
                let outcome = self
                    .doc_host
                    .finish_queued_message_edit_with_attachments(
                        &p.chat_id,
                        &p.id,
                        &p.lease_id,
                        action,
                        p.text.as_deref(),
                        p.expected_text_hash.as_deref(),
                        p.attachments.as_deref(),
                    )
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(
                    &serde_json::to_value(outcome).map_err(|e| RpcError::Failed(e.to_string()))?,
                )
            }
            methods::MOVE_QUEUED_MESSAGE => {
                let p: QueuedMessageParams = parse_params(params)?;
                let changed = self
                    .doc_host
                    .move_queued_message(&p.chat_id, &p.id, p.to_index)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "changed": changed }))
            }
            methods::REMOVE_QUEUED_MESSAGE => {
                let p: QueuedMessageParams = parse_params(params)?;
                let removed = self
                    .doc_host
                    .remove_queued_message(&p.chat_id, &p.id)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "removed": removed }))
            }
            methods::SEND_QUEUED_MESSAGE_NOW => {
                let p: QueuedMessageParams = parse_params(params)?;
                let sent = self
                    .doc_host
                    .send_queued_now(&p.chat_id, &p.id)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "sent": sent }))
            }
            methods::STEER_QUEUED_MESSAGE_NOW => {
                let p: QueuedMessageParams = parse_params(params)?;
                let sent = self
                    .doc_host
                    .steer_queued_now(&p.chat_id, &p.id)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "sent": sent }))
            }
            methods::PROBE_SYNC => {
                // Focus probes remain cheap. An explicit Retry may also allow
                // one fresh, shared auth attempt before its cooldown expires.
                if params.get("retry").and_then(serde_json::Value::as_bool) == Some(true)
                    && let Some(auth) = &self.auth
                {
                    auth.retry_refresh();
                }
                self.workspace.probe();
                self.doc_host.probe_open_chats();
                self.doc_host.probe_edge_reachability();
                RpcReply::value(&serde_json::json!({}))
            }
            methods::SYNC_STATUS => {
                fn room_json(s: &rcode_sync::RoomStatsSnapshot) -> serde_json::Value {
                    serde_json::json!({
                        "connected": s.connected,
                        "synced": s.synced,
                        "lastPushedMs": s.last_pushed_ms,
                        "lastAckMs": s.last_ack_ms,
                        "rejoins": s.rejoins,
                        "probes": s.probes,
                        "fullResyncs": s.full_resyncs,
                        "disconnects": s.disconnects,
                        "rejected": s.rejected,
                    })
                }
                fn chat2_json(s: &rcode_sync::ChatStatsSnapshot) -> serde_json::Value {
                    serde_json::json!({
                        "connected": s.connected,
                        "cursor": s.cursor,
                        "headSeq": s.head_seq,
                        "seqFloor": s.seq_floor,
                        "checkpointSeq": s.checkpoint_seq,
                        "checkpointSize": s.checkpoint_size,
                        "rowCount": s.row_count,
                        "rowBytes": s.row_bytes,
                        "pendingPushes": s.pending_pushes,
                        "rejoins": s.rejoins,
                        "disconnects": s.disconnects,
                        "rejected": s.rejected,
                        "serverResets": s.server_resets,
                    })
                }
                let workspace = self.workspace.sync_status();
                let chats: Vec<serde_json::Value> = self
                    .doc_host
                    .sync_statuses()
                    .iter()
                    .map(|(chat_id, room)| {
                        serde_json::json!({
                            "chatId": chat_id,
                            "room": room.as_ref().map(chat2_json),
                            "state": self.doc_host.chat_sync_state(chat_id),
                        })
                    })
                    .collect();
                RpcReply::value(&serde_json::json!({
                    "deviceId": self.doc_host.device_id(),
                    "nowMs": crate::now_ms(),
                    "workspace": workspace.as_ref().map(room_json),
                    "chats": chats,
                    "resources": self.doc_host.sync_resources(),
                }))
            }
            methods::WATCH_CONNECTIVITY => Ok(RpcReply::Stream(watch_stream(
                self.doc_host.watch_connectivity(),
            ))),
            methods::WATCH_TRANSFERS => Ok(RpcReply::Stream(watch_stream(
                self.doc_host.watch_transfers(),
            ))),
            methods::WATCH_PREVIEWS => {
                let p: rcode_proto::WatchPreviewsParams = parse_params(params)?;
                if self
                    .workspace
                    .chat(&p.chat_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .is_none()
                {
                    return Err(RpcError::Failed("Project session not found".into()));
                }
                let previews = self
                    .previews
                    .as_ref()
                    .ok_or_else(|| RpcError::Failed("Preview discovery unavailable".into()))?;
                let catalog = previews.catalog().clone();
                let changes = catalog.subscribe();
                let chats = self.workspace.watch_chats();
                let workspace = self.workspace.clone();
                // Previews are local-only: the snapshot filters to services
                // this device hosts in the chat's checkout.
                let stream = futures::stream::unfold(
                    (changes, chats, true, workspace, catalog, p.chat_id),
                    |(mut changes, mut chats, first, workspace, catalog, chat_id)| async move {
                        if !first {
                            tokio::select! {
                                result = changes.changed() => { if result.is_err() { return None; } }
                                result = chats.changed() => { if result.is_err() { return None; } }
                            }
                        }
                        let mut snapshot = changes.borrow_and_update().clone();
                        chats.borrow_and_update();
                        let chat = workspace.chat(&chat_id).ok().flatten();
                        let cwd = chat.and_then(|c| c.cwd).map(|cwd| {
                            std::path::PathBuf::from(&cwd)
                                .canonicalize()
                                .unwrap_or_else(|_| cwd.into())
                        });
                        snapshot.project_name = cwd
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|s| s.to_string_lossy().into_owned());
                        // Only services this device hosts are routable locally.
                        snapshot.services.retain(|service| {
                            service.device_id == catalog.device_id()
                                && cwd.as_ref().is_some_and(|cwd| {
                                    cwd == std::path::Path::new(&service.project_cwd)
                                })
                        });
                        let value = serde_json::to_value(snapshot).ok()?;
                        Some((value, (changes, chats, false, workspace, catalog, chat_id)))
                    },
                );
                Ok(RpcReply::Stream(Box::pin(stream)))
            }
            methods::WATCH_CHATS => {
                Ok(RpcReply::Stream(self.catalog_stream(methods::WATCH_CHATS, self.workspace.watch_chats())))
            }
            methods::WATCH_SIDEBAR_PREFERENCES => Ok(RpcReply::Stream(watch_stream(
                self.workspace.watch_sidebar_preferences(),
            ))),
            methods::WATCH_DEVICES => Ok(RpcReply::Stream(self.catalog_stream(
                methods::WATCH_DEVICES, self.workspace.watch_devices(),
            ))),
            methods::WATCH_SPACES => Ok(RpcReply::Stream(self.catalog_stream(
                methods::WATCH_SPACES, self.workspace.watch_spaces(),
            ))),
            methods::WATCH_SESSIONS => {
                // Local live statuses merged with remote devices' workspace rows.
                let merged = self
                    .workspace
                    .merged_sessions_watch(self.sessions.watch_sessions());
                Ok(RpcReply::Stream(self.catalog_stream(methods::WATCH_SESSIONS, merged)))
            }
            methods::LOCAL_DEVICE => {
                RpcReply::value(&serde_json::json!({ "deviceId": self.doc_host.device_id() }))
            }
            methods::UPDATE_STATUS => Ok(RpcReply::Stream(watch_stream(self.updater()?.watch()))),
            methods::APPLY_UPDATE => {
                let version = self
                    .updater()?
                    .apply()
                    .await
                    .map_err(|e| RpcError::Failed(format!("{e:#}")))?;
                RpcReply::value(&serde_json::json!({ "ok": true, "version": version }))
            }
            methods::MUTATE => {
                let p: MutateParams = parse_params(params)?;
                let sidebar_pins = matches!(&p, MutateParams::ChangeSidebarPin { .. });
                self.mutate(p).await?;
                if sidebar_pins {
                    return RpcReply::value(&serde_json::json!({
                        "ok": true, "sidebarPreferences": self.workspace.sidebar_preferences_snapshot(),
                    }));
                }
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::WATCH_CHECKOUT_DIFFS => {
                Ok(RpcReply::Stream(watch_stream(self.diff_sync.watch_diffs())))
            }
            methods::WATCH_WORKSPACE_GIT_STATUS => {
                let request: rcode_proto::WatchWorkspaceFilesRequest = parse_params(params)?;
                let workspace = self.workspace_files.resolve_target(&request.target).await?;
                let rx = self.diff_sync.watch_git_statuses();
                // Only this authorized checkout crosses the connection. None means
                // unavailable, including plain folders and initial/restarting engines.
                let stream = futures::stream::unfold(
                    (rx, workspace.checkout_id, None, false),
                    |(mut rx, checkout_id, mut previous, mut emitted)| async move {
                        loop {
                            if emitted {
                                rx.changed().await.ok()?;
                            }
                            let next = rx
                                .borrow_and_update()
                                .iter()
                                .find(|s| s.checkout_id == checkout_id)
                                .cloned();
                            if !emitted || previous != next {
                                emitted = true;
                                previous = next.clone();
                                let value =
                                    serde_json::to_value(rcode_proto::WorkspaceGitStatusFrame {
                                        status: next,
                                    })
                                    .ok()?;
                                return Some((value, (rx, checkout_id, previous, emitted)));
                            }
                        }
                    },
                );
                Ok(RpcReply::Stream(stream.boxed()))
            }
            methods::WATCH_CHECKOUT_CHANGE_REQUEST => {
                let p: CheckoutChangeRequestParams = parse_params(params)?;
                let cwd = self.change_request_root(&p.cwd).await?;
                let stream = self
                    .change_requests
                    .watch_for_branch(&cwd, p.branch.as_deref())
                    .await
                    .map_err(|error| RpcError::Failed(error.to_string()))?
                    .filter_map(|status| async move { serde_json::to_value(status).ok() });
                Ok(RpcReply::Stream(stream.boxed()))
            }
            methods::GET_CHANGE_REQUEST_REPOSITORY => {
                #[derive(Deserialize)]
                struct P {
                    cwd: String,
                    #[serde(default)]
                    repository: Option<String>,
                }
                let p: P = parse_params(params)?;
                // Only a chat or project checkout on this device: a caller
                // never probes arbitrary directories for their remotes.
                let cwd = self.change_request_root(&p.cwd).await?;
                let cwd = cwd.as_path();
                let resolver = crate::source_control::ChangeRequestResolver::new();
                let repository = match p.repository {
                    Some(repository) => {
                        resolver
                            .matching_repository_for_checkout(cwd, &repository, &self.github)
                            .await
                    }
                    None => resolver.repository_for_checkout(cwd).await,
                };
                RpcReply::value(&repository)
            }
            methods::LIST_CHANGE_REQUEST_PAGE => {
                #[derive(Deserialize)]
                struct P {
                    repository: String,
                    #[serde(default)]
                    filter: rcode_proto::ChangeRequestFilter,
                    #[serde(default)]
                    after: Option<String>,
                    #[serde(default)]
                    refresh: bool,
                    /// Serve the first page's on-disk copy when there is one.
                    #[serde(default)]
                    cached: bool,
                }
                let p: P = parse_params(params)?;
                if !crate::source_control::valid_pr_repository(&p.repository) {
                    return Err(RpcError::BadParams("repository must be owner/repo".into()));
                }
                if p.after
                    .as_deref()
                    .is_some_and(|cursor| !crate::source_control::valid_page_cursor(cursor))
                {
                    return Err(RpcError::BadParams("invalid page cursor".into()));
                }
                let page = self
                    .github
                    .list_page(
                        &p.repository,
                        p.filter,
                        p.after.as_deref(),
                        p.refresh,
                        p.cached,
                    )
                    .await
                    .map_err(change_request_rpc_error)?;
                RpcReply::value(&page)
            }
            methods::POST_CHANGE_REQUEST_COMMENT => {
                #[derive(Deserialize)]
                struct P {
                    url: String,
                    body: String,
                }
                let p: P = parse_params(params)?;
                if p.body.trim().is_empty() || p.body.len() > 60_000 {
                    return Err(RpcError::BadParams(
                        "Comment must contain 1–60000 bytes".into(),
                    ));
                }
                let comment = self
                    .github
                    .post_comment(&p.url, &p.body)
                    .await
                    .map_err(change_request_rpc_error)?;
                RpcReply::value(&comment)
            }
            methods::GET_CHANGE_REQUEST | methods::GET_CHANGE_REQUEST_DIFF => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    url: String,
                    #[serde(default)]
                    refresh: bool,
                    #[serde(default)]
                    head_ref_oid: Option<String>,
                    #[serde(default)]
                    base_ref_oid: Option<String>,
                }
                let p: P = parse_params(params)?;
                let value = self
                    .github
                    .detail_request(
                        &p.url,
                        method == methods::GET_CHANGE_REQUEST_DIFF,
                        p.refresh,
                        p.head_ref_oid.as_deref(),
                        p.base_ref_oid.as_deref(),
                    )
                    .await
                    .map_err(change_request_rpc_error)?;
                RpcReply::value(&value)
            }
            // One-shot scoped capture for the Changes pane: `branch` diffs the
            // working tree against merge-base(baseRef, HEAD); `turn` diffs the
            // turn-start tree snapshot against the current tree; anything else
            // is the plain working-tree capture.
            methods::GET_CHECKOUT_DIFF => {
                // Keep the scoped-diff future off the dispatcher's stack. The
                // per-commit path adds another nested git-capture future.
                Box::pin(async move {
                    #[derive(Deserialize)]
                    #[serde(rename_all = "camelCase")]
                    struct P {
                        cwd: String,
                        #[serde(default)]
                        mode: String,
                        base_ref: Option<String>,
                        chat_id: Option<String>,
                        commit_sha: Option<String>,
                    }
                    let p: P = parse_params(params)?;
                    let identity = self
                        .repos
                        .checkout_identity(std::path::Path::new(&p.cwd))
                        .await
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                    let root = identity.root.as_path();
                    let snapshot = match p.mode.as_str() {
                        "branch" => {
                            let base_ref = p
                                .base_ref
                                .as_deref()
                                .ok_or_else(|| RpcError::Failed("baseRef required".into()))?;
                            let base = crate::diff_sync::merge_base(root, base_ref)
                                .await
                                .map_err(|e| RpcError::Failed(e.to_string()))?;
                            crate::diff_sync::capture_diff_against(&self.repos, root, Some(&base))
                                .await
                        }
                        // One commit's own changes (History → per-commit tab):
                        // parent (or the empty tree) vs the commit itself.
                        "commit" => {
                            let sha = p
                                .commit_sha
                                .as_deref()
                                .ok_or_else(|| RpcError::Failed("commitSha required".into()))?;
                            crate::diff_sync::capture_commit_diff(&self.repos, root, sha).await
                        }
                        "turn" => {
                            let chat_id = p
                                .chat_id
                                .as_deref()
                                .ok_or_else(|| RpcError::Failed("chatId required".into()))?;
                            let snapshot = self
                                .diff_sync
                                .turn_snapshot(chat_id)
                                .filter(|s| s.root == identity.root)
                                .ok_or_else(|| RpcError::Failed("no turn recorded".into()))?;
                            crate::diff_sync::capture_turn_diff(&self.repos, root, &snapshot.tree)
                                .await
                        }
                        _ => crate::diff_sync::capture_diff(&self.repos, root).await,
                    }
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                    RpcReply::value(&rcode_proto::CheckoutDiff {
                        checkout_id: identity.id,
                        device_id: self.doc_host.device_id().to_string(),
                        cwd: identity.root.to_string_lossy().to_string(),
                        patch: snapshot.patch,
                        files: snapshot.files,
                        additions: snapshot.additions,
                        deletions: snapshot.deletions,
                        truncated: snapshot.truncated,
                        checksum: snapshot.checksum,
                        updated_at: chrono::Utc::now(),
                    })
                })
                .await
            }
            methods::DISCARD_WORKING_TREE => {
                // This destructive branch performs several nested filesystem
                // futures. Box it so unrelated RPC calls do not inherit that
                // state in the already-large dispatcher stack frame.
                Box::pin(async move {
                    let p: DiscardWorkingTreeParams = parse_params(params)?;
                    let chat = self
                        .workspace
                        .chat(&p.chat_id)
                        .map_err(|e| RpcError::Failed(e.to_string()))?
                        .ok_or_else(|| RpcError::Failed("chat not found".into()))?;
                    if chat.device_id != self.doc_host.device_id() {
                        return Err(RpcError::Failed("chat is not hosted by this device".into()));
                    }
                    let cwd = chat
                        .cwd
                        .as_deref()
                        .ok_or_else(|| RpcError::Failed("chat has no checkout".into()))?;
                    let identity = self
                        .repos
                        .checkout_identity(std::path::Path::new(cwd))
                        .await
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                    if identity.id != p.checkout_id {
                        return Err(RpcError::Failed(
                            "chat checkout changed since the confirmation was opened".into(),
                        ));
                    }

                    // Refuse the mutation when any local chat on this exact
                    // checkout has a live run. We never interrupt an agent as a
                    // side effect of discarding files.
                    let chats = self.workspace.watch_chats().borrow().clone();
                    for candidate in chats {
                        if candidate.device_id != self.doc_host.device_id() {
                            continue;
                        }
                        let same_checkout =
                            if candidate.checkout_id.as_deref() == Some(identity.id.as_str()) {
                                true
                            } else if let Some(candidate_cwd) = candidate.cwd.as_deref() {
                                self.repos
                                    .checkout_identity(std::path::Path::new(candidate_cwd))
                                    .await
                                    .is_ok_and(|candidate_identity| {
                                        candidate_identity.id == identity.id
                                    })
                            } else {
                                false
                            };
                        if same_checkout
                            && self
                                .sessions
                                .session_status(&candidate.id)
                                .is_some_and(|session| {
                                    matches!(
                                        session.status,
                                        rcode_proto::SessionStatus::Working
                                            | rcode_proto::SessionStatus::AwaitingInput
                                    )
                                })
                        {
                            return Err(RpcError::Failed(
                                "an agent is active in this working tree".into(),
                            ));
                        }
                    }

                    let snapshot = self
                        .diff_sync
                        .discard_working_tree(&identity, &p.expected_checksum)
                        .await
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                    RpcReply::value(&serde_json::json!({
                        "ok": true,
                        "checksum": snapshot.checksum,
                    }))
                })
                .await
            }
            methods::GET_CHECKOUT_FILE_DIFF_TEXT => {
                // This branch contains several large nested async futures. Keep it
                // behind an allocation so every unrelated RPC does not carry that
                // state in `EngineRpc::handle`'s stack frame.
                Box::pin(async move {
                    let p: rcode_proto::GetCheckoutFileDiffTextRequest = parse_params(params)?;
                    let identity =
                        Box::pin(self.repos.checkout_identity(std::path::Path::new(&p.cwd)))
                            .await
                            .map_err(|error| RpcError::Failed(error.to_string()))?;
                    if identity.id != p.checkout_id {
                        return Err(RpcError::Failed("checkoutId does not match cwd".into()));
                    }
                    let root = identity.root.as_path();
                    // A turn's base tree lives in the snapshot's own object
                    // store, so the snapshot rides along with its sha.
                    let mut turn_snapshot = None;
                    let (snapshot, base, target) = match p.mode.as_str() {
                        "branch" => {
                            let base_ref = p
                                .base_ref
                                .as_deref()
                                .ok_or_else(|| RpcError::Failed("baseRef required".into()))?;
                            let base = Box::pin(crate::diff_sync::merge_base(root, base_ref))
                                .await
                                .map_err(|error| RpcError::Failed(error.to_string()))?;
                            let snapshot = Box::pin(crate::diff_sync::capture_diff_against(
                                &self.repos,
                                root,
                                Some(&base),
                            ))
                            .await
                            .map_err(|error| RpcError::Failed(error.to_string()))?;
                            (snapshot, base, None)
                        }
                        "commit" => {
                            let sha = p
                                .commit_sha
                                .as_deref()
                                .ok_or_else(|| RpcError::Failed("commitSha required".into()))?;
                            let base =
                                Box::pin(crate::diff_sync::commit_diff_base(root, sha)).await;
                            let snapshot = Box::pin(crate::diff_sync::capture_commit_diff(
                                &self.repos,
                                root,
                                sha,
                            ))
                            .await
                            .map_err(|error| RpcError::Failed(error.to_string()))?;
                            (snapshot, base, Some(sha.to_string()))
                        }
                        "turn" => {
                            let chat_id = p
                                .chat_id
                                .as_deref()
                                .ok_or_else(|| RpcError::Failed("chatId required".into()))?;
                            let turn = self
                                .diff_sync
                                .turn_snapshot(chat_id)
                                .filter(|snapshot| snapshot.root == identity.root)
                                .ok_or_else(|| RpcError::Failed("no turn recorded".into()))?;
                            let snapshot = Box::pin(crate::diff_sync::capture_turn_diff(
                                &self.repos,
                                root,
                                &turn.tree,
                            ))
                            .await
                            .map_err(|error| RpcError::Failed(error.to_string()))?;
                            let base = turn.tree.tree().to_string();
                            turn_snapshot = Some(turn.tree);
                            (snapshot, base, None)
                        }
                        _ => {
                            let base = Box::pin(crate::diff_sync::working_diff_base(root))
                                .await
                                .map_err(|error| RpcError::Failed(error.to_string()))?;
                            let snapshot =
                                Box::pin(crate::diff_sync::capture_diff(&self.repos, root))
                                    .await
                                    .map_err(|error| RpcError::Failed(error.to_string()))?;
                            (snapshot, base, None)
                        }
                    };
                    let stale = || rcode_proto::CheckoutFileDiffText {
                        diff_checksum: p.diff_checksum.clone(),
                        old_text: None,
                        new_text: None,
                        old_content_hash: None,
                        new_content_hash: None,
                        binary: false,
                        truncated: false,
                        stale: true,
                    };
                    if snapshot.checksum != p.diff_checksum {
                        return RpcReply::value(&stale());
                    }
                    let file = snapshot
                        .files
                        .iter()
                        .find(|file| file.path == p.path)
                        .ok_or_else(|| {
                            RpcError::Failed("path is not part of diff snapshot".into())
                        })?;
                    let pair = Box::pin(crate::diff_sync::read_diff_file_text_at(
                        root,
                        &base,
                        turn_snapshot.as_ref(),
                        target.as_deref(),
                        file,
                    ))
                    .await
                    .map_err(|error| RpcError::Failed(error.to_string()))?;
                    let current = match p.mode.as_str() {
                        "branch" => {
                            Box::pin(crate::diff_sync::capture_diff_against(
                                &self.repos,
                                root,
                                Some(&base),
                            ))
                            .await
                        }
                        "turn" => match turn_snapshot.as_ref() {
                            Some(turn) => {
                                Box::pin(crate::diff_sync::capture_turn_diff(
                                    &self.repos,
                                    root,
                                    turn,
                                ))
                                .await
                            }
                            None => Err(crate::EngineError::Other("no turn recorded".into())),
                        },
                        "commit" => {
                            let sha = p
                                .commit_sha
                                .as_deref()
                                .ok_or_else(|| RpcError::Failed("commitSha required".into()))?;
                            Box::pin(crate::diff_sync::capture_commit_diff(
                                &self.repos,
                                root,
                                sha,
                            ))
                            .await
                        }
                        _ => Box::pin(crate::diff_sync::capture_diff(&self.repos, root)).await,
                    }
                    .map_err(|error| RpcError::Failed(error.to_string()))?;
                    if current.checksum != p.diff_checksum {
                        return RpcReply::value(&stale());
                    }
                    RpcReply::value(&rcode_proto::CheckoutFileDiffText {
                        diff_checksum: p.diff_checksum,
                        old_text: pair.old_text,
                        new_text: pair.new_text,
                        old_content_hash: pair.old_content_hash,
                        new_content_hash: pair.new_content_hash,
                        binary: pair.binary,
                        truncated: pair.truncated,
                        stale: false,
                    })
                })
                .await
            }
            methods::LIST_REPOS => RpcReply::value(&self.repos.list().await),
            methods::ADD_REPO => {
                #[derive(Deserialize)]
                struct P {
                    path: String,
                }
                let p: P = parse_params(params)?;
                let repo = self
                    .repos
                    .add(&p.path)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&repo)
            }
            methods::CLONE_REPO => {
                #[derive(Deserialize)]
                struct P {
                    url: String,
                }
                let p: P = parse_params(params)?;
                let repo = self
                    .repos
                    .clone_repo(&p.url)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&repo)
            }
            methods::CREATE_REPO => {
                #[derive(Deserialize)]
                struct P {
                    name: String,
                }
                let p: P = parse_params(params)?;
                let repo = self
                    .repos
                    .create(&p.name)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&repo)
            }
            methods::LIST_BRANCHES => {
                let p: RepoPathParams = parse_params(params)?;
                let branches = self
                    .repos
                    .branches(std::path::Path::new(&p.repo_path))
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&branches)
            }
            methods::LIST_REFS => {
                let p: RepoPathParams = parse_params(params)?;
                let refs = self
                    .repos
                    .refs(std::path::Path::new(&p.repo_path))
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&refs)
            }
            methods::LIST_GIT_HISTORY => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    cwd: String,
                    #[serde(default)]
                    cursor: usize,
                    #[serde(default = "default_git_history_limit")]
                    limit: usize,
                }
                fn default_git_history_limit() -> usize {
                    crate::repos::GIT_HISTORY_DEFAULT_LIMIT
                }
                let p: P = parse_params(params)?;
                let history = self
                    .repos
                    .history(std::path::Path::new(&p.cwd), p.cursor, p.limit)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&history)
            }
            methods::SEARCH_GIT_HISTORY => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    cwd: String,
                    query: String,
                    #[serde(default)]
                    cursor: usize,
                    #[serde(default = "default_git_history_search_limit")]
                    limit: usize,
                }
                fn default_git_history_search_limit() -> usize {
                    crate::repos::GIT_HISTORY_DEFAULT_LIMIT
                }
                let p: P = parse_params(params)?;
                let history = self
                    .repos
                    .search_history(std::path::Path::new(&p.cwd), &p.query, p.cursor, p.limit)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&history)
            }
            methods::RESOLVE_GIT_AVATARS => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct P {
                    cwd: String,
                    authors: Vec<GitAvatarAuthor>,
                    #[serde(default)]
                    cursor: usize,
                    #[serde(default = "default_git_avatar_limit")]
                    limit: usize,
                }
                #[derive(Deserialize)]
                struct GitAvatarAuthor {
                    sha: String,
                    email: String,
                }
                fn default_git_avatar_limit() -> usize {
                    crate::repos::GIT_HISTORY_DEFAULT_LIMIT
                }
                let p: P = parse_params(params)?;
                let authors: Vec<_> = p
                    .authors
                    .into_iter()
                    .take(crate::repos::GIT_HISTORY_MAX_LIMIT)
                    .filter(|author| author.sha.len() <= 64 && author.email.len() <= 512)
                    .map(|author| (author.sha, author.email))
                    .collect();
                let avatar_paths = self
                    .repos
                    .history_avatar_urls(std::path::Path::new(&p.cwd), &authors, p.cursor, p.limit)
                    .await;
                let mut avatars = std::collections::HashMap::new();
                for (email, path) in avatar_paths {
                    if let Ok(bytes) = tokio::fs::read(path).await {
                        avatars.insert(
                            email,
                            base64::engine::general_purpose::STANDARD.encode(bytes),
                        );
                    }
                }
                RpcReply::value(&avatars)
            }
            methods::FETCH_ALL => {
                let p: RepoPathParams = parse_params(params)?;
                self.repos
                    .fetch_all(std::path::Path::new(&p.repo_path))
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                // Remote refs are repository state too. Force the checkout
                // watchers to publish a fresh snapshot instead of waiting for
                // the repair tick (some platforms do not report packed-refs).
                self.diff_sync.sync_all();
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::SWITCH_REF => {
                let p: SwitchRefParams = parse_params(params)?;
                let branch = self
                    .repos
                    .switch_ref(std::path::Path::new(&p.repo_path), &p.ref_name)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "branch": branch }))
            }
            methods::GET_WORKTREE_SETTINGS => RpcReply::value(&self.repos.worktree_settings()),
            methods::SET_WORKTREE_SETTINGS => {
                let settings: rcode_proto::WorktreeSettings = parse_params(params)?;
                let status = self
                    .repos
                    .set_worktree_settings(settings)
                    .await
                    .map_err(|error| RpcError::Failed(error.to_string()))?;
                RpcReply::value(&status)
            }
            methods::LIST_FOLDERS => {
                let p: ListFoldersParams = parse_params(params)?;
                let listing = self
                    .repos
                    .list_folders(p.path)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&listing)
            }
            methods::SEARCH_HOME_FOLDERS => {
                let request: rcode_proto::SearchHomeFoldersRequest = parse_params(params)?;
                if request.query.chars().count() > 256 {
                    return Err(RpcError::BadParams(
                        "SearchHomeFolders query must not exceed 256 characters".into(),
                    ));
                }
                let limit = request
                    .limit
                    .map_or(crate::repos::HOME_FOLDER_RESULTS, usize::from)
                    .min(crate::repos::HOME_FOLDER_MAX_RESULTS);
                let result = self
                    .repos
                    .search_home_folders(request.query, limit)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&result)
            }
            methods::RELEASE_HOME_FOLDER_SEARCH => {
                self.repos.release_home_folder_search();
                RpcReply::value(&serde_json::json!({}))
            }
            methods::LIST_DRIVES => {
                let drives = self
                    .repos
                    .list_drives()
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&rcode_proto::DriveListing { drives })
            }
            methods::SEARCH_FILES => {
                let p: FileSearchParams = parse_params(params)?;
                if p.query.chars().count() > 256 {
                    return Err(RpcError::BadParams(
                        "SearchFiles query must not exceed 256 characters".into(),
                    ));
                }
                let matches = tokio::time::timeout(FILE_SEARCH_RPC_TIMEOUT, async {
                    let root = self.file_search_root(&p).await?;
                    let featured_paths = p
                        .chat_id
                        .as_deref()
                        .filter(|_| p.query.is_empty())
                        .map(|chat_id| self.featured_file_paths(chat_id))
                        .unwrap_or_default();
                    self.repos
                        .search_files(root, p.query, featured_paths)
                        .await
                        .map_err(|e| RpcError::Failed(e.to_string()))
                })
                .await
                .map_err(|_| RpcError::Failed("file search timed out".into()))??;
                RpcReply::value(&matches)
            }
            methods::LIST_PROJECT_MEMORY => {
                let mut target: rcode_proto::WorkspaceTarget = parse_params(params)?;
                target.memory = false;
                let workspace = self
                    .workspace_files
                    .resolve_target(&target)
                    .await
                    .map_err(RpcError::from)?;
                let config_root = self.config_root.clone();
                let listing = tokio::task::spawn_blocking(move || {
                    crate::project_memory::list(&workspace.root, &config_root)
                })
                .await
                .map_err(|error| RpcError::Failed(error.to_string()))?
                .map_err(|error| RpcError::Failed(error.to_string()))?;
                RpcReply::value(&listing)
            }
            methods::LIST_WORKSPACE_DIRECTORY => {
                let request: rcode_proto::ListWorkspaceDirectoryRequest = parse_params(params)?;
                let page = tokio::time::timeout(
                    crate::workspace_files::WORKSPACE_FILE_RPC_TIMEOUT,
                    self.workspace_files.list_directory(request),
                )
                .await
                .map_err(|_| RpcError::Failed("workspace directory listing timed out".into()))?
                .map_err(RpcError::from)?;
                RpcReply::value(&page)
            }
            methods::SEARCH_WORKSPACE_FILES => {
                let request: rcode_proto::SearchWorkspaceFilesRequest = parse_params(params)?;
                let matches = tokio::time::timeout(
                    crate::workspace_files::WORKSPACE_FILE_RPC_TIMEOUT,
                    self.workspace_files.search(request),
                )
                .await
                .map_err(|_| RpcError::Failed("workspace file search timed out".into()))?
                .map_err(RpcError::from)?;
                RpcReply::value(&matches)
            }
            methods::SEARCH_WORKSPACE_CONTENT => {
                let request: rcode_proto::SearchWorkspaceContentRequest = parse_params(params)?;
                let result = tokio::time::timeout(
                    crate::workspace_files::WORKSPACE_FILE_RPC_TIMEOUT,
                    self.workspace_files.search_content(request),
                )
                .await
                .map_err(|_| RpcError::Failed("workspace content search timed out".into()))?
                .map_err(RpcError::from)?;
                RpcReply::value(&result)
            }
            methods::WARM_WORKSPACE_SEARCH => {
                let request: rcode_proto::WarmWorkspaceSearchRequest = parse_params(params)?;
                let result = tokio::time::timeout(
                    crate::workspace_files::WORKSPACE_FILE_RPC_TIMEOUT,
                    self.workspace_files.warm_search(request),
                )
                .await
                .map_err(|_| RpcError::Failed("workspace search warm-up timed out".into()))?
                .map_err(RpcError::from)?;
                RpcReply::value(&result)
            }
            methods::READ_WORKSPACE_IMAGE => {
                let request: rcode_proto::ReadWorkspaceImageRequest = parse_params(params)?;
                let chunk = tokio::time::timeout(
                    crate::workspace_files::WORKSPACE_FILE_RPC_TIMEOUT,
                    self.workspace_files.read_image(request),
                )
                .await
                .map_err(|_| RpcError::Failed("Workspace image read timed out".into()))?
                .map_err(RpcError::from)?;
                RpcReply::value(&chunk)
            }
            methods::READ_WORKSPACE_FILE => {
                let request: rcode_proto::ReadWorkspaceFileRequest = parse_params(params)?;
                let file = tokio::time::timeout(
                    crate::workspace_files::WORKSPACE_FILE_RPC_TIMEOUT,
                    self.workspace_files.read_file(request),
                )
                .await
                .map_err(|_| RpcError::Failed("workspace file read timed out".into()))?
                .map_err(RpcError::from)?;
                RpcReply::value(&file)
            }
            methods::DELETE_WORKSPACE_ENTRY => {
                let request: rcode_proto::DeleteWorkspaceEntryRequest = parse_params(params)?;
                let outcome = self
                    .workspace_files
                    .delete_entry(request)
                    .await
                    .map_err(RpcError::from)?;
                RpcReply::value(&outcome)
            }
            methods::MOVE_WORKSPACE_ENTRY => {
                let request: rcode_proto::MoveWorkspaceEntryRequest = parse_params(params)?;
                let outcome = self
                    .workspace_files
                    .move_entry(request)
                    .await
                    .map_err(RpcError::from)?;
                RpcReply::value(&outcome)
            }
            methods::WRITE_WORKSPACE_FILE => {
                let request: rcode_proto::WriteWorkspaceFileRequest = parse_params(params)?;
                let outcome = tokio::time::timeout(
                    crate::workspace_files::WORKSPACE_FILE_RPC_TIMEOUT,
                    self.workspace_files.write_file(request),
                )
                .await
                .map_err(|_| RpcError::Failed("workspace file write timed out".into()))?
                .map_err(RpcError::from)?;
                RpcReply::value(&outcome)
            }
            methods::WATCH_WORKSPACE_FILES => {
                let request: rcode_proto::WatchWorkspaceFilesRequest = parse_params(params)?;
                let subscription = self
                    .workspace_files
                    .watch_files(request)
                    .await
                    .map_err(RpcError::from)?;
                let stream = futures::stream::unfold(subscription, |mut subscription| async move {
                    let changes = subscription.recv().await?;
                    let value = serde_json::to_value(changes).ok()?;
                    Some((value, subscription))
                });
                Ok(RpcReply::Stream(stream.boxed()))
            }
            methods::CREATE_WORKTREE => {
                let p: CreateWorktreeParams = parse_params(params)?;
                // A worktree request may name the project space it belongs
                // to. The repository must then be that space's own root, so a
                // client cannot claim one project while creating a worktree
                // in another.
                if let Some(space_id) = p.space_id.as_deref() {
                    let space = self.local_project_space(space_id)?;
                    let space_root = std::fs::canonicalize(&space.path)
                        .map_err(|_| RpcError::Failed("Project root is unavailable".into()))?;
                    let repo_root = std::fs::canonicalize(&p.repo_path).map_err(|_| {
                        RpcError::Failed("Worktree repository is unavailable".into())
                    })?;
                    if space_root != repo_root {
                        return Err(RpcError::Failed(
                            "Worktree repository does not match project space".into(),
                        ));
                    }
                }
                let worktree = self
                    .repos
                    .create_worktree(std::path::Path::new(&p.repo_path), &p.branch)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                let outcome = CreateWorktreeOutcome { worktree };
                RpcReply::value(&outcome)
            }
            methods::DELETE_WORKTREE => {
                let p: DeleteWorktreeParams = parse_params(params)?;
                self.repos
                    .delete_worktree(
                        std::path::Path::new(&p.repo_path),
                        std::path::Path::new(&p.worktree_path),
                    )
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::OPEN_TERMINAL => {
                let p: OpenTerminalParams = parse_params(params)?;
                // Prefer an explicit real path (new-chat canvas has no row
                // yet). `space-canvas:{spaceId}` names the selected project
                // so a missing/tilde cwd still lands in that folder.
                let chat_cwd = self
                    .workspace
                    .chat(&p.chat_id)
                    .ok()
                    .flatten()
                    .and_then(|chat| chat.cwd);
                let space_cwd = canvas_space_id(&p.chat_id).and_then(|space_id| {
                    self.workspace
                        .space(space_id)
                        .ok()
                        .flatten()
                        .map(|space| space.path)
                });
                let cwd = resolve_open_terminal_cwd(
                    p.cwd,
                    chat_cwd,
                    space_cwd,
                    &self.repos.scratch_root(),
                )
                .map_err(|error| {
                    tracing::warn!(%error, "terminal working directory resolution failed");
                    RpcError::Failed(rcode_proto::terminal_errors::TerminalOpenFailure::WorkingDirectoryUnavailable.encode())
                })?;
                let session = self
                    .terminals
                    .open(&cwd, p.cols, p.rows)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&session)
            }
            methods::SUBSCRIBE_TERMINAL => {
                let p: SubscribeTerminalParams = parse_params(params)?;
                let rx = self
                    .terminals
                    .subscribe(&p.terminal_id, p.after_seq)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                let stream = futures::stream::unfold(rx, |mut rx| async move {
                    let event = rx.recv().await?;
                    let value = serde_json::to_value(&event).ok()?;
                    Some((value, rx))
                });
                Ok(RpcReply::Stream(stream.boxed()))
            }
            methods::WRITE_TERMINAL => {
                let p: WriteTerminalParams = parse_params(params)?;
                self.terminals
                    .write(&p.terminal_id, &p.data)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::RESIZE_TERMINAL => {
                let p: ResizeTerminalParams = parse_params(params)?;
                self.terminals
                    .resize(&p.terminal_id, p.cols, p.rows)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::CLOSE_TERMINAL => {
                let p: TerminalIdParams = parse_params(params)?;
                self.terminals
                    .close(&p.terminal_id)
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::UPLOAD_CHUNK => {
                let p: UploadChunkParams = parse_params(params)?;
                let uploads = self.uploads.clone();
                tokio::task::spawn_blocking(move || uploads.append(&p.upload_id, &p.data, p.seq))
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "ok": true }))
            }
            methods::UPLOAD_COMMIT => {
                let p: UploadCommitParams = parse_params(params)?;
                let uploads = self.uploads.clone();
                let path = tokio::task::spawn_blocking(move || uploads.commit(&p.upload_id, &p.file_name))
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                // Bytes just landed on this device: any command deferred on
                // them (queued-attachment refs) is executable NOW.
                self.doc_host.kick_drains();
                RpcReply::value(&serde_json::json!({ "path": path }))
            }
            methods::READ_ATTACHMENT_CHUNK => {
                let p: ReadAttachmentChunkParams = parse_params(params)?;
                // Path jail: the uploads dir plus every workspace-known chat cwd.
                let roots: Vec<std::path::PathBuf> = self
                    .workspace
                    .read_chats()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|chat| chat.cwd)
                    .map(std::path::PathBuf::from)
                    .collect();
                let uploads = self.uploads.clone();
                let chunk = tokio::task::spawn_blocking(move || {
                    uploads.read_chunk(&p.path, p.offset, &roots)
                })
                .await
                .map_err(|e| RpcError::Failed(e.to_string()))?
                .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&chunk)
            }
            methods::PUBLISH_VISUAL => {
                let p: PublishVisualParams = parse_params(params)?;
                let title: String = p
                    .title
                    .trim()
                    .chars()
                    .take(rcode_proto::VISUAL_MAX_TITLE_CHARS)
                    .collect();
                if title.is_empty() {
                    return Err(RpcError::BadParams("title is empty".into()));
                }
                if p.script.trim().is_empty() || p.script.len() > rcode_proto::VISUAL_MAX_BYTES {
                    return Err(RpcError::BadParams(format!(
                        "script must be 1 to {} bytes",
                        rcode_proto::VISUAL_MAX_BYTES
                    )));
                }
                // Before decoding and storing anything (checked again below).
                if !self.sessions.has_run(&p.chat_id) {
                    return Err(no_turn_for_visual());
                }
                let id = format!("v:{}", crate::new_id());
                let assets = decode_visual_assets(&p.assets)?;
                // Run the script the way the desktop will (setup, render,
                // every draw, every control) before anyone sees it, so the
                // agent gets its errors with line numbers and can fix them.
                let uploads = self.uploads.clone();
                let (stored_id, script) = (id.clone(), p.script);
                let (path, report) = tokio::task::spawn_blocking(move || {
                    let files = rcode_visual::Assets::new(
                        assets
                            .iter()
                            .map(|(name, bytes)| (name.clone(), bytes.as_slice().into()))
                            .collect(),
                    );
                    let checked = rcode_visual::check_with_assets(
                        &script,
                        rcode_visual::seed_for(&stored_id),
                        files,
                    );
                    let report = checked
                        .map_err(|e| RpcError::BadParams(format!("visual script failed: {e}")))?;
                    // Assets first: a script is only ever found with its files.
                    uploads
                        .store_visual_assets(&stored_id, &assets)
                        .map_err(|e| RpcError::Failed(e.to_string()))?;
                    let path = uploads.store_visual(&stored_id, &script).map_err(|e| {
                        uploads.remove_visual(&stored_id);
                        RpcError::Failed(e.to_string())
                    })?;
                    Ok::<_, RpcError>((path, report))
                })
                .await
                .map_err(|e| RpcError::Failed(e.to_string()))??;
                let event = AgentEvent::Visual {
                    id: id.clone(),
                    path: path.clone(),
                    title: title.clone(),
                };
                if !self.sessions.inject_event(&p.chat_id, event) {
                    self.uploads.remove_visual(&id);
                    return Err(no_turn_for_visual());
                }
                RpcReply::value(&serde_json::json!({
                    "id": id,
                    "title": title,
                    "checkedInteractions": report.interactions,
                    "printed": report.logs,
                }))
            }
            methods::FETCH_TOOL_BLOB => {
                let p: FetchToolBlobParams = parse_params(params)?;
                let text = self
                    .doc_host
                    .fetch_tool_blob(&p.blob_ref)
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?;
                RpcReply::value(&serde_json::json!({ "text": text }))
            }
            other => Err(RpcError::UnknownMethod(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn local_opening_tail_arrives_before_full_mirror_and_keeps_all_history() {
        use crate::doc_host::{DocHost, DocHostConfig};
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(rcode_sync::DocsStore::open(dir.path()).unwrap());
        let host = DocHost::new(
            store,
            DocHostConfig {
                device_id: "viewer".into(),
                default_harness: rcode_proto::HarnessId::Mock,
                edge: None,
            },
        );
        let handle = host.open("whale").unwrap();
        handle
            .doc()
            .push_message(&rcode_doc::SessionMessageEntry {
                id: "turn".into(),
                role: rcode_doc::MessageRole::Assistant,
                parts: (0..500)
                    .map(|i| rcode_doc::MessagePart::Text {
                        id: format!("part-{i}"),
                        text: "local text".into(),
                    })
                    .collect(),
                created_at: 0,
                device_id: "host".into(),
                status: None,
                continuation_of: None,
                duration_ms: None,
                excluded: false,
            })
            .unwrap();
        // Hold publication blocked: the opening must not await the full mirror.
        let held = handle.clone();
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker = std::thread::spawn(move || {
            held.import_transcript(|| {
                locked_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
        });
        locked_rx.recv().unwrap();
        let opening = tokio::time::timeout(
            Duration::from_secs(2),
            opening_doc_messages_stream(host.clone(), "whale".into()),
        )
        .await;
        release_tx.send(()).unwrap();
        blocker.join().unwrap();
        let mut stream = opening
            .expect("first paint must not wait for publication")
            .unwrap();
        let preview = stream.next().await.unwrap();
        assert_eq!(preview["historyPending"], true);
        assert_eq!(preview["reset"][0]["parts"].as_array().unwrap().len(), 128);
        assert_eq!(preview["reset"][0]["parts"][0]["id"], "part-372");
        // Changes between preview and subscribe must appear in the full reset.
        handle
            .write_user_message("arrived", "new local message", 1)
            .unwrap();
        let full = stream.next().await.unwrap();
        assert!(full.get("historyPending").is_none());
        assert_eq!(full["reset"][0]["parts"].as_array().unwrap().len(), 500);
        assert_eq!(full["reset"][1]["id"], "arrived");
        handle
            .write_user_message("live", "after attach", 2)
            .unwrap();
        let live = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap();
        let mut entries = Vec::new();
        for value in [full, live] {
            let update: rcode_doc::TranscriptUpdate = serde_json::from_value(value).unwrap();
            rcode_doc::apply_transcript_frame(&mut entries, update.frame).unwrap();
        }
        assert_eq!(entries.len(), 3);
        assert_eq!(entries.last().unwrap().id, "live");
        host.shutdown_workers().await;
    }

    #[test]
    fn list_models_tolerates_retired_force_flag() {
        let old: ListModelsParams =
            serde_json::from_value(serde_json::json!({"harness":"native"})).unwrap();
        assert_eq!(old.harness, HarnessId::Native);
        let forced: ListModelsParams =
            serde_json::from_value(serde_json::json!({"harness":"native","force":true})).unwrap();
        assert_eq!(forced.harness, HarnessId::Native);
    }

    #[test]
    fn sidebar_preferences_mutation_accepts_desktop_wire_shape() {
        let p: MutateParams = parse_params(serde_json::json!({
            "op": "changeSidebarPin",
            "change": {"action":"move","sessionId":"chat-b","before":"chat-a","after":null},
        }))
        .expect("sidebar preferences params");
        assert!(matches!(
            p,
            MutateParams::ChangeSidebarPin { change: rcode_proto::SidebarPinChange::Move { session_id, before, .. } }
                if session_id == "chat-b" && before.as_deref() == Some("chat-a")
        ));
    }

    #[test]
    fn local_device_is_not_forwardable() {
        assert!(!forwardable(methods::GET_CUSTOM_INSTRUCTIONS));
        assert!(!forwardable(methods::SAVE_CUSTOM_INSTRUCTIONS));
        assert!(!forwardable(methods::LOCAL_DEVICE));
        assert!(!forwardable(methods::FOCUS_CHAT));
        assert!(!forwardable(methods::ENGINE_INFO));
        assert!(!forwardable(methods::ENGINE_READY));
        assert!(forwardable(methods::QUEUE_COMMAND));
        assert!(forwardable(methods::WATCH_SESSION_STATE));
        assert!(is_stream_method(methods::WATCH_SESSION_STATE));
        assert!(forwardable(methods::FETCH_TOOL_BLOB));
        assert!(forwardable(methods::SEARCH_FILES));
        assert!(forwardable(methods::SEARCH_GIT_HISTORY));
        assert!(forwardable(methods::FETCH_ALL));
        assert!(forwardable(methods::RESOLVE_GIT_AVATARS));
        assert!(forwardable(methods::WATCH_CHECKOUT_CHANGE_REQUEST));
        assert!(is_stream_method(methods::WATCH_CHECKOUT_CHANGE_REQUEST));
        assert!(forwardable(methods::DISCARD_WORKING_TREE));
        assert!(forwardable(methods::LIST_WORKSPACE_DIRECTORY));
        assert!(forwardable(methods::LIST_PROJECT_MEMORY));
        assert!(forwardable(methods::SEARCH_WORKSPACE_FILES));
        assert!(forwardable(methods::READ_WORKSPACE_FILE));
        assert!(forwardable(methods::READ_WORKSPACE_IMAGE));
        assert!(forwardable(methods::DELETE_WORKSPACE_ENTRY));
        assert!(forwardable(methods::MOVE_WORKSPACE_ENTRY));
        assert!(forwardable(methods::WRITE_WORKSPACE_FILE));
        assert!(forwardable(methods::WATCH_WORKSPACE_FILES));
        assert!(forwardable(methods::WATCH_WORKSPACE_GIT_STATUS));
        assert!(!is_stream_method(methods::LIST_WORKSPACE_DIRECTORY));
        assert!(!is_stream_method(methods::SEARCH_WORKSPACE_FILES));
        assert!(forwardable(methods::WARM_WORKSPACE_SEARCH));
        assert!(!is_stream_method(methods::WARM_WORKSPACE_SEARCH));
        assert!(forwardable(methods::SEARCH_WORKSPACE_CONTENT));
        assert!(!is_stream_method(methods::SEARCH_WORKSPACE_CONTENT));
        assert!(forwardable(methods::SEARCH_HOME_FOLDERS));
        assert!(forwardable(methods::RELEASE_HOME_FOLDER_SEARCH));
        assert!(!is_stream_method(methods::READ_WORKSPACE_FILE));
        assert!(!is_stream_method(methods::WRITE_WORKSPACE_FILE));
        assert!(is_stream_method(methods::WATCH_WORKSPACE_FILES));
        assert!(is_stream_method(methods::WATCH_WORKSPACE_GIT_STATUS));
        assert!(forwardable(methods::LIST_CHANGE_REQUEST_PAGE));
        assert!(!is_stream_method(methods::LIST_CHANGE_REQUEST_PAGE));
        assert!(forwardable(methods::GET_CHANGE_REQUEST_REPOSITORY));
        assert!(forwardable(methods::GET_CHANGE_REQUEST));
        assert!(forwardable(methods::GET_CHANGE_REQUEST_DIFF));
        assert!(forwardable(methods::POST_CHANGE_REQUEST_COMMENT));
        assert!(!is_stream_method(methods::POST_CHANGE_REQUEST_COMMENT));
        assert!(!is_stream_method(methods::GET_CHANGE_REQUEST));
        assert!(!is_stream_method(methods::GET_CHANGE_REQUEST_DIFF));
    }

    /// Every forwardable unary method gets a bounded reply deadline —
    /// interactive calls fail fast, network-bound git/update calls get the
    /// long leash, and nothing awaits forever (the "Sending…" wedge).
    #[test]
    fn forward_deadlines_are_tiered_and_bounded() {
        for method in [methods::LIST_MODELS, methods::LIST_COMMANDS] {
            assert_eq!(
                forward_deadline(method),
                std::time::Duration::from_secs(100)
            );
        }
        use std::time::Duration;
        assert_eq!(
            forward_deadline(methods::CREATE_WORKTREE),
            Duration::from_secs(120)
        );
        assert_eq!(
            forward_deadline(methods::CLONE_REPO),
            Duration::from_secs(15 * 60)
        );
        assert_eq!(
            forward_deadline(methods::LIST_BRANCHES),
            Duration::from_secs(30)
        );
        assert_eq!(
            forward_deadline(methods::QUEUE_COMMAND),
            Duration::from_secs(30)
        );
        assert_eq!(
            forward_deadline(methods::LIST_CHANGE_REQUEST_PAGE),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn pull_request_errors_are_stable_and_sanitized() {
        for (error, expected) in [
            (
                ChangeRequestError::CliUnavailable,
                capability_errors::PULL_REQUESTS_CLI_UNAVAILABLE,
            ),
            (
                ChangeRequestError::Authentication,
                capability_errors::PULL_REQUESTS_AUTHENTICATION,
            ),
            (
                ChangeRequestError::RateLimited,
                capability_errors::PULL_REQUESTS_RATE_LIMITED,
            ),
            (
                ChangeRequestError::Timeout,
                capability_errors::PULL_REQUESTS_TIMEOUT,
            ),
            (
                ChangeRequestError::Decode,
                capability_errors::PULL_REQUESTS_DECODE,
            ),
            (
                ChangeRequestError::CommandFailed,
                capability_errors::PULL_REQUESTS_COMMAND_FAILED,
            ),
        ] {
            assert!(matches!(
                change_request_rpc_error(error),
                RpcError::Capability(code) if code == expected
            ));
        }
    }

    #[test]
    fn open_terminal_cwd_prefers_explicit_then_chat_then_space_then_home() {
        let home = crate::repos::session_home_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            resolve_open_terminal_cwd(
                Some("/proj".into()),
                Some("/chat".into()),
                Some("/space".into()),
                std::path::Path::new("/scratch")
            )
            .unwrap(),
            "/proj"
        );
        assert_eq!(
            resolve_open_terminal_cwd(
                None,
                Some("/chat".into()),
                Some("/space".into()),
                std::path::Path::new("/scratch")
            )
            .unwrap(),
            "/chat"
        );
        assert_eq!(
            resolve_open_terminal_cwd(
                Some("~".into()),
                None,
                Some("/space".into()),
                std::path::Path::new("/scratch")
            )
            .unwrap(),
            "/space",
            "tilde is a fallback, not an override of the selected project"
        );
        assert_eq!(
            resolve_open_terminal_cwd(
                None,
                None,
                Some("/space".into()),
                std::path::Path::new("/scratch")
            )
            .unwrap(),
            "/space"
        );
        assert_eq!(
            resolve_open_terminal_cwd(None, None, None, std::path::Path::new("/scratch")).unwrap(),
            home
        );
        assert_eq!(
            resolve_open_terminal_cwd(
                Some("~".into()),
                Some("/chat".into()),
                None,
                std::path::Path::new("/scratch")
            )
            .unwrap(),
            "/chat"
        );
        assert_eq!(
            resolve_open_terminal_cwd(
                Some("  ".into()),
                Some("/chat".into()),
                None,
                std::path::Path::new("/scratch")
            )
            .unwrap(),
            "/chat"
        );
        assert_eq!(canvas_space_id("space-canvas:s1"), Some("s1"));
        assert_eq!(canvas_space_id("space-canvas:"), None);
        assert_eq!(canvas_space_id("chat-1"), None);
    }

    #[test]
    fn tool_file_paths_keep_workspace_activity_only() {
        assert_eq!(
            tool_file_path(&ToolCall::EditFile {
                path: "src/main.rs".into(),
                old_string: None,
                new_string: None,
            }),
            Some("src/main.rs")
        );
        assert_eq!(
            tool_file_path(&ToolCall::Exec {
                command: "cargo test".into(),
            }),
            None
        );
    }
}

#[cfg(test)]
mod context_usage_tests {
    use super::*;
    use futures::StreamExt;
    use std::sync::Arc;

    #[tokio::test]
    async fn replay_cutoff_travels_with_coalesced_backfill_and_live_content() {
        use crate::doc_host::{DocHost, DocHostConfig};
        use rcode_sync::chat_client::ChatDocSink;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(rcode_sync::DocsStore::open(dir.path()).unwrap());
        let host = DocHost::new(
            store.clone(),
            DocHostConfig {
                device_id: "viewer".into(),
                default_harness: rcode_proto::HarnessId::Mock,
                edge: None,
            },
        );
        let handle = host.open("replay-chat").unwrap();
        let sink = crate::chat2_host::EngineChatSink::new(&handle.doc_arc(), store, "replay-chat")
            .with_handle(Arc::downgrade(&handle));
        let mut stream = doc_messages_stream(handle.watch_messages(), handle.doc_arc());
        let first: rcode_doc::TranscriptUpdate =
            serde_json::from_value(stream.next().await.unwrap()).unwrap();
        assert!(first.replay_baseline.unwrap().entries.is_empty());

        let source = rcode_doc::SessionDoc::init("replay-chat").unwrap();
        let append = |id: &str| {
            source
                .push_message(&rcode_doc::SessionMessageEntry {
                    id: id.into(),
                    role: rcode_doc::MessageRole::Assistant,
                    parts: vec![rcode_doc::MessagePart::Text {
                        id: "text".into(),
                        text: id.into(),
                    }],
                    created_at: 0,
                    device_id: "writer".into(),
                    status: Some(rcode_doc::MessageStatus::Streaming),
                    continuation_of: None,
                    duration_ms: None,
                    excluded: false,
                })
                .unwrap()
        };
        append("cached");
        sink.apply_checkpoint(&source.export_snapshot().unwrap(), 0)
            .unwrap();
        let checkpoint: rcode_doc::TranscriptUpdate = serde_json::from_value(
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(
            checkpoint
                .replay_baseline
                .unwrap()
                .entries
                .contains_key("cached")
        );

        let version = source.doc().oplog_vv();
        append("away");
        sink.apply_replay_row(
            &source
                .doc()
                .export(loro::ExportMode::updates(&version))
                .unwrap(),
            1,
        );
        let version = source.doc().oplog_vv();
        append("live");
        sink.apply_row(
            &source
                .doc()
                .export(loro::ExportMode::updates(&version))
                .unwrap(),
            2,
        );
        // Neither the doc worker nor the RPC consumer ran between these
        // imports. They must not flatten their different presentation origins.
        let update: rcode_doc::TranscriptUpdate = serde_json::from_value(
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let cutoff = update.replay_baseline.unwrap();
        assert!(cutoff.entries.contains_key("away"));
        assert!(!cutoff.entries.contains_key("live"));
        let mut entries = vec![];
        rcode_doc::apply_transcript_frame(&mut entries, checkpoint.frame).unwrap();
        rcode_doc::apply_transcript_frame(&mut entries, update.frame).unwrap();
        assert_eq!(entries.len(), 3);

        let version = source.doc().oplog_vv();
        append("next-live");
        sink.apply_row(
            &source
                .doc()
                .export(loro::ExportMode::updates(&version))
                .unwrap(),
            3,
        );
        let update: rcode_doc::TranscriptUpdate = serde_json::from_value(
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(
            update.replay_baseline.is_none(),
            "live updates must not resend the history watermark"
        );
        let mut reopened = doc_messages_stream(handle.watch_messages(), handle.doc_arc());
        let opening: rcode_doc::TranscriptUpdate =
            serde_json::from_value(reopened.next().await.unwrap()).unwrap();
        assert_eq!(
            opening.replay_baseline.unwrap().entries.len(),
            4,
            "reopening includes all existing content as history"
        );
        host.shutdown_workers().await;
    }

    #[tokio::test]
    async fn replay_metadata_and_backfill_leave_interleaved_local_content_live() {
        use crate::doc_host::{DocHost, DocHostConfig};
        use rcode_sync::chat_client::ChatDocSink;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(rcode_sync::DocsStore::open(dir.path()).unwrap());
        let host = DocHost::new(
            store.clone(),
            DocHostConfig {
                device_id: "host".into(),
                default_harness: rcode_proto::HarnessId::Mock,
                edge: None,
            },
        );
        let handle = host.open("interleaved").unwrap();
        let sink = crate::chat2_host::EngineChatSink::new(&handle.doc_arc(), store, "interleaved")
            .with_handle(Arc::downgrade(&handle));
        let mut stream = doc_messages_stream(handle.watch_messages(), handle.doc_arc());
        stream.next().await.unwrap();
        let source = rcode_doc::SessionDoc::init("interleaved").unwrap();
        sink.apply_checkpoint(&source.export_snapshot().unwrap(), 0)
            .unwrap();
        let entry = |id: &str| rcode_doc::SessionMessageEntry {
            id: id.into(),
            role: rcode_doc::MessageRole::Assistant,
            parts: vec![rcode_doc::MessagePart::Text {
                id: "text".into(),
                text: id.into(),
            }],
            created_at: 0,
            device_id: "host".into(),
            status: Some(rcode_doc::MessageStatus::Streaming),
            continuation_of: None,
            duration_ms: None,
            excluded: false,
        };
        handle.doc().push_message(&entry("local-before")).unwrap();
        source.update_context_usage(Some(10), Some(100)).unwrap();
        sink.apply_replay_row(&source.export_snapshot().unwrap(), 1);
        let update: rcode_doc::TranscriptUpdate = serde_json::from_value(
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(
            update.replay_baseline.is_none(),
            "metadata must not reset ongoing live animations"
        );
        let mut entries = vec![];
        rcode_doc::apply_transcript_frame(&mut entries, update.frame).unwrap();
        assert_eq!(entries[0].id, "local-before");
        let version = source.doc().oplog_vv();
        source.push_message(&entry("historical")).unwrap();
        handle.doc().push_message(&entry("local-between")).unwrap();
        sink.apply_replay_row(
            &source
                .doc()
                .export(loro::ExportMode::updates(&version))
                .unwrap(),
            2,
        );
        handle.doc().push_message(&entry("local-after")).unwrap();
        let update: rcode_doc::TranscriptUpdate = serde_json::from_value(
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let baseline = update.replay_baseline.unwrap();
        assert_eq!(baseline.entries.len(), 1);
        assert!(baseline.entries.contains_key("historical"));
        rcode_doc::apply_transcript_frame(&mut entries, update.frame).unwrap();
        assert_eq!(entries.len(), 4);
        host.shutdown_workers().await;
    }

    #[tokio::test]
    async fn replay_preserves_each_watchers_opening_cutoff_without_consuming_live_text() {
        use crate::doc_host::{DocHost, DocHostConfig};
        use rcode_sync::chat_client::ChatDocSink;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(rcode_sync::DocsStore::open(dir.path()).unwrap());
        let host = DocHost::new(
            store.clone(),
            DocHostConfig {
                device_id: "viewer".into(),
                default_harness: rcode_proto::HarnessId::Mock,
                edge: None,
            },
        );
        let handle = host.open("cached-replay").unwrap();
        let sink =
            crate::chat2_host::EngineChatSink::new(&handle.doc_arc(), store, "cached-replay")
                .with_handle(Arc::downgrade(&handle));
        let source = rcode_doc::SessionDoc::init("cached-replay").unwrap();
        let mut writer = rcode_doc::SegmentWriter::begin(&source, "reply", "host", 0).unwrap();
        let text = |id: &str, value: &str| rcode_doc::MessagePart::Text {
            id: id.into(),
            text: value.into(),
        };
        let cached = text("body", "café histórico");
        writer.sync(&[cached.clone()]).unwrap();
        sink.apply_checkpoint(&source.export_snapshot().unwrap(), 0)
            .unwrap();
        // Cached content exists before the first watcher and never enters
        // the changed-parts tracker. It may not have been painted yet.
        let mut first = doc_messages_stream(handle.watch_messages(), handle.doc_arc());
        let opening: rcode_doc::TranscriptUpdate =
            serde_json::from_value(first.next().await.unwrap()).unwrap();
        assert_eq!(
            opening.replay_baseline.unwrap().entries["reply"]["body"],
            "café histórico".len()
        );

        let live = text("body", "café histórico y nuevo");
        writer.sync(&[live.clone()]).unwrap();
        sink.apply_row(&source.export_snapshot().unwrap(), 1);
        let update: rcode_doc::TranscriptUpdate = serde_json::from_value(
            tokio::time::timeout(std::time::Duration::from_secs(2), first.next())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(update.replay_baseline.is_none());
        // A later subscriber sees a longer historical prefix, but must not
        // change the first subscriber's ongoing live animation.
        let mut second = doc_messages_stream(handle.watch_messages(), handle.doc_arc());
        let opening: rcode_doc::TranscriptUpdate =
            serde_json::from_value(second.next().await.unwrap()).unwrap();
        assert_eq!(
            opening.replay_baseline.unwrap().entries["reply"]["body"],
            "café histórico y nuevo".len()
        );

        let mut parts = vec![live];
        for ix in 0..2 {
            let id = format!("recovered-{ix}");
            parts.push(text(&id, "otro bloque histórico"));
            writer.sync(&parts).unwrap();
            sink.apply_replay_row(&source.export_snapshot().unwrap(), 2 + ix);
            for (stream, expected) in [
                (&mut first, "café histórico".len()),
                (&mut second, "café histórico y nuevo".len()),
            ] {
                let update: rcode_doc::TranscriptUpdate = serde_json::from_value(
                    tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                        .await
                        .unwrap()
                        .unwrap(),
                )
                .unwrap();
                let baseline = update.replay_baseline.unwrap();
                assert_eq!(
                    baseline.entries["reply"].get("body"),
                    Some(&expected),
                    "replay must retain this watcher's opening cutoff, excluding later live bytes"
                );
                assert_eq!(
                    baseline.entries["reply"][&id],
                    "otro bloque histórico".len()
                );
                assert_eq!(baseline.entries["reply"].len(), 2 + ix as usize);
            }
        }
        host.shutdown_workers().await;
    }

    #[tokio::test]
    async fn reopening_rearms_history_for_previously_live_text() {
        use crate::doc_host::{DocHost, DocHostConfig};
        use rcode_sync::chat_client::ChatDocSink;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(rcode_sync::DocsStore::open(dir.path()).unwrap());
        let host = DocHost::new(
            store.clone(),
            DocHostConfig {
                device_id: "viewer".into(),
                default_harness: rcode_proto::HarnessId::Mock,
                edge: None,
            },
        );
        let handle = host.open("reopen").unwrap();
        let sink = crate::chat2_host::EngineChatSink::new(&handle.doc_arc(), store, "reopen")
            .with_handle(Arc::downgrade(&handle));
        let source = rcode_doc::SessionDoc::init("reopen").unwrap();
        let mut writer = rcode_doc::SegmentWriter::begin(&source, "reply", "host", 0).unwrap();
        let part = |text: &str| rcode_doc::MessagePart::Text {
            id: "body".into(),
            text: text.into(),
        };
        let mut stream = doc_messages_stream(handle.watch_messages(), handle.doc_arc());
        stream.next().await.unwrap();
        writer.sync(&[part("live")]).unwrap();
        sink.apply_row(&source.export_snapshot().unwrap(), 1);
        let _: serde_json::Value =
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap();
        drop(stream);
        // No unwatched commit clears provenance before the new attach.
        let mut stream = doc_messages_stream(handle.watch_messages(), handle.doc_arc());
        stream.next().await.unwrap();
        writer.sync(&[part("live plus recovered")]).unwrap();
        sink.apply_replay_row(&source.export_snapshot().unwrap(), 2);
        let update: rcode_doc::TranscriptUpdate = serde_json::from_value(
            tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            update.replay_baseline.unwrap().entries["reply"]["body"],
            "live plus recovered".len()
        );
        host.shutdown_workers().await;
    }

    #[tokio::test]
    async fn context_only_commits_reach_remote_watch_and_reconnect() {
        let host = rcode_doc::SessionDoc::init("context-chat").unwrap();
        host.update_context_usage(Some(42000), Some(200000))
            .unwrap();
        // The viewing engine reads a replicated document, with no harness process.
        let remote = Arc::new(rcode_doc::SessionDoc::from_doc(loro::LoroDoc::new()));
        remote
            .doc()
            .import(&host.export_snapshot().unwrap())
            .unwrap();
        let (tx, rx) = watch::channel(crate::doc_host::TranscriptSnapshot::default());
        let mut stream = doc_messages_stream(rx, remote.clone());
        let first = stream.next().await.unwrap();
        assert_eq!(first["contextUsage"]["tokens"], 42000);
        assert!(first.get("reset").is_some());
        let version = host.doc().oplog_vv();
        host.update_context_usage(Some(0), None).unwrap();
        remote
            .doc()
            .import(
                &host
                    .doc()
                    .export(loro::ExportMode::updates(&version))
                    .unwrap(),
            )
            .unwrap();
        tx.send_replace(crate::doc_host::TranscriptSnapshot::default());
        let update = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(update["contextUsage"]["tokens"], 0);
        assert_eq!(update["contextUsage"]["window"], 200000);
        let mut reconnect = doc_messages_stream(tx.subscribe(), remote.clone());
        assert_eq!(
            reconnect.next().await.unwrap()["contextUsage"],
            update["contextUsage"]
        );
        remote.clear_context_usage().unwrap();
        tx.send_replace(crate::doc_host::TranscriptSnapshot::default());
        assert!(stream.next().await.unwrap()["contextUsage"].is_null());
    }
}
