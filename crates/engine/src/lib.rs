//! rcode-engine — the headless backend: sessions engine, doc host + command executor,
//! run journal + crash recovery, and the IPC RPC server.
//!
//! Spec: ARCHITECTURE.md §5 and docs/research/feature-inventory.md §3. M2 surface:
//! sessions + docs + commands + minimal IPC. Terminals, repos/diffs, uploads, auth,
//! agent accounts, and the device-room host land in later milestones.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
pub use rcode_proto::{EngineInfo, HarnessId};
use rcode_rpc::{RpcError, RpcReply, RpcService, methods};

use rcode_sync::DocsStore;

pub mod auth;
mod change_request_store;
pub mod change_requests;
pub mod chat2_host;
pub mod custom_instructions;
mod chat_persistence;
pub mod diff_sync;
pub mod doc_host;
mod fs_watch;
pub(crate) mod home_index;
mod http_error;
pub mod instance_lock;
pub mod profile;
mod provider_models;
mod model_catalog;
pub use model_catalog::start as refresh_model_catalog;
mod project_memory;
pub mod registry;
pub mod repos;
pub mod rpc;
pub mod run_journal;
pub mod sessions;
mod goals;
pub mod source_control;
pub mod spaces;
pub mod terminals;
mod transcript_history;
mod transcript_window;
mod message_ids;
pub mod uploads;
pub mod workspace_files;
pub mod workspace_host;
pub mod workspace_search;
mod worktree_settings;

pub use auth::{Auth, AuthConfig, AuthState, AuthUser, OrgMembership};
pub use change_requests::{ChangeRequestCacheKey, CheckoutChangeRequests};
pub use diff_sync::{
    CheckoutDiffSync, DiffFileTextPair, DiffSidecar, DiffSnapshot, TreeSnapshot, TurnSnapshot,
    capture_commit_diff, capture_diff, capture_diff_against, capture_turn_diff,
    discard_working_tree, merge_base, read_diff_file_text, snapshot_tree, working_diff_base,
};
pub use doc_host::{ChatDocHandle, DocHost, DocHostConfig, EdgeConfig};
pub use instance_lock::InstanceLock;
pub use profile::{DataRoots, EngineProfile};
pub use registry::{
    HarnessDescriptor, HarnessRegistry, default_registry, registry_with_native,
    registry_with_native_and_store,
};
pub use repos::{CheckoutIdentity, Repos, worktree_branch_from_title};
pub use rpc::EngineRpc;
pub use run_journal::{JournalError, RunJournal};
pub use sessions::{JournaledEvent, SessionsEngine, SteerOutcome};
pub use source_control::{
    BranchHeadContext, ChangeRequestError, ChangeRequestProvider, ChangeRequestResolution,
    ChangeRequestResolver, CheckoutChangeRequestLookup, CheckoutSourceContext, GitHubCli,
    GitRemote, parse_git_remote,
};
pub use spaces::SpacesSync;
pub use terminals::Terminals;
pub use uploads::{AttachmentChunk, Uploads};
pub use workspace_files::WorkspaceFiles;
pub use workspace_host::{
    DEFAULT_ORG_ID, DEFAULT_USER_ID, WORKSPACE_DOC_ID, WorkspaceHost, WorkspaceHostConfig,
};
pub use workspace_search::WorkspaceSearch;

