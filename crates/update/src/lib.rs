//! rcode-update — release checking and self-update, shared by the engine (the
//! background checker + `ApplyUpdate`), the CLI (`rcode update`), and the UI
//! (its own report-only checker, the "Check for Updates…" menu item, the
//! sidebar update strip, and the desktop install paths).
//!
//! Release layout (see `.github/workflows/release.yml` and `dist/install.sh`):
//! artifacts are release assets on GitHub, fetched from
//! `.../releases/latest/download/*` — no service of our own is involved.
//! `manifest.json` carries the latest version plus a sha256 per artifact;
//! `latest.txt` (version only) remains as the fallback for releases published
//! before the manifest existed.
//!
//! Install kinds and their update paths:
//! - **Managed** (`~/.rcode/app/<ver>` + `current` symlink — the curl|sh
//!   installer and the Linux tarball's `install.sh`): download the headless
//!   tarball into a new versioned dir, flip the symlink, then restart the
//!   service (daemon) or relaunch (desktop). Same flow the installer script
//!   performs, natively.
//! - **MacApp** (running out of an app bundle): download the app tarball, swap the
//!   bundle directory, relaunch. Driven by the UI.
//! - **WindowsPortable** (the Windows installer or portable zip — both carry
//!   `rcode-update.json`): swap the executable in place. Driven by the UI.
//! - **Unmanaged** (source builds, hand-copied binaries): report only — the
//!   UI's advisory strip links to [`RELEASES_PAGE`].
//!
//! Desktop installs download as soon as a release is seen and install either
//! on "Restart to update" or when the app next quits, so an app nobody
//! restarts deliberately still lands on the new version at its next launch.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, bail};
use futures::StreamExt as _;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

#[cfg(windows)]
pub mod windows;

/// The version compiled into this binary (the workspace version).
pub const fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Successful checks repeat this often. The feed is a sub-kilobyte,
/// edge-cached document, so hourly polling is free and bounds how long a
/// long-running app can sit on a stale release.
const CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// Failed checks (offline boot, captive portal, transient edge error) back off
/// through these delays, then stay at the last one.
const CHECK_RETRY: [Duration; 4] = [
    Duration::from_secs(60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(15 * 60),
    Duration::from_secs(30 * 60),
];
/// Check deadlines are wall-clock times: a monotonic sleep stops counting
/// while the machine sleeps, so a laptop closed overnight used to push its
/// next check out by the whole night. The loop re-reads the clock this often.
const SCHEDULE_TICK: Duration = Duration::from_secs(60);
/// The engine's first check waits out engine boot (room joins, doc re-sync).
const ENGINE_INITIAL_DELAY: Duration = Duration::from_secs(20);
/// The desktop checker only needs the window on screen first.
const DESKTOP_INITIAL_DELAY: Duration = Duration::from_secs(2);
/// While an auto-apply is deferred behind active sessions, re-probe idleness
/// this often.
const IDLE_RECHECK: Duration = Duration::from_secs(5 * 60);
/// A staged binary must answer `--version` within this window.
const STAGED_VERSION_TIMEOUT: Duration = Duration::from_secs(30);
/// Release metadata is tiny. Bound both its buffered size and total transfer
/// time so a compromised or misconfigured public feed cannot hold a checker
/// forever or make every local install buffer an unbounded response.
const RELEASE_METADATA_MAX_BYTES: usize = 1024 * 1024;
const RELEASE_METADATA_TIMEOUT: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Release metadata
// ---------------------------------------------------------------------------

/// `{edge}/releases/manifest.json` — written by the release workflow.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    /// Artifact file name → metadata. Empty for pre-manifest releases resolved
    /// via `latest.txt`; such releases can be discovered but cannot be installed.
    #[serde(default)]
    pub files: BTreeMap<String, FileMeta>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileMeta {
    #[serde(default)]
    pub sha256: Option<String>,
}

/// Artifact-name platform pair matching the packaging scripts. Unsupported
/// updater targets retain their real OS name rather than impersonating Linux.
pub fn platform_key() -> (&'static str, &'static str) {
    let os = std::env::consts::OS;
    let arch = match (os, std::env::consts::ARCH) {
        ("macos", "aarch64") => "arm64",
        (_, arch) => arch,
    };
    (os, arch)
}

fn managed_updates_supported(os: &str) -> bool {
    matches!(os, "linux" | "macos")
}

fn require_managed_update_platform() -> anyhow::Result<()> {
    if !managed_updates_supported(std::env::consts::OS) {
        bail!(
            "managed updates are not supported on {}",
            std::env::consts::OS
        );
    }
    Ok(())
}

fn require_mac_app_update_platform() -> anyhow::Result<()> {
    if std::env::consts::OS != "macos" {
        bail!(
            "macOS app updates are not supported on {}",
            std::env::consts::OS
        );
    }
    Ok(())
}

/// `rcode-<ver>-<os>-<arch>.tar.gz` — the headless/CLI tarball (Linux CI builds).
pub fn headless_artifact(version: &str) -> String {
    let (os, arch) = platform_key();
    format!("rcode-{version}-{os}-{arch}.tar.gz")
}

/// `rcode-<ver>-macos-<arch>-app.tar.gz` — the macOS app update payload.
pub fn mac_app_artifact(version: &str) -> String {
    let (_, arch) = platform_key();
    format!("rcode-{version}-macos-{arch}-app.tar.gz")
}

fn parse_version(v: &str) -> Option<(Vec<u64>, semver::Prerelease)> {
    let v = v.trim().strip_prefix('v').unwrap_or(v.trim());
    let core = if let Some((core, build)) = v.split_once('+') {
        if build.is_empty() {
            return None;
        }
        semver::BuildMetadata::new(build).ok()?;
        core
    } else {
        v
    };
    let (core, prerelease) = if let Some((core, pre)) = core.split_once('-') {
        if pre.is_empty() {
            return None;
        }
        (core, semver::Prerelease::new(pre).ok()?)
    } else {
        (core, semver::Prerelease::EMPTY)
    };
    // Historical dotted-numeric versions such as 0.1 and 0.1.0.1 remain valid.
    let numbers = core
        .split('.')
        .map(|part| {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        })
        .collect::<Option<Vec<u64>>>()?;
    (!numbers.is_empty()).then_some((numbers, prerelease))
}

/// Compare numeric versions and SemVer prereleases. Stable releases sort after
/// prereleases of the same version; build metadata does not change precedence.
/// Invalid versions never trigger an update.
pub fn version_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// Fetch the newest release metadata: `manifest.json`, falling back to
/// `latest.txt` (version only, no checksums) for pre-manifest releases.
pub async fn fetch_latest(release_base: &str) -> anyhow::Result<Manifest> {
    let base = resolve_release_base(release_base)?;
    let client = http_client()?;
    let manifest_url = format!("{base}/manifest.json");
    match fetch_release_metadata(&client, &manifest_url).await {
        Ok(response) if response.status.is_success() => {
            let manifest: Manifest =
                serde_json::from_slice(&response.body).context("parsing manifest.json")?;
            if manifest.version.trim().is_empty() {
                bail!("manifest.json has an empty version");
            }
            return Ok(manifest);
        }
        Ok(response) => {
            tracing::debug!(status = %response.status, "manifest.json unavailable; trying latest.txt")
        }
        Err(err) => tracing::debug!(error = %err, "manifest.json fetch failed; trying latest.txt"),
    }
    let latest_url = format!("{base}/latest.txt");
    let response = fetch_release_metadata(&client, &latest_url)
        .await
        .context("fetching latest.txt")?;
    anyhow::ensure!(
        response.status.is_success(),
        "fetching latest.txt: HTTP {}",
        response.status
    );
    let version = std::str::from_utf8(&response.body)
        .context("reading latest.txt as UTF-8")?
        .trim()
        .to_string();
    if version.is_empty() {
        bail!("latest.txt is empty");
    }
    Ok(Manifest {
        version,
        files: BTreeMap::new(),
    })
}

#[derive(Debug)]
struct ReleaseMetadataResponse {
    status: reqwest::StatusCode,
    body: Vec<u8>,
}

async fn fetch_release_metadata(
    client: &reqwest::Client,
    url: &str,
) -> anyhow::Result<ReleaseMetadataResponse> {
    fetch_release_metadata_with_limits(
        client,
        url,
        RELEASE_METADATA_TIMEOUT,
        RELEASE_METADATA_MAX_BYTES,
    )
    .await
}

async fn fetch_release_metadata_with_limits(
    client: &reqwest::Client,
    url: &str,
    timeout: Duration,
    max_bytes: usize,
) -> anyhow::Result<ReleaseMetadataResponse> {
    // This response defines the trusted artifact digests. A mirror must not
    // be allowed to replace both the artifact and its expected checksum.
    let response = fetch_release_metadata_attempt(client, url, timeout, max_bytes).await?;
    anyhow::ensure!(
        response.status.is_success(),
        "release metadata HTTP {}",
        response.status
    );
    if url.ends_with("/manifest.json") {
        let manifest: Manifest =
            serde_json::from_slice(&response.body).context("parsing manifest.json")?;
        anyhow::ensure!(
            parse_version(&manifest.version).is_some(),
            "manifest.json has an invalid version"
        );
    } else if url.ends_with("/latest.txt") {
        anyhow::ensure!(
            parse_version(std::str::from_utf8(&response.body)?).is_some(),
            "latest.txt has an invalid version"
        );
    }
    Ok(response)
}

async fn fetch_release_metadata_attempt(
    client: &reqwest::Client,
    url: &str,
    timeout: Duration,
    max_bytes: usize,
) -> anyhow::Result<ReleaseMetadataResponse> {
    let request = async {
        let response = client
            .get(url)
            // Intermediaries must revalidate: the edge already bounds its own
            // cache to a minute, and a proxy holding an old manifest for
            // longer is exactly the "prompt appears late" failure.
            .header(reqwest::header::CACHE_CONTROL, "no-cache")
            .send()
            .await
            .with_context(|| format!("fetching {url}"))?;
        let status = response.status();
        if !status.is_success() {
            return Ok(ReleaseMetadataResponse {
                status,
                body: Vec::new(),
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            bail!("release metadata exceeds {max_bytes} bytes");
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("reading release metadata")?;
            anyhow::ensure!(
                body.len().saturating_add(chunk.len()) <= max_bytes,
                "release metadata exceeds {max_bytes} bytes"
            );
            body.extend_from_slice(&chunk);
        }
        Ok(ReleaseMetadataResponse { status, body })
    };
    tokio::time::timeout(timeout, request)
        .await
        .with_context(|| format!("fetching {url} exceeded the metadata deadline"))?
}

fn http_client() -> anyhow::Result<reqwest::Client> {
    http_client_with_timeouts(Duration::from_secs(8), Duration::from_secs(20))
}

fn http_client_with_timeouts(connect: Duration, read: Duration) -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(connect)
        // Inactivity timeout, not a total download cap: slow progressing
        // updates remain viable on constrained links.
        .read_timeout(read)
        .user_agent(concat!("rcode/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("too many update redirects");
            }
            if attempt.previous().iter().any(|url| url.scheme() == "https")
                && attempt.url().scheme() != "https"
            {
                return attempt.error("update redirect would downgrade HTTPS");
            }
            attempt.follow()
        }))
        .build()
        .context("building http client")
}

fn validate_release_override(value: &str) -> anyhow::Result<String> {
    let url = reqwest::Url::parse(value.trim()).context("invalid update feed URL")?;
    anyhow::ensure!(
        url.scheme() == "https" && url.host_str().is_some(),
        "update feed must use HTTPS"
    );
    anyhow::ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "update feed must be a base URL without credentials, query, or fragment"
    );
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

/// The project's GitHub releases page — the advisory update strip opens this
/// for unmanaged installs (source builds, hand-copied binaries), where no
/// updater flow exists to drive.
pub const RELEASES_PAGE: &str = "https://github.com/chengliang4810/RCode/releases";

/// The newest release's page — the download destination offered when this
/// installation cannot replace itself.
pub const LATEST_RELEASE_PAGE: &str = "https://github.com/chengliang4810/RCode/releases/latest";

