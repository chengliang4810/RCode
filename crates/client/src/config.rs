//! Construction inputs for the user-configured host and this device.
//! The platform persists pairing credentials in Keychain / Keystore.

use std::path::PathBuf;

/// Static per-install configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    /// User-configured host base URL. The historical field name is retained
    /// for platform bindings; no deployment is selected by default.
    pub edge_url: String,
    /// Writable directory for registry/doc snapshots and caches. Scoped by the
    /// platform to the paired identity (a different pairing starts fresh).
    pub data_dir: PathBuf,
    /// This device's stable id (`ios-xxxxxxxx`) — stamped on every command
    /// and queue row this device writes.
    pub device_id: String,
    /// Human name for presence/attribution ("Wing's iPhone").
    pub device_name: String,
    /// `ios` / `android` — viewer platforms never host sessions.
    pub platform: String,
    /// App version string (diagnostics only).
    pub app_version: String,
}

impl ClientConfig {
    pub fn new(edge_url: impl Into<String>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            edge_url: edge_url.into(),
            data_dir: data_dir.into(),
            device_id: format!("ios-{}", &crate::new_id()[..8]),
            device_name: "iPhone".into(),
            platform: "ios".into(),
            app_version: env!("CARGO_PKG_VERSION").into(),
        }
    }

    pub(crate) fn edge_base(&self) -> &str {
        self.edge_url.trim_end_matches('/')
    }
}

/// How this client authenticates against the host it was pointed at.
///
/// A client carries the bearer its host expects; it has no account session
/// or refresh token.
#[derive(Clone, PartialEq, Eq)]
pub enum Credentials {
    /// The bearer is `userId@orgId`.
    Paired { user_id: String, org_id: String },
    /// Fully offline, deterministic dataset with a simulated host.
    Demo(DemoOptions),
    /// Direct engine access. The platform stores this secret in its key store.
    Direct { api_key: String },
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct { .. } => f.write_str("Direct { api_key: <redacted> }"),
            Self::Paired { user_id, org_id } => f.debug_struct("Paired")
                .field("user_id", user_id).field("org_id", org_id).finish(),
            Self::Demo(options) => f.debug_tuple("Demo").field(options).finish(),
        }
    }
}

impl Credentials {
    pub fn is_demo(&self) -> bool {
        matches!(self, Credentials::Demo(_))
    }

    pub fn org_id(&self) -> &str {
        match self {
            Credentials::Direct { .. } => "direct",
            Credentials::Paired { org_id, .. } => org_id,
            Credentials::Demo(_) => "demo",
        }
    }

    pub fn user_id(&self) -> &str {
        match self {
            Credentials::Direct { .. } => "direct",
            Credentials::Paired { user_id, .. } => user_id,
            Credentials::Demo(_) => "demo",
        }
    }
}

/// Demo-mode knobs (the legacy app's `-demo` launch args).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoOptions {
    pub fixture: DemoFixture,
    /// Size of the flagship chat's transcript (`chat-veil`).
    pub transcript_scale: TranscriptScale,
    pub stream_speed: StreamSpeed,
    /// Repeat the scripted reply 12× (long streaming stress, `-longreply`).
    pub long_reply: bool,
}

impl Default for DemoOptions {
    fn default() -> Self {
        Self {
            fixture: DemoFixture::Standard,
            transcript_scale: TranscriptScale::Normal,
            stream_speed: StreamSpeed::Realistic,
            long_reply: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoFixture {
    /// Devices, projects, pinned + sectioned sessions, PRs in every state.
    Standard,
    /// No projects and no sessions (`-no-projects`): empty-state screens.
    NoProjects,
    /// Only this phone — no execution hosts (`-ios-only`).
    IosOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptScale {
    /// Hand-written fixtures.
    Normal,
    /// 120 synthetic turns (`-big`).
    Big,
    /// 600 synthetic turns (`-huge`).
    Huge,
    /// Arbitrary synthetic size (benchmarks).
    Turns(u32),
}

impl TranscriptScale {
    pub fn turns(self) -> Option<u32> {
        match self {
            TranscriptScale::Normal => None,
            TranscriptScale::Big => Some(120),
            TranscriptScale::Huge => Some(600),
            TranscriptScale::Turns(n) => Some(n),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamSpeed {
    /// Word by word at 30–140ms — what a real harness feels like.
    Realistic,
    /// A word every ~4ms: stresses the per-update path (benchmarks).
    Fast,
}