pub(crate) const LEGACY_UNKNOWN_DEVICE_NAME: &str = "unknown-device";

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Token(#[from] rcode_rpc::TokenError),
    #[error("doc: {0}")]
    Doc(#[from] rcode_doc::DocError),
    #[error("journal: {0}")]
    Journal(#[from] run_journal::JournalError),
    #[error("store: {0}")]
    Store(#[from] rcode_sync::StoreError),
    #[error("harness: {0}")]
    Harness(#[from] rcode_harness::HarnessError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// Epoch millis now — the doc/journal timestamp base.
pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub(crate) fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// The two directories this runtime reads (see [`DataRoots`]). The caller
    /// resolves them: `apps/rcode` owns the platform default, the
    /// `RCODE_DATA_DIR` override, and the `location` pointer.
    pub roots: DataRoots,
    /// Edge base URL.
    pub edge_url: String,
    /// Explicit development bearer for edge room joins. Local-only runtimes
    /// otherwise use the device's fixed local identity from [`Auth`].
    pub edge_token: Option<String>,
    /// Localhost IPC port for the UI.
    pub ipc_port: u16,
    /// Harness for doc-command runs on chats without a workspace `config` row.
    pub default_harness: HarnessId,
    /// Workspace-doc org (`ws/{orgId}` room). `None` = `$RCODE_ORG_ID` or the dev default.
    pub org_id: Option<String>,
}

/// The assembled engine core — also constructible without the IPC server for tests
/// and the in-process (headed) mode.
pub struct EngineCore {
    pub sessions: SessionsEngine,
    pub doc_host: DocHost,
    pub workspace: WorkspaceHost,
    pub registry: Arc<HarnessRegistry>,
    pub repos: Repos,
    pub workspace_files: WorkspaceFiles,
    pub terminals: Terminals,
    pub previews: rcode_preview::PreviewService,
    pub change_requests: CheckoutChangeRequests,
    pub diff_sync: CheckoutDiffSync,
    pub spaces_sync: SpacesSync,
    pub uploads: Uploads,
    pub device_id: String,
    /// Where the installation's own settings live: `agent-settings.json`,
    /// harness preferences, provider keys. Distinct from
    /// [`Repos::data_dir`] when the user moved the bulk data root, and the
    /// authority for files the settings pages write — the same root the
    /// harness was built against.
    config_root: std::path::PathBuf,
    /// Auth service (attached by [`Engine::run`]; a lazy dev-mode instance otherwise).
    auth: std::sync::Mutex<Option<Auth>>,
    /// Peer link cache for `targetDeviceId` routing (attached when edge+auth are ready).
    links: std::sync::Mutex<Option<Arc<rcode_rpc::LinkCache>>>,
    /// Release checker (attached by [`Engine::assemble_runtime`]) — the
    /// UpdateStatus stream + ApplyUpdate.
    updater: std::sync::Mutex<Option<rcode_update::Updater>>,
    /// The updater's token-change wake forwarder — owned so shutdown can end it.
    updater_wake: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Exclusive data-dir lock — held for the engine's lifetime (single-instance).
    _instance_lock: InstanceLock,
}

impl EngineCore {
    /// Open stores under `data_dir`, wire sessions ⇄ doc host ⇄ workspace host, and
    /// recover stale journals from a previous crash. One directory for both
    /// roots — see [`DataRoots::same`].
    pub fn assemble(
        data_dir: &Path,
        registry: Arc<HarnessRegistry>,
        default_harness: HarnessId,
        edge: Option<EdgeConfig>,
    ) -> Result<Self, EngineError> {
        Self::assemble_with_roots(DataRoots::same(data_dir), registry, default_harness, edge)
    }

    /// Assemble with the two roots named separately.
    pub fn assemble_with_roots(
        roots: DataRoots,
        registry: Arc<HarnessRegistry>,
        default_harness: HarnessId,
        edge: Option<EdgeConfig>,
    ) -> Result<Self, EngineError> {
        let org_id = env_or("RCODE_ORG_ID", DEFAULT_ORG_ID);
        let user_id = env_or("RCODE_USER_ID", DEFAULT_USER_ID);
        let profile = EngineProfile::new(&roots, &org_id, &user_id);
        Self::assemble_with_profile(profile, registry, default_harness, edge)
    }

    /// Assemble the engine against one resolved, immutable workspace profile.
    pub fn assemble_with_profile(
        profile: EngineProfile,
        registry: Arc<HarnessRegistry>,
        default_harness: HarnessId,
        edge: Option<EdgeConfig>,
    ) -> Result<Self, EngineError> {
        // Both roots exist before anything opens: the config root carries the
        // lock and the identity files, the data root the stores.
        std::fs::create_dir_all(profile.config_root())?;
        std::fs::create_dir_all(profile.data_root())?;
        // Single-instance guard: two engines on one config root would race the
        // SQLite snapshots + journals, and one store root cannot have two
        // writers. Taken before any store opens or the IPC port binds; held
        // (and kernel-released on crash) for the engine's life.
        let lock = InstanceLock::acquire(profile.config_root())?;
        Self::assemble_with_profile_locked(profile, registry, default_harness, edge, lock)
    }

    /// Assemble against a pre-acquired [`InstanceLock`]. The headed app takes
    /// the lock before binding the IPC port so the listener owner and the
    /// data-dir owner cannot diverge when several viewports bootstrap at once.
    pub fn assemble_with_profile_locked(
        profile: EngineProfile,
        registry: Arc<HarnessRegistry>,
        default_harness: HarnessId,
        edge: Option<EdgeConfig>,
        lock: InstanceLock,
    ) -> Result<Self, EngineError> {
        let config_root = profile.config_root();
        std::fs::create_dir_all(config_root)?;
        let legacy_uploads_root = profile.claim_legacy_uploads_root()?;
        let device_id = load_or_create_device_id(config_root)?;
        // This device's harness enablement (Settings → Providers) rides the
        // config root — per-device, like the provider keys beside it.
        registry.load_prefs(config_root);
        let store = Arc::new(DocsStore::open(profile.store_root())?);
        let journal = Arc::new(RunJournal::open(profile.store_root().join("journals"))?);
        let sessions = SessionsEngine::new(device_id.clone(), journal, registry.clone());
        let doc_host = DocHost::new(
            store.clone(),
            DocHostConfig {
                device_id: device_id.clone(),
                default_harness,
                edge: edge.clone(),
            },
        );
        let workspace = WorkspaceHost::open(
            store,
            WorkspaceHostConfig {
                device_id: device_id.clone(),
                device_name: local_device_name(&device_id),
                platform: std::env::consts::OS.to_string(),
                org_id: profile.org_id().to_string(),
                user_id: profile.user_id().to_string(),
                edge: edge.clone(),
            },
        )?;
        doc_host.set_workspace(workspace.clone());
        doc_host.set_sessions(sessions.clone());
        sessions.set_doc_host(doc_host.clone());
        // Post-run auto-titling: the sessions engine sees the turn finish, the
        // workspace owns the chat row the title lands on.
        sessions.set_workspace(workspace.clone());
        match sessions.recover_stale() {
            Ok(0) => {}
            Ok(recovered) => tracing::info!(recovered, "stale sessions recovered on boot"),
            Err(err) => tracing::error!(error = %err, "stale-session recovery failed"),
        }
        doc_host.spawn_transcript_salvage(profile.store_root().join("journals"));
        let repos = Repos::new(profile.data_root(), &device_id);
        sessions.set_scratch_root(repos.scratch_root());
        doc_host.set_repos(repos.clone());
        let change_requests = CheckoutChangeRequests::start(repos.clone(), &device_id);
        let workspace_files =
            WorkspaceFiles::new(repos.clone(), workspace.clone(), device_id.clone()).with_memory_config(config_root.to_path_buf());
        let terminals = Terminals::new();
        let previews = rcode_preview::PreviewService::new(
            profile.store_root().join("previews.json"),
            device_id.clone(),
            local_device_name(&device_id),
        )
        .map_err(|e| EngineError::Other(e.to_string()))?;
        let uploads = Uploads::from_root_with_fallback(
            profile.uploads_root(),
            legacy_uploads_root.as_deref(),
        );
        // Queued-attachment support: the doc host resolves `pending://` refs
        // against this store and pushes staged bytes to remote hosts.
        doc_host.set_uploads(uploads.clone());
        // Logins started from another device publish their callback port to
        // the P2P service, which serves it to that device alone.
        let diff_sync = CheckoutDiffSync::start(repos.clone(), workspace.clone(), &device_id, edge);
        // Turn starts snapshot the checkout tree — the "Latest turn" diff base.
        let turn_diff = diff_sync.clone();
        sessions.set_turn_listener(Arc::new(move |chat_id, cwd| {
            turn_diff.note_turn_start(chat_id, cwd);
        }));
        let spaces_sync = SpacesSync::start(repos.clone(), workspace.clone(), &device_id);
        Ok(Self {
            sessions,
            doc_host,
            workspace,
            registry,
            repos,
            workspace_files,
            terminals,
            previews,
            change_requests,
            diff_sync,
            spaces_sync,
            uploads,
            device_id,
            config_root: config_root.to_path_buf(),
            auth: std::sync::Mutex::new(None),
            links: std::sync::Mutex::new(None),
            updater: std::sync::Mutex::new(None),
            updater_wake: std::sync::Mutex::new(None),
            _instance_lock: lock,
        })
    }

    /// The config root: identity, credentials and capability settings.
    ///
    /// The settings pages must write here rather than to the data root, or a
    /// user who pointed the bulk data elsewhere would edit a file the running
    /// harness never reads.
    pub fn config_root(&self) -> &std::path::Path {
        &self.config_root
    }

    /// Attach the auth service (before building the RPC service / relays).
    pub fn set_auth(&self, auth: Auth) {
        *self
            .auth
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(auth);
    }

    /// The attached auth service, or a lazily-created local one (in-process
    /// embeds still answer AuthStatus honestly).
    pub fn auth(&self) -> Auth {
        let mut slot = self
            .auth
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slot.get_or_insert_with(|| {
            let dev_user = std::env::var("RCODE_EDGE_TOKEN")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "dev-user".into());
            let mut config = AuthConfig::new(std::env::temp_dir());
            config.dev_user_id = dev_user;
            Auth::new(config)
        })
        .clone()
    }

    /// Attach the peer link cache — enables `targetDeviceId` routing,
    /// [`Self::dial_device`], and the doc host's queued-attachment transfers.
    pub fn set_links(&self, links: Arc<rcode_rpc::LinkCache>) {
        self.doc_host.set_links(links.clone());
        *self
            .links
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(links);
    }

    pub fn links(&self) -> Option<Arc<rcode_rpc::LinkCache>> {
        self.links
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Attach the release checker (before building the RPC service).
    pub fn set_updater_wake(&self, handle: tokio::task::JoinHandle<()>) {
        *self
            .updater_wake
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handle);
    }

    pub fn set_updater(&self, updater: rcode_update::Updater) {
        *self
            .updater
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(updater);
    }

    pub fn updater(&self) -> Option<rcode_update::Updater> {
        self.updater
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// A live RPC client to another device's engine through its relay DO (the router's
    /// dial seam). Cached per device; invalidated + re-dialed on failure.
    pub async fn dial_device(
        &self,
        device_id: &str,
    ) -> Result<Arc<rcode_rpc::RpcClient>, EngineError> {
        let links = self
            .links()
            .ok_or_else(|| EngineError::Other("peer links unavailable (offline)".into()))?;
        links
            .client(device_id)
            .await
            .map_err(|e| EngineError::Other(e.to_string()))
    }

    /// Start hosting our device room: serve the full RPC surface to relay clients and
    /// warm-open chat docs on nudges (§7 cold-chat command delivery). The token source
    /// re-reads auth on every (re)dial, so token refreshes take effect at reconnect.
    pub fn start_host_relay(&self, edge_url: &str) -> rcode_rpc::HostRelay {
        let auth = self.auth();
        let config =
            rcode_rpc::HostRelayConfig::new(edge_url, self.device_id.clone(), Arc::new(auth));
        let doc_host = self.doc_host.clone();
        let on_nudge: rcode_rpc::NudgeHandler = Arc::new(move |chat_id: String| {
            match doc_host.enqueue_wakeup(&chat_id) {
                Ok(()) => true,
                Err(err) => {
                    tracing::warn!(chat = %chat_id, %err, "nudge: durable admission failed; withholding ACK");
                    false
                }
            }
        });
        rcode_rpc::HostRelay::spawn(config, self.rpc_service(), on_nudge)
    }

    pub fn rpc_service(&self) -> Arc<EngineRpc> {
        let mut rpc = EngineRpc::new(
            self.sessions.clone(),
            self.doc_host.clone(),
            self.workspace.clone(),
            self.registry.clone(),
            self.repos.clone(),
            self.workspace_files.clone(),
            self.terminals.clone(),
            self.change_requests.clone(),
            self.diff_sync.clone(),
            self.uploads.clone(),
            self.config_root.clone(),
        )
        .with_auth(self.auth())
        .with_previews(self.previews.clone());
        if let Some(links) = self.links() {
            rpc = rpc.with_links(links);
        }
        if let Some(updater) = self.updater() {
            rpc = rpc.with_updater(updater);
        }
        Arc::new(rpc)
    }

    /// Revoke every account-scoped transport before any slower graceful
    /// draining. Connected sockets remain authorized by their handshake, so
    /// clearing credentials alone is not a security boundary.
    pub fn disconnect_edge(&self) {
        self.previews.stop();
        if let Some(links) = self.links() {
            links.disconnect_all();
        }
        self.doc_host.disconnect_edge();
        self.workspace.disconnect_edge();
    }

    /// Graceful teardown: settle live runs (streaming entries stamped `aborted`),
    /// kill live PTYs, stamp our workspace `lastSeenAt`, and flush every open doc
    /// snapshot.
    pub async fn shutdown(&self) {
        self.previews.shutdown().await;
        // A run interruption transitions its chat to Idle, and Idle normally
        // releases the next queued row. Freeze first so quitting never starts
        // recovered work while the engine is being torn down.
        self.doc_host.pause_all_queues();
        self.sessions.shutdown().await;
        self.terminals.shutdown();
        self.change_requests.shutdown();
        // Cancel + await every worker that can reach Edge before flushing: a
        // replaced synced runtime must not keep polling releases or draining
        // the attachment outbox under the old identity after Local boots.
        let wake = self
            .updater_wake
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(wake) = wake {
            wake.abort();
            let _ = wake.await;
        }
        let updater = self
            .updater
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(updater) = updater {
            updater.shutdown().await;
        }
        self.diff_sync.shutdown().await;
        self.workspace_files.shutdown().await;
        self.spaces_sync.shutdown().await;
        self.doc_host.shutdown_workers().await;
        self.doc_host.flush_all();
        self.workspace.shutdown();
        // Break the sessions ⇄ doc-host retain cycle so the replaced graph can
        // actually be freed once the last handle drops.
        self.sessions.clear_doc_host();
        self.sessions.clear_workspace();
    }
}

pub struct Engine {
    pub config: EngineConfig,
}

/// A fully assembled identity-scoped engine plus the relay handle whose lifetime
/// keeps this device reachable. Used by both the headless server and the headed
/// in-process engine so their production authentication paths cannot diverge.
pub struct EngineRuntime {
    core: EngineCore,
    host_relay: std::sync::Mutex<Option<rcode_rpc::HostRelay>>,
}

/// IPC-only lifecycle control owned by `rcode headless`. The regular
/// [`EngineRpc`] deliberately does not expose this method, so a viewport
/// attached to another headed process cannot shut down that process's engine.
struct HeadlessRpc {
    inner: Arc<dyn RpcService>,
    stop_tx: tokio::sync::mpsc::UnboundedSender<()>,
}

#[async_trait]
impl RpcService for HeadlessRpc {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<RpcReply, RpcError> {
        if method != methods::STOP_ENGINE {
            return self.inner.handle(method, params).await;
        }

        let stop_tx = self.stop_tx.clone();
        // Let the unary success frame reach the client before `Engine::run`
        // aborts the IPC server and drains the runtime.
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let _ = stop_tx.send(());
        });
        RpcReply::value(&serde_json::json!({ "ok": true }))
    }
}

impl EngineRuntime {
    pub fn core(&self) -> &EngineCore {
        &self.core
    }

    pub fn disconnect_edge(&self) {
        // Revoke remote reachability before graceful draining. Sessions may
        // need time to settle; no authenticated relay RPC may enter during
        // that window after sign-out.
        self.host_relay
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        self.core.disconnect_edge();
    }

    pub async fn shutdown(&self) {
        self.disconnect_edge();
        self.core.shutdown().await;
    }
}

impl Drop for EngineRuntime {
    fn drop(&mut self) {
        self.host_relay
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        Self { config }
    }

    /// Resolve the local auth configuration for headed and headless modes.
    /// Local-only: no network, no WorkOS, a fixed device identity.
    pub async fn build_auth(config: &EngineConfig) -> Auth {
        let mut auth_config = AuthConfig::new(config.roots.config().to_path_buf());
        if let Some(token) = &config.edge_token {
            auth_config.dev_user_id = token.clone();
        }
        Auth::new(auth_config)
    }

    /// Resolve the runtime's profile: the identity it stores under.
    ///
    /// There is one shape. The org comes from the bearer when it carries one
    /// (`user@org`), then the explicit configuration, then the environment; the
    /// user comes from the device's own auth.
    pub fn resolve_profile(config: &EngineConfig, auth: &Auth) -> EngineProfile {
        let token_org = config
            .edge_token
            .as_deref()
            .and_then(|token| token.split_once('@'))
            .map(|(_, org)| org.to_string())
            .filter(|org| !org.is_empty());
        let org_id = token_org
            .or(config.org_id.clone())
            .unwrap_or_else(|| env_or("RCODE_ORG_ID", DEFAULT_ORG_ID));
        let user_id = auth
            .user_id()
            .unwrap_or_else(|| env_or("RCODE_USER_ID", DEFAULT_USER_ID));
        EngineProfile::new(&config.roots, &org_id, &user_id)
    }

    /// Resolve the one-shot identity served before profile stores are available.
    pub fn engine_info(config: &EngineConfig) -> Result<EngineInfo, EngineError> {
        std::fs::create_dir_all(config.roots.config())?;
        Ok(EngineInfo {
            device_id: load_or_create_device_id(config.roots.config())?,
            cursor_sdk_version: None,
            capabilities: rcode_proto::capabilities::current(),
        })
    }

    /// Open one already-resolved profile. Synced profiles always keep their
    /// Edge supervisors alive; temporary token or network failures are runtime
    /// states, not a reason to permanently assemble an offline engine.
    pub async fn assemble_runtime(
        config: &EngineConfig,
        auth: Auth,
        profile: EngineProfile,
    ) -> anyhow::Result<EngineRuntime> {
        Self::assemble_runtime_inner(config, auth, profile, None).await
    }

    /// Like [`Self::assemble_runtime`], but against an [`InstanceLock`] the
    /// caller already holds on the profile's device root (headed bootstrap
    /// acquires it before binding IPC).
    pub async fn assemble_runtime_with_lock(
        config: &EngineConfig,
        auth: Auth,
        profile: EngineProfile,
        lock: InstanceLock,
    ) -> anyhow::Result<EngineRuntime> {
        Self::assemble_runtime_inner(config, auth, profile, Some(lock)).await
    }

    async fn assemble_runtime_inner(
        config: &EngineConfig,
        auth: Auth,
        profile: EngineProfile,
        lock: Option<InstanceLock>,
    ) -> anyhow::Result<EngineRuntime> {
        // The edge is an explicit, offline-capable opt-in: it needs both a
        // bearer (which identifies this runtime to the rooms) and a base URL to
        // dial. Neither has a default — the deployment this build shipped with
        // is gone — so a half-configured pair stays local and says which half
        // is missing, rather than dialing a host nobody chose. The local Auth
        // always exposes `dev_user_id` as its bearer; that alone does not opt in.
        let edge_token = config
            .edge_token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty());
        let edge_url = config.edge_url.trim();
        let edge_url = (!edge_url.is_empty()).then_some(edge_url);
        if edge_token.is_some() != edge_url.is_some() {
            tracing::warn!(
                token = edge_token.is_some(),
                url = edge_url.is_some(),
                "the edge needs both RCODE_EDGE_TOKEN and RCODE_EDGE_URL; this runtime stays local"
            );
        }
        let edge_enabled = edge_token.is_some() && edge_url.is_some();
        if edge_enabled {
            // OS network-path events (macOS NWPathMonitor): the instant the
            // path returns every parked reconnect backoff redials, and while
            // the OS says there is no path the dial loops park instead of
            // burning attempts. No-op on platforms without a monitor.
            rcode_sync::net_path::spawn_path_monitor();
        }
        let device_id = load_or_create_device_id(profile.config_root())?;
        let edge = edge_enabled.then(|| {
            EdgeConfig::new(config.edge_url.clone(), Arc::new(auth.clone())).with_device(device_id)
        });

        // Resolved before `profile` is moved into the assemble call below; the
        // native harness reads its provider config from here, and the settings
        // RPCs reach the same store through it.
        let (registry, _provider_store) = registry_with_native_and_store(profile.config_root());
        let registry = Arc::new(registry);
        let core = match lock {
            Some(lock) => EngineCore::assemble_with_profile_locked(
                profile,
                Arc::clone(&registry),
                config.default_harness,
                edge.clone(),
                lock,
            )?,
            None => EngineCore::assemble_with_profile(
                profile,
                registry,
                config.default_harness,
                edge.clone(),
            )?,
        };
        core.set_auth(auth.clone());
        model_catalog::start(core.config_root().to_path_buf());
        let preview_workspace = core.workspace.clone();
        let preview_device = core.device_id.clone();
        let projects = Arc::new(move || {
            preview_workspace
                .read_chats()
                .unwrap_or_default()
                .into_iter()
                .filter(|chat| chat.device_id == preview_device)
                .filter_map(|chat| chat.cwd.map(std::path::PathBuf::from))
                .collect()
        });
        core.previews.start(projects).await;
        // Release checker: polls {edge}/releases hourly (wall clock); headless
        // installs with RCODE_AUTO_UPDATE=1 apply + restart themselves — gated
        // on quiescence so a restart never lands under a live run or open PTY.
        // Spawned for every install: application updates must not depend on
        // workspace sync being enabled — the feed is the public release feed
        // (served without authentication), not an edge feature.
        let quiescent: rcode_update::QuiescentCheck = {
            let sessions = core.sessions.clone();
            let terminals = core.terminals.clone();
            Arc::new(move || !sessions.any_active() && !terminals.any_open())
        };
        let updater =
            rcode_update::Updater::spawn(rcode_update::default_release_base(), Some(quiescent));
        if let Some(mut token_changes) = edge.as_ref().and_then(EdgeConfig::token_changes) {
            let updater_for_tokens = updater.clone();
            let wake = tokio::spawn(async move {
                while token_changes.changed().await.is_ok() {
                    updater_for_tokens.check_now();
                }
            });
            core.set_updater_wake(wake);
        }
        core.set_updater(updater);
        tracing::info!(device_id = %core.device_id, "engine core assembled");

        let host_relay = edge.as_ref().map(|edge| {
            let mut link_config =
                rcode_rpc::LinkCacheConfig::new(edge.url.clone(), Arc::new(auth.clone()));
            // Registry-dark dial gate: devices with no recent presence fail
            // fast with zero dials; presence returning un-parks them (the
            // peer-alive hook below clears any cooldown at the same moment).
            let workspace_for_liveness = core.workspace.clone();
            link_config.liveness = Some(Arc::new(move |device_id: &str| {
                workspace_for_liveness.peer_liveness(device_id)
            }));
            let links = rcode_rpc::LinkCache::new(link_config);
            let links_for_presence = links.clone();
            core.workspace
                .set_peer_alive_hook(Arc::new(move |device_id: &str| {
                    links_for_presence.reset_cooldown(device_id);
                }));
            core.set_links(links);
            core.start_host_relay(&edge.url)
        });

        Ok(EngineRuntime {
            core,
            host_relay: std::sync::Mutex::new(host_relay),
        })
    }

    /// Run until ctrl-c: auth (local), sessions engine + doc host + command
    /// executor, IPC server, and — when an explicit edge bearer is configured —
    /// the device-room host relay + peer link cache (targetDeviceId routing).
    pub async fn run(self) -> anyhow::Result<()> {
        let config = self.config;
        tracing::info!(
            config_root = %config.roots.config().display(),
            data_root = %config.roots.data().display(),
            "engine starting"
        );

        std::fs::create_dir_all(config.roots.config())?;
        std::fs::create_dir_all(config.roots.data())?;
        let auth = Self::build_auth(&config).await;
        let profile = Self::resolve_profile(&config, &auth);

        let runtime = Self::assemble_runtime(&config, auth, profile).await?;
        // The desktop app or `rcode update` may install a newer binary under
        // a running service; restart into it once no run or terminal is live.
        if let Some(updater) = runtime.core().updater() {
            updater.restart_when_superseded();
        }

        // A daemon exists to serve this port, so a bind failure is fatal here —
        // unlike the headed app, which can still work over its in-process
        // transport (see `serve_ipc`).
        let (stop_tx, mut stop_rx) = tokio::sync::mpsc::unbounded_channel();
        let service: Arc<dyn RpcService> = Arc::new(HeadlessRpc {
            inner: runtime.core().rpc_service(),
            stop_tx,
        });
        let server = serve_ipc(config.ipc_port, service).await?;

        tokio::select! {
            result = shutdown_signal() => result?,
            requested = stop_rx.recv() => {
                if requested.is_some() {
                    tracing::info!("headless shutdown requested over IPC");
                }
            }
        }
        tracing::info!("shutting down");
        server.abort();
        runtime.shutdown().await;
        Ok(())
    }
}

/// Ctrl-C or SIGTERM. systemd/launchd stop (and the auto-updater's service
/// restart) deliver SIGTERM — without catching it the daemon dies mid-write
/// and every stop takes the crash-recovery path instead of the graceful drain.
async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = sigterm.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

/// Serve the typed RPC on the localhost IPC port.
///
/// Both engines call this: the headless daemon, and the headed app's embedded
/// engine. That second case is the point — an embedded engine that keeps the
/// port to itself forces anyone wanting a second viewport (the terminal app) to
/// stop the desktop app, start a daemon, and start it again in the right order.
/// Serving here means any viewport can just attach.
///
/// Localhost only, exactly as before: this widens *which process* can serve the
/// port, not who can reach it.
pub async fn serve_ipc(
    port: u16,
    service: std::sync::Arc<dyn rcode_rpc::RpcService>,
) -> std::io::Result<tokio::task::JoinHandle<()>> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    tracing::info!(port, "IPC server listening");
    Ok(tokio::spawn(rcode_rpc::serve_ws_listener(
        listener, service,
    )))
}