/// Where release artifacts and the updater manifest are fetched from.
///
/// GitHub Releases. The desktop build has no service of its own to serve them —
/// the edge this used to be derived from is part of the cloud surface this
/// project does not have — and a release asset URL is the one part of
/// publishing that needs no infrastructure at all.
pub const RELEASE_BASE: &str = "https://github.com/chengliang4810/RCode/releases/latest/download";

/// [`RELEASE_BASE`], unless `RCODE_RELEASES_URL` names a mirror.
///
/// The override exists for a local feed and for tests, which point it at an
/// address that cannot answer so a download path can be exercised without
/// reaching the network.
pub fn default_release_base() -> String {
    std::env::var("RCODE_RELEASES_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| RELEASE_BASE.to_owned())
}

fn resolve_release_base(base: &str) -> anyhow::Result<String> {
    if let Ok(url) = std::env::var("RCODE_RELEASES_URL")
        && !url.trim().is_empty()
    {
        return validate_release_override(&url);
    }
    #[cfg(windows)]
    if let Some(url) = windows::release_url()? {
        return Ok(url.trim_end_matches('/').to_owned());
    }
    Ok(base.trim_end_matches('/').to_owned())
}

// ---------------------------------------------------------------------------
// Install-kind detection
// ---------------------------------------------------------------------------

/// How this binary was installed — decides the update path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallKind {
    /// `~/.rcode/app/<ver>/rcode` behind the `current` symlink
    /// (curl|sh installer, the Linux tarball's `install.sh`, or a previous
    /// `rcode update`).
    Managed { app_root: PathBuf },
    /// Running out of a macOS `.app` bundle.
    MacApp { bundle: PathBuf },
    /// Windows installer or portable package with an explicit update-feed
    /// configuration.
    #[cfg(windows)]
    WindowsPortable { directory: PathBuf },
    /// Source build or hand-copied binary — updates are report-only.
    Unmanaged,
}

/// Why an installation that normally updates itself cannot right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateBlocker {
    /// macOS App Translocation ran the app from a randomized read-only path
    /// (launched from Downloads or a disk image without being moved first).
    Translocated,
    /// Running straight from the mounted disk image.
    DiskImage,
    /// The directory holding the installation is not writable by this user.
    NotWritable(PathBuf),
}

impl std::fmt::Display for UpdateBlocker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Translocated | Self::DiskImage => f.write_str(
                "RCode is running from a temporary, read-only location. Move RCode to your \
                 Applications folder and reopen it to turn on updates.",
            ),
            Self::NotWritable(dir) => write!(
                f,
                "RCode doesn't have permission to replace itself in {}.",
                dir.display()
            ),
        }
    }
}

impl InstallKind {
    /// Whether the desktop app downloads and installs updates itself.
    pub fn supports_desktop_update(&self) -> bool {
        match self {
            Self::MacApp { .. } => true,
            // The desktop path relaunches a GUI binary; on Linux that binary
            // is the managed install itself.
            Self::Managed { .. } => cfg!(target_os = "linux"),
            #[cfg(windows)]
            Self::WindowsPortable { .. } => true,
            Self::Unmanaged => false,
        }
    }

    /// Why this installation cannot replace itself, when it can't. Checked
    /// before downloading so the UI can explain the fix instead of failing
    /// after a wasted download with a raw rename error.
    pub fn desktop_update_blocker(&self) -> Option<UpdateBlocker> {
        match self {
            Self::MacApp { bundle } => mac_bundle_blocker(bundle, dir_writable),
            Self::Managed { app_root } => {
                (!dir_writable(app_root)).then(|| UpdateBlocker::NotWritable(app_root.clone()))
            }
            #[cfg(windows)]
            Self::WindowsPortable { directory } => {
                (!dir_writable(directory)).then(|| UpdateBlocker::NotWritable(directory.clone()))
            }
            Self::Unmanaged => None,
        }
    }

    pub async fn stage_desktop(
        &self,
        release_base: &str,
        manifest: &Manifest,
        data_dir: &Path,
    ) -> anyhow::Result<PathBuf> {
        match self {
            Self::MacApp { .. } => stage_mac_app(release_base, manifest, data_dir).await,
            Self::Managed { app_root } if self.supports_desktop_update() => {
                stage_headless(release_base, manifest, app_root).await
            }
            #[cfg(windows)]
            Self::WindowsPortable { directory } => {
                windows::stage(release_base, manifest, directory).await
            }
            _ => bail!("this installation does not support desktop updates"),
        }
    }

    /// Install a staged update. With `relaunch`, the new version starts once
    /// this process exits ("Restart to update"); without it the update takes
    /// effect at the next launch ("install on quit"). Either way the caller
    /// must quit after this succeeds.
    pub fn apply_desktop(&self, staged: &Path, relaunch: bool) -> anyhow::Result<()> {
        match self {
            Self::MacApp { bundle } => {
                apply_mac_app(staged, bundle)?;
                // Another process may still be copying this shared cache.
                // Reclamation needs explicit ownership/leases, not a PathBuf.
                if relaunch {
                    relaunch_after_exit(Path::new("/usr/bin/open"), bundle);
                }
                Ok(())
            }
            Self::Managed { app_root } if self.supports_desktop_update() => {
                let version = staged
                    .file_name()
                    .and_then(|name| name.to_str())
                    .context("staged install has no version directory")?;
                apply_headless(app_root, version)?;
                if relaunch {
                    let binary = app_root.join("current").join("rcode");
                    relaunch_after_exit(&binary, Path::new(""));
                }
                Ok(())
            }
            #[cfg(windows)]
            Self::WindowsPortable { directory } => windows::apply(staged, directory, relaunch),
            _ => bail!("this installation does not support desktop updates"),
        }
    }
}

fn mac_bundle_blocker(bundle: &Path, writable: impl Fn(&Path) -> bool) -> Option<UpdateBlocker> {
    if bundle
        .components()
        .any(|part| part.as_os_str() == "AppTranslocation")
    {
        return Some(UpdateBlocker::Translocated);
    }
    let parent = bundle.parent()?;
    if writable(parent) {
        return None;
    }
    if bundle.starts_with("/Volumes") {
        return Some(UpdateBlocker::DiskImage);
    }
    Some(UpdateBlocker::NotWritable(parent.to_path_buf()))
}

/// Probe by creating (and removing) a file: permission bits alone miss
/// read-only mounts, ACLs, and sandboxed locations.
fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".rcode-write-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            true
        }
        // A leftover probe from a crashed run still proves writability.
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

pub fn detect_install() -> InstallKind {
    let Ok(exe) = std::env::current_exe() else {
        return InstallKind::Unmanaged;
    };
    let home = std::env::var_os("HOME").map(PathBuf::from);
    detect_install_from(&exe, home.as_deref())
}

fn detect_install_from(exe: &Path, home: Option<&Path>) -> InstallKind {
    detect_install_from_for_os(exe, home, std::env::consts::OS)
}

fn detect_install_from_for_os(exe: &Path, home: Option<&Path>, os: &str) -> InstallKind {
    #[cfg(windows)]
    if os == "windows" && windows::is_managed(exe) {
        return InstallKind::WindowsPortable {
            directory: exe.parent().unwrap().to_owned(),
        };
    }
    // Never interpret a coincidental Windows `%HOME%\.rcode\app` layout as
    // the Unix symlink-managed installation.
    if !managed_updates_supported(os) {
        return InstallKind::Unmanaged;
    }
    if let Some(home) = home {
        // `current_exe` resolves the `current` symlink to the versioned dir.
        let app_root = home.join(".rcode").join("app");
        if exe.starts_with(&app_root) {
            return InstallKind::Managed { app_root };
        }
    }
    for ancestor in exe.ancestors() {
        if ancestor.extension().is_some_and(|ext| ext == "app")
            && exe.starts_with(ancestor.join("Contents").join("MacOS"))
        {
            return InstallKind::MacApp {
                bundle: ancestor.to_path_buf(),
            };
        }
    }
    InstallKind::Unmanaged
}

/// The version currently installed on disk for `kind`, read without executing
/// anything. A long-running process compares it with its own version to notice
/// that something else (the desktop app, `rcode update`, a re-run installer)
/// has already replaced its binary.
pub fn installed_version(kind: &InstallKind) -> Option<String> {
    let version = match kind {
        InstallKind::Managed { app_root } => std::fs::read_link(app_root.join("current"))
            .ok()?
            .file_name()?
            .to_str()?
            .to_owned(),
        InstallKind::MacApp { bundle } => bundle_short_version(
            &std::fs::read_to_string(bundle.join("Contents/Info.plist")).ok()?,
        )?,
        _ => return None,
    };
    parse_version(&version).map(|_| version)
}

/// Full release version from the packaging template. Apple's bundle version
/// fields contain numeric versions; older bundles stored the release version
/// directly in `CFBundleShortVersionString`.
fn bundle_short_version(plist: &str) -> Option<String> {
    for key in ["RCodeVersion", "CFBundleShortVersionString"] {
        let Some((_, after_key)) = plist.split_once(&format!("<key>{key}</key>")) else {
            continue;
        };
        let value = after_key.trim_start().strip_prefix("<string>")?;
        let end = value.find("</string>")?;
        return Some(value[..end].trim().to_owned());
    }
    None
}

// ---------------------------------------------------------------------------
// Download + verify
// ---------------------------------------------------------------------------

/// Stream `{edge}/releases/<file>` to `dest`, requiring the manifest SHA-256.
/// Writes through a `.partial` sidecar so an interrupted download never
/// leaves a plausible-looking artifact behind.
pub async fn download_release_file(
    release_base: &str,
    manifest: &Manifest,
    file: &str,
    dest: &Path,
) -> anyhow::Result<()> {
    let expected = release_checksum(manifest, file)?;
    let url = format!("{}/{file}", resolve_release_base(release_base)?);
    rcode_github::download(
        &http_client()?,
        &url,
        dest,
        Some(expected),
        4 * 1024 * 1024 * 1024,
    )
    .await
}

fn release_checksum<'a>(manifest: &'a Manifest, file: &str) -> anyhow::Result<&'a str> {
    let expected = manifest
        .files
        .get(file)
        .and_then(|m| m.sha256.as_deref())
        .with_context(|| format!("updates require a SHA-256 checksum for {file}"))?;
    anyhow::ensure!(
        expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid SHA-256 checksum for {file}"
    );
    Ok(expected)
}

#[derive(Serialize, Deserialize)]
struct StageVerification {
    artifact_sha256: String,
    executable_sha256: String,
}

