//! Where the agent's models come from.
//!
//! Providers and models live in `{data_dir}/providers.json`, separate from
//! `ui-settings.json`, because this file holds API keys. Keeping credentials
//! out of the UI settings file means the settings file can be synced, shown or
//! pasted into a bug report without leaking anything.
//!
//! The shape is deliberately flat — a list of providers, each with a list of
//! models — because that is what the user asked for: several vendors, several
//! models each, chosen per conversation. A model is addressed by its `id`,
//! which is unique across providers, so a conversation records one string and
//! the sub-agent override takes the same form.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use rcode_proto::ReasoningLevel;
use serde::{Deserialize, Serialize};

/// A subagent selection keeps provider, model, and optional effort together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSelection {
    pub provider_id: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningLevel>,
}

impl ModelSelection {
    pub fn selection(&self) -> String {
        if let Some(reasoning) = self.reasoning {
            return format!("model:{}", serde_json::to_string(&(
                &self.provider_id, &self.model_id, reasoning,
            )).unwrap());
        }
        format!(
            "model:{}",
            serde_json::to_string(&[&self.provider_id, &self.model_id]).unwrap()
        )
    }
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.strip_prefix("model:")?;
        let (provider_id, model_id, reasoning) = if let Ok((provider, model, level)) =
            serde_json::from_str::<(String, String, ReasoningLevel)>(raw) {
            (provider, model, Some(level))
        } else {
            let [provider, model]: [String; 2] = serde_json::from_str(raw).ok()?;
            (provider, model, None)
        };
        if provider_id.trim().is_empty() || model_id.trim().is_empty() {
            return None;
        }
        Some(Self {
            provider_id,
            model_id,
            reasoning,
        })
    }
    pub fn find<'a>(
        &self,
        providers: &'a ProvidersConfig,
    ) -> Option<(&'a ProviderConfig, &'a ModelConfig)> {
        let provider = providers
            .providers
            .iter()
            .find(|provider| provider.id == self.provider_id)?;
        Some((
            provider,
            provider
                .models
                .iter()
                .find(|model| model.id == self.model_id)?,
        ))
    }
}

/// Old conversation selections migrate to a real model; they no longer name a route.
pub fn is_retired_model_selection(id: &str) -> bool {
    matches!(
        id,
        "route:haiku" | "route:sonnet" | "route:opus" | "route:fable"
    )
}

/// Which wire protocol a provider speaks.
///
/// Matches `rcode_model::ProviderProtocol` but is defined here so the settings
/// file does not depend on the model crate — and so an unknown value written by
/// a newer build degrades to an error message rather than a parse failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// `POST /v1/chat/completions`
    OpenAiChat,
    /// `POST /v1/responses`
    OpenAiResponses,
    /// `POST /v1/messages`
    AnthropicMessages,
}

impl Protocol {
    /// The default endpoint for a provider that does not set one.
    pub fn default_endpoint(self) -> &'static str {
        match self {
            Self::OpenAiChat | Self::OpenAiResponses => "https://api.openai.com/v1",
            Self::AnthropicMessages => "https://api.anthropic.com/v1",
        }
    }
}

/// What a model accepts as input.
///
/// Declared, never inferred: a vendor name is not evidence that a model reads
/// images, and a wrong guess fails at the vendor as a 400 the user cannot act
/// on. [`InputFormat::Text`] is a statement that images must not be sent, and
/// the run honours it rather than attaching them anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InputFormat {
    /// Text only.
    Text,
    /// Text and images.
    #[default]
    TextAndImages,
}

impl InputFormat {
    /// Whether an image may be attached to a request for this model.
    pub fn accepts_images(self) -> bool {
        matches!(self, Self::TextAndImages)
    }

    /// The stable identifier used on the wire and in `providers.json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::TextAndImages => "text-and-images",
        }
    }
}