/// Best-effort human name for this device's registry row.
fn local_device_name(device_id: &str) -> String {
    select_local_device_name(
        [
            std::env::var("RCODE_DEVICE_NAME").ok(),
            native_friendly_device_name(),
            std::env::var("HOSTNAME").ok(),
            gethostname::gethostname().into_string().ok(),
            std::fs::read_to_string("/etc/hostname").ok(),
        ],
        device_id,
        std::env::consts::OS,
    )
}

fn select_local_device_name(
    candidates: impl IntoIterator<Item = Option<String>>,
    device_id: &str,
    platform: &str,
) -> String {
    candidates
        .into_iter()
        .flatten()
        .map(|name| name.trim().to_string())
        .find(|name| !name.is_empty())
        .unwrap_or_else(|| {
            let platform = match platform {
                "macos" => "macOS",
                "windows" => "Windows",
                "linux" => "Linux",
                _ => "Local",
            };
            let short_id: String = device_id.chars().take(8).collect();
            format!("{platform} device {short_id}")
        })
}

#[cfg(target_os = "macos")]
fn native_friendly_device_name() -> Option<String> {
    let output = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(not(target_os = "macos"))]
fn native_friendly_device_name() -> Option<String> {
    #[cfg(target_os = "windows")]
    return std::env::var("COMPUTERNAME").ok();

    #[cfg(not(target_os = "windows"))]
    None
}

