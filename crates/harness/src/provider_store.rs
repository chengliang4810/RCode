//! The shared provider configuration: one snapshot, one writer.
//!
//! `providers.json` is read by a run at start and written by the settings page.
//! Left as a plain file that both sides open whenever they feel like it, that
//! arrangement has three failure modes, and this module exists to close them:
//!
//! - **A run must not change vendor underneath itself.** A run takes one
//!   snapshot when it starts and keeps it — provider, model, resolved
//!   credentials — for its whole life. Saving a new configuration does not
//!   reach a conversation already in progress.
//! - **Two writers must not lose each other's work.** Saves carry the revision
//!   they were drafted against, and a save whose revision is stale is refused
//!   rather than clobbering. A settings window left open while the file is
//!   edited by hand is the ordinary case.
//! - **A failure must not destroy the configuration.** The write goes to a
//!   temporary file in the same directory and is renamed into place, so an
//!   interrupted save leaves the previous file intact and the in-memory
//!   snapshot still serving.
//!
//! Credentials never leave in a readable form. The view a UI receives says
//! whether a key is set, and names the environment variable when it is one —
//! never the value, and never a mask that could be written back over the real
//! key by a save that did not mean to touch it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use sha2::{Digest, Sha256};

use crate::providers::{ModelConfig, Protocol, ProviderConfig, ProvidersConfig, ProvidersError};

/// The revision of a configuration file that does not exist.
///
/// A fixed string rather than a hash of nothing, so "the file is absent" is
/// distinguishable from "the file is empty" and both have a stable revision to
/// draft against. A UI that opens on a fresh install can then save, and a
/// second window that also opened before the first saved will be refused.
const ABSENT_REVISION: &str = "absent";

/// A configuration and the revision it was read at.
///
/// Cloned by `Arc`, so a run holding one pays nothing to keep it.
#[derive(Debug, Clone)]
pub struct ProviderSnapshot {
    pub config: ProvidersConfig,
    /// A digest of the file's bytes, or [`ABSENT_REVISION`].
    pub revision: String,
}

/// What a caller may do to a provider's credential.
///
/// `Keep` ensures that an empty edit does not damage a key whose value the UI
/// has not requested explicitly.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CredentialUpdate {
    Keep,
    /// Store this literal key.
    ReplaceLiteral {
        value: String,
    },
    /// Preserve the legacy environment-variable mode for older clients.
    UseEnvironment {
        name: String,
    },
    /// Preserve the legacy clear operation for older clients.
    Clear,
}

/// The credential as a UI may see it.
///
/// `Debug` is derived deliberately: nothing here is secret. The *value* never
/// reaches this type — see the module note.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CredentialView {
    /// No key is configured.
    None,
    /// A literal key is stored. The value is not carried.
    Stored,
    /// The key comes from an environment variable, named here.
    ///
    /// `set` reports whether the variable is set **in this process**, which is
    /// a property of how the application was launched rather than of the file.
    /// The UI needs it to say "configured but not visible here" instead of
    /// leaving the user to guess why a run fails.
    Environment { name: String, set: bool },
}

/// One model, as a UI may see it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelView {
    pub id: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// What the model accepts as input, as the settings page must show it.
    #[serde(default)]
    pub input_format: crate::providers::InputFormat,
    /// The model's effort ladder. `Vec` rather than a set: the order is what
    /// the picker offers them in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_levels: Vec<rcode_proto::ReasoningLevel>,
}

/// A vendor model discovered through its models API, before local configuration.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredModel {
    pub model: String,
    pub display_name: Option<String>,
    pub owned_by: Option<String>,
}

/// One provider, as a UI may see it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub protocol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub credential: CredentialView,
    pub models: Vec<ModelView>,
}

/// The whole configuration, as a UI may see it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettingsView {
    pub revision: String,
    pub providers: Vec<ProviderView>,
}

/// Why a save was refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SaveError {
    /// The draft was written against a revision that is no longer current.
    /// The UI keeps its draft and asks the user to reload.
    #[error("the configuration changed since this was opened")]
    Conflict { current: String },
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Io(String),
}

/// The prefix `SaveProviderSettings` puts on a refusal that is a conflict.
///
/// The reason travels as an error string because that is what the RPC channel
/// carries, and a caller that has to distinguish "the file moved" from "the
/// file is wrong" needs something stable to match. Named here so both sides
/// name the same thing.
pub const CONFLICT_CODE: &str = "revision-conflict";