fn executable_digest(binary: &Path) -> anyhow::Result<String> {
    use sha2::{Digest as _, Sha256};
    let mut source = std::fs::File::open(binary)?;
    let mut hash = Sha256::new();
    std::io::copy(&mut source, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

fn record_verified_stage(root: &Path, binary: &Path, expected: &str) -> anyhow::Result<()> {
    let verification = StageVerification {
        artifact_sha256: expected.to_owned(),
        executable_sha256: executable_digest(binary)?,
    };
    std::fs::write(
        root.join(".rcode-verified.json"),
        serde_json::to_vec(&verification)?,
    )?;
    Ok(())
}

/// Old, incomplete or modified caches must never be executed as a version
/// probe. The marker is written only after a checksum-verified extraction.
#[cfg(test)]
fn cached_stage_verified(root: &Path, binary: &Path, expected: &str) -> bool {
    verified_stage_record(root, binary)
        .is_some_and(|record| record.artifact_sha256.eq_ignore_ascii_case(expected))
}

fn verified_stage_record(root: &Path, binary: &Path) -> Option<StageVerification> {
    let record: StageVerification =
        serde_json::from_slice(&std::fs::read(root.join(".rcode-verified.json")).ok()?).ok()?;
    (record.artifact_sha256.len() == 64
        && record
            .artifact_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        && record.executable_sha256 == executable_digest(binary).ok()?)
    .then_some(record)
}

/// Verified version directories are immutable: other callers can still hold
/// their paths as an apply source. A changed artifact needs a new version.
fn reusable_verified_stage(root: &Path, binary: &Path, expected: &str) -> anyhow::Result<bool> {
    let Some(record) = verified_stage_record(root, binary) else {
        return Ok(false);
    };
    anyhow::ensure!(
        record.artifact_sha256.eq_ignore_ascii_case(expected),
        "release artifact conflict at {}: a verified cache has a different SHA-256; publish the changed artifact under a new version",
        root.display()
    );
    Ok(true)
}

fn run(program: &str, args: &[&str]) -> anyhow::Result<()> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("running {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} {} failed ({}): {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Run a freshly unpacked binary's `--version` before it can replace anything:
/// proves the payload is complete, executable on this machine, and the release
/// the manifest promised.
async fn verify_staged_binary(binary: &Path, version: &str) -> anyhow::Result<()> {
    let output = tokio::time::timeout(
        STAGED_VERSION_TIMEOUT,
        tokio::process::Command::new(binary)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("staged binary version check timed out")?
    .with_context(|| format!("running {} --version", binary.display()))?;
    let reported = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    anyhow::ensure!(
        output.status.success() && reported == format!("rcode {version}"),
        "staged binary reported {reported:?} (exit {}), expected \"rcode {version}\"",
        output.status
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Managed (symlink) installs — the daemon/VPS path and Linux desktop
// ---------------------------------------------------------------------------

fn update_stage_directory(app_root: &Path, version: &str) -> anyhow::Result<tempfile::TempDir> {
    std::fs::create_dir_all(app_root)?;
    tempfile::Builder::new()
        .prefix(&format!(".stage-{version}-{}-", std::process::id()))
        .tempdir_in(app_root)
        .with_context(|| format!("creating a stage in {}", app_root.display()))
}

/// Serialize only cache replacement, including across CLI/desktop processes.
/// Downloading and version probes never hold this per-installation file lock.
fn promote_verified_stage(
    unpacked: &Path,
    dest: &Path,
    binary_relative: &Path,
    expected: &str,
) -> anyhow::Result<()> {
    let app_root = dest.parent().context("release stage has no parent")?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(app_root.join(".stage-promote.lock"))?;
    lock.lock().context("locking release stage promotion")?;
    if reusable_verified_stage(dest, &dest.join(binary_relative), expected)? {
        return Ok(());
    }
    if dest.exists() {
        std::fs::remove_dir_all(dest).context("removing an unverified cached stage")?;
    }
    std::fs::rename(unpacked, dest).with_context(|| format!("moving {} into place", dest.display()))
}

/// Download + unpack the headless tarball into `app_root/<ver>` (idempotent —
/// an already-staged version is reused). Returns the versioned dir.
pub async fn stage_headless(
    release_base: &str,
    manifest: &Manifest,
    app_root: &Path,
) -> anyhow::Result<PathBuf> {
    // Reject unsupported targets before creating a stage or making a request.
    require_managed_update_platform()?;
    let version = &manifest.version;
    anyhow::ensure!(
        parse_version(version).is_some(),
        "invalid release version {version:?}"
    );
    let file = headless_artifact(version);
    let expected = release_checksum(manifest, &file)?;
    let dest = app_root.join(version);
    if reusable_verified_stage(&dest, &dest.join("rcode"), expected)? {
        return Ok(dest);
    }
    // Each caller owns its directory. Dropping this guard also cleans up when
    // the download future is cancelled, without touching another caller's stage.
    let stage_directory = update_stage_directory(app_root, version)?;
    let stage = stage_directory.path();
    let result = async {
        let tarball = stage.join(&file);
        download_release_file(release_base, manifest, &file, &tarball).await?;
        let unpacked = stage.join("unpacked");
        std::fs::create_dir_all(&unpacked)?;
        // Tarball root is the versioned stage dir (see scripts/package-linux.sh);
        // strip it exactly as install.sh does.
        run(
            "tar",
            &[
                "-xzf",
                &tarball.to_string_lossy(),
                "-C",
                &unpacked.to_string_lossy(),
                "--strip-components=1",
            ],
        )?;
        if !unpacked.join("rcode").is_file() {
            bail!("tarball {file} did not contain a rcode binary");
        }
        verify_staged_binary(&unpacked.join("rcode"), version).await?;
        record_verified_stage(&unpacked, &unpacked.join("rcode"), expected)?;
        promote_verified_stage(&unpacked, &dest, Path::new("rcode"), expected)?;
        Ok(dest.clone())
    }
    .await;
    result
}

/// Atomically repoint `app_root/current` at `app_root/<ver>` (symlink to a temp
/// name, then rename over — never a window with no `current`).
pub fn apply_headless(app_root: &Path, version: &str) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let target = app_root.join(version);
        if !target.join("rcode").exists() {
            bail!("{} is not a staged install", target.display());
        }
        let temporary = tempfile::Builder::new()
            .prefix(".current-")
            .tempdir_in(app_root)?;
        let tmp = temporary.path().join("next");
        std::os::unix::fs::symlink(&target, &tmp).context("creating current symlink")?;
        std::fs::rename(&tmp, app_root.join("current")).context("swapping current symlink")?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (app_root, version);
        require_managed_update_platform()?;
        unreachable!("supported managed-update platforms are Unix")
    }
}

/// Whether this process is the engine service [`restart_service`] manages:
/// systemd places it in the `rcode.service` cgroup; launchd names the job in
/// `XPC_SERVICE_NAME`.
pub fn running_as_installed_service() -> bool {
    if cfg!(target_os = "macos") {
        std::env::var("XPC_SERVICE_NAME").is_ok_and(|label| label == "com.rcode")
    } else if cfg!(target_os = "linux") {
        std::fs::read_to_string("/proc/self/cgroup")
            .is_ok_and(|cgroups| in_rcode_service_cgroup(&cgroups))
    } else {
        false
    }
}

fn in_rcode_service_cgroup(cgroups: &str) -> bool {
    cgroups
        .lines()
        .filter_map(|line| line.rsplit(':').next())
        .any(|path| path.split('/').any(|part| part == "rcode.service"))
}

/// Restart the installed engine service (the same units `rcode daemon` and the
/// curl|sh installer manage). Called after a symlink swap so the running daemon
/// picks up the new binary. Only queues the restart: the caller may be the
/// service itself, which must stay responsive to the stop signal that follows.
pub fn restart_service() -> anyhow::Result<()> {
    require_managed_update_platform()?;
    if cfg!(target_os = "macos") {
        let output = std::process::Command::new("id").arg("-u").output()?;
        let uid = String::from_utf8_lossy(&output.stdout).trim().to_string();
        run(
            "launchctl",
            &["kickstart", "-k", &format!("gui/{uid}/com.rcode")],
        )
    } else {
        run(
            "systemctl",
            &["--user", "--no-block", "restart", "rcode.service"],
        )
    }
}

// ---------------------------------------------------------------------------
// macOS app-bundle installs — the desktop path
// ---------------------------------------------------------------------------

/// Download + unpack the app tarball into `{data_dir}/updates/<ver>/RCode.app`
/// (idempotent). The bundle is unpacked beside its final name and renamed in
/// only after its binary answers with the expected version, so an interrupted
/// unpack can never be mistaken for a staged update. Returns the staged bundle.
pub async fn stage_mac_app(
    release_base: &str,
    manifest: &Manifest,
    data_dir: &Path,
) -> anyhow::Result<PathBuf> {
    // Reject unsupported targets before creating a stage or making a request.
    require_mac_app_update_platform()?;
    let version = &manifest.version;
    anyhow::ensure!(
        parse_version(version).is_some(),
        "invalid release version {version:?}"
    );
    let file = mac_app_artifact(version);
    let expected = release_checksum(manifest, &file)?;
    let updates = data_dir.join("updates");
    let dir = updates.join(version);
    let staged = dir.join("RCode.app");
    let staged_binary = staged.join("Contents/MacOS/rcode");
    if reusable_verified_stage(&dir, &staged_binary, expected)? {
        return Ok(staged);
    }
    let stage_directory = update_stage_directory(&updates, version)?;
    let tarball = stage_directory.path().join(&file);
    download_release_file(release_base, manifest, &file, &tarball).await?;
    let unpack = stage_directory.path().join("unpacked");
    std::fs::create_dir_all(&unpack)?;
    run(
        "tar",
        &[
            "-xzf",
            &tarball.to_string_lossy(),
            "-C",
            &unpack.to_string_lossy(),
        ],
    )?;
    let unpacked = unpack.join("RCode.app");
    let unpacked_binary = unpacked.join("Contents/MacOS/rcode");
    if !unpacked_binary.exists() {
        bail!("app tarball {file} did not contain RCode.app");
    }
    verify_staged_binary(&unpacked_binary, version).await?;
    record_verified_stage(&unpack, &unpacked_binary, expected)?;
    promote_verified_stage(
        &unpack,
        &dir,
        Path::new("RCode.app/Contents/MacOS/rcode"),
        expected,
    )?;
    Ok(staged)
}

/// Swap the installed bundle for the staged one: `ditto` the staged copy next to
/// the target (metadata-preserving, cross-volume safe), then two renames — the
/// old bundle is restored if the second rename fails.
pub fn apply_mac_app(staged: &Path, bundle: &Path) -> anyhow::Result<()> {
    require_mac_app_update_platform()?;
    apply_mac_app_transaction(
        staged,
        bundle,
        |source, fresh| {
            run(
                "ditto",
                &[&source.to_string_lossy(), &fresh.to_string_lossy()],
            )
        },
        |from, to| std::fs::rename(from, to),
    )
}

/// Resolve the target's parent and any final symlink without requiring the
/// bundle itself to exist during another install's brief rename boundary.
fn bundle_install_target(bundle: &Path) -> anyhow::Result<PathBuf> {
    let mut target = bundle.to_owned();
    for _ in 0..32 {
        let parent = target
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let name = target.file_name().context("app bundle has no name")?;
        let parent = parent
            .canonicalize()
            .context("resolving app installation directory")?;
        let candidate = parent.join(name);
        match std::fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let link = std::fs::read_link(&candidate)?;
                target = if link.is_absolute() {
                    link
                } else {
                    parent.join(link)
                };
            }
            Ok(_) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Err(error) => return Err(error.into()),
        }
    }
    bail!("app bundle has too many symbolic links")
}

fn apply_mac_app_transaction(
    staged: &Path,
    bundle: &Path,
    copy: impl FnOnce(&Path, &Path) -> anyhow::Result<()>,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> anyhow::Result<()> {
    let bundle = bundle_install_target(bundle)?;
    let parent = bundle
        .parent()
        .context("app bundle has no parent directory")?;
    let mut lock_name = std::ffi::OsString::from(".");
    lock_name.push(bundle.file_name().context("app bundle has no name")?);
    lock_name.push(".update.lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(parent.join(lock_name))?;
    // Stable per actual destination, shared by GUI/CLI and independent opens.
    // Only copying, swapping and rollback hold this lock; no download or UI wait.
    lock.lock().context("locking app bundle installation")?;
    let transaction = tempfile::Builder::new()
        .prefix(".rcode-install-")
        .tempdir_in(parent)?;
    let fresh = transaction.path().join("new.app");
    let old = transaction.path().join("old.app");
    copy(staged, &fresh).context("copying the staged app bundle")?;
    rename(&bundle, &old).context("moving the current app aside")?;
    if let Err(error) = rename(&fresh, &bundle) {
        if let Err(rollback) = rename(&old, &bundle) {
            // This is the only surviving installation. Never let TempDir's
            // destructor reclaim it when rollback could not restore its name.
            let recovery = transaction.keep().join("old.app");
            bail!(
                "installing the new app bundle failed: {error}; rollback failed: {rollback}; original app preserved at {}",
                recovery.display()
            );
        }
        return Err(error).context("installing the new app bundle; original app restored");
    }
    Ok(())
}

/// Detached relauncher: waits for THIS process to exit, then runs `program`
/// (with `argument` unless empty). Launching before exit would race the
/// single-instance engine lock and the IPC port. Paths travel as positional
/// parameters, never spliced into the script. The caller quits after this.
pub fn relaunch_after_exit(program: &Path, argument: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        let pid = std::process::id().to_string();
        let script = r#"while /bin/kill -0 "$1" 2>/dev/null; do sleep 0.2; done
if [ -n "$3" ]; then exec "$2" "$3"; else exec "$2"; fi"#;
        let mut command = std::process::Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(script)
            .arg("rcode-relaunch")
            .arg(&pid)
            .arg(program)
            .arg(argument)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0);
        if let Err(err) = command.spawn() {
            tracing::error!(error = %err, "failed to spawn the relauncher");
        }
    }
    #[cfg(not(unix))]
    let _ = (program, argument);
}

// ---------------------------------------------------------------------------
// Background checker
// ---------------------------------------------------------------------------

/// What the checker reports (the engine's `UpdateStatus` stream, the desktop
/// strip). Version facts only — download/apply progress is owned by whoever
/// drives the update (UI or CLI).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub current_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub update_available: bool,
    /// Epoch ms of the last successful check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl UpdateStatus {
    fn initial() -> Self {
        Self {
            current_version: current_version().to_string(),
            latest_version: None,
            update_available: false,
            checked_at: None,
            error: None,
        }
    }
}

/// `RCODE_AUTO_UPDATE=1|true|yes` — headless daemons apply updates themselves.
fn auto_update_enabled() -> bool {
    std::env::var("RCODE_AUTO_UPDATE")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

/// `RCODE_AUTO_UPDATE=0|false|no` — the desktop app then only reports: no
/// background download and no install on quit. Unset means on.
pub fn desktop_auto_update_enabled() -> bool {
    std::env::var("RCODE_AUTO_UPDATE")
        .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no"))
        .unwrap_or(true)
}

/// "Nothing would be interrupted by a restart right now" — wired by the engine
/// to its live-run and open-terminal registries. `None` = no gate.
pub type QuiescentCheck = Arc<dyn Fn() -> bool + Send + Sync>;

/// Who runs the checker — decides what it may do beyond reporting.
enum Role {
    /// The engine: managed installs with `RCODE_AUTO_UPDATE` apply themselves,
    /// and a service daemon restarts into a binary someone else installed.
    Engine { quiescent: Option<QuiescentCheck> },
    /// The desktop app: report only — downloading and installing are UI
    /// decisions (see `crates/ui/src/app_update.rs`).
    Desktop,
}

/// Background release checker: polls `{edge}/releases` hourly on a wall-clock
/// schedule (so sleep/wake cannot stretch it), backs off on failure, and
/// publishes [`UpdateStatus`] over a watch channel. In the engine, managed
/// installs with `RCODE_AUTO_UPDATE` set stage + apply + service restart on
/// their own — but only in a quiet window: while `quiescent` reports activity,
/// the apply defers and re-probes every [`IDLE_RECHECK`].
#[derive(Clone)]
pub struct Updater {
    release_base: String,
    role: Arc<Role>,
    status_tx: Arc<watch::Sender<UpdateStatus>>,
    /// Wakes the loop; `forced` says whether the wake demands a check or only
    /// a fresh look at the wall clock.
    wake_tx: Arc<watch::Sender<u64>>,
    forced: Arc<AtomicBool>,
    /// Set by `rcode headless`: this process is the installed service and may
    /// restart itself into a newer installed binary.
    service: Arc<AtomicBool>,
    /// Flips to true exactly once; the check loop selects against it so
    /// cancellation lands at any await point (no tokio-util in this crate).
    shutdown_tx: Arc<watch::Sender<bool>>,
    check_task: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl Updater {
    /// Spawn the engine's check loop (must run on a tokio runtime).
    pub fn spawn(release_base: String, quiescent: Option<QuiescentCheck>) -> Self {
        Self::spawn_role(release_base, Role::Engine { quiescent })
    }

    /// Spawn the desktop app's report-only check loop (must run on a tokio
    /// runtime).
    pub fn spawn_desktop(release_base: String) -> Self {
        Self::spawn_role(release_base, Role::Desktop)
    }

    fn spawn_role(release_base: String, role: Role) -> Self {
        let initial_delay = match role {
            Role::Engine { .. } => ENGINE_INITIAL_DELAY,
            Role::Desktop => DESKTOP_INITIAL_DELAY,
        };
        let (status_tx, _) = watch::channel(UpdateStatus::initial());
        let (wake_tx, _) = watch::channel(0);
        let (shutdown_tx, _) = watch::channel(false);
        let updater = Self {
            release_base,
            role: Arc::new(role),
            status_tx: Arc::new(status_tx),
            wake_tx: Arc::new(wake_tx),
            forced: Arc::new(AtomicBool::new(false)),
            service: Arc::new(AtomicBool::new(false)),
            shutdown_tx: Arc::new(shutdown_tx),
            check_task: Arc::new(std::sync::Mutex::new(None)),
        };
        // Subscribe before spawning so an immediate `check_now` cannot land
        // before the background task first polls and be lost behind the
        // initial delay.
        let wakes = updater.wake_tx.subscribe();
        let for_loop = updater.clone();
        let task = tokio::spawn(async move { for_loop.check_loop(wakes, initial_delay).await });
        *updater.check_task.lock().unwrap() = Some(task);
        updater
    }

    /// Stop the check loop and wait for it to exit — a replaced runtime must
    /// not keep polling `{edge}/releases` (or auto-applying) in the background.
    /// Idempotent, and callable from any clone.
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
        let task = self
            .check_task
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    pub fn watch(&self) -> watch::Receiver<UpdateStatus> {
        self.status_tx.subscribe()
    }

    /// Check immediately, for example when authentication recovers after the
    /// process started offline.
    pub fn check_now(&self) {
        self.forced.store(true, Ordering::SeqCst);
        self.wake_tx
            .send_modify(|epoch| *epoch = epoch.wrapping_add(1));
    }

    /// Re-read the wall clock now (window activation, system wake): checks
    /// only if one is due, so frequent pokes cost nothing.
    pub fn poke(&self) {
        self.wake_tx
            .send_modify(|epoch| *epoch = epoch.wrapping_add(1));
    }

    /// Called by `rcode headless`: when this process is the installed engine
    /// service and something else installs a newer binary, restart into it at
    /// the next quiet moment. A hand-started `rcode headless` is left alone —
    /// restarting the service unit would start a second engine beside it.
    pub fn restart_when_superseded(&self) {
        if !running_as_installed_service() {
            return;
        }
        self.service.store(true, Ordering::SeqCst);
        self.poke();
    }

    /// One user-requested check, awaited: publishes the result like a
    /// scheduled check and returns it.
    pub async fn check(&self) -> anyhow::Result<UpdateStatus> {
        match fetch_latest(&self.release_base).await {
            Ok(manifest) => {
                self.publish(manifest);
                Ok(self.status_tx.borrow().clone())
            }
            Err(err) => {
                self.status_tx
                    .send_modify(|s| s.error = Some(format!("{err:#}")));
                Err(err)
            }
        }
    }

    fn quiescent_now(&self) -> bool {
        match &*self.role {
            Role::Engine { quiescent } => quiescent.as_ref().is_none_or(|check| check()),
            Role::Desktop => true,
        }
    }

    async fn check_loop(&self, mut wakes: watch::Receiver<u64>, initial_delay: Duration) {
        let mut shutdown = self.shutdown_tx.subscribe();
        // Shutdown must cut the loop at ANY await point — including mid
        // `check_once()` / `auto_apply_when_idle()` HTTP — so the whole body
        // races the flag rather than checking it between iterations.
        tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => {}
            _ = async {
                tokio::select! {
                    _ = tokio::time::sleep(initial_delay) => {}
                    _ = wakes.changed() => {}
                }
                let mut schedule = Schedule::default();
                let mut superseded = SupersededRestart::default();
                let mut auto_apply: AutoApply<'_> = None;
                loop {
                    let forced = self.forced.swap(false, Ordering::SeqCst);
                    if forced || schedule.due(SystemTime::now()) {
                        let ok = self.check_once().await;
                        schedule.record(SystemTime::now(), ok);
                        if ok
                            && self.status_tx.borrow().update_available
                            && matches!(*self.role, Role::Engine { .. })
                            && auto_update_enabled()
                            && let InstallKind::Managed { .. } = detect_install()
                            && auto_apply.is_none()
                        {
                            auto_apply = Some(Box::pin(self.auto_apply_when_idle()));
                        }
                    }
                    if self.service.load(Ordering::SeqCst) {
                        superseded.poll(|| self.quiescent_now());
                    }
                    let wait = schedule.wait(SystemTime::now());
                    wait_for_check_or_apply(&mut wakes, wait, &mut auto_apply).await;
                }
            } => {}
        }
    }

    /// Sessions must never die to an update: pre-stage the download now
    /// (harmless while busy), wait for a quiet window (no live runs, no open
    /// terminals), then apply — which re-fetches the manifest (so a long defer
    /// lands on whatever is newest) and reuses the staged dir, keeping the
    /// idle→restart gap to well under a second.
    async fn auto_apply_when_idle(&self) {
        if let InstallKind::Managed { app_root } = detect_install() {
            match fetch_latest(&self.release_base).await {
                Ok(manifest) if version_newer(&manifest.version, current_version()) => {
                    if let Err(err) = stage_headless(&self.release_base, &manifest, &app_root).await
                    {
                        tracing::warn!(error = %err, "auto-update staging failed");
                        return;
                    }
                }
                Ok(_) => return,
                Err(err) => {
                    tracing::warn!(error = %err, "auto-update staging fetch failed");
                    return;
                }
            }
        }
        let mut deferred = false;
        loop {
            while !self.quiescent_now() {
                if !deferred {
                    deferred = true;
                    tracing::info!("auto-update deferred: sessions or terminals active");
                }
                tokio::time::sleep(IDLE_RECHECK).await;
            }
            // `apply_inner` fetches and stages again so a long defer lands on
            // the newest release. Re-check quiescence immediately before the
            // symlink swap: work can begin while that network/disk I/O runs.
            match self.apply_inner(true).await {
                Ok(Some(version)) => {
                    tracing::info!(%version, "auto-update applied; service restarting");
                    return;
                }
                Ok(None) => {
                    if !deferred {
                        deferred = true;
                        tracing::info!(
                            "auto-update deferred: activity began while staging the update"
                        );
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "auto-update failed");
                    return;
                }
            }
        }
    }

    fn publish(&self, manifest: Manifest) {
        let status = UpdateStatus {
            current_version: current_version().to_string(),
            update_available: version_newer(&manifest.version, current_version()),
            latest_version: Some(manifest.version),
            checked_at: Some(now_ms()),
            error: None,
        };
        if status.update_available {
            tracing::info!(
                latest = status.latest_version.as_deref().unwrap_or(""),
                current = %status.current_version,
                "update available"
            );
        }
        self.status_tx.send_replace(status);
    }

    /// One check; returns false on fetch failure (retry sooner).
    async fn check_once(&self) -> bool {
        match fetch_latest(&self.release_base).await {
            Ok(manifest) => {
                self.publish(manifest);
                true
            }
            Err(err) => {
                tracing::debug!(error = %err, "update check failed");
                self.status_tx
                    .send_modify(|s| s.error = Some(format!("{err:#}")));
                false
            }
        }
    }

    /// Stage + apply the newest release on THIS device (managed installs only),
    /// then restart the service after a short delay so the caller's RPC reply
    /// flushes before systemd/launchd kills this process.
    pub async fn apply(&self) -> anyhow::Result<String> {
        self.apply_inner(false)
            .await?
            .ok_or_else(|| anyhow::anyhow!("update became busy before apply"))
    }

    /// `require_quiescent` is used by automatic updates. It deliberately checks
    /// after all network and staging I/O and directly before the destructive
    /// swap/restart boundary; `None` tells the caller to wait and try again.
    async fn apply_inner(&self, require_quiescent: bool) -> anyhow::Result<Option<String>> {
        let InstallKind::Managed { app_root } = detect_install() else {
            bail!(
                "this install is not update-managed — the desktop app updates from its UI; \
                 source builds update via git"
            );
        };
        let manifest = fetch_latest(&self.release_base).await?;
        if !version_newer(&manifest.version, current_version()) {
            bail!("already up to date ({})", current_version());
        }
        stage_headless(&self.release_base, &manifest, &app_root).await?;
        if require_quiescent && !self.quiescent_now() {
            return Ok(None);
        }
        apply_headless(&app_root, &manifest.version)?;
        tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(800)).await;
            if let Err(err) = restart_service() {
                tracing::warn!(error = %err, "service restart failed — restart the engine to finish the update");
            }
        });
        Ok(Some(manifest.version))
    }
}

