//! Local models.dev metadata shared by manual entry and model discovery.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use rcode_proto::ReasoningLevel;
use serde::Deserialize;

use crate::providers::{InputFormat, Protocol};

pub const FILE_NAME: &str = "models.json";
pub const DEFAULT_CONTEXT: u32 = 200_000;
pub const DEFAULT_OUTPUT: u32 = 128_000;
pub const BODY_LIMIT: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelMetadata {
    pub name: Option<String>,
    pub context_window: u32,
    pub max_tokens: u32,
    pub input_format: InputFormat,
    pub reasoning_levels: Vec<ReasoningLevel>,
}

impl Default for ModelMetadata {
    fn default() -> Self {
        Self {
            name: None,
            context_window: DEFAULT_CONTEXT,
            max_tokens: DEFAULT_OUTPUT,
            input_format: InputFormat::Text,
            reasoning_levels: Vec::new(),
        }
    }
}

#[derive(Deserialize)]
struct Provider {
    api: Option<String>,
    #[serde(default)]
    models: BTreeMap<String, CatalogModel>,
}

#[derive(Deserialize)]
struct CatalogModel {
    #[serde(default)]
    id: String,
    name: Option<String>,
    canonical_model_id: Option<String>,
    #[serde(default)]
    limit: Limits,
    #[serde(default)]
    modalities: Modalities,
    #[serde(default)]
    reasoning: bool,
    #[serde(default)]
    reasoning_options: Vec<ReasoningOption>,
}

#[derive(Default, Deserialize)]
struct Limits {
    context: Option<u32>,
    output: Option<u32>,
}

#[derive(Default, Deserialize)]
struct Modalities {
    #[serde(default)]
    input: Vec<String>,
}

#[derive(Deserialize)]
struct ReasoningOption {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    values: Vec<Option<String>>,
}

struct Entry {
    provider: String,
    host: Option<String>,
    origin: bool,
    model: CatalogModel,
}

#[derive(Default)]
pub struct ModelCatalog {
    entries: Vec<Entry>,
    aliases: HashMap<String, Vec<usize>>,
}

fn host(url: &str) -> Option<String> {
    url::Url::parse(url.trim().trim_end_matches('#'))
        .ok()?
        .host_str()
        .map(str::to_owned)
}

impl ModelCatalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > BODY_LIMIT {
            return Err("model catalog is too large".into());
        }
        let providers: BTreeMap<String, Provider> = serde_json::from_slice(bytes)
            .map_err(|error| format!("invalid models.dev catalog: {error}"))?;
        let mut catalog = Self::default();
        for (provider, data) in providers {
            for (key, model) in data.models {
                let mut aliases = vec![key.clone(), model.id.clone(), format!("{provider}/{key}")];
                if let Some(canonical) = &model.canonical_model_id {
                    aliases.push(canonical.clone());
                }
                let origin = model
                    .canonical_model_id
                    .as_deref()
                    .and_then(|id| id.split_once('/'))
                    .is_some_and(|(owner, _)| owner == provider);
                let index = catalog.entries.len();
                aliases.sort();
                aliases.dedup();
                for alias in aliases.into_iter().filter(|alias| !alias.is_empty()) {
                    catalog
                        .aliases
                        .entry(alias.to_ascii_lowercase())
                        .or_default()
                        .push(index);
                }
                catalog.entries.push(Entry {
                    provider: provider.clone(),
                    host: data.api.as_deref().and_then(host),
                    origin,
                    model,
                });
            }
        }
        if catalog.entries.is_empty() {
            return Err("empty models.dev catalog".into());
        }
        // Some original vendors omit canonical_model_id while gateways supply
        // it. Their references still identify the original vendor's own entry.
        let canonical_ids = catalog
            .entries
            .iter()
            .filter_map(|entry| entry.model.canonical_model_id.clone())
            .collect::<Vec<_>>();
        for canonical in canonical_ids {
            if let Some((owner, _)) = canonical.split_once('/') {
                if let Some(indices) = catalog.aliases.get(&canonical.to_ascii_lowercase()) {
                    for &index in indices {
                        if catalog.entries[index].provider == owner {
                            catalog.entries[index].origin = true;
                        }
                    }
                }
            }
        }
        Ok(catalog)
    }

    pub fn lookup(
        &self,
        model: &str,
        provider: &str,
        endpoint: &str,
        protocol: Protocol,
    ) -> ModelMetadata {
        let query = model.trim().to_ascii_lowercase();
        let Some(candidates) = self.aliases.get(&query) else {
            return ModelMetadata::default();
        };
        let endpoint_host = host(endpoint);
        let qualified = query.split_once('/').map(|(owner, _)| owner);
        // Prefer the selected vendor, then an explicitly qualified model, then
        // its canonical origin. Equal ambiguous proxy entries are not evidence.
        let score = |entry: &Entry| {
            if endpoint_host.is_some() && entry.host == endpoint_host {
                4
            } else if entry.provider.eq_ignore_ascii_case(provider) {
                3
            } else if qualified == Some(entry.provider.as_str()) {
                2
            } else if entry.origin {
                1
            } else {
                0
            }
        };
        let best = candidates
            .iter()
            .map(|&i| score(&self.entries[i]))
            .max()
            .unwrap_or(0);
        let mut matches = candidates
            .iter()
            .map(|&i| &self.entries[i])
            .filter(|entry| score(entry) == best);
        let Some(entry) = matches.next() else {
            return ModelMetadata::default();
        };
        let metadata = entry.model.metadata(protocol);
        if matches.any(|entry| {
            let other = entry.model.metadata(protocol);
            other.context_window != metadata.context_window
                || other.max_tokens != metadata.max_tokens
                || other.input_format != metadata.input_format
                || other.reasoning_levels != metadata.reasoning_levels
        }) {
            return ModelMetadata::default();
        }
        metadata
    }
}

