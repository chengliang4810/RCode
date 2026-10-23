//! Anthropic's public marketplace, installed into the personal plugin catalog.

use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime},
};

pub const REPOSITORY: &str = "anthropics/claude-plugins-official";
pub const HOMEPAGE: &str = "https://github.com/anthropics/claude-plugins-official";
#[derive(Clone, Copy, Debug)]
pub struct MarketSource {
    pub id: &'static str,
    pub name: &'static str,
    pub repository: &'static str,
    pub catalog_name: &'static str,
}
pub const SOURCES: [MarketSource; 3] = [
    MarketSource {
        id: "official",
        name: "Claude Code",
        repository: REPOSITORY,
        catalog_name: "claude-plugins-official",
    },
    MarketSource {
        id: "ponytail",
        name: "Ponytail",
        repository: "DietrichGebert/ponytail",
        catalog_name: "ponytail",
    },
    MarketSource {
        id: "superpowers",
        name: "Superpowers",
        repository: "obra/superpowers",
        catalog_name: "superpowers-dev",
    },
];
fn market_source(id: &str) -> anyhow::Result<&'static MarketSource> {
    SOURCES
        .iter()
        .find(|source| source.id == id)
        .context("unknown built-in marketplace")
}
const CATALOG_LIMIT: usize = 8 * 1024 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(60 * 60);
const ARCHIVE_LIMIT: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub author: Value,
    pub source: Value,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default = "strict_default")]
    pub strict: bool,
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, Value>,
}
fn strict_default() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Catalog {
    pub name: String,
    pub plugins: Vec<Entry>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub plugins: Vec<Item>,
    pub cached: bool,
    #[serde(default)]
    pub stale: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub name: String,
    pub description: String,
    pub author: Option<String>,
    pub installed: bool,
    pub unsupported: bool,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name != "."
        && name != ".."
        && !name.ends_with(['.', ' '])
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}

fn parse_catalog(bytes: &[u8], market: &MarketSource) -> anyhow::Result<Catalog> {
    let catalog: Catalog = serde_json::from_slice(bytes).context("invalid marketplace JSON")?;
    ensure!(
        catalog.name == market.catalog_name,
        "unexpected marketplace identity"
    );
    ensure!(
        !catalog.plugins.is_empty() && catalog.plugins.len() <= 5000,
        "invalid marketplace size"
    );
    let mut names = HashSet::new();
    for entry in &catalog.plugins {
        ensure!(
            valid_name(&entry.name) && names.insert(&entry.name),
            "invalid or duplicate plugin name"
        );
    }
    Ok(catalog)
}

fn cache_path(config: &Path, market: &MarketSource) -> PathBuf {
    config
        .join("cache")
        // Do not reuse catalogs previously obtained from unauthenticated mirrors.
        .join(format!("marketplace-origin-{}.json", market.id))
}

async fn catalog(
    config: &Path,
    market: &MarketSource,
    refresh: bool,
) -> anyhow::Result<(Catalog, bool, bool)> {
    catalog_with_fetch(config, market, refresh, || async {
        let url = format!(
            "https://raw.githubusercontent.com/{}/main/.claude-plugin/marketplace.json",
            market.repository
        );
        rcode_github::fetch_origin(
            &rcode_github::client()?,
            &url,
            CATALOG_LIMIT,
            Duration::from_secs(20),
            |bytes| Ok((parse_catalog(bytes, market)?, bytes.to_vec())),
        )
        .await
    })
    .await
}

fn cache_is_stale(modified: Option<SystemTime>, now: SystemTime) -> bool {
    modified
        .and_then(|modified| now.duration_since(modified).ok())
        .is_none_or(|age| age >= CACHE_TTL)
}

async fn cached_catalog(path: &Path, market: &MarketSource) -> Option<(Catalog, bool, bool)> {
    let bytes = tokio::fs::read(path).await.ok()?;
    let catalog = parse_catalog(&bytes, market).ok()?;
    let modified = tokio::fs::metadata(path)
        .await
        .ok()
        .and_then(|m| m.modified().ok());
    Some((catalog, true, cache_is_stale(modified, SystemTime::now())))
}