type AutoApply<'a> = Option<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>>;

/// Keep a deferred update alive while the service continues to process manual
/// checks, wake notifications, scheduled polls and superseded-version restarts.
async fn wait_for_check_or_apply(
    wakes: &mut watch::Receiver<u64>,
    wait: Duration,
    auto_apply: &mut AutoApply<'_>,
) {
    tokio::select! {
        _ = tokio::time::sleep(wait) => {}
        _ = wakes.changed() => {}
        _ = async {
            match auto_apply.as_mut() {
                Some(apply) => apply.await,
                None => std::future::pending().await,
            }
        } => { *auto_apply = None; }
    }
}

/// Wall-clock check deadlines with failure backoff.
#[derive(Debug, Default)]
struct Schedule {
    /// `None` until the first check: due immediately.
    next_due: Option<SystemTime>,
    failures: usize,
}

impl Schedule {
    fn due(&self, now: SystemTime) -> bool {
        match self.next_due {
            None => true,
            Some(due) => match due.duration_since(now) {
                Err(_) => true,
                // A deadline further out than any interval means the clock
                // jumped backwards; don't wait for it to catch up.
                Ok(remaining) => remaining.is_zero() || remaining > CHECK_INTERVAL,
            },
        }
    }

    fn record(&mut self, now: SystemTime, ok: bool) {
        let delay = if ok {
            self.failures = 0;
            CHECK_INTERVAL
        } else {
            let delay = CHECK_RETRY[self.failures.min(CHECK_RETRY.len() - 1)];
            self.failures = self.failures.saturating_add(1);
            delay
        };
        self.next_due = Some(now + delay);
    }

