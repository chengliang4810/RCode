//! The store's guarantees: one writer, no lost work, no leaked key.
use super::*;
use crate::providers::{ModelConfig, Protocol, ProviderConfig};

fn entry(id: &str, credential: CredentialUpdate) -> ProviderDraftEntry {
    ProviderDraftEntry {
        id: id.to_owned(),
        label: None,
        protocol: Protocol::OpenAiChat,
        base_url: None,
        models: vec![ModelConfig {
            id: format!("{id}-model"),
            model: "some-model".to_owned(),
            label: None,
            context_window: Some(200_000),
            max_tokens: None,
            input_format: Default::default(),
            reasoning_levels: Vec::new(),
        }],
        credential,
    }
}

fn draft(entries: Vec<ProviderDraftEntry>) -> ProviderDraft {
    ProviderDraft {
        default_model: None,
        providers: entries,
        web_search: None,
    }
}

fn store(dir: &Path) -> ProviderStore {
    ProviderStore::open(dir).expect("opens")
}

#[test]
fn a_fresh_directory_reads_as_an_empty_configuration() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());
    assert!(store.view().providers.is_empty());
    assert_eq!(store.snapshot().revision, ABSENT_REVISION);
}

#[test]
fn providers_without_models_can_be_saved_incrementally() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());
    let mut provider = entry("new", CredentialUpdate::Keep);
    provider.models.clear();
    let saved = store.save(draft(vec![provider]), &store.view().revision).expect("saves setup");
    assert_eq!(saved.providers.len(), 1);
    assert!(saved.providers[0].models.is_empty());
    assert!(store.snapshot().config.catalogue().is_empty());
    assert!(store.snapshot().config.resolved_default().is_none());
    let reopened = ProviderStore::open(dir.path()).expect("reopens setup");
    assert_eq!(reopened.view().providers, saved.providers);
}

#[test]
fn saving_writes_the_file_and_moves_the_revision() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());
    let before = store.view().revision;

    let view = store
        .save(
            draft(vec![entry("openai", CredentialUpdate::Clear)]),
            &before,
        )
        .expect("saves");

    assert_ne!(view.revision, before, "a save must move the revision");
    assert_eq!(view.providers.len(), 1);
    assert_eq!(view.providers[0].id, "openai");
    // And the file is really there.
    assert!(ProvidersConfig::path(dir.path()).exists());
}

#[test]
fn a_save_drafted_against_a_stale_revision_is_refused() {
    // The settings window left open while the file changes underneath it.
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());
    let stale = store.view().revision;
    store
        .save(draft(vec![entry("first", CredentialUpdate::Clear)]), &stale)
        .expect("first save");

    let error = store
        .save(
            draft(vec![entry("second", CredentialUpdate::Clear)]),
            &stale,
        )
        .expect_err("a stale draft must be refused");
    assert!(matches!(error, SaveError::Conflict { .. }), "got {error}");

    // The first save survived: the second did not clobber it.
    let view = store.view();
    assert_eq!(view.providers.len(), 1);
    assert_eq!(view.providers[0].id, "first");
}

#[test]
fn the_view_never_carries_the_key() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());
    store
        .save(
            draft(vec![entry(
                "openai",
                CredentialUpdate::ReplaceLiteral {
                    value: "sk-super-secret".to_owned(),
                },
            )]),
            ABSENT_REVISION,
        )
        .expect("saves");

    let view = store.view();
    assert_eq!(view.providers[0].credential, CredentialView::Stored);
    let json = serde_json::to_string(&view).expect("serialize");
    assert!(
        !json.contains("sk-super-secret"),
        "the key reached the view: {json}"
    );
}

#[test]
fn a_legacy_environment_credential_remains_hidden_from_settings() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());
    store
        .save(
            draft(vec![entry(
                "openai",
                CredentialUpdate::UseEnvironment {
                    name: "OPENAI_KEY".to_owned(),
                },
            )]),
            ABSENT_REVISION,
        )
        .expect("saves");

    let view = store.view();
    assert_eq!(view.providers[0].credential, CredentialView::Stored);
    let json = serde_json::to_string(&view).expect("serialize");
    assert!(!json.contains("OPENAI_KEY"));
    assert!(!json.contains("sk-from-env"));
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.config.providers[0].api_key.as_deref(),
        Some("$OPENAI_KEY")
    );
}