async fn catalog_with_fetch<F, Fut>(
    config: &Path,
    market: &MarketSource,
    refresh: bool,
    fetch: F,
) -> anyhow::Result<(Catalog, bool, bool)>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<(Catalog, Vec<u8>)>>,
{
    let path = cache_path(config, market);
    if !refresh {
        if let Some(cached) = cached_catalog(&path, market).await {
            return Ok(cached);
        }
    }
    match fetch().await {
        Ok((catalog, bytes)) => {
            // A failed cache write does not discard a successfully loaded marketplace.
            let parent = path.parent().unwrap();
            if tokio::fs::create_dir_all(parent).await.is_ok() {
                if let Ok(temporary) = tempfile::NamedTempFile::new_in(parent) {
                    if std::fs::write(temporary.path(), bytes).is_ok() {
                        temporary.persist(&path).ok();
                    }
                }
            }
            Ok((catalog, false, false))
        }
        Err(error) => {
            if let Some(cached) = cached_catalog(&path, market).await {
                return Ok(cached);
            }
            Err(error)
        }
    }
}

pub async fn list(config: &Path, source_id: &str, refresh: bool) -> anyhow::Result<Listing> {
    let (catalog, cached, stale) = catalog(config, market_source(source_id)?, refresh).await?;
    let installed: HashSet<_> = crate::discover(config)
        .into_iter()
        .map(|p| p.name)
        .collect();
    let plugins = catalog
        .plugins
        .into_iter()
        .map(|entry| Item {
            installed: installed.contains(&entry.name),
            unsupported: entry.extra.contains_key("lspServers")
                && !["skills", "commands", "agents", "hooks", "mcpServers"]
                    .iter()
                    .any(|key| entry.extra.contains_key(*key)),
            author: entry
                .author
                .as_str()
                .or_else(|| entry.author.get("name").and_then(Value::as_str))
                .map(str::to_owned),
            name: entry.name,
            description: entry.description,
        })
        .collect();
    Ok(Listing {
        plugins,
        cached,
        stale,
    })
}

#[derive(Debug, PartialEq)]
struct Source {
    repo: String,
    revision: String,
    path: PathBuf,
}

fn relative_path(value: &str) -> anyhow::Result<PathBuf> {
    let value = value.strip_prefix("./").unwrap_or(value);
    ensure!(
        !value.contains('\\') && !value.contains(':') && !value.starts_with('/'),
        "invalid plugin path"
    );
    let path = PathBuf::from(value);
    ensure!(
        path.components().all(|c| matches!(c, Component::Normal(value) if !value.to_string_lossy().ends_with(['.', ' ']))),
        "plugin path escapes repository"
    );
    Ok(path)
}

fn source(entry: &Entry, market: &MarketSource) -> anyhow::Result<Source> {
    if let Some(path) = entry.source.as_str() {
        return Ok(Source {
            repo: market.repository.into(),
            revision: "main".into(),
            path: relative_path(path)?,
        });
    }
    let kind = entry
        .source
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default();
    ensure!(
        matches!(kind, "url" | "github" | "git-subdir"),
        "unsupported plugin source: {kind}"
    );
    let raw = entry
        .source
        .get("url")
        .or_else(|| entry.source.get("repo"))
        .and_then(Value::as_str)
        .context("missing plugin repository")?;
    let repo = raw
        .strip_prefix("https://github.com/")
        .unwrap_or(raw)
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let parts: Vec<_> = repo.split('/').collect();
    ensure!(
        parts.len() == 2 && parts.iter().all(|p| valid_name(p)),
        "plugin source must be a public GitHub repository"
    );
    let revision = entry
        .source
        .get("sha")
        .or_else(|| entry.source.get("ref"))
        .and_then(Value::as_str)
        .unwrap_or("HEAD");
    ensure!(
        !revision.is_empty()
            && revision
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-._/".contains(&c)),
        "invalid GitHub revision"
    );
    let path = relative_path(
        entry
            .source
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )?;
    Ok(Source {
        repo: repo.into(),
        revision: revision.into(),
        path,
    })
}