    /// Monotonic sleep until the next look at the clock.
    fn wait(&self, now: SystemTime) -> Duration {
        self.next_due
            .and_then(|due| due.duration_since(now).ok())
            .unwrap_or(Duration::ZERO)
            .clamp(Duration::from_secs(1), SCHEDULE_TICK)
    }
}

/// Restarts a service daemon into a newer binary that is already installed
/// (the desktop app swapped the bundle, `rcode update` flipped the symlink).
/// Waits for quiescence, and tries at most once per installed version so a
/// broken service manager cannot turn into a restart loop.
#[derive(Debug, Default)]
struct SupersededRestart {
    attempted: Option<String>,
    deferred_logged: bool,
}

impl SupersededRestart {
    fn poll(&mut self, quiescent: impl Fn() -> bool) {
        let Some(installed) = installed_version(&detect_install()) else {
            return;
        };
        self.poll_installed(&installed, current_version(), quiescent, restart_service);
    }

    fn poll_installed(
        &mut self,
        installed: &str,
        running: &str,
        quiescent: impl Fn() -> bool,
        restart: impl FnOnce() -> anyhow::Result<()>,
    ) {
        if !version_newer(installed, running) || self.attempted.as_deref() == Some(installed) {
            return;
        }
        if !quiescent() {
            if !self.deferred_logged {
                self.deferred_logged = true;
                tracing::info!(
                    installed,
                    running,
                    "newer version installed; restart deferred: sessions or terminals active"
                );
            }
            return;
        }
        self.attempted = Some(installed.to_owned());
        tracing::info!(
            installed,
            running,
            "newer version installed; restarting service"
        );
        if let Err(err) = restart() {
            tracing::warn!(error = %err, "service restart failed — restart the engine to finish the update");
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn every_update_requires_a_valid_checksum_before_download() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("payload.tar.gz");
        let file = "rcode-1.2.3-linux-x86_64.tar.gz";
        for digest in [
            None,
            Some(""),
            Some("1234"),
            Some("gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg"),
        ] {
            let manifest = Manifest {
                version: "1.2.3".into(),
                files: [(
                    file.into(),
                    FileMeta {
                        sha256: digest.map(str::to_owned),
                    },
                )]
                .into(),
            };
            let error = download_release_file("http://127.0.0.1:1", &manifest, file, &dest)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("SHA-256 checksum"), "{error:#}");
            assert!(!dest.exists());
            assert!(!dest.with_extension("partial").exists());
        }
    }