#[cfg(all(test, windows))]
mod identity_lock_retry_tests {
    use super::*;

    /// A concurrent holder of the lock file (share_mode(0)) fails the open
    /// with ERROR_SHARING_VIOLATION, which Rust reports as
    /// ErrorKind::Uncategorized, not PermissionDenied. acquire must retry
    /// through that error until the holder releases; before the fix the
    /// retry loop never matched it and startup failed outright.
    #[test]
    fn acquire_retries_through_sharing_violations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device-id.lock");

        // Hold the file exclusively for 50ms, then release. The retry loop
        // has a 200 x 5ms budget, so the timing is comfortable.
        {
            use std::os::windows::fs::OpenOptionsExt;
            let holder = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .share_mode(0)
                .open(&path)
                .unwrap();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                drop(holder);
            });
        }

        let lock = DeviceIdentityLock::acquire(dir.path());
        assert!(
            lock.is_ok(),
            "acquire did not retry through the sharing violation"
        );
    }
}

#[cfg(test)]
mod device_name_tests {
    use super::select_local_device_name;

    fn name(candidates: &[Option<&str>], device_id: &str, platform: &str) -> String {
        select_local_device_name(
            candidates
                .iter()
                .map(|candidate| candidate.map(str::to_string)),
            device_id,
            platform,
        )
    }