#[test]
fn the_saved_key_is_available_only_through_explicit_lookup() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());
    store
        .save(
            draft(vec![entry(
                "openai",
                CredentialUpdate::ReplaceLiteral {
                    value: "sk-reveal-me".to_owned(),
                },
            )]),
            ABSENT_REVISION,
        )
        .expect("saves");

    let view_json = serde_json::to_string(&store.view()).expect("serialize view");
    assert!(!view_json.contains("sk-reveal-me"));
    assert_eq!(
        store
            .api_key("openai", &std::collections::BTreeMap::new())
            .as_deref(),
        Some("sk-reveal-me")
    );
    assert_eq!(
        store.api_key("missing", &std::collections::BTreeMap::new()),
        None
    );
}

#[test]
fn keep_preserves_a_key_the_draft_never_saw() {
    // The failure this prevents: a UI that was shown `Stored` and not the
    // value would otherwise save an empty key over a working one.
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());
    store
        .save(
            draft(vec![entry(
                "openai",
                CredentialUpdate::ReplaceLiteral {
                    value: "sk-keep-me".to_owned(),
                },
            )]),
            ABSENT_REVISION,
        )
        .expect("saves");

    // A second save that only renames the provider.
    let mut renamed = entry("openai", CredentialUpdate::Keep);
    renamed.label = Some("OpenAI".to_owned());
    store
        .save(draft(vec![renamed]), &store.view().revision)
        .expect("saves");

    assert_eq!(
        store.snapshot().config.providers[0].api_key.as_deref(),
        Some("sk-keep-me")
    );
}

#[test]
fn a_draft_that_says_nothing_about_search_keeps_what_is_stored() {
    // The settings page does not edit search. A save from it sends nothing for
    // that field, and a default-constructed value would wipe an endpoint and
    // token the user configured by hand — with nothing on the page showing it
    // had happened.
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());

    let mut first = draft(vec![entry("openai", CredentialUpdate::Clear)]);
    first.web_search = Some(rcode_agent::tools::WebSearchConfig {
        endpoint: "https://search.test".to_owned(),
        token: Some("search-secret".to_owned()),
    });
    store.save(first, ABSENT_REVISION).expect("saves");

    // A second save that says nothing about search.
    store
        .save(
            draft(vec![entry("openai", CredentialUpdate::Keep)]),
            &store.view().revision,
        )
        .expect("saves");

    let snapshot = store.snapshot();
    assert_eq!(snapshot.config.web_search.endpoint, "https://search.test");
    assert_eq!(
        snapshot.config.web_search.token.as_deref(),
        Some("search-secret"),
        "the search token must survive a save that did not mention it"
    );
}

#[test]
fn a_literal_key_replacement_is_trimmed() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());
    store
        .save(
            draft(vec![entry(
                "openai",
                CredentialUpdate::ReplaceLiteral {
                    value: "  sk-new  ".to_owned(),
                },
            )]),
            ABSENT_REVISION,
        )
        .expect("saves");
    assert_eq!(
        store.snapshot().config.providers[0].api_key.as_deref(),
        Some("sk-new")
    );
}

#[test]
fn clear_removes_the_key() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());
    store
        .save(
            draft(vec![entry(
                "openai",
                CredentialUpdate::ReplaceLiteral {
                    value: "sk-gone".to_owned(),
                },
            )]),
            ABSENT_REVISION,
        )
        .expect("saves");
    store
        .save(
            draft(vec![entry("openai", CredentialUpdate::Clear)]),
            &store.view().revision,
        )
        .expect("saves");

    assert!(store.snapshot().config.providers[0].api_key.is_none());
    assert_eq!(store.view().providers[0].credential, CredentialView::None);
}

#[test]
fn an_empty_literal_is_refused_rather_than_read_as_clear() {
    // A blank field is a half-finished edit, not a request to delete the key.
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());
    let error = store
        .save(
            draft(vec![entry(
                "openai",
                CredentialUpdate::ReplaceLiteral {
                    value: "   ".to_owned(),
                },
            )]),
            ABSENT_REVISION,
        )
        .expect_err("refused");
    assert!(matches!(error, SaveError::Invalid(_)), "got {error}");
}