    #[test]
    fn cached_stages_require_the_current_artifact_and_unchanged_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let binary = tmp.path().join("rcode");
        std::fs::write(&binary, b"verified executable").unwrap();
        let digest = "a".repeat(64);
        assert!(!cached_stage_verified(tmp.path(), &binary, &digest));
        record_verified_stage(tmp.path(), &binary, &digest).unwrap();
        assert!(cached_stage_verified(tmp.path(), &binary, &digest));
        assert!(!cached_stage_verified(tmp.path(), &binary, &"b".repeat(64)));
        std::fs::write(&binary, b"modified executable").unwrap();
        assert!(!cached_stage_verified(tmp.path(), &binary, &digest));
    }

    #[tokio::test]
    async fn deferred_auto_apply_does_not_block_checks_or_get_cancelled_by_wakes() {
        let (wake_tx, mut wakes) = watch::channel(0);
        let (complete_tx, complete_rx) = tokio::sync::oneshot::channel();
        let finished = Arc::new(AtomicBool::new(false));
        let applied = finished.clone();
        let mut auto_apply: AutoApply<'_> = Some(Box::pin(async move {
            complete_rx.await.unwrap();
            applied.store(true, Ordering::SeqCst);
        }));
        wake_tx.send(1).unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            wait_for_check_or_apply(&mut wakes, Duration::from_secs(3600), &mut auto_apply),
        )
        .await
        .expect("manual checks remain responsive while the update is deferred");
        assert!(auto_apply.is_some());
        assert!(!finished.load(Ordering::SeqCst));
        // A scheduled poll is responsive too, without dropping the staged job.
        wait_for_check_or_apply(&mut wakes, Duration::ZERO, &mut auto_apply).await;
        assert!(auto_apply.is_some());
        complete_tx.send(()).unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            wait_for_check_or_apply(&mut wakes, Duration::from_secs(3600), &mut auto_apply),
        )
        .await
        .unwrap();
        assert!(auto_apply.is_none());
        assert!(finished.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn stalled_update_headers_and_body_time_out_but_progressing_body_survives() {
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for stall_body in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                socket.read(&mut request).await.unwrap();
                if stall_body {
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
                        .await
                        .unwrap();
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            });
            let client =
                http_client_with_timeouts(Duration::from_millis(200), Duration::from_millis(100))
                    .unwrap();
            let request = async {
                client
                    .get(format!("http://{address}"))
                    .send()
                    .await?
                    .bytes()
                    .await
            };
            let error = tokio::time::timeout(Duration::from_secs(1), request)
                .await
                .expect("bounded read")
                .unwrap_err();
            assert!(error.is_timeout());
            server.abort();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n")
                .await
                .unwrap();
            for _ in 0..10 {
                socket.write_all(b"x").await.unwrap();
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
        });
        let client =
            http_client_with_timeouts(Duration::from_secs(1), Duration::from_millis(200)).unwrap();
        assert_eq!(
            client
                .get(format!("http://{address}"))
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap()
                .len(),
            10
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn release_metadata_has_size_and_total_time_limits() {
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Reject an advertised oversized body before buffering it.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n01234567890")
                .await
                .unwrap();
        });
        let client =
            http_client_with_timeouts(Duration::from_secs(1), Duration::from_secs(1)).unwrap();
        let error = fetch_release_metadata_with_limits(
            &client,
            &format!("http://{address}"),
            Duration::from_secs(1),
            10,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("exceeds 10 bytes"));
        server.await.unwrap();

        // Enforce the same cap when Content-Length is absent.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n01234567890")
                .await
                .unwrap();
        });
        let error = fetch_release_metadata_with_limits(
            &client,
            &format!("http://{address}"),
            Duration::from_secs(1),
            10,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("exceeds 10 bytes"));
        server.await.unwrap();

        // Progressing bytes stay below the client's inactivity timeout but
        // must still obey the metadata operation's total deadline.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n")
                .await
                .unwrap();
            for _ in 0..10 {
                if socket.write_all(b"x").await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        });
        let error = fetch_release_metadata_with_limits(
            &client,
            &format!("http://{address}"),
            Duration::from_millis(80),
            100,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("metadata deadline"));
        server.abort();
    }

    #[tokio::test]
    async fn stalled_update_tls_handshake_has_a_connect_deadline() {
        use std::time::Duration;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
        });
        let client =
            http_client_with_timeouts(Duration::from_millis(100), Duration::from_secs(30)).unwrap();
        let error = tokio::time::timeout(
            Duration::from_secs(1),
            client.get(format!("https://{address}")).send(),
        )
        .await
        .expect("bounded TLS handshake")
        .unwrap_err();
        assert!(error.is_timeout());
        server.abort();
    }

    #[test]
    fn update_feed_override_requires_an_https_base_url() {
        assert_eq!(
            validate_release_override(" https://example.com/releases/ ").unwrap(),
            "https://example.com/releases"
        );
        for url in [
            "http://example.com/releases",
            "file:///tmp/update",
            "https://user:password@example.com",
            "https://example.com?feed=x",
            "https://example.com/#fragment",
        ] {
            assert!(validate_release_override(url).is_err(), "accepted {url}");
        }
    }

    #[test]
    fn version_compare() {
        assert!(version_newer("0.1.1", "0.1.0"));
        assert!(version_newer("0.2.0", "0.1.9"));
        assert!(version_newer("0.1.10", "0.1.9"));
        assert!(version_newer("v0.1.1", "0.1.0"));
        assert!(version_newer("0.1.0.1", "0.1.0"));
        assert!(!version_newer("0.1.0", "0.1.0"));
        assert!(!version_newer("0.1.0", "0.1.1"));
        // Garbage never counts as newer.
        assert!(!version_newer("", "0.1.0"));
        assert!(!version_newer("nightly", "0.1.0"));
    }

    #[test]
    fn beta_versions_compare_without_blocking_updates() {
        assert!(version_newer("0.0.2-beta", "0.0.1-beta"));
        assert!(version_newer("0.0.1", "0.0.1-beta"));
        assert!(version_newer("0.0.1-beta.10", "0.0.1-beta.2"));
        assert!(version_newer("v0.0.1-beta.1", "0.0.1-beta"));
        assert!(!version_newer("0.0.1-beta", "0.0.1"));
        assert!(!version_newer("0.0.1-beta+build.2", "0.0.1-beta+build.1"));
        for invalid in ["0.0.1-", "0.0.1+", "0.0.1-beta..1", "0.0.1-beta.01"] {
            assert!(!version_newer(invalid, "0.0.1-beta"));
            assert!(parse_version(invalid).is_none());
        }
    }

    #[test]
    fn install_kind_detection() {
        assert_eq!(
            detect_install_from_for_os(
                Path::new("/home/u/.rcode/app/0.1.1/rcode"),
                Some(Path::new("/home/u")),
                "linux",
            ),
            InstallKind::Managed {
                app_root: PathBuf::from("/home/u/.rcode/app")
            }
        );
        assert_eq!(
            detect_install_from_for_os(
                Path::new("/Applications/RCode.app/Contents/MacOS/rcode"),
                Some(Path::new("/Users/u")),
                "macos",
            ),
            InstallKind::MacApp {
                bundle: PathBuf::from("/Applications/RCode.app")
            }
        );
        // A path merely containing `.app` without the bundle layout is not a bundle.
        assert_eq!(
            detect_install_from_for_os(Path::new("/tmp/foo.app/rcode"), None, "macos"),
            InstallKind::Unmanaged
        );
        assert_eq!(
            detect_install_from_for_os(
                Path::new("/src/target/release/rcode"),
                Some(Path::new("/home/u")),
                "linux",
            ),
            InstallKind::Unmanaged
        );
    }

    #[test]
    fn artifact_names_match_packaging() {
        let (os, arch) = platform_key();
        assert!(headless_artifact("0.2.0").starts_with("rcode-0.2.0-"));
        assert_eq!(
            headless_artifact("0.2.0"),
            format!("rcode-0.2.0-{os}-{arch}.tar.gz")
        );
        assert!(mac_app_artifact("0.2.0").ends_with("-app.tar.gz"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_is_not_misclassified_as_linux() {
        assert_eq!(platform_key().0, "windows");
    }

    #[cfg(windows)]
    #[test]
    fn windows_install_is_always_unmanaged() {
        assert_eq!(
            detect_install_from(
                Path::new(r"C:\Users\u\.rcode\app\0.2.0\rcode.exe"),
                Some(Path::new(r"C:\Users\u")),
            ),
            InstallKind::Unmanaged
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_rejects_platform_specific_updates_before_side_effects() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = Manifest {
            version: "9.9.9".into(),
            files: BTreeMap::new(),
        };
        let app_root = tmp.path().join("app");
        let data_dir = tmp.path().join("data");

        let managed_err = stage_headless("http://127.0.0.1:1", &manifest, &app_root)
            .await
            .unwrap_err();
        assert!(managed_err.to_string().contains("not supported on windows"));
        assert!(!app_root.exists(), "managed staging must not touch disk");

        let mac_err = stage_mac_app("http://127.0.0.1:1", &manifest, &data_dir)
            .await
            .unwrap_err();
        assert!(mac_err.to_string().contains("not supported on windows"));
        assert!(!data_dir.exists(), "macOS staging must not touch disk");

        assert!(
            apply_headless(&app_root, &manifest.version)
                .unwrap_err()
                .to_string()
                .contains("not supported on windows")
        );
        assert!(
            apply_mac_app(&data_dir.join("RCode.app"), &data_dir.join("Installed.app"))
                .unwrap_err()
                .to_string()
                .contains("not supported on windows")
        );
        assert!(
            restart_service()
                .unwrap_err()
                .to_string()
                .contains("not supported on windows")
        );
    }

    #[test]
    fn manifest_parses_with_and_without_files() {
        let full: Manifest = serde_json::from_str(
            r#"{"version":"0.1.1","files":{"rcode-0.1.1-linux-x86_64.tar.gz":{"sha256":"abc"}}}"#,
        )
        .unwrap();
        assert_eq!(full.version, "0.1.1");
        assert_eq!(
            full.files["rcode-0.1.1-linux-x86_64.tar.gz"]
                .sha256
                .as_deref(),
            Some("abc")
        );
        let bare: Manifest = serde_json::from_str(r#"{"version":"0.1.1"}"#).unwrap();
        assert!(bare.files.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn headless_symlink_swap() {
        let tmp = tempfile::tempdir().unwrap();
        let app_root = tmp.path().join("app");
        for ver in ["0.1.0", "0.1.1"] {
            std::fs::create_dir_all(app_root.join(ver)).unwrap();
            std::fs::write(app_root.join(ver).join("rcode"), ver).unwrap();
        }
        apply_headless(&app_root, "0.1.0").unwrap();
        assert_eq!(
            std::fs::read_link(app_root.join("current")).unwrap(),
            app_root.join("0.1.0")
        );
        // Swap over an existing symlink.
        apply_headless(&app_root, "0.1.1").unwrap();
        assert_eq!(
            std::fs::read_link(app_root.join("current")).unwrap(),
            app_root.join("0.1.1")
        );
        // Unstaged version refuses.
        assert!(apply_headless(&app_root, "0.2.0").is_err());
    }
    #[test]
    fn schedule_is_wall_clock_with_failure_backoff() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut schedule = Schedule::default();
        assert!(schedule.due(start), "the first check is due immediately");

        schedule.record(start, true);
        assert!(!schedule.due(start + Duration::from_secs(59 * 60)));
        // A machine that slept through the deadline is due the moment it
        // looks at the clock again — no monotonic remainder to wait out.
        assert!(schedule.due(start + Duration::from_secs(9 * 60 * 60)));
        assert_eq!(schedule.wait(start), SCHEDULE_TICK);
        // A clock that jumped backwards does not postpone checks.
        assert!(schedule.due(start - Duration::from_secs(24 * 60 * 60)));

        let mut backoff = Vec::new();
        for _ in 0..6 {
            schedule.record(start, false);
            backoff.push(schedule.next_due.unwrap().duration_since(start).unwrap());
        }
        assert_eq!(
            backoff,
            [
                CHECK_RETRY[0],
                CHECK_RETRY[1],
                CHECK_RETRY[2],
                CHECK_RETRY[3],
                CHECK_RETRY[3],
                CHECK_RETRY[3]
            ]
        );
        schedule.record(start, true);
        assert_eq!(schedule.failures, 0);
    }

    #[test]
    fn superseded_service_restarts_once_when_quiet() {
        let restarts = std::cell::Cell::new(0);
        let restart = || {
            restarts.set(restarts.get() + 1);
            Ok(())
        };
        let mut superseded = SupersededRestart::default();
        // Same or older installed versions never restart.
        superseded.poll_installed("0.2.0", "0.2.0", || true, restart);
        superseded.poll_installed("0.1.9", "0.2.0", || true, restart);
        assert_eq!(restarts.get(), 0);
        // Busy: deferred, not attempted.
        superseded.poll_installed("0.2.1", "0.2.0", || false, restart);
        assert_eq!(restarts.get(), 0);
        // Quiet: exactly one attempt per installed version.
        superseded.poll_installed("0.2.1", "0.2.0", || true, restart);
        superseded.poll_installed("0.2.1", "0.2.0", || true, restart);
        assert_eq!(restarts.get(), 1);
        superseded.poll_installed("0.2.2", "0.2.0", || true, restart);
        assert_eq!(restarts.get(), 2);
    }

    #[test]
    fn systemd_service_cgroup_is_recognized() {
        assert!(in_rcode_service_cgroup(
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/rcode.service\n"
        ));
        assert!(in_rcode_service_cgroup(
            "12:pids:/user.slice/user@1000.service/rcode.service\n1:name=systemd:/x\n"
        ));
        assert!(!in_rcode_service_cgroup(
            "0::/user.slice/user-1000.slice/session-3.scope\n"
        ));
        assert!(!in_rcode_service_cgroup(
            "0::/user.slice/user@1000.service/app.slice/rcode.service.d\n"
        ));
    }

    #[test]
    fn bundle_version_reads_the_packaged_plist() {
        let template = include_str!("../../../dist/macos/Info.plist");
        let plist = template
            .replace("__VERSION__", "0.0.1-beta")
            .replace("__NUMERIC_VERSION__", "0.0.1");
        assert_eq!(bundle_short_version(&plist).as_deref(), Some("0.0.1-beta"));
        let legacy = "<plist><key>CFBundleShortVersionString</key><string>0.3.1</string></plist>";
        assert_eq!(bundle_short_version(legacy).as_deref(), Some("0.3.1"));
        assert_eq!(bundle_short_version("<plist></plist>"), None);
    }

    #[cfg(unix)]
    #[test]
    fn installed_version_follows_the_current_symlink_and_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let app_root = tmp.path().join("app");
        std::fs::create_dir_all(app_root.join("0.4.0")).unwrap();
        std::fs::write(app_root.join("0.4.0").join("rcode"), "").unwrap();
        let managed = InstallKind::Managed {
            app_root: app_root.clone(),
        };
        assert_eq!(installed_version(&managed), None, "no current link yet");
        apply_headless(&app_root, "0.4.0").unwrap();
        assert_eq!(installed_version(&managed).as_deref(), Some("0.4.0"));

        let bundle = tmp.path().join("RCode.app");
        std::fs::create_dir_all(bundle.join("Contents")).unwrap();
        std::fs::write(
            bundle.join("Contents/Info.plist"),
            include_str!("../../../dist/macos/Info.plist").replace("__VERSION__", "0.4.1"),
        )
        .unwrap();
        assert_eq!(
            installed_version(&InstallKind::MacApp { bundle }).as_deref(),
            Some("0.4.1")
        );
        assert_eq!(installed_version(&InstallKind::Unmanaged), None);
    }

    #[test]
    fn mac_bundles_that_cannot_replace_themselves_are_explained() {
        let writable = |_: &Path| true;
        let read_only = |_: &Path| false;
        assert_eq!(
            mac_bundle_blocker(Path::new("/Applications/RCode.app"), writable),
            None
        );
        assert_eq!(
            mac_bundle_blocker(
                Path::new("/private/var/folders/x/T/AppTranslocation/ABC/d/RCode.app"),
                writable
            ),
            Some(UpdateBlocker::Translocated)
        );
        assert_eq!(
            mac_bundle_blocker(Path::new("/Volumes/RCode/RCode.app"), read_only),
            Some(UpdateBlocker::DiskImage)
        );
        // An external drive the user can write to updates in place.
        assert_eq!(
            mac_bundle_blocker(Path::new("/Volumes/Work/RCode.app"), writable),
            None
        );
        assert_eq!(
            mac_bundle_blocker(Path::new("/Applications/RCode.app"), read_only),
            Some(UpdateBlocker::NotWritable(PathBuf::from("/Applications")))
        );
    }

    #[test]
    fn writability_probe_leaves_nothing_behind() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(dir_writable(tmp.path()));
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
        assert!(!dir_writable(&tmp.path().join("missing")));
    }

    /// Serves `body` for every request until aborted.
    async fn serve_forever(body: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    let mut request = [0; 4096];
                    let _ = socket.read(&mut request).await;
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                });
            }
        });
        (base, server)
    }

    #[tokio::test]
    async fn desktop_checker_reports_a_newer_release_on_demand() {
        let (base, server) = serve_forever(br#"{"version":"999.0.0","files":{}}"#.to_vec()).await;
        let updater = Updater::spawn_desktop(base);
        let status = updater.check().await.unwrap();
        assert!(status.update_available);
        assert_eq!(status.latest_version.as_deref(), Some("999.0.0"));
        assert_eq!(status.current_version, current_version());
        assert!(status.checked_at.is_some());
        assert_eq!(*updater.watch().borrow(), status);
        updater.shutdown().await;
        server.abort();
    }

    /// A headless tarball whose `rcode` reports `reported` from `--version`.
    #[cfg(unix)]
    fn fake_headless_tarball(dir: &Path, reported: &str) -> Vec<u8> {
        use std::os::unix::fs::PermissionsExt as _;
        let root = dir.join("rcode-pkg");
        std::fs::create_dir_all(&root).unwrap();
        let binary = root.join("rcode");
        std::fs::write(&binary, format!("#!/bin/sh\necho \"rcode {reported}\"\n")).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let tarball = dir.join("pkg.tar.gz");
        run(
            "tar",
            &[
                "-czf",
                &tarball.to_string_lossy(),
                "-C",
                &dir.to_string_lossy(),
                "rcode-pkg",
            ],
        )
        .unwrap();
        std::fs::read(tarball).unwrap()
    }

    #[tokio::test]
    async fn cancelled_headless_stage_cleans_only_its_own_directory() {
        let root = tempfile::tempdir().unwrap();
        let sibling = update_stage_directory(root.path(), "9.9.9").unwrap();
        let sibling_file = sibling.path().join("keep");
        std::fs::write(&sibling_file, b"another download").unwrap();
        let app_root = root.path().to_owned();
        let (ready, started) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let directory = update_stage_directory(&app_root, "9.9.9").unwrap();
            ready.send(directory.path().to_owned()).unwrap();
            std::future::pending::<()>().await;
            drop(directory);
        });
        let cancelled = started.await.unwrap();
        assert_ne!(cancelled, sibling.path());
        assert!(cancelled.is_dir());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!cancelled.exists());
        assert_eq!(std::fs::read(sibling_file).unwrap(), b"another download");
    }

    #[test]
    fn concurrent_headless_promotions_preserve_the_verified_cache() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("9.9.9");
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(dest.join("rcode"), b"old unverified cache").unwrap();
        let expected = "a".repeat(64);
        let stages: Vec<_> = (0..2)
            .map(|_| {
                let stage = update_stage_directory(root.path(), "9.9.9").unwrap();
                let unpacked = stage.path().join("unpacked");
                std::fs::create_dir(&unpacked).unwrap();
                std::fs::write(unpacked.join("rcode"), b"verified image").unwrap();
                record_verified_stage(&unpacked, &unpacked.join("rcode"), &expected).unwrap();
                stage
            })
            .collect();
        assert_ne!(stages[0].path(), stages[1].path());
        let barrier = std::sync::Barrier::new(stages.len());
        std::thread::scope(|scope| {
            let tasks: Vec<_> = stages
                .iter()
                .map(|stage| {
                    let barrier = &barrier;
                    let dest = &dest;
                    let expected = &expected;
                    scope.spawn(move || {
                        barrier.wait();
                        promote_verified_stage(
                            &stage.path().join("unpacked"),
                            dest,
                            Path::new("rcode"),
                            expected,
                        )
                    })
                })
                .collect();
            for task in tasks {
                task.join().unwrap().unwrap();
            }
        });
        assert!(cached_stage_verified(&dest, &dest.join("rcode"), &expected));
        assert_eq!(
            std::fs::read(dest.join("rcode")).unwrap(),
            b"verified image"
        );
    }

    #[test]
    fn failed_headless_promotion_does_not_remove_another_stage() {
        let root = tempfile::tempdir().unwrap();
        let first = update_stage_directory(root.path(), "9.9.9").unwrap();
        let second = update_stage_directory(root.path(), "9.9.9").unwrap();
        let keep = second.path().join("keep");
        std::fs::write(&keep, b"another download").unwrap();
        // A directory in the lock file's place makes promotion fail before
        // touching a destination or any other in-progress download.
        std::fs::create_dir(root.path().join(".stage-promote.lock")).unwrap();
        let abandoned = first.path().to_owned();
        assert!(
            promote_verified_stage(
                first.path(),
                &root.path().join("9.9.9"),
                Path::new("rcode"),
                &"a".repeat(64)
            )
            .is_err()
        );
        drop(first);
        assert!(!abandoned.exists());
        assert_eq!(std::fs::read(keep).unwrap(), b"another download");
    }

    #[test]
    fn staging_another_version_preserves_completed_and_active_sources() {
        let root = tempfile::tempdir().unwrap();
        let active = update_stage_directory(root.path(), "3.0.0").unwrap();
        std::fs::write(active.path().join("downloading"), b"in progress").unwrap();
        for version in ["1.0.0", "2.0.0"] {
            let incoming = update_stage_directory(root.path(), version).unwrap();
            std::fs::write(incoming.path().join("rcode"), version).unwrap();
            record_verified_stage(
                incoming.path(),
                &incoming.path().join("rcode"),
                &"a".repeat(64),
            )
            .unwrap();
            promote_verified_stage(
                incoming.path(),
                &root.path().join(version),
                Path::new("rcode"),
                &"a".repeat(64),
            )
            .unwrap();
        }
        assert_eq!(
            std::fs::read(root.path().join("1.0.0/rcode")).unwrap(),
            b"1.0.0"
        );
        assert_eq!(
            std::fs::read(root.path().join("2.0.0/rcode")).unwrap(),
            b"2.0.0"
        );
        assert!(active.path().join("downloading").is_file());
        assert!(root.path().join(".stage-promote.lock").is_file());
    }

    #[test]
    fn a_different_artifact_for_a_verified_version_is_rejected_without_replacing_it() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("1.0.0");
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(dest.join("rcode"), b"original verified bytes").unwrap();
        record_verified_stage(&dest, &dest.join("rcode"), &"a".repeat(64)).unwrap();
        let incoming = update_stage_directory(root.path(), "1.0.0").unwrap();
        std::fs::write(incoming.path().join("rcode"), b"different bytes").unwrap();
        record_verified_stage(
            incoming.path(),
            &incoming.path().join("rcode"),
            &"b".repeat(64),
        )
        .unwrap();
        let error =
            promote_verified_stage(incoming.path(), &dest, Path::new("rcode"), &"b".repeat(64))
                .unwrap_err();
        assert!(error.to_string().contains("release artifact conflict"));
        assert!(error.to_string().contains("new version"));
        assert_eq!(
            std::fs::read(dest.join("rcode")).unwrap(),
            b"original verified bytes"
        );
        assert!(cached_stage_verified(
            &dest,
            &dest.join("rcode"),
            &"a".repeat(64)
        ));
        assert!(incoming.path().join("rcode").is_file());
    }

    fn write_test_bundle(bundle: &Path, bytes: &[u8]) {
        let binary = bundle.join("Contents/MacOS/rcode");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(binary, bytes).unwrap();
    }

    fn copy_test_bundle(source: &Path, target: &Path) -> anyhow::Result<()> {
        let bytes = std::fs::read(source.join("Contents/MacOS/rcode"))?;
        write_test_bundle(target, &bytes);
        Ok(())
    }

    #[test]
    fn concurrent_app_bundle_transactions_serialize_the_same_destination() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let root = tempfile::tempdir().unwrap();
        let bundle = root.path().join("RCode.app");
        let source = root.path().join("cached.app");
        write_test_bundle(&bundle, b"old");
        write_test_bundle(&source, b"new");
        let barrier = std::sync::Barrier::new(2);
        let active = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            let tasks: Vec<_> = (0..2)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        apply_mac_app_transaction(
                            &source,
                            &bundle,
                            |from, to| {
                                assert_eq!(active.fetch_add(1, Ordering::SeqCst), 0);
                                std::thread::sleep(Duration::from_millis(20));
                                let result = copy_test_bundle(from, to);
                                active.fetch_sub(1, Ordering::SeqCst);
                                result
                            },
                            |from, to| std::fs::rename(from, to),
                        )
                    })
                })
                .collect();
            for task in tasks {
                task.join().unwrap().unwrap();
            }
        });
        assert_eq!(
            std::fs::read(bundle.join("Contents/MacOS/rcode")).unwrap(),
            b"new"
        );
        assert!(source.join("Contents/MacOS/rcode").is_file());
        assert!(std::fs::read_dir(root.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".rcode-install-")
        }));
    }

    struct InstallSubprocess {
        child: std::process::Child,
        log: PathBuf,
    }

    impl Drop for InstallSubprocess {
        fn drop(&mut self) {
            // Also runs on an assertion/timeout: no child may outlive its
            // temporary installation directory or keep its file lock open.
            if !matches!(self.child.try_wait(), Ok(Some(_))) {
                let _ = self.child.kill();
            }
            let _ = self.child.wait();
        }
    }

    impl InstallSubprocess {
        fn spawn(root: &Path, role: &str) -> Self {
            let log = root.join(format!("{role}.log"));
            let output = std::fs::File::create(&log).unwrap();
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "tests::app_bundle_install_subprocess",
                    "--ignored",
                    "--nocapture",
                ])
                .env("RCODE_TEST_INSTALL_ROOT", root)
                .env("RCODE_TEST_INSTALL_ROLE", role)
                .stdin(std::process::Stdio::null())
                .stdout(output.try_clone().unwrap())
                .stderr(output);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt as _;
                command.creation_flags(0x08000000); // CREATE_NO_WINDOW
            }
            Self {
                child: command.spawn().unwrap(),
                log,
            }
        }

        fn wait_for_file(&mut self, path: &Path) {
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            while !path.is_file() {
                if let Some(status) = self.child.try_wait().unwrap() {
                    panic!(
                        "install subprocess exited {status} before {}: {}",
                        path.display(),
                        std::fs::read_to_string(&self.log).unwrap_or_default()
                    );
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "install subprocess timed out waiting for {}: {}",
                    path.display(),
                    std::fs::read_to_string(&self.log).unwrap_or_default()
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        fn wait_success(&mut self) {
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    assert!(
                        status.success(),
                        "install subprocess failed {status}: {}",
                        std::fs::read_to_string(&self.log).unwrap_or_default()
                    );
                    return;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "install subprocess did not finish: {}",
                    std::fs::read_to_string(&self.log).unwrap_or_default()
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    #[test]
    #[ignore = "helper invoked only by the isolated cross-process installation test"]
    fn app_bundle_install_subprocess() {
        let root = PathBuf::from(
            std::env::var_os("RCODE_TEST_INSTALL_ROOT").expect("parent fixture root"),
        );
        assert_eq!(
            std::fs::read(root.join("fixture-owner")).unwrap(),
            b"isolated update transaction fixture"
        );
        let role = std::env::var("RCODE_TEST_INSTALL_ROLE").expect("parent fixture role");
        assert!(matches!(role.as_str(), "a" | "b"));
        std::fs::write(root.join(format!("{role}.ready")), b"ready").unwrap();
        if role == "b" {
            // This is a real OS process, not another handle in one process.
            // Prove A's live transaction owns the stable destination lock.
            let probe = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(root.join(".RCode.app.update.lock"))
                .unwrap();
            assert!(
                matches!(probe.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
                "the other process must hold the destination lock"
            );
            std::fs::write(root.join("b.blocked"), b"blocked").unwrap();
        }
        let source = root.join(format!("source-{role}.app"));
        let bundle = if role == "b" {
            root.join("alternate/../RCode.app")
        } else {
            root.join("RCode.app")
        };
        apply_mac_app_transaction(
            &source,
            &bundle,
            |from, to| {
                std::fs::write(root.join(format!("{role}.entered")), b"entered")?;
                if role == "a" {
                    let deadline = std::time::Instant::now() + Duration::from_secs(15);
                    while !root.join("release-a").is_file() {
                        anyhow::ensure!(
                            std::time::Instant::now() < deadline,
                            "parent did not release transaction A"
                        );
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
                copy_test_bundle(from, to)
            },
            |from, to| std::fs::rename(from, to),
        )
        .unwrap();
        std::fs::write(root.join(format!("{role}.done")), b"done").unwrap();
    }

    #[test]
    fn separate_processes_serialize_app_bundle_installation_and_preserve_shared_sources() {
        let root = tempfile::Builder::new()
            .prefix("rcode-install-process-")
            .tempdir()
            .unwrap();
        std::fs::write(
            root.path().join("fixture-owner"),
            b"isolated update transaction fixture",
        )
        .unwrap();
        std::fs::create_dir(root.path().join("alternate")).unwrap();
        let bundle = root.path().join("RCode.app");
        let first = root.path().join("source-a.app");
        let second = root.path().join("source-b.app");
        write_test_bundle(&bundle, b"old installation");
        write_test_bundle(&first, b"first update");
        write_test_bundle(&second, b"second update");
        let mut a = InstallSubprocess::spawn(root.path(), "a");
        a.wait_for_file(&root.path().join("a.entered"));
        let mut b = InstallSubprocess::spawn(root.path(), "b");
        b.wait_for_file(&root.path().join("b.ready"));
        b.wait_for_file(&root.path().join("b.blocked"));
        assert!(!root.path().join("b.entered").exists());
        assert_eq!(
            std::fs::read(bundle.join("Contents/MacOS/rcode")).unwrap(),
            b"old installation"
        );
        std::fs::write(root.path().join("release-a"), b"release").unwrap();
        a.wait_success();
        b.wait_success();
        assert!(root.path().join("a.done").is_file());
        assert!(root.path().join("b.done").is_file());
        assert!(root.path().join(".RCode.app.update.lock").is_file());
        assert_eq!(
            std::fs::read(first.join("Contents/MacOS/rcode")).unwrap(),
            b"first update"
        );
        assert_eq!(
            std::fs::read(second.join("Contents/MacOS/rcode")).unwrap(),
            b"second update"
        );
        assert_eq!(
            std::fs::read(bundle.join("Contents/MacOS/rcode")).unwrap(),
            b"second update"
        );
    }

    #[test]
    fn a_failed_bundle_swap_restores_the_previous_installation() {
        let root = tempfile::tempdir().unwrap();
        let bundle = root.path().join("RCode.app");
        let source = root.path().join("cached.app");
        write_test_bundle(&bundle, b"old");
        write_test_bundle(&source, b"new");
        let mut renames = 0;
        let error = apply_mac_app_transaction(&source, &bundle, copy_test_bundle, |from, to| {
            renames += 1;
            if renames == 2 {
                return Err(std::io::Error::other("injected swap failure"));
            }
            std::fs::rename(from, to)
        })
        .unwrap_err();
        assert!(error.to_string().contains("original app restored"));
        assert_eq!(
            std::fs::read(bundle.join("Contents/MacOS/rcode")).unwrap(),
            b"old"
        );
        assert!(source.join("Contents/MacOS/rcode").is_file());
    }

    #[test]
    fn a_failed_rollback_preserves_the_only_backup_and_reports_its_path() {
        let root = tempfile::tempdir().unwrap();
        let bundle = root.path().join("RCode.app");
        let source = root.path().join("cached.app");
        write_test_bundle(&bundle, b"old");
        write_test_bundle(&source, b"new");
        let mut renames = 0;
        let error = apply_mac_app_transaction(&source, &bundle, copy_test_bundle, |from, to| {
            renames += 1;
            if renames >= 2 {
                return Err(std::io::Error::other("injected rename failure"));
            }
            std::fs::rename(from, to)
        })
        .unwrap_err();
        assert!(!bundle.exists());
        let recovery = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".rcode-install-")
            })
            .unwrap()
            .join("old.app");
        assert_eq!(
            std::fs::read(recovery.join("Contents/MacOS/rcode")).unwrap(),
            b"old"
        );
        assert!(error.to_string().contains("rollback failed"));
        assert!(error.to_string().contains(&recovery.display().to_string()));
        assert!(source.join("Contents/MacOS/rcode").is_file());
    }

    #[test]
    fn a_failed_bundle_copy_keeps_the_installation_and_shared_cache() {
        let root = tempfile::tempdir().unwrap();
        let bundle = root.path().join("RCode.app");
        let source = root.path().join("cached.app");
        write_test_bundle(&bundle, b"old");
        write_test_bundle(&source, b"new");
        assert!(
            apply_mac_app_transaction(
                &source,
                &bundle,
                |_, fresh| {
                    write_test_bundle(fresh, b"partial");
                    bail!("injected copy failure")
                },
                |from, to| std::fs::rename(from, to)
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read(bundle.join("Contents/MacOS/rcode")).unwrap(),
            b"old"
        );
        assert!(source.join("Contents/MacOS/rcode").is_file());
        assert!(std::fs::read_dir(root.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".rcode-install-")
        }));
    }

    #[cfg(unix)]
    #[test]
    fn bundle_aliases_use_the_actual_installation_target() {
        let root = tempfile::tempdir().unwrap();
        let bundle = root.path().join("RCode.app");
        let alias = root.path().join("Alias.app");
        let source = root.path().join("cached.app");
        write_test_bundle(&bundle, b"old");
        write_test_bundle(&source, b"new");
        std::os::unix::fs::symlink("RCode.app", &alias).unwrap();
        assert_eq!(
            bundle_install_target(&alias).unwrap(),
            bundle_install_target(&bundle).unwrap()
        );
        apply_mac_app_transaction(&source, &alias, copy_test_bundle, |from, to| {
            std::fs::rename(from, to)
        })
        .unwrap();
        assert!(
            std::fs::symlink_metadata(&alias)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read(bundle.join("Contents/MacOS/rcode")).unwrap(),
            b"new"
        );
    }

    #[cfg(target_os = "macos")]
    fn fake_mac_app_tarball(directory: &Path, reported: &str) -> Vec<u8> {
        use std::os::unix::fs::PermissionsExt as _;
        let binary = directory.join("RCode.app/Contents/MacOS/rcode");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, format!("#!/bin/sh\necho \"rcode {reported}\"\n")).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let tarball = directory.join("app.tar.gz");
        run(
            "tar",
            &[
                "-czf",
                &tarball.to_string_lossy(),
                "-C",
                &directory.to_string_lossy(),
                "RCode.app",
            ],
        )
        .unwrap();
        std::fs::read(tarball).unwrap()
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn mac_staging_and_installation_leave_all_verified_apply_sources_available() {
        use sha2::{Digest as _, Sha256};
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let mut sources = Vec::new();
        for version in ["1.0.0", "2.0.0"] {
            let bytes =
                fake_mac_app_tarball(&root.path().join(format!("package-{version}")), version);
            let manifest = Manifest {
                version: version.into(),
                files: [(
                    mac_app_artifact(version),
                    FileMeta {
                        sha256: Some(format!("{:x}", Sha256::digest(&bytes))),
                    },
                )]
                .into(),
            };
            let (base, server) = serve_forever(bytes).await;
            let staged = stage_mac_app(&base, &manifest, &data).await;
            server.abort();
            sources.push(staged.unwrap());
        }
        let installed = root.path().join("installed/RCode.app");
        write_test_bundle(&installed, b"old installation");
        InstallKind::MacApp {
            bundle: installed.clone(),
        }
        .apply_desktop(&sources[1], false)
        .unwrap();
        for source in &sources {
            assert!(
                source.join("Contents/MacOS/rcode").is_file(),
                "another caller must retain its source after a newer stage/apply"
            );
        }
        let expected = std::fs::read(sources[1].join("Contents/MacOS/rcode")).unwrap();
        assert_eq!(
            std::fs::read(installed.join("Contents/MacOS/rcode")).unwrap(),
            expected
        );
        let conflict = Manifest {
            version: "2.0.0".into(),
            files: [(
                mac_app_artifact("2.0.0"),
                FileMeta {
                    sha256: Some("0".repeat(64)),
                },
            )]
            .into(),
        };
        let error = stage_mac_app("http://127.0.0.1:1", &conflict, &data)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("release artifact conflict"));
        assert_eq!(
            std::fs::read(sources[1].join("Contents/MacOS/rcode")).unwrap(),
            expected
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn concurrent_mac_app_stagers_preserve_other_active_downloads() {
        use sha2::{Digest as _, Sha256};
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let other = update_stage_directory(&data.join("updates"), "8.8.8").unwrap();
        std::fs::write(other.path().join("keep"), b"other download").unwrap();
        let bytes = fake_mac_app_tarball(&root.path().join("package"), "9.9.9");
        let expected = format!("{:x}", Sha256::digest(&bytes));
        let manifest = Manifest {
            version: "9.9.9".into(),
            files: [(
                mac_app_artifact("9.9.9"),
                FileMeta {
                    sha256: Some(expected.clone()),
                },
            )]
            .into(),
        };
        let (base, server) = serve_forever(bytes).await;
        let (first, second) = tokio::join!(
            stage_mac_app(&base, &manifest, &data),
            stage_mac_app(&base, &manifest, &data),
        );
        server.abort();
        let staged = first.unwrap();
        assert_eq!(staged, second.unwrap());
        assert!(cached_stage_verified(
            staged.parent().unwrap(),
            &staged.join("Contents/MacOS/rcode"),
            &expected
        ));
        assert_eq!(
            std::fs::read(other.path().join("keep")).unwrap(),
            b"other download"
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn failed_mac_app_stage_cleans_only_its_owned_directory() {
        use sha2::{Digest as _, Sha256};
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let updates = data.join("updates");
        let other = update_stage_directory(&updates, "8.8.8").unwrap();
        std::fs::write(other.path().join("keep"), b"other download").unwrap();
        let bytes = b"this is not a tarball".to_vec();
        let manifest = Manifest {
            version: "9.9.9".into(),
            files: [(
                mac_app_artifact("9.9.9"),
                FileMeta {
                    sha256: Some(format!("{:x}", Sha256::digest(&bytes))),
                },
            )]
            .into(),
        };
        let (base, server) = serve_forever(bytes).await;
        assert!(stage_mac_app(&base, &manifest, &data).await.is_err());
        server.abort();
        assert!(!updates.join("9.9.9").exists());
        assert_eq!(
            std::fs::read(other.path().join("keep")).unwrap(),
            b"other download"
        );
        assert!(std::fs::read_dir(&updates).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".stage-9.9.9-")
        }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn concurrent_headless_stagers_for_one_release_both_succeed() {
        use sha2::{Digest as _, Sha256};
        let root = tempfile::tempdir().unwrap();
        let app_root = root.path().join("app");
        let bytes = fake_headless_tarball(&root.path().join("package"), "9.9.9");
        let expected = format!("{:x}", Sha256::digest(&bytes));
        let manifest = Manifest {
            version: "9.9.9".into(),
            files: [(
                headless_artifact("9.9.9"),
                FileMeta {
                    sha256: Some(expected.clone()),
                },
            )]
            .into(),
        };
        let (base, server) = serve_forever(bytes).await;
        let (first, second) = tokio::join!(
            stage_headless(&base, &manifest, &app_root),
            stage_headless(&base, &manifest, &app_root),
        );
        server.abort();
        let staged = first.unwrap();
        assert_eq!(staged, second.unwrap());
        assert!(cached_stage_verified(
            &staged,
            &staged.join("rcode"),
            &expected
        ));
        assert!(std::fs::read_dir(&app_root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".stage-9.9.9-")
        }));
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_headless_symlink_swaps_use_independent_temporary_paths() {
        let root = tempfile::tempdir().unwrap();
        for version in ["1.0.0", "2.0.0"] {
            std::fs::create_dir(root.path().join(version)).unwrap();
            std::fs::write(root.path().join(version).join("rcode"), b"binary").unwrap();
        }
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            let tasks: Vec<_> = (0..8)
                .map(|i| {
                    let barrier = &barrier;
                    let app_root = root.path();
                    scope.spawn(move || {
                        barrier.wait();
                        apply_headless(app_root, if i % 2 == 0 { "1.0.0" } else { "2.0.0" })
                    })
                })
                .collect();
            for task in tasks {
                task.join().unwrap().unwrap();
            }
        });
        assert!(root.path().join("current/rcode").is_file());
        assert!(std::fs::read_dir(root.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".current-")
        }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn headless_staging_rejects_a_binary_that_is_not_the_promised_release() {
        use sha2::{Digest as _, Sha256};

        let tmp = tempfile::tempdir().unwrap();
        let app_root = tmp.path().join("app");
        std::fs::create_dir_all(&app_root).unwrap();
        let manifest = |bytes: &[u8]| Manifest {
            version: "9.9.9".into(),
            files: [(
                headless_artifact("9.9.9"),
                FileMeta {
                    sha256: Some(format!("{:x}", Sha256::digest(bytes))),
                },
            )]
            .into(),
        };

        let wrong = fake_headless_tarball(&tmp.path().join("wrong"), "1.0.0");
        let (base, server) = serve_forever(wrong.clone()).await;
        let error = stage_headless(&base, &manifest(&wrong), &app_root)
            .await
            .unwrap_err();
        server.abort();
        assert!(
            format!("{error:#}").contains("expected \"rcode 9.9.9\""),
            "unexpected error: {error:#}"
        );
        assert!(!app_root.join("9.9.9").exists());
        assert_eq!(std::fs::read_dir(&app_root).unwrap().count(), 0);

        let right = fake_headless_tarball(&tmp.path().join("right"), "9.9.9");
        let (base, server) = serve_forever(right.clone()).await;
        let staged = stage_headless(&base, &manifest(&right), &app_root)
            .await
            .unwrap();
        server.abort();
        assert_eq!(staged, app_root.join("9.9.9"));
        assert!(staged.join("rcode").is_file());

        // A verified cache is reusable without a network request. A modified
        // executable must instead be replaced from the trusted artifact.
        assert_eq!(
            stage_headless("http://127.0.0.1:1", &manifest(&right), &app_root)
                .await
                .unwrap(),
            staged
        );
        std::fs::write(staged.join("rcode"), b"unverified cached executable").unwrap();
        let (base, server) = serve_forever(right.clone()).await;
        stage_headless(&base, &manifest(&right), &app_root)
            .await
            .unwrap();
        server.abort();
        assert!(cached_stage_verified(
            &staged,
            &staged.join("rcode"),
            release_checksum(&manifest(&right), &headless_artifact("9.9.9")).unwrap()
        ));
    }
}