/// One model a provider offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    /// Unique across every provider. This is what a conversation records.
    pub id: String,
    /// The name the provider itself expects, e.g. `gpt-5`.
    pub model: String,
    /// Shown in the picker. Falls back to `model` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The context window, if the vendor publishes one. Absent means unknown —
    /// nothing derives it from the model name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    /// Cap on one reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// What this model accepts as input.
    ///
    /// Defaults to [`InputFormat::TextAndImages`] for a file written before
    /// the field existed, which is what those builds did: they attached every
    /// image unconditionally. Defaulting to `Text` would take image support
    /// away from a vision model the moment this build read the file.
    #[serde(default, skip_serializing_if = "input_format_is_default")]
    pub input_format: InputFormat,
    /// The reasoning levels this model accepts, weakest first. Empty means no
    /// effort ladder, and the picker shows no chip.
    ///
    /// Per model rather than per provider because the ladder is a property of
    /// the model: the same vendor ships a non-reasoning model and a reasoning
    /// one. The harness vocabulary is [`rcode_proto::ReasoningLevel`], which is
    /// longer than any of the three protocols accepts — see
    /// `rcode_harness::native`'s `effort_for`, which maps the top of the ladder
    /// onto the vendor's ceiling rather than sending a string it would reject.
    ///
    /// The order is canonical rather than chosen: it is the order the composer
    /// offers the levels in, and the settings page writes them weakest first
    /// whatever order they arrive in. A custom order would be a second, weaker
    /// statement of the same ladder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_levels: Vec<ReasoningLevel>,
}

impl ModelConfig {
    /// What the picker shows.
    pub fn display_label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.model)
    }
}

/// Whether an input format is the one a missing field means.
///
/// Written only when it differs, so a file that predates the field is not
/// rewritten carrying a declaration its author never made.
fn input_format_is_default(input_format: &InputFormat) -> bool {
    *input_format == InputFormat::default()
}

/// One vendor.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    /// Stable identifier, e.g. `openai`, `deepseek`.
    pub id: String,
    /// Shown in the picker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Which protocol to speak. A vendor offering both Chat Completions and
    /// Responses needs two entries — that is the honest way to model it, since
    /// they are different wires with different failure modes.
    pub protocol: Protocol,
    /// Overrides [`Protocol::default_endpoint`] — the point of this field is
    /// the OpenAI-compatible vendors that ship their own base URL.
    /// A trailing `#` marks a complete request URL; adapters remove the marker
    /// and skip automatic protocol path completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// The API key, or a legacy `$VAR` reference to read it from the environment.
    /// A literal key is stored in cleartext; see the module note on why this
    /// file is separate from the UI settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelConfig>,
}

// ProvidersConfig 的 Debug 会递归到模型 provider，两个密钥域都需要脱敏。
impl fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url.as_ref().map(|_| "<configured>"))
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("models", &self.models)
            .finish()
    }
}

impl ProviderConfig {
    pub fn display_label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.id)
    }

    /// The endpoint to send to.
    pub fn endpoint(&self) -> &str {
        self.base_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .unwrap_or_else(|| self.protocol.default_endpoint())
    }

    /// Resolve a literal key or a legacy `$VAR` reference.
    pub fn resolve_api_key(&self, environment: &BTreeMap<String, String>) -> Option<String> {
        let raw = self.api_key.as_deref()?.trim();
        if let Some(variable) = raw.strip_prefix('$') {
            return environment.get(variable).cloned();
        }
        (!raw.is_empty()).then(|| raw.to_owned())
    }

    /// Whether a legacy `$VAR` reference is not set in this process.
    pub fn missing_environment_variable(
        &self,
        environment: &BTreeMap<String, String>,
    ) -> Option<&str> {
        let variable = self.api_key.as_deref()?.trim().strip_prefix('$')?;
        environment
            .contains_key(variable)
            .then_some(())
            .map_or(Some(variable), |_| None)
    }
}