pub async fn install(config: &Path, source_id: &str, name: &str) -> anyhow::Result<()> {
    ensure!(valid_name(name), "invalid plugin name");
    ensure!(
        !config.join(crate::PLUGINS_DIR).join(name).exists(),
        "plugin {name} is already installed"
    );
    let market = market_source(source_id)?;
    let (catalog, _, _) = catalog(config, market, false).await?;
    let entry = catalog
        .plugins
        .into_iter()
        .find(|p| p.name == name)
        .context("plugin is no longer in the marketplace")?;
    let source = source(&entry, market)?;
    let client = rcode_github::client()?;
    let revision = resolve_revision(&client, &source).await?;
    let url = format!(
        "https://github.com/{}/archive/{}.zip",
        source.repo, revision
    );
    tokio::fs::create_dir_all(config).await?;
    let temporary = tempfile::tempdir_in(config)?;
    let archive = temporary.path().join("source.zip");
    // A Git commit ID is not a SHA-256 digest of its ZIP archive. Fetch from
    // GitHub itself: a syntactically valid mirror ZIP cannot prove its identity.
    let bytes = rcode_github::fetch_origin(
        &client,
        &url,
        ARCHIVE_LIMIT as usize,
        Duration::from_secs(180),
        |bytes| {
            zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("invalid plugin ZIP")?;
            Ok(bytes.to_vec())
        },
    )
    .await?;
    tokio::fs::write(&archive, bytes).await?;
    let config = config.to_owned();
    tokio::task::spawn_blocking(move || {
        let _temporary = temporary;
        publish_archive(&config, &entry, &archive, &source.path)
    })
    .await?
}

fn is_commit_sha(revision: &str) -> bool {
    revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_commit_sha(bytes: &[u8]) -> anyhow::Result<String> {
    #[derive(Deserialize)]
    struct Commit {
        sha: String,
    }
    let commit: Commit = serde_json::from_slice(bytes).context("invalid GitHub commit response")?;
    ensure!(is_commit_sha(&commit.sha), "invalid GitHub commit SHA");
    Ok(commit.sha)
}

/// Branches, tags, HEAD and abbreviated SHAs remain supported, but the archive
/// request uses the immutable commit resolved by the official GitHub API.
async fn resolve_revision(client: &reqwest::Client, source: &Source) -> anyhow::Result<String> {
    resolve_revision_with_fetch(source, |url| async move {
        rcode_github::fetch_origin(
            client,
            &url,
            1024 * 1024,
            Duration::from_secs(30),
            parse_commit_sha,
        )
        .await
    })
    .await
}

async fn resolve_revision_with_fetch<F, Fut>(source: &Source, fetch: F) -> anyhow::Result<String>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<String>>,
{
    if is_commit_sha(&source.revision) {
        return Ok(source.revision.clone());
    }
    let mut url = reqwest::Url::parse(&format!(
        "https://api.github.com/repos/{}/commits/",
        source.repo
    ))?;
    url.path_segments_mut()
        .expect("GitHub URL has path segments")
        .pop_if_empty()
        .push(&source.revision);
    fetch(url.to_string()).await
}

fn publish_archive(
    config: &Path,
    entry: &Entry,
    archive: &Path,
    source_path: &Path,
) -> anyhow::Result<()> {
    let temporary = tempfile::tempdir_in(config)?;
    let staging = temporary.path().join("plugin");
    extract(archive, source_path, &staging)?;
    normalize(
        entry,
        &staging,
        &config.join(crate::PLUGINS_DIR).join(&entry.name),
    )?;
    super::read_plugin(
        &staging,
        &super::manifest_path(&staging),
        Some(&super::StateFile::default()),
    )?;
    let plugins = config.join(crate::PLUGINS_DIR);
    std::fs::create_dir_all(&plugins)?;
    let destination = plugins.join(&entry.name);
    static INSTALL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = INSTALL_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    ensure!(
        !destination.exists(),
        "plugin {} is already installed",
        entry.name
    );
    std::fs::rename(&staging, &destination).context("publishing plugin")
}

fn extract(archive: &Path, subdir: &Path, destination: &Path) -> anyhow::Result<()> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
    ensure!(
        zip.len() <= 100_000,
        "repository archive has too many entries"
    );
    let mut total = 0u64;
    let mut files = 0;
    let mut root_name = None;
    for index in 0..zip.len() {
        let mut file = zip.by_index(index)?;
        let path = file.enclosed_name().context("unsafe ZIP path")?;
        let mut components = path.components();
        let root = components.next().context("empty ZIP path")?;
        if let Some(expected) = &root_name {
            ensure!(expected == &root.as_os_str(), "multiple ZIP roots");
        } else {
            root_name = Some(root.as_os_str().to_owned());
        }
        let path = components.as_path();
        let Ok(relative) = path.strip_prefix(subdir) else {
            continue;
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        ensure!(
            relative
                .components()
                .all(|c| matches!(c, Component::Normal(value) if !value.to_string_lossy().ends_with(['.', ' ']) && !value.to_string_lossy().contains([':', '\\']))),
            "unsafe ZIP path"
        );
        ensure!(
            file.unix_mode()
                .is_none_or(|mode| mode & 0o170000 != 0o120000),
            "plugin symlinks are not supported"
        );
        let output = destination.join(relative);
        if file.is_dir() {
            std::fs::create_dir_all(&output)?;
            continue;
        }
        total = total.saturating_add(file.size());
        files += 1;
        ensure!(
            total <= 512 * 1024 * 1024 && file.size() <= 32 * 1024 * 1024 && files <= 10_000,
            "plugin archive exceeds extraction limits"
        );
        std::fs::create_dir_all(output.parent().context("invalid archive path")?)?;
        let mut target = std::fs::File::create(output)?;
        let expected = file.size();
        let copied = std::io::copy(
            &mut std::io::Read::take(&mut file, 32 * 1024 * 1024 + 1),
            &mut target,
        )?;
        ensure!(
            copied == expected && copied <= 32 * 1024 * 1024,
            "invalid extracted file size"
        );
        #[cfg(unix)]
        if let Some(mode) = file.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            target.set_permissions(std::fs::Permissions::from_mode(mode & 0o777))?;
        }
    }
    ensure!(files > 0, "plugin directory is missing from archive");
    Ok(())
}