    #[test]
    fn explicit_override_wins_and_is_trimmed() {
        assert_eq!(
            name(
                &[Some("  Studio Mac  "), Some("system-host")],
                "17bc0aa2-rest",
                "macos"
            ),
            "Studio Mac"
        );
    }

    #[test]
    fn native_friendly_name_wins_over_hostnames() {
        assert_eq!(
            name(
                &[
                    None,
                    Some("MacBook Pro de Jose"),
                    None,
                    Some("MacBook-Pro.local"),
                ],
                "17bc0aa2-rest",
                "macos"
            ),
            "MacBook Pro de Jose"
        );
    }

    #[test]
    fn windows_computer_name_is_used_when_present() {
        assert_eq!(
            name(
                &[None, Some("DESKTOP-123"), Some("shell-host")],
                "17bc0aa2-rest",
                "windows"
            ),
            "DESKTOP-123"
        );
    }

    #[test]
    fn blank_candidates_are_ignored() {
        assert_eq!(
            name(
                &[Some("  "), None, Some("\n"), Some("linux-box")],
                "17bc0aa2-rest",
                "linux"
            ),
            "linux-box"
        );
    }

    #[test]
    fn final_fallback_is_platform_specific_and_distinct() {
        assert_eq!(
            name(&[None, Some(" ")], "17bc0aa2-rest", "linux"),
            "Linux device 17bc0aa2"
        );
    }
}