/// A draft a UI submits, with each provider's credential intent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDraft {
    /// Legacy in-process field, omitted from the settings RPC and ignored on save.
    #[serde(skip)]
    pub default_model: Option<String>,
    pub providers: Vec<ProviderDraftEntry>,
    /// The search configuration, or `None` to leave the stored one alone.
    ///
    /// Optional because the settings page does not edit search: a save from it
    /// sends nothing here, and a default-constructed value would wipe an
    /// endpoint and token the user configured by hand — silently, because
    /// nothing on that page would show it had happened.
    #[serde(default)]
    pub web_search: Option<rcode_agent::tools::WebSearchConfig>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDraftEntry {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    pub protocol: Protocol,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelConfig>,
    /// Credential updates support only literal API keys. Missing means `Keep`.
    #[serde(default = "default_credential_update")]
    pub credential: CredentialUpdate,
}

fn default_credential_update() -> CredentialUpdate {
    CredentialUpdate::Keep
}

/// The one configuration this process serves.
pub struct ProviderStore {
    data_dir: PathBuf,
    /// The current snapshot. Read-heavy and rarely written, which is what the
    /// read-write lock is for.
    current: RwLock<Arc<ProviderSnapshot>>,
    /// Held for the whole of a save: read the file, check, write, swap. Without
    /// it two saves could both read the same revision and both proceed.
    write: Mutex<()>,
}