impl CatalogModel {
    fn metadata(&self, protocol: Protocol) -> ModelMetadata {
        let context_window = self
            .limit
            .context
            .filter(|&n| n > 0)
            .unwrap_or(DEFAULT_CONTEXT);
        let max_tokens = self
            .limit
            .output
            .filter(|&n| n > 0)
            .unwrap_or(DEFAULT_OUTPUT.min(context_window));
        let mut reasoning_levels = Vec::new();
        if self.reasoning {
            for option in &self.reasoning_options {
                if option.kind == "effort" {
                    reasoning_levels.extend(option.values.iter().flatten().filter_map(|value| {
                        serde_json::from_value::<ReasoningLevel>(serde_json::Value::String(
                            value.clone(),
                        ))
                        .ok()
                    }));
                }
            }
            // The Anthropic adapter already implements these token-budget tiers.
            // A boolean/toggle alone does not establish an effort ladder.
            if reasoning_levels.is_empty()
                && protocol == Protocol::AnthropicMessages
                && self
                    .reasoning_options
                    .iter()
                    .any(|option| option.kind == "budget_tokens")
            {
                reasoning_levels = vec![
                    ReasoningLevel::Minimal,
                    ReasoningLevel::Low,
                    ReasoningLevel::Medium,
                    ReasoningLevel::High,
                ];
            }
        }
        reasoning_levels.sort();
        reasoning_levels.dedup();
        ModelMetadata {
            name: self.name.clone().filter(|name| !name.trim().is_empty()),
            context_window,
            max_tokens,
            input_format: if self.modalities.input.iter().any(|mode| mode == "image") {
                InputFormat::TextAndImages
            } else {
                InputFormat::Text
            },
            reasoning_levels,
        }
    }
}

pub fn cache_path(root: &Path) -> PathBuf {
    root.join("cache").join(FILE_NAME)
}

struct Cached {
    stamp: (Option<SystemTime>, u64),
    catalog: Arc<ModelCatalog>,
}