/// Trimmed env var or the given default.
fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// Stable per-installation device id, persisted at `{data_dir}/device-id`.
fn load_or_create_device_id(data_dir: &Path) -> Result<String, EngineError> {
    std::fs::create_dir_all(data_dir)?;
    // EngineInfo is resolved before the lifetime InstanceLock is acquired, so
    // identity creation and legacy repair need their own short critical section.
    // The OS releases this lock after a crash; unlike a create_new lockfile it
    // cannot strand an installation permanently.
    let _identity_lock = DeviceIdentityLock::acquire(data_dir)?;
    let path = data_dir.join("device-id");
    let recovering_empty = match std::fs::read_to_string(&path) {
        Ok(id) if !id.trim().is_empty() => return Ok(id.trim().to_string()),
        // Older releases used truncate+write. A crash between those operations
        // left a zero-byte file that is safe to replace with a fresh identity.
        Ok(_) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => return Err(err.into()),
    };

    let id = new_id();
    let temp_path = data_dir.join(format!(
        ".device-id.tmp-{}-{}",
        std::process::id(),
        new_id()
    ));
    let write_result = (|| -> Result<(), EngineError> {
        let mut temp = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        temp.write_all(id.as_bytes())?;
        temp.sync_all()?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(err);
    }

    // Fresh installs use create-if-absent. Legacy empty files need an atomic
    // same-directory replacement on Unix; the Windows fallback runs under the
    // identity lock and remains recoverable if interrupted.
    let publish_result = if recovering_empty {
        match std::fs::read_to_string(&path) {
            Ok(id) if !id.trim().is_empty() => {
                let _ = std::fs::remove_file(&temp_path);
                return Ok(id.trim().to_string());
            }
            Ok(_) => replace_empty_device_id(&temp_path, &path),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                std::fs::hard_link(&temp_path, &path)
            }
            Err(err) => Err(err),
        }
    } else {
        std::fs::hard_link(&temp_path, &path)
    };
    let _ = std::fs::remove_file(&temp_path);
    match publish_result {
        Ok(()) => Ok(id),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let winner = std::fs::read_to_string(&path)?;
            if winner.trim().is_empty() {
                Err(EngineError::Other(format!(
                    "invalid device identity {}: file is empty",
                    path.display()
                )))
            } else {
                Ok(winner.trim().to_string())
            }
        }
        Err(err) => Err(err.into()),
    }
}