fn normalize(entry: &Entry, root: &Path, installed_root: &Path) -> anyhow::Result<()> {
    let manifest_path = super::manifest_path(root);
    let mut manifest: Value = if manifest_path.is_file() {
        serde_json::from_slice(&std::fs::read(&manifest_path)?)?
    } else {
        ensure!(!entry.strict, "plugin has no .claude-plugin/plugin.json");
        json!({})
    };
    let object = manifest
        .as_object_mut()
        .context("plugin manifest must be an object")?;
    if !entry.strict {
        for key in [
            "commands",
            "skills",
            "agents",
            "hooks",
            "mcpServers",
            "lspServers",
        ] {
            if let Some(value) = entry.extra.get(key) {
                object.entry(key).or_insert_with(|| value.clone());
            }
        }
    }
    object.insert("name".into(), json!(entry.name));
    if !object.contains_key("description") {
        object.insert("description".into(), json!(entry.description));
    }
    if !object.contains_key("version") {
        if let Some(version) = &entry.version {
            object.insert("version".into(), json!(version));
        }
    }
    if !object.contains_key("author") && !entry.author.is_null() {
        object.insert("author".into(), entry.author.clone());
    }
    for (key, target) in [
        ("hooks", "hooks/marketplace-hooks.json"),
        ("mcpServers", ".marketplace-mcp.json"),
    ] {
        if object.get(key).is_some_and(Value::is_object) {
            let value = object.remove(key).unwrap();
            let value = if key == "mcpServers" && value.get("mcpServers").is_none() {
                json!({"mcpServers": value})
            } else {
                value
            };
            let path = root.join(target);
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(path, serde_json::to_vec_pretty(&value)?)?;
            object.insert(key.into(), json!(format!("./{target}")));
        }
    }
    for key in ["commands", "skills", "agents", "hooks", "mcpServers"] {
        if let Some(value) = object.get(key) {
            let paths: Vec<&str> = if let Some(path) = value.as_str() {
                vec![path]
            } else {
                value
                    .as_array()
                    .context("contribution paths must be strings")?
                    .iter()
                    .map(|v| v.as_str().context("invalid contribution path"))
                    .collect::<anyhow::Result<_>>()?
            };
            for path in paths {
                relative_path(path)?;
            }
        }
    }
    std::fs::create_dir_all(manifest_path.parent().unwrap())?;
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    let plugin = super::read_plugin(root, &manifest_path, Some(&super::StateFile::default()))?;
    ensure!(
        !plugin.contributions.is_empty(),
        "plugin has no supported contributions (LSP-only plugins are not supported)"
    );
    // Claude's MCP files also commonly omit the mcpServers wrapper.
    for path in plugin
        .contributions
        .mcp
        .iter()
        .chain(plugin.contributions.hooks.iter())
    {
        let mut value: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        if plugin.contributions.mcp.contains(path) && value.get("mcpServers").is_none() {
            value = json!({"mcpServers": value});
        }
        if plugin.contributions.mcp.contains(path) {
            expand_plugin_root(&mut value, &installed_root.to_string_lossy());
        }
        std::fs::write(path, serde_json::to_vec_pretty(&value)?)?;
    }
    Ok(())
}

fn expand_plugin_root(value: &mut Value, root: &str) {
    match value {
        Value::String(text) => *text = text.replace("${CLAUDE_PLUGIN_ROOT}", root),
        Value::Array(values) => {
            for value in values {
                expand_plugin_root(value, root);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                expand_plugin_root(value, root);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