#[test]
fn an_invalid_configuration_is_refused_before_anything_is_written() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());
    let mut bad = entry("openai", CredentialUpdate::Clear);
    bad.models[0].max_tokens = Some(500_000); // over the 200k window
    let error = store
        .save(draft(vec![bad]), ABSENT_REVISION)
        .expect_err("refused");
    assert!(matches!(error, SaveError::Invalid(_)), "got {error}");

    assert!(
        !ProvidersConfig::path(dir.path()).exists(),
        "a refused save must not leave a file"
    );
    assert_eq!(store.view().revision, ABSENT_REVISION);
}

#[test]
fn a_malformed_file_is_an_error_not_an_empty_configuration() {
    // Starting empty would look like the user's setup had vanished — and the
    // next save would make that true.
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(ProvidersConfig::path(dir.path()), "{ not json").expect("write");
    assert!(ProviderStore::open(dir.path()).is_err());
}

#[test]
fn a_save_leaves_no_temporary_file_behind() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());
    store
        .save(
            draft(vec![entry("openai", CredentialUpdate::Clear)]),
            ABSENT_REVISION,
        )
        .expect("saves");

    let leftovers: Vec<String> = std::fs::read_dir(dir.path())
        .expect("reads")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("tmp"))
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}

#[test]
fn a_reopened_store_serves_what_was_saved() {
    let dir = tempfile::tempdir().expect("temp dir");
    {
        let store = store(dir.path());
        store
            .save(
                draft(vec![entry("openai", CredentialUpdate::Clear)]),
                ABSENT_REVISION,
            )
            .expect("saves");
    }
    let reopened = store(dir.path());
    assert_eq!(reopened.view().providers[0].id, "openai");
}

#[test]
fn a_snapshot_taken_before_a_save_does_not_change() {
    // A run keeps what it started with: saving must not reach it.
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());
    store
        .save(
            draft(vec![entry("first", CredentialUpdate::Clear)]),
            ABSENT_REVISION,
        )
        .expect("saves");
    let held = store.snapshot();

    store
        .save(
            draft(vec![entry("second", CredentialUpdate::Clear)]),
            &store.view().revision,
        )
        .expect("saves");

    assert_eq!(held.config.providers[0].id, "first");
    assert_eq!(store.snapshot().config.providers[0].id, "second");
}

// ─── Validation ─────────────────────────────────────────────────────────────

fn config(provider: ProviderConfig) -> ProvidersConfig {
    ProvidersConfig {
        providers: vec![provider],
        default_model: None,
        web_search: Default::default(),
    }
}

fn provider(id: &str, base_url: Option<&str>) -> ProviderConfig {
    ProviderConfig {
        id: id.to_owned(),
        label: None,
        protocol: Protocol::OpenAiChat,
        base_url: base_url.map(str::to_owned),
        api_key: Some("sk".to_owned()),
        models: vec![ModelConfig {
            id: format!("{id}-model"),
            model: "vendor-name".to_owned(),
            label: None,
            context_window: None,
            max_tokens: None,
            input_format: Default::default(),
            reasoning_levels: Vec::new(),
        }],
    }
}

#[test]
fn a_sound_configuration_validates() {
    assert!(config(provider("openai", None)).validate().is_ok());
    assert!(
        config(provider("compat", Some("https://api.example.com/v1")))
            .validate()
            .is_ok()
    );
}

#[test]
fn provider_ids_must_be_unique_and_well_formed() {
    let blank = config(provider("  ", None));
    assert!(blank.validate().expect_err("refused").contains("empty"));

    let spaced = config(provider(" openai", None));
    assert!(spaced.validate().expect_err("refused").contains("space"));

    let duplicate = ProvidersConfig {
        providers: vec![provider("same", None), provider("same", None)],
        default_model: None,
        web_search: Default::default(),
    };
    assert!(
        duplicate
            .validate()
            .expect_err("refused")
            .contains("share the id")
    );
}