struct DeviceIdentityLock {
    _file: std::fs::File,
}

impl DeviceIdentityLock {
    fn acquire(data_dir: &Path) -> Result<Self, EngineError> {
        let path = data_dir.join("device-id.lock");
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);

        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
            let mut retries = 200;
            let file = loop {
                match options.open(&path) {
                    Ok(file) => break file,
                    // share_mode(0) means a concurrent holder fails the open
                    // with ERROR_SHARING_VIOLATION (raw os error 32), which
                    // Rust maps to ErrorKind::Uncategorized, not
                    // PermissionDenied. Retry through both.
                    Err(err)
                        if (err.raw_os_error()
                            == Some(
                                windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32,
                            )
                            || err.kind() == std::io::ErrorKind::PermissionDenied)
                            && retries > 0 =>
                    {
                        retries -= 1;
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(err) => return Err(err.into()),
                }
            };
            return Ok(Self { _file: file });
        }

        #[cfg(not(windows))]
        let file = options.open(&path)?;

        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            loop {
                if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                    break;
                }
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() != Some(libc::EINTR) {
                    return Err(err.into());
                }
            }
        }

        #[cfg(not(windows))]
        Ok(Self { _file: file })
    }
}

fn replace_empty_device_id(temp_path: &Path, path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::rename(temp_path, path)
    }
    #[cfg(not(unix))]
    {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        std::fs::hard_link(temp_path, path)
    }
}