/// Reuse the parsed index until the atomically replaced local file changes.
/// Startup warms it on a worker thread; individual keystrokes only stat + query.
pub fn cached(root: &Path) -> Arc<ModelCatalog> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Cached>>> = OnceLock::new();
    let path = cache_path(root);
    let mut cache = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let metadata = std::fs::metadata(&path).ok();
    let stamp = metadata.as_ref().map(|m| (m.modified().ok(), m.len()));
    if let Some(stamp) = stamp {
        if cache.get(&path).is_none_or(|entry| entry.stamp != stamp) {
            let catalog = (stamp.1 <= BODY_LIMIT as u64)
                .then(|| std::fs::read(&path).ok())
                .flatten()
                .and_then(|bytes| ModelCatalog::parse(&bytes).ok());
            let catalog = catalog
                .map(Arc::new)
                .or_else(|| cache.get(&path).map(|entry| entry.catalog.clone()))
                .unwrap_or_default();
            cache.insert(path.clone(), Cached { stamp, catalog });
        }
    }
    cache
        .get(&path)
        .map(|entry| entry.catalog.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "openai": {"models": {"gpt-test": {
                "id":"gpt-test", "name":"GPT Test",
                "limit":{"context":400000,"output":128000},
                "modalities":{"input":["text","image"]}, "reasoning":true,
                "reasoning_options":[{"type":"effort","values":[null,"none","low","high","xhigh","high"]}]
            }}},
            "proxy": {"api":"https://proxy.test/v1", "models":{"gpt-test": {
                "id":"gpt-test", "canonical_model_id":"openai/gpt-test",
                "limit":{"context":64000,"output":8000}, "modalities":{"input":["text"]}
            }}},
            "anthropic":{"models":{"budget-model": {
                "id":"budget-model", "reasoning":true,
                "reasoning_options":[{"type":"budget_tokens","min":1024}]
            }}}
        })).unwrap()
    }

    #[test]
    fn lookup_uses_origin_qualified_names_and_selected_vendor_limits() {
        let catalog = ModelCatalog::parse(&fixture()).unwrap();
        let original = catalog.lookup(
            " GPT-TEST ",
            "custom",
            "https://custom.test",
            Protocol::OpenAiChat,
        );
        assert_eq!(
            (original.context_window, original.max_tokens),
            (400000, 128000)
        );
        assert_eq!(original.input_format, InputFormat::TextAndImages);
        assert_eq!(
            original.reasoning_levels,
            [
                ReasoningLevel::Low,
                ReasoningLevel::High,
                ReasoningLevel::XHigh
            ]
        );
        assert_eq!(
            catalog.lookup("openai/gpt-test", "custom", "", Protocol::OpenAiChat),
            original
        );
        let proxy = catalog.lookup(
            "gpt-test",
            "custom",
            "https://proxy.test/v1/responses#",
            Protocol::OpenAiResponses,
        );
        assert_eq!((proxy.context_window, proxy.max_tokens), (64000, 8000));
        assert_eq!(proxy.input_format, InputFormat::Text);
        assert!(proxy.reasoning_levels.is_empty());
        assert_eq!(
            catalog.lookup("missing", "", "", Protocol::OpenAiChat),
            ModelMetadata::default()
        );
    }

    #[test]
    fn only_declared_efforts_or_supported_budget_tiers_are_filled() {
        let catalog = ModelCatalog::parse(&fixture()).unwrap();
        let budget = catalog.lookup("budget-model", "anthropic", "", Protocol::AnthropicMessages);
        assert_eq!(budget.reasoning_levels.len(), 4);
        assert!(
            catalog
                .lookup("budget-model", "anthropic", "", Protocol::OpenAiChat)
                .reasoning_levels
                .is_empty()
        );
    }

    #[test]
    fn cache_reloads_replaced_files_and_preserves_previous_valid_index() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(cached(root.path()).entries.len(), 0);
        std::fs::create_dir_all(root.path().join("cache")).unwrap();
        std::fs::write(cache_path(root.path()), fixture()).unwrap();
        let first = cached(root.path());
        assert!(Arc::ptr_eq(&first, &cached(root.path())));
        std::fs::write(cache_path(root.path()), b"invalid").unwrap();
        assert!(Arc::ptr_eq(&first, &cached(root.path())));
        assert!(ModelCatalog::parse(b"{}").is_err());
        let mut fixture = fixture();
        fixture.push(b' ');
        std::fs::write(cache_path(root.path()), fixture).unwrap();
        assert!(!Arc::ptr_eq(&first, &cached(root.path())));
    }
}