/// The whole file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvidersConfig {
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    /// Ignored legacy default. New conversations resolve from remembered picks
    /// or the first configured model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    /// 独立搜索配置；默认空，在工具调用时解析到内置免费端点。
    /// 不继承任何模型 provider 的 API key。
    #[serde(default)]
    pub web_search: rcode_agent::tools::WebSearchConfig,
}

/// Identifiers become keys and, in the UI, addresses. Kept strict and narrow
/// so two configurations cannot disagree about how to spell the same thing.
fn validate_identifier(kind: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("a {kind} id is empty"));
    }
    if value != value.trim() {
        return Err(format!(
            "the {kind} id `{value}` has leading or trailing space"
        ));
    }
    if let Some(character) = value.chars().find(|character| character.is_control()) {
        return Err(format!(
            "the {kind} id `{value}` contains a control character ({character:?})"
        ));
    }
    Ok(())
}

/// A base URL must be a plain `http`/`https` origin path.
///
/// A final `#` is an exact-endpoint marker, not a URL fragment to send.
/// Credentials in the URL and other fragments are refused: the header the request
/// carries already holds the key, and a second copy inside the URL ends up in
/// logs and error messages.
fn validate_base_url(value: &str) -> Result<(), String> {
    let value = value.trim();
    let parsed = url::Url::parse(value.strip_suffix('#').unwrap_or(value))
        .map_err(|error| format!("`{value}` is not a valid URL: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!(
            "`{value}` uses the `{}` scheme; only http and https are supported",
            parsed.scheme()
        ));
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err(format!("`{value}` has no host"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(format!(
            "`{value}` carries credentials; put the key in the API key field instead"
        ));
    }
    if parsed.fragment().is_some() {
        return Err(format!(
            "`{value}` has a fragment, which is not part of an endpoint"
        ));
    }
    Ok(())
}

const FILE_NAME: &str = "providers.json";

impl ProvidersConfig {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }

    /// Load from `{data_dir}/providers.json`.
    ///
    /// A missing file is an empty configuration, not an error: a fresh install
    /// has no providers until the user adds one. A *malformed* file is an
    /// error, because silently starting empty would look like the user's setup
    /// vanished.
    pub fn load(data_dir: &Path) -> Result<Self, ProvidersError> {
        let path = Self::path(data_dir);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(ProvidersError::Io(error.to_string())),
        };
        serde_json::from_str(&text).map_err(|error| ProvidersError::Parse(error.to_string()))
    }

    /// Write the file, creating the directory if needed.
    pub fn save(&self, data_dir: &Path) -> Result<(), ProvidersError> {
        std::fs::create_dir_all(data_dir).map_err(|error| ProvidersError::Io(error.to_string()))?;
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| ProvidersError::Parse(error.to_string()))?;
        // Write beside the final name, then rename: an interrupted save must
        // not leave a truncated file holding the user's API keys.
        let path = Self::path(data_dir);
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text).map_err(|error| ProvidersError::Io(error.to_string()))?;
        std::fs::rename(&temporary, &path).map_err(|error| ProvidersError::Io(error.to_string()))
    }

    /// Find a provider and model by the model's globally unique id.
    pub fn find(&self, model_id: &str) -> Option<(&ProviderConfig, &ModelConfig)> {
        self.providers.iter().find_map(|provider| {
            provider
                .models
                .iter()
                .find(|model| model.id == model_id)
                .map(|model| (provider, model))
        })
    }

    /// The first configured model, used when a caller has no remembered pick.
    pub fn resolved_default(&self) -> Option<&ModelConfig> {
        self.providers
            .iter()
            .find_map(|provider| provider.models.first())
    }

    /// Every model, with the provider it belongs to, for the picker.
    pub fn catalogue(&self) -> Vec<(&ProviderConfig, &ModelConfig)> {
        self.providers
            .iter()
            .flat_map(|provider| provider.models.iter().map(move |model| (provider, model)))
            .collect()
    }

    /// Check the configuration against the rules a saved file must satisfy.
    ///
    /// Returns every problem rather than the first, because a settings page
    /// showing one error at a time makes the user fix them one save at a time.
    /// The messages name the offending value, not just the field: "model `x`
    /// appears in two providers" is actionable in a way that "duplicate model"
    /// is not.
    pub fn validate(&self) -> Result<(), String> {
        let mut problems: Vec<String> = Vec::new();
        let mut seen_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut seen_models: std::collections::HashMap<&str, &str> =
            std::collections::HashMap::new();

        if self.providers.is_empty() {
            // Not an error: a fresh install has none, and the settings page
            // starts from exactly this.
        }

        for provider in &self.providers {
            if let Err(problem) = validate_identifier("provider", &provider.id) {
                problems.push(problem);
            }
            if !seen_ids.insert(provider.id.as_str()) {
                problems.push(format!("two providers share the id `{}`", provider.id));
            }

            if let Some(base_url) = provider
                .base_url
                .as_deref()
                .filter(|url| !url.trim().is_empty())
            {
                if let Err(problem) = validate_base_url(base_url) {
                    problems.push(format!("provider `{}`: {problem}", provider.id));
                }
            }

            // Providers can be configured incrementally. An empty model list
            // is a valid saved setup; it contributes no runnable catalog rows.

            for model in &provider.models {
                if let Err(problem) = validate_identifier("model", &model.id) {
                    problems.push(format!("provider `{}`: {problem}", provider.id));
                }
                if let Some(previous) = seen_models.insert(&model.id, &provider.id) {
                    problems.push(format!(
                        "the model id `{}` appears in both `{previous}` and `{}`; ids must be                          unique across providers, because a conversation records only the id",
                        model.id, provider.id
                    ));
                }
                if model.model.trim().is_empty() {
                    problems.push(format!(
                        "provider `{}`: model `{}` has no vendor name",
                        provider.id, model.id
                    ));
                }
                if let Some(max_tokens) = model.max_tokens {
                    if max_tokens == 0 {
                        problems.push(format!(
                            "provider `{}`: model `{}` has `maxTokens` of zero",
                            provider.id, model.id
                        ));
                    }
                    if let Some(window) = model.context_window {
                        if window == 0 {
                            problems.push(format!(
                                "provider `{}`: model `{}` has a context window of zero",
                                provider.id, model.id
                            ));
                        } else if max_tokens > window {
                            problems.push(format!(
                                "provider `{}`: model `{}` allows {max_tokens} output tokens in a                                  {window}-token window",
                                provider.id, model.id
                            ));
                        }
                    }
                }
            }
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems.join("; "))
        }
    }

    /// Report what a model id cannot be used for, if anything.
    ///
    /// Checks the things that are *known* to be wrong before a request is
    /// attempted, so the failure names the actual problem.
    pub fn diagnose(
        &self,
        model_id: &str,
        environment: &BTreeMap<String, String>,
    ) -> Result<(&ProviderConfig, &ModelConfig), ProvidersError> {
        let (provider, model) = self
            .find(model_id)
            .ok_or_else(|| ProvidersError::UnknownModel(model_id.to_owned()))?;
        if let Some(variable) = provider.missing_environment_variable(environment) {
            return Err(ProvidersError::MissingEnvironmentVariable {
                provider: provider.id.clone(),
                variable: variable.to_owned(),
            });
        }
        if provider.resolve_api_key(environment).is_none() {
            return Err(ProvidersError::NoApiKey {
                provider: provider.id.clone(),
            });
        }
        Ok((provider, model))
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProvidersError {
    #[error("providers.json: {0}")]
    Io(String),
    #[error("providers.json is not valid: {0}")]
    Parse(String),
    #[error("no model is configured with the id `{0}`")]
    UnknownModel(String),
    #[error("provider `{provider}` has no API key")]
    NoApiKey { provider: String },
    #[error("provider `{provider}` reads its key from ${variable}, which is not set")]
    MissingEnvironmentVariable { provider: String, variable: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn provider(id: &str, protocol: Protocol, api_key: Option<&str>) -> ProviderConfig {
        ProviderConfig {
            id: id.to_owned(),
            label: None,
            protocol,
            base_url: None,
            api_key: api_key.map(str::to_owned),
            models: vec![ModelConfig {
                id: format!("{id}-model"),
                model: "the-model".to_owned(),
                label: None,
                context_window: Some(200_000),
                max_tokens: None,
                input_format: crate::providers::InputFormat::TextAndImages,
                reasoning_levels: Vec::new(),
            }],
        }
    }

    #[test]
    fn an_endpoint_falls_back_to_the_protocol_default() {
        let openai = provider("openai", Protocol::OpenAiChat, None);
        assert_eq!(openai.endpoint(), "https://api.openai.com/v1");

        let anthropic = provider("anthropic", Protocol::AnthropicMessages, None);
        assert_eq!(anthropic.endpoint(), "https://api.anthropic.com/v1");
    }

    #[test]
    fn a_base_url_overrides_the_default() {
        let mut compat = provider("deepseek", Protocol::OpenAiChat, None);
        compat.base_url = Some("https://api.deepseek.com/v1".to_owned());
        assert_eq!(compat.endpoint(), "https://api.deepseek.com/v1");
    }

    #[test]
    fn chat_and_responses_are_distinct_protocols() {
        // A vendor offering both needs two entries; they are different wires.
        assert_ne!(Protocol::OpenAiChat, Protocol::OpenAiResponses);
        assert_eq!(
            Protocol::OpenAiResponses.default_endpoint(),
            "https://api.openai.com/v1"
        );
    }

    #[test]
    fn a_literal_key_is_used_as_is() {
        let config = provider("openai", Protocol::OpenAiChat, Some("sk-literal"));
        assert_eq!(
            config.resolve_api_key(&environment(&[])).as_deref(),
            Some("sk-literal")
        );
    }

    #[test]
    fn an_unconfigured_model_diagnoses_a_missing_key() {
        let config = ProvidersConfig {
            providers: vec![provider("openai", Protocol::OpenAiChat, None)],
            default_model: None,
            web_search: Default::default(),
        };

        let error = config
            .diagnose("openai-model", &environment(&[]))
            .expect_err("no key");
        assert_eq!(
            error,
            ProvidersError::NoApiKey {
                provider: "openai".to_owned(),
            }
        );
    }

    #[test]
    fn diagnose_names_a_missing_key() {
        let config = ProvidersConfig {
            providers: vec![provider("openai", Protocol::OpenAiChat, None)],
            default_model: None,
            web_search: Default::default(),
        };

        let error = config
            .diagnose("openai-model", &environment(&[]))
            .expect_err("no key");
        assert_eq!(
            error,
            ProvidersError::NoApiKey {
                provider: "openai".to_owned(),
            }
        );
    }

    #[test]
    fn diagnose_names_an_unknown_model() {
        let config = ProvidersConfig::default();
        let error = config
            .diagnose("nope", &environment(&[]))
            .expect_err("unknown");
        assert_eq!(error, ProvidersError::UnknownModel("nope".to_owned()));
    }

    #[test]
    fn a_configured_model_diagnoses_clean() {
        let config = ProvidersConfig {
            providers: vec![provider("openai", Protocol::OpenAiChat, Some("sk"))],
            default_model: None,
            web_search: Default::default(),
        };
        assert!(config.diagnose("openai-model", &environment(&[])).is_ok());
    }

    #[test]
    fn a_model_id_is_found_across_providers() {
        let config = ProvidersConfig {
            providers: vec![
                provider("openai", Protocol::OpenAiChat, Some("sk")),
                provider("anthropic", Protocol::AnthropicMessages, Some("sk")),
            ],
            default_model: None,
            web_search: Default::default(),
        };

        let (provider, _) = config.find("anthropic-model").expect("found");
        assert_eq!(provider.id, "anthropic");
    }

    #[test]
    fn the_default_falls_back_to_the_first_model() {
        let config = ProvidersConfig {
            providers: vec![provider("openai", Protocol::OpenAiChat, Some("sk"))],
            default_model: None,
            web_search: Default::default(),
        };
        assert_eq!(
            config.resolved_default().map(|model| model.id.as_str()),
            Some("openai-model")
        );
    }

    #[test]
    fn a_legacy_explicit_default_is_ignored() {
        let mut second = provider("anthropic", Protocol::AnthropicMessages, Some("sk"));
        second.models[0].id = "chosen".to_owned();
        let config = ProvidersConfig {
            providers: vec![provider("openai", Protocol::OpenAiChat, Some("sk")), second],
            default_model: Some("chosen".to_owned()),
            web_search: Default::default(),
        };
        assert_eq!(
            config.resolved_default().map(|model| model.id.as_str()),
            Some("openai-model")
        );
    }

    #[test]
    fn a_missing_legacy_default_does_not_change_first_model_fallback() {
        let config = ProvidersConfig {
            providers: vec![provider("openai", Protocol::OpenAiChat, Some("sk"))],
            default_model: Some("gone".to_owned()),
            web_search: Default::default(),
        };
        assert_eq!(
            config.resolved_default().map(|model| model.id.as_str()),
            Some("openai-model")
        );
    }

    #[test]
    fn a_label_falls_back_to_the_model_name() {
        let model = ModelConfig {
            id: "a".to_owned(),
            model: "gpt-5".to_owned(),
            label: None,
            context_window: None,
            max_tokens: None,
            input_format: Default::default(),
            reasoning_levels: Vec::new(),
        };
        assert_eq!(model.display_label(), "gpt-5");
    }

    #[test]
    fn a_missing_file_is_an_empty_configuration() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = ProvidersConfig::load(dir.path()).expect("loads");
        assert!(config.providers.is_empty());
    }

    #[test]
    fn a_configuration_round_trips_through_disk() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = ProvidersConfig {
            providers: vec![provider("openai", Protocol::OpenAiChat, Some("$KEY"))],
            default_model: Some("openai-model".to_owned()),
            web_search: Default::default(),
        };

        config.save(dir.path()).expect("saves");
        let loaded = ProvidersConfig::load(dir.path()).expect("loads");
        assert_eq!(loaded, config);
    }

    #[test]
    fn search_configuration_defaults_to_empty_for_old_files() {
        let config: ProvidersConfig =
            serde_json::from_str(r#"{"providers":[],"defaultModel":null}"#)
                .expect("legacy config loads");
        assert_eq!(
            config.web_search,
            rcode_agent::tools::WebSearchConfig::default()
        );
        assert!(config.web_search.endpoint.is_empty());
        assert!(config.web_search.token.is_none());
        let value = serde_json::to_value(ProvidersConfig::default()).expect("serialize defaults");
        assert_eq!(value["webSearch"], serde_json::json!({}));
    }

    #[test]
    fn a_file_that_predates_the_input_format_still_accepts_images() {
        // The field arrived after the file did. An older file attached every
        // image unconditionally, so defaulting to `Text` here would take image
        // support away from a vision model the moment this build read it.
        let config: ProvidersConfig = serde_json::from_str(
            r#"{"providers":[{"id":"openai","protocol":"open_ai_chat",
                 "models":[{"id":"m","model":"gpt-5"}]}],"defaultModel":null}"#,
        )
        .expect("legacy config loads");
        let model = &config.providers[0].models[0];
        assert_eq!(model.input_format, InputFormat::TextAndImages);
        assert!(model.input_format.accepts_images());
        assert!(model.reasoning_levels.is_empty());
        // Absent fields are not written back, so a load-and-save of an old file
        // does not grow it.
        let value = serde_json::to_value(model).expect("serialize");
        assert!(value.get("inputFormat").is_none(), "{value}");
        assert!(value.get("reasoningLevels").is_none(), "{value}");
    }

    #[test]
    fn a_declared_ladder_and_text_only_input_round_trip() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut config = ProvidersConfig {
            providers: vec![provider("openai", Protocol::OpenAiChat, Some("sk"))],
            default_model: None,
            web_search: Default::default(),
        };
        config.providers[0].models[0].input_format = InputFormat::Text;
        config.providers[0].models[0].reasoning_levels =
            vec![ReasoningLevel::Low, ReasoningLevel::High];

        let value = serde_json::to_value(&config.providers[0].models[0]).expect("serialize");
        assert_eq!(value["inputFormat"], serde_json::json!("text"));
        assert_eq!(value["reasoningLevels"], serde_json::json!(["low", "high"]));

        config.save(dir.path()).expect("saves");
        let loaded = ProvidersConfig::load(dir.path()).expect("loads");
        assert_eq!(loaded, config);
        assert_eq!(
            loaded.providers[0].models[0].input_format,
            InputFormat::Text
        );
        assert_eq!(
            loaded.providers[0].models[0].reasoning_levels,
            vec![ReasoningLevel::Low, ReasoningLevel::High]
        );
    }

    #[test]
    fn search_configuration_round_trips_without_inheriting_model_keys() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut config = ProvidersConfig {
            providers: vec![provider(
                "openai",
                Protocol::OpenAiChat,
                Some("model-secret"),
            )],
            default_model: None,
            web_search: Default::default(),
        };
        assert!(config.web_search.token.is_none());
        assert_eq!(
            serde_json::to_value(&config.web_search).expect("search defaults"),
            serde_json::json!({})
        );
        config.web_search.endpoint = "https://search.test".into();
        config.web_search.token = Some("search-secret".into());
        config.save(dir.path()).expect("save search config");
        let loaded = ProvidersConfig::load(dir.path()).expect("load search config");
        assert_eq!(loaded, config);
        assert_eq!(loaded.web_search.token.as_deref(), Some("search-secret"));
        assert_eq!(loaded.providers[0].api_key.as_deref(), Some("model-secret"));
        let debug = format!("{loaded:?}");
        assert!(!debug.contains("model-secret"));
        assert!(!debug.contains("search-secret"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn search_endpoint_and_token_can_each_be_omitted() {
        let config: ProvidersConfig =
            serde_json::from_str(r#"{"webSearch":{"endpoint":"https://search.test"}}"#)
                .expect("endpoint-only config");
        assert_eq!(config.web_search.endpoint, "https://search.test");
        assert!(config.web_search.token.is_none());
        let config: ProvidersConfig =
            serde_json::from_str(r#"{"webSearch":{"token":"search-token"}}"#)
                .expect("token-only config");
        assert!(config.web_search.endpoint.is_empty());
        assert_eq!(config.web_search.token.as_deref(), Some("search-token"));
    }

    #[test]
    fn a_malformed_file_is_an_error_not_an_empty_configuration() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(ProvidersConfig::path(dir.path()), "{ not json").expect("writes");

        // Starting empty here would look like the user's setup vanished.
        let error = ProvidersConfig::load(dir.path()).expect_err("malformed");
        assert!(matches!(error, ProvidersError::Parse(_)));
    }

    #[test]
    fn saving_leaves_no_temporary_file_behind() {
        let dir = tempfile::tempdir().expect("temp dir");
        ProvidersConfig::default().save(dir.path()).expect("saves");

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("reads")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }

    #[test]
    fn the_catalogue_lists_every_model_with_its_provider() {
        let config = ProvidersConfig {
            providers: vec![
                provider("openai", Protocol::OpenAiChat, Some("sk")),
                provider("anthropic", Protocol::AnthropicMessages, Some("sk")),
            ],
            default_model: None,
            web_search: Default::default(),
        };

        let catalogue = config.catalogue();
        assert_eq!(catalogue.len(), 2);
        assert_eq!(catalogue[0].0.id, "openai");
        assert_eq!(catalogue[1].0.id, "anthropic");
    }
}