#[test]
fn a_model_id_must_be_unique_across_providers() {
    // A conversation records only the model id, so two providers offering the
    // same one is genuinely ambiguous.
    let mut first = provider("a", None);
    let mut second = provider("b", None);
    first.models[0].id = "shared".to_owned();
    second.models[0].id = "shared".to_owned();
    let config = ProvidersConfig {
        providers: vec![first, second],
        default_model: None,
        web_search: Default::default(),
    };
    let error = config.validate().expect_err("refused");
    assert!(error.contains("appears in both"), "got: {error}");
}

#[test]
fn a_base_url_may_not_carry_credentials_or_a_fragment() {
    let with_credentials = config(provider("x", Some("https://user:pw@api.example.com")));
    assert!(
        with_credentials
            .validate()
            .expect_err("refused")
            .contains("credentials")
    );

    let with_fragment = config(provider("x", Some("https://api.example.com#frag")));
    assert!(
        with_fragment
            .validate()
            .expect_err("refused")
            .contains("fragment")
    );

    let bad_scheme = config(provider("x", Some("ftp://api.example.com")));
    assert!(
        bad_scheme
            .validate()
            .expect_err("refused")
            .contains("scheme")
    );
}

#[test]
fn base_urls_and_exact_endpoint_markers_round_trip_without_rewriting() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = store(dir.path());
    for protocol in [Protocol::OpenAiChat, Protocol::OpenAiResponses, Protocol::AnthropicMessages] {
        for base in [
            "https://www.aidawan.fun",
            "https://www.aidawan.fun/v1",
            "https://www.aidawan.fun/custom/invoke?api-version=2026-01#",
        ] {
            let mut provider = entry("gateway", CredentialUpdate::Keep);
            provider.protocol = protocol;
            provider.base_url = Some(base.into());
            let view = store.save(draft(vec![provider]), &store.view().revision).expect("saves URL");
            assert_eq!(view.providers[0].base_url.as_deref(), Some(base));
            let reopened = ProviderStore::open(dir.path()).expect("reopens URL");
            assert_eq!(reopened.snapshot().config.providers[0].endpoint(), base);
        }
    }
}

#[test]
fn a_model_needs_a_vendor_name() {
    let mut broken = provider("x", None);
    broken.models[0].model = "  ".to_owned();
    assert!(
        config(broken)
            .validate()
            .expect_err("refused")
            .contains("vendor name")
    );
}

#[test]
fn the_output_limit_may_not_exceed_the_window() {
    let mut broken = provider("x", None);
    broken.models[0].context_window = Some(1000);
    broken.models[0].max_tokens = Some(2000);
    assert!(
        config(broken)
            .validate()
            .expect_err("refused")
            .contains("output tokens")
    );
}

#[test]
fn a_provider_without_models_is_a_valid_incomplete_setup() {
    let mut pending = provider("x", None);
    pending.models.clear();
    let config = config(pending);
    config.validate().expect("saves incremental setup");
    assert!(config.catalogue().is_empty());
    assert!(config.resolved_default().is_none());
}

#[test]
fn a_legacy_default_is_cleared_by_the_settings_page_save() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut config = config(provider("x", None));
    config.default_model = Some("legacy-model".to_owned());
    config.save(dir.path()).expect("save legacy config");
    let store = store(dir.path());

    store
        .save(
            draft(vec![entry("x", CredentialUpdate::Keep)]),
            &store.view().revision,
        )
        .expect("settings save");

    assert_eq!(store.snapshot().config.default_model, None);
}

#[test]
fn a_legacy_default_model_is_ignored_by_validation() {
    let config = ProvidersConfig {
        providers: vec![provider("x", None)],
        default_model: Some("ghost".to_owned()),
        web_search: Default::default(),
    };
    assert!(config.validate().is_ok());
}

#[test]
fn validation_reports_every_problem_not_just_the_first() {
    // A page showing one error per save makes the user fix them one at a time.
    let mut first = provider("", None);
    first.models[0].model = String::new();
    let config = ProvidersConfig {
        providers: vec![first],
        default_model: Some("ghost".to_owned()),
        web_search: Default::default(),
    };
    let error = config.validate().expect_err("refused");
    assert!(error.contains("empty"), "got: {error}");
    assert!(error.contains("vendor name"), "got: {error}");
}