impl ProviderStore {
    /// Open the store, reading whatever is on disk.
    ///
    /// A malformed file is reported, not silently replaced by an empty
    /// configuration — an empty one would look like the user's setup had
    /// vanished, and the next save would make that true.
    pub fn open(data_dir: &Path) -> Result<Self, ProvidersError> {
        let snapshot = read_snapshot(data_dir)?;
        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            current: RwLock::new(Arc::new(snapshot)),
            write: Mutex::new(()),
        })
    }

    /// A store over a file that could not be read.
    ///
    /// Holds an empty configuration so the application starts, but reports the
    /// absent revision — which means a save will re-read the file, fail to
    /// parse it, and refuse. That is the point: a corrupt configuration must
    /// not be silently replaced by an empty one, and the user has to be told
    /// what is wrong rather than discovering it when their setup vanishes.
    pub fn degraded(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            current: RwLock::new(Arc::new(ProviderSnapshot {
                config: ProvidersConfig::default(),
                revision: ABSENT_REVISION.to_owned(),
            })),
            write: Mutex::new(()),
        }
    }

    /// The current configuration. Cheap: one `Arc` clone.
    pub fn snapshot(&self) -> Arc<ProviderSnapshot> {
        Arc::clone(&self.read())
    }

    /// The redacted view a settings page renders.
    pub fn view(&self) -> ProviderSettingsView {
        let snapshot = self.read();
        ProviderSettingsView {
            revision: snapshot.revision.clone(),
            providers: snapshot
                .config
                .providers
                .iter()
                .map(provider_view)
                .collect(),
        }
    }

    /// Read one provider's resolved key after an explicit user request.
    pub fn api_key(
        &self,
        provider_id: &str,
        environment: &std::collections::BTreeMap<String, String>,
    ) -> Option<String> {
        self.read()
            .config
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)?
            .resolve_api_key(environment)
    }

    /// Write a draft, if it was drafted against the current revision.
    ///
    /// See the module note for what this guarantees. The steps are ordered so
    /// that a failure at any point leaves both the file and the in-memory
    /// snapshot as they were.
    pub fn save(
        &self,
        draft: ProviderDraft,
        expected_revision: &str,
    ) -> Result<ProviderSettingsView, SaveError> {
        let _writing = self.write.lock().unwrap_or_else(PoisonError::into_inner);

        // 1. Re-read, so the check is against the file rather than against the
        //    snapshot this process happens to hold.
        let on_disk =
            read_snapshot(&self.data_dir).map_err(|error| SaveError::Io(error.to_string()))?;
        if on_disk.revision != expected_revision {
            return Err(SaveError::Conflict {
                current: on_disk.revision,
            });
        }

        // 2. Build and validate.
        let config = draft.into_config(&on_disk.config)?;
        config.validate().map_err(SaveError::Invalid)?;

        // 3-4. Write beside the file, then rename into place.
        write_atomically(&self.data_dir, &config)
            .map_err(|error| SaveError::Io(error.to_string()))?;

        // 5. Only now does the in-memory snapshot move.
        let fresh =
            read_snapshot(&self.data_dir).map_err(|error| SaveError::Io(error.to_string()))?;
        *self.write_current() = Arc::new(fresh.clone());
        Ok(ProviderSettingsView {
            revision: fresh.revision.clone(),
            providers: fresh.config.providers.iter().map(provider_view).collect(),
        })
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Arc<ProviderSnapshot>> {
        self.current.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write_current(&self) -> std::sync::RwLockWriteGuard<'_, Arc<ProviderSnapshot>> {
        self.current.write().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ProviderDraft {
    /// Apply this draft to the configuration it was drafted from.
    ///
    /// Takes the previous configuration so a `Keep` credential can carry the
    /// stored value forward: the UI never had it, so it cannot send it back,
    /// and without this a save that only renamed a provider would erase its
    /// key.
    fn into_config(&self, previous: &ProvidersConfig) -> Result<ProvidersConfig, SaveError> {
        let mut providers = Vec::with_capacity(self.providers.len());
        for entry in &self.providers {
            let prior = previous.providers.iter().find(|p| p.id == entry.id);
            let api_key = match &entry.credential {
                CredentialUpdate::Keep => prior.and_then(|p| p.api_key.clone()),
                CredentialUpdate::ReplaceLiteral { value } => {
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        return Err(SaveError::Invalid(format!(
                            "provider `{}`: the key is empty; use Keep to preserve it",
                            entry.id
                        )));
                    }
                    Some(trimmed.to_owned())
                }
                CredentialUpdate::UseEnvironment { name } => {
                    let name = name.trim();
                    if name.is_empty() {
                        return Err(SaveError::Invalid(format!(
                            "provider `{}`: the environment variable name is empty",
                            entry.id
                        )));
                    }
                    Some(format!("${name}"))
                }
                CredentialUpdate::Clear => None,
            };

            providers.push(ProviderConfig {
                id: entry.id.trim().to_owned(),
                label: entry.label.clone(),
                protocol: entry.protocol,
                base_url: entry.base_url.clone(),
                api_key,
                models: entry.models.clone(),
            });
        }

        Ok(ProvidersConfig {
            providers,
            default_model: None,
            web_search: self
                .web_search
                .clone()
                .unwrap_or_else(|| previous.web_search.clone()),
        })
    }
}

/// The redacted view of one provider.
fn provider_view(provider: &ProviderConfig) -> ProviderView {
    let credential = match provider.api_key.as_deref().map(str::trim) {
        None | Some("") => CredentialView::None,
        Some(_) => CredentialView::Stored,
    };
    ProviderView {
        id: provider.id.clone(),
        label: provider.label.clone(),
        protocol: protocol_name(provider.protocol).to_owned(),
        base_url: provider.base_url.clone(),
        credential,
        models: provider
            .models
            .iter()
            .map(|model| ModelView {
                id: model.id.clone(),
                model: model.model.clone(),
                label: model.label.clone(),
                context_window: model.context_window,
                max_tokens: model.max_tokens,
                input_format: model.input_format,
                reasoning_levels: model.reasoning_levels.clone(),
            })
            .collect(),
    }
}

fn protocol_name(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenAiChat => "openAiChat",
        Protocol::OpenAiResponses => "openAiResponses",
        Protocol::AnthropicMessages => "anthropicMessages",
    }
}

/// Read the file and hash it.
fn read_snapshot(data_dir: &Path) -> Result<ProviderSnapshot, ProvidersError> {
    let path = ProvidersConfig::path(data_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ProviderSnapshot {
                config: ProvidersConfig::default(),
                revision: ABSENT_REVISION.to_owned(),
            });
        }
        Err(error) => return Err(ProvidersError::Io(error.to_string())),
    };
    // Parsed before the revision is computed, so a corrupt file is reported as
    // corrupt rather than handed back as a configuration with a revision.
    let config: ProvidersConfig =
        serde_json::from_slice(&bytes).map_err(|error| ProvidersError::Parse(error.to_string()))?;
    Ok(ProviderSnapshot {
        config,
        revision: digest(&bytes),
    })
}

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Write the configuration beside the real file, then rename it into place.
fn write_atomically(data_dir: &Path, config: &ProvidersConfig) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let text = serde_json::to_string_pretty(config)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let path = ProvidersConfig::path(data_dir);
    // A unique temporary name, so two processes cannot collide on one.
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&temporary, text)?;
    // Same directory, so the rename is atomic on every platform that matters.
    std::fs::rename(&temporary, &path)
}

#[cfg(test)]
#[path = "provider_store_tests.rs"]
mod tests;
