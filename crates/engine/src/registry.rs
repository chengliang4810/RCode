//! The engine's Native agent catalog and device-local preferences.
//! Tests can register additional harnesses without installing external CLIs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use rcode_harness::{Harness, HarnessError, mock::MockHarness};
use rcode_proto::{AgentEvent, DoneStatus, HarnessId, ReasoningLevel, SteeringMode};

/// What `ListHarnesses` reports per harness.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessDescriptor {
    pub id: HarnessId,
    pub name: String,
    pub supports_steering: bool,
    pub steering_mode: SteeringMode,
    pub reasoning_levels: Vec<ReasoningLevel>,
    /// Whether the agent's CLI is present on the listing device (the settings
    /// enable-gate). Defaults true so catalogs from engines predating the
    /// field never read as uninstallable.
    #[serde(default = "default_installed")]
    pub installed: bool,
    /// Whether the listing device offers this harness (Settings → Providers).
    /// `None` — the catalog came from an engine predating the setting — means
    /// "unknown": consumers fall back to detection (see [`descriptor_enabled`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl HarnessDescriptor {
    /// Whether this harness can accept a prompt inside the turn that is
    /// currently running. Turn-boundary steering is still useful to the
    /// automatic queue drain, but it is not the non-interrupting "Steer"
    /// action exposed on an individual queued row.
    pub fn steers_mid_turn(&self) -> bool {
        self.supports_steering && self.steering_mode == SteeringMode::StepBoundary
    }
}

fn default_installed() -> bool {
    true
}

/// Whether detection alone may switch a harness on. The mock always "resolves"
/// but is a test rig, and counting it would let the last real agent be turned
/// off (the composer needs something it can actually run). Test registries may
/// explicitly enable it; production registers Native.
fn auto_enabled(id: HarnessId) -> bool {
    id != HarnessId::Mock
}

/// A descriptor's effective enabled flag. `None` — a catalog from an engine
/// predating the setting — falls back to detection, the same rule new devices
/// start from (see [`HarnessRegistry::enabled_set`]).
pub fn descriptor_enabled(descriptor: &HarnessDescriptor) -> bool {
    descriptor
        .enabled
        .unwrap_or_else(|| descriptor.installed && auto_enabled(descriptor.id))
}

fn describe(harness: &dyn Harness) -> HarnessDescriptor {
    HarnessDescriptor {
        id: harness.id(),
        name: harness.display_name().to_string(),
        supports_steering: harness.supports_steering(),
        steering_mode: harness.steering_mode(),
        reasoning_levels: harness.reasoning_levels().to_vec(),
        installed: harness.installed(),
        enabled: None,
    }
}

/// The persisted shape of `harness-prefs.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct HarnessPrefsFile {
    /// The user's explicit opt-OUTS. Enablement otherwise follows detection,
    /// so the file only records "no" — an agent installed later turns itself
    /// on without a trip to Settings.
    disabled: Vec<HarnessId>,
    titles: TitleSettings,
    /// The allow-list written back when enablement was a fixed default set.
    /// Read once, folded into `disabled`, and never written again.
    #[serde(skip_serializing)]
    enabled: Option<Vec<HarnessId>>,
}

/// Per-device automatic session title preferences.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TitleSettings {
    /// None follows the session harness, using a supported installed fallback.
    pub harness: Option<HarnessId>,
    /// None selects the cheapest model offered by the selected harness.
    pub model: Option<String>,
}

type Factory = Box<dyn Fn() -> Result<Arc<dyn Harness>, HarnessError> + Send + Sync>;
type InstalledProbe = Box<dyn Fn() -> bool + Send + Sync>;

enum Slot {
    Ready(Arc<dyn Harness>),
    Lazy {
        descriptor: HarnessDescriptor,
        /// Re-run on every `descriptors()` call — a CLI installed mid-session
        /// shows up on the next settings/picker open, no restart needed.
        installed: InstalledProbe,
        factory: Factory,
    },
}

pub struct HarnessRegistry {
    slots: Mutex<HashMap<HarnessId, Slot>>,
    order: Mutex<Vec<HarnessId>>,
    /// This device's enabled set; `None` inner value = the default set.
    prefs: Mutex<HarnessPrefsFile>,
    /// Where the prefs persist; `None` (tests, bare registries) skips writes.
    prefs_path: Mutex<Option<PathBuf>>,
}

impl Default for HarnessRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl HarnessRegistry {
    pub async fn discover_models(
        &self,
        id: HarnessId,
    ) -> Result<Vec<rcode_proto::Model>, HarnessError> {
        let harness = self.resolve(id)?;
        tokio::spawn(async move { harness.models().await })
            .await
            .map_err(|error| {
                HarnessError::Protocol(format!("model discovery task failed: {error}"))
            })?
    }

    pub async fn discover_commands(
        &self,
        id: HarnessId,
        cwd: &Path,
    ) -> Result<Vec<rcode_proto::SlashCommand>, HarnessError> {
        let cwd = cwd.to_owned();
        let harness = self.resolve(id)?;
        tokio::spawn(async move { harness.commands_for(&cwd).await })
            .await
            .map_err(|error| {
                HarnessError::Protocol(format!("command discovery task failed: {error}"))
            })?
    }

    pub async fn discover_skills(
        &self,
        id: HarnessId,
        cwd: &Path,
    ) -> Result<Option<Vec<rcode_proto::invocation::Skill>>, HarnessError> {
        let cwd = cwd.to_owned();
        let harness = self.resolve(id)?;
        tokio::spawn(async move { harness.skills(&cwd).await })
            .await
            .map_err(|error| {
                HarnessError::Protocol(format!("skill discovery task failed: {error}"))
            })?
    }

    pub fn new() -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
            prefs: Mutex::new(HarnessPrefsFile::default()),
            prefs_path: Mutex::new(None),
        }
    }

    fn slots(&self) -> MutexGuard<'_, HashMap<HarnessId, Slot>> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn order(&self) -> MutexGuard<'_, Vec<HarnessId>> {
        self.order.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn prefs(&self) -> MutexGuard<'_, HarnessPrefsFile> {
        self.prefs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Load `harness-prefs.json` from the engine data dir and remember the
    /// path for writes. Corrupt/missing files fall back to the default set.
    pub fn load_prefs(&self, data_dir: &Path) {
        let path = data_dir.join("harness-prefs.json");
        let loaded = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<HarnessPrefsFile>(&text).ok())
            .unwrap_or_default();
        *self.prefs() = loaded;
        *self
            .prefs_path
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(path);
        self.migrate_legacy_prefs();
        self.heal_empty_enablement();
    }

    /// Fold a legacy allow-list into the opt-out shape: a registered harness
    /// missing from it was a deliberate "no", so it stays off. Rewrites the
    /// file once, which is what lets later installs auto-enable.
    ///
    /// "Deliberate" only holds for agents that existed when the file was
    /// written; one registered since has no entry in the old list to read.
    /// [`Self::heal_empty_enablement`] catches that gap.
    fn migrate_legacy_prefs(&self) {
        let legacy = { self.prefs().enabled.take() };
        let Some(legacy) = legacy else { return };
        let registered: Vec<HarnessId> = self.order().iter().copied().collect();
        let disabled: Vec<HarnessId> = registered
            .into_iter()
            .filter(|id| auto_enabled(*id) && !legacy.contains(id))
            .collect();
        self.prefs().disabled = disabled;
        self.persist_prefs();
    }

    /// Keep the invariant `set_enabled` enforces ("cannot disable the last
    /// enabled harness") true of the file too.
    ///
    /// A file can spell out what no interaction can build: a legacy allow-list
    /// folded by a registry that registers one agent it never knew (the
    /// built-in agent alone is the shipped case — the composer would then meet
    /// an empty catalog and refuse every new send), or a hand-edited/foreign
    /// file naming every runnable agent as disabled. Re-enabling the first
    /// runnable one restores the state Settings would refuse to leave behind.
    ///
    /// A device with nothing runnable keeps its empty set: that is the truth,
    /// and the composer says so instead of running something that cannot run.
    /// Registered harnesses are what make a candidate runnable, so prefs must
    /// be loaded after registration (the engine's order).
    fn heal_empty_enablement(&self) {
        if !self.enabled_set().is_empty() {
            return;
        }
        // Collected before the probes run: `descriptors()` takes `slots` then
        // `order`, so holding `order` across a probe would invert the locks.
        let registered: Vec<HarnessId> = self.order().iter().copied().collect();
        let Some(id) = registered
            .into_iter()
            .find(|id| auto_enabled(*id) && self.installed_for(*id))
        else {
            return;
        };
        self.prefs().disabled.retain(|harness| *harness != id);
        self.persist_prefs();
        tracing::warn!(
            harness = ?id,
            "harness-prefs disabled every runnable agent; re-enabled the first one"
        );
    }

    /// What this device offers: every harness whose CLI is FOUND, minus the
    /// user's explicit opt-outs. Enablement follows detection, so installing
    /// an agent is all it takes for it to appear in the composer.
    pub fn enabled_set(&self) -> Vec<HarnessId> {
        // Both guards drop before the installed probes run: `descriptors()`
        // takes `slots` then `order`, so holding `order` across a probe (which
        // takes `slots`) would invert the lock order.
        let registered: Vec<HarnessId> = self.order().iter().copied().collect();
        let disabled = self.prefs().disabled.clone();
        registered
            .into_iter()
            .filter(|id| auto_enabled(*id) && !disabled.contains(id) && self.installed_for(*id))
            .collect()
    }

    /// Whether this device's CLI probe passes for `id` (no spawn, no resolve).
    fn installed_for(&self, id: HarnessId) -> bool {
        match self.slots().get(&id) {
            Some(Slot::Ready(harness)) => harness.installed(),
            Some(Slot::Lazy { installed, .. }) => installed(),
            None => false,
        }
    }

    /// Flip one harness's enablement and persist. Refuses unknown harnesses,
    /// enabling one whose CLI is missing (the settings gate, enforced where
    /// the state lives), and disabling the last enabled harness — under
    /// detection-based enablement everything enabled is runnable, so the
    /// last one standing is always worth protecting (the composer needs
    /// something to run). A harness whose CLI is missing is never enabled
    /// in the first place, so turning it off is a clean no-op.
    pub fn set_enabled(&self, id: HarnessId, on: bool) -> Result<(), String> {
        if !self.slots().contains_key(&id) {
            return Err(format!("unknown harness {id:?}"));
        }
        if on && !auto_enabled(id) {
            return Err(format!("{id:?} cannot be enabled from Settings"));
        }
        if on && !self.installed_for(id) {
            return Err(format!("{id:?} CLI is not installed on this device"));
        }
        let enabled = self.enabled_set();
        match (on, enabled.contains(&id)) {
            (true, false) => {
                let mut prefs = self.prefs();
                prefs.disabled.retain(|h| *h != id);
            }
            (false, true) => {
                if enabled.len() == 1 {
                    return Err("cannot disable the last enabled harness".into());
                }
                let mut prefs = self.prefs();
                prefs.disabled.push(id);
            }
            _ => return Ok(()),
        }
        self.persist_prefs();
        Ok(())
    }

    /// Best-effort atomic write (temp + rename, the ui-settings pattern).
    fn persist_prefs(&self) {
        let Some(path) = self
            .prefs_path
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        else {
            return;
        };
        let json = match serde_json::to_string_pretty(&*self.prefs()) {
            Ok(json) => json,
            Err(err) => {
                tracing::warn!(error = %err, "harness-prefs serialize failed");
                return;
            }
        };
        let tmp = path.with_extension("json.tmp");
        if let Err(err) = std::fs::write(&tmp, json).and_then(|()| std::fs::rename(&tmp, &path)) {
            tracing::warn!(error = %err, "harness-prefs save failed");
        }
    }

    pub fn title_settings(&self) -> TitleSettings {
        self.prefs().titles.clone()
    }

    pub fn set_title_settings(&self, mut settings: TitleSettings) -> Result<(), String> {
        if let Some(id) = settings.harness {
            if !rcode_harness::supports_titles(id) || !self.enabled_set().contains(&id) {
                return Err("Choose an enabled harness that supports title generation".into());
            }
        } else if settings.model.is_some() {
            return Err("Choose a title harness before choosing a model".into());
        }
        settings.model = settings.model.filter(|model| !model.trim().is_empty());
        self.prefs().titles = settings;
        self.persist_prefs();
        Ok(())
    }

    pub fn register(&self, harness: Arc<dyn Harness>) {
        let id = harness.id();
        if self.slots().insert(id, Slot::Ready(harness)).is_none() {
            self.order().push(id);
        }
    }

    /// Register a slot resolved on first `resolve` (the factory result is
    /// cached). `installed` is the CLI-presence probe run per `descriptors()`
    /// call; it must never spawn.
    pub fn register_lazy(
        &self,
        descriptor: HarnessDescriptor,
        installed: InstalledProbe,
        factory: Factory,
    ) {
        let id = descriptor.id;
        if self
            .slots()
            .insert(
                id,
                Slot::Lazy {
                    descriptor,
                    installed,
                    factory,
                },
            )
            .is_none()
        {
            self.order().push(id);
        }
    }

    pub fn resolve(&self, id: HarnessId) -> Result<Arc<dyn Harness>, HarnessError> {
        let mut slots = self.slots();
        match slots.get(&id) {
            Some(Slot::Ready(harness)) => Ok(harness.clone()),
            Some(Slot::Lazy { factory, .. }) => {
                let harness = factory()?;
                slots.insert(id, Slot::Ready(harness.clone()));
                Ok(harness)
            }
            None => Err(HarnessError::NotInstalled(format!("{id:?}"))),
        }
    }

    /// Catalog for `ListHarnesses` — never forces a lazy resolve.
    pub fn descriptors(&self) -> Vec<HarnessDescriptor> {
        let enabled = self.enabled_set();
        let slots = self.slots();
        self.order()
            .iter()
            .filter_map(|id| {
                let mut descriptor = match slots.get(id) {
                    Some(Slot::Ready(harness)) => describe(harness.as_ref()),
                    Some(Slot::Lazy {
                        descriptor,
                        installed,
                        ..
                    }) => HarnessDescriptor {
                        installed: installed(),
                        ..descriptor.clone()
                    },
                    None => return None,
                };
                descriptor.enabled = Some(enabled.contains(id));
                Some(descriptor)
            })
            .collect()
    }
}

/// The registry production uses: the native agent, and nothing else.
///
/// Separate from [`default_registry`] because the native harness reads the
/// user's provider configuration, which lives under the data directory — a
/// path only the engine knows.
///
/// `MockHarness` is deliberately *not* registered here. It is test
/// infrastructure, and building it in would compile a fixed script of demo
/// events into every shipped binary for a harness no production picker offers.
pub fn registry_with_native(data_dir: &std::path::Path) -> HarnessRegistry {
    registry_with_native_and_store(data_dir).0
}

/// The registry production uses, plus the provider store it shares with the
/// settings RPCs.
///
/// One store, two holders: the harness reads it at the start of each run, the
/// settings page reads and writes it. Two instances of the same file would
/// drift, and a save's revision check would be against the wrong one.
pub fn registry_with_native_and_store(
    data_dir: &std::path::Path,
) -> (
    HarnessRegistry,
    Arc<rcode_harness::provider_store::ProviderStore>,
) {
    rcode_harness::shell_env::prewarm();
    let store = match rcode_harness::provider_store::ProviderStore::open(data_dir) {
        Ok(store) => Arc::new(store),
        Err(error) => {
            // A corrupt file must not stop the application from starting, and
            // must not be silently replaced either. The harness gets an empty
            // configuration behind that store, and the user sees the parse
            // error when they open the settings page.
            tracing::error!(%error, "providers.json could not be read; starting with none");
            Arc::new(rcode_harness::provider_store::ProviderStore::degraded(
                data_dir,
            ))
        }
    };
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(rcode_harness::native::NativeHarness::from_store(
        Arc::clone(&store),
        data_dir,
    )));
    (registry, store)
}

/// The mock-only registry, for tests and for callers with no data directory.
///
/// Kept separate so the mock stays off the production path (see
/// [`registry_with_native`]). A production engine uses that one.
pub fn default_registry() -> HarnessRegistry {
    // Warm the login-shell PATH snapshot in the background so the first
    // resolve doesn't pay the shell-startup latency inline.
    rcode_harness::shell_env::prewarm();
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(MockHarness {
        script: vec![
            AgentEvent::TextDelta {
                text: "## Streaming pipeline\n\nEvery turn flows through the same path:\n\n".into(),
            },
            AgentEvent::TextDelta {
                text: "1. **Doc command** — the composer queues a durable `run` entry\n2. **Host executor** — the chat's host device marks it processed, then dispatches\n3. **Fold** — events fold into parts and diff into the Loro doc every 120ms\n\n".into(),
            },
            AgentEvent::ToolCall {
                id: "mock-tool-1".into(),
                call: rcode_proto::ToolCall::Exec {
                    command: "cargo test --workspace".into(),
                },
            },
            AgentEvent::ToolResult {
                id: "mock-tool-1".into(),
                is_error: false,
                output: None,
                diff: None,
            },
            AgentEvent::ToolCall {
                id: "mock-tool-2".into(),
                call: rcode_proto::ToolCall::Exec {
                    command: "git log -5 --oneline --decorate && git merge-base HEAD origin/main"
                        .into(),
                },
            },
            AgentEvent::ToolResult {
                id: "mock-tool-2".into(),
                is_error: false,
                output: None,
                diff: None,
            },
            AgentEvent::TextDelta {
                text: "The `SegmentWriter` appends into `LoroText` so the oplog stays RLE-merged:\n\n```rust\nfolded = fold_event_into_parts(&folded, &event);\nwriter.sync(&folded)?; // 120ms coalesced commits\n```\n\nSynced to every device through the session room. *Mock harness reporting in.*".into(),
            },
            AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: None,
            },
        ],
    }));
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mid_turn_steering_requires_support_and_a_step_boundary() {
        let mut descriptor = HarnessDescriptor {
            id: HarnessId::Mock,
            name: "Mock".into(),
            supports_steering: true,
            steering_mode: SteeringMode::StepBoundary,
            reasoning_levels: Vec::new(),
            installed: true,
            enabled: Some(true),
        };
        assert!(descriptor.steers_mid_turn());

        descriptor.steering_mode = SteeringMode::TurnBoundary;
        assert!(!descriptor.steers_mid_turn());

        descriptor.steering_mode = SteeringMode::StepBoundary;
        descriptor.supports_steering = false;
        assert!(!descriptor.steers_mid_turn());
    }

    #[test]
    fn lazy_slot_lists_without_resolving() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let registry = HarnessRegistry::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        registry.register_lazy(
            HarnessDescriptor {
                id: HarnessId::Mock,
                name: "Lazy Mock".into(),
                supports_steering: true,
                steering_mode: SteeringMode::StepBoundary,
                reasoning_levels: vec![],
                installed: true,
                enabled: None,
            },
            Box::new(|| false),
            Box::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Err(HarnessError::NotInstalled("nope".into()))
            }),
        );
        let listed = registry.descriptors();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Lazy Mock");
        // The listing runs the probe, not the stored placeholder.
        assert!(!listed[0].installed);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "listing must not force a resolve"
        );
        assert!(registry.resolve(HarnessId::Mock).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// Catalogs serialized by engines that predate the `installed`/`enabled`
    /// fields must keep deserializing — installed, and enabled per the
    /// detection fallback.
    #[test]
    fn descriptor_without_new_fields_parses_with_fallbacks() {
        let parse = |id: &str| -> HarnessDescriptor {
            serde_json::from_str(&format!(
                r#"{{
                    "id": "{id}",
                    "name": "x",
                    "supportsSteering": true,
                    "steeringMode": "step-boundary",
                    "reasoningLevels": []
                }}"#
            ))
            .unwrap()
        };
        let claude = parse("claude-code");
        assert!(claude.installed);
        assert_eq!(claude.enabled, None);
        // Unknown enablement follows detection: a found CLI is offered...
        assert!(descriptor_enabled(&claude));
        // ...and one this device never found is not.
        let missing = HarnessDescriptor {
            installed: false,
            ..parse("grok")
        };
        assert!(!descriptor_enabled(&missing));
    }

    /// A registry slot for the tests below: installed probe fixed, factory
    /// never expected to run.
    fn test_slot(registry: &HarnessRegistry, id: HarnessId, installed: bool) {
        registry.register_lazy(
            HarnessDescriptor {
                id,
                name: format!("{id:?}"),
                supports_steering: true,
                steering_mode: SteeringMode::StepBoundary,
                reasoning_levels: vec![],
                installed: true,
                enabled: None,
            },
            Box::new(move || installed),
            Box::new(|| Err(HarnessError::NotInstalled("test slot".into()))),
        );
    }

    /// `descriptors()` stamps the per-device enabled flag; `set_enabled`
    /// guards the gate (no enabling missing CLIs, no disabling the last one)
    /// and persists across a reload.
    #[test]
    fn enablement_stamps_guards_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let registry = HarnessRegistry::new();
        registry.load_prefs(dir.path());
        test_slot(&registry, HarnessId::ClaudeCode, true);
        test_slot(&registry, HarnessId::Codex, true);
        test_slot(&registry, HarnessId::Grok, true);
        test_slot(&registry, HarnessId::Hermes, false);

        // Enablement follows detection: the three found CLIs are on with no
        // prefs file at all, and the missing one is off.
        let flags: Vec<(HarnessId, Option<bool>)> = registry
            .descriptors()
            .into_iter()
            .map(|d| (d.id, d.enabled))
            .collect();
        assert_eq!(
            flags,
            vec![
                (HarnessId::ClaudeCode, Some(true)),
                (HarnessId::Codex, Some(true)),
                (HarnessId::Grok, Some(true)),
                (HarnessId::Hermes, Some(false)),
            ]
        );

        // The gate: a missing CLI can't be enabled; unknown ids refuse.
        assert!(registry.set_enabled(HarnessId::Hermes, true).is_err());
        assert!(registry.set_enabled(HarnessId::Pi, true).is_err());
        assert!(registry.set_enabled(HarnessId::Mock, true).is_err());
        // Installed CLIs toggle both ways; no-op flips are fine.
        registry.set_enabled(HarnessId::Grok, true).unwrap();
        registry.set_enabled(HarnessId::Grok, true).unwrap();
        registry.set_enabled(HarnessId::Codex, false).unwrap();
        registry.set_enabled(HarnessId::ClaudeCode, false).unwrap();
        // Grok is the last one standing — refusing keeps the composer usable.
        assert!(registry.set_enabled(HarnessId::Grok, false).is_err());
        assert_eq!(registry.enabled_set(), vec![HarnessId::Grok]);

        // A fresh registry over the same data dir reads the persisted opt-outs.
        let reloaded = HarnessRegistry::new();
        reloaded.load_prefs(dir.path());
        test_slot(&reloaded, HarnessId::ClaudeCode, true);
        test_slot(&reloaded, HarnessId::Codex, true);
        test_slot(&reloaded, HarnessId::Grok, true);
        assert_eq!(reloaded.enabled_set(), vec![HarnessId::Grok]);
    }

    /// The point of following detection: an agent installed after the user has
    /// already edited Settings turns itself on, while the ones they switched
    /// off stay off. An allow-list can't express that — it can't tell "the
    /// user said no" from "this wasn't installed yet".
    #[test]
    fn newly_found_harnesses_enable_themselves_without_reviving_opt_outs() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let registry = HarnessRegistry::new();
        registry.load_prefs(dir.path());
        test_slot(&registry, HarnessId::ClaudeCode, true);
        test_slot(&registry, HarnessId::Codex, true);
        // Not installed yet — the probe flips when the user installs the CLI.
        let found = Arc::new(AtomicBool::new(false));
        let probe = Arc::clone(&found);
        registry.register_lazy(
            HarnessDescriptor {
                id: HarnessId::Grok,
                name: "Grok".into(),
                supports_steering: true,
                steering_mode: SteeringMode::TurnBoundary,
                reasoning_levels: vec![],
                installed: true,
                enabled: None,
            },
            Box::new(move || probe.load(Ordering::SeqCst)),
            Box::new(|| Err(HarnessError::NotInstalled("test slot".into()))),
        );
        registry.set_enabled(HarnessId::Codex, false).unwrap();
        assert_eq!(registry.enabled_set(), vec![HarnessId::ClaudeCode]);

        // The CLI appears mid-session; no restart, no visit to Settings.
        found.store(true, Ordering::SeqCst);
        assert_eq!(
            registry.enabled_set(),
            vec![HarnessId::ClaudeCode, HarnessId::Grok]
        );
        // The opt-out survives the reload that picks the new agent up.
        let reloaded = HarnessRegistry::new();
        reloaded.load_prefs(dir.path());
        test_slot(&reloaded, HarnessId::ClaudeCode, true);
        test_slot(&reloaded, HarnessId::Codex, true);
        test_slot(&reloaded, HarnessId::Grok, true);
        assert_eq!(
            reloaded.enabled_set(),
            vec![HarnessId::ClaudeCode, HarnessId::Grok]
        );
    }

    #[test]
    fn antigravity_detection_ignores_legacy_opt_in_and_preserves_opt_out() {
        let dir = tempfile::tempdir().unwrap();
        let registry = HarnessRegistry::new();
        registry.load_prefs(dir.path());
        test_slot(&registry, HarnessId::ClaudeCode, true);
        test_slot(&registry, HarnessId::Antigravity, true);
        let prefs: HarnessPrefsFile =
            serde_json::from_str(r#"{"optedIn":["antigravity"],"disabled":["antigravity"]}"#)
                .unwrap();
        *registry.prefs() = prefs;
        assert_eq!(registry.enabled_set(), vec![HarnessId::ClaudeCode]);

        registry.set_enabled(HarnessId::Antigravity, true).unwrap();
        let both = vec![HarnessId::ClaudeCode, HarnessId::Antigravity];
        assert_eq!(registry.enabled_set(), both);

        let reloaded = HarnessRegistry::new();
        reloaded.load_prefs(dir.path());
        test_slot(&reloaded, HarnessId::ClaudeCode, true);
        test_slot(&reloaded, HarnessId::Antigravity, true);
        assert_eq!(reloaded.enabled_set(), both);

        reloaded.set_enabled(HarnessId::Antigravity, false).unwrap();
        assert_eq!(reloaded.enabled_set(), vec![HarnessId::ClaudeCode]);
    }

    /// The mock resolves on every machine, so detection alone would enable it
    /// and let the last REAL agent be switched off — leaving a composer with
    /// nothing runnable behind a guard that thinks it is covered.
    #[test]
    fn detection_never_enables_the_mock() {
        let registry = default_registry();
        let enabled = registry.enabled_set();
        assert!(!enabled.contains(&HarnessId::Mock), "{enabled:?}");
        let mock = registry
            .descriptors()
            .into_iter()
            .find(|d| d.id == HarnessId::Mock)
            .expect("mock slot");
        assert!(mock.installed);
        assert_eq!(mock.enabled, Some(false));
    }

    /// A prefs file from the fixed-default era is an allow-list: everything
    /// registered and absent from it was a deliberate "no", so it converts to
    /// opt-outs rather than silently re-enabling on the next launch.
    #[test]
    fn legacy_allow_list_migrates_to_opt_outs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("harness-prefs.json"),
            r#"{ "enabled": ["claude-code"] }"#,
        )
        .unwrap();
        let registry = HarnessRegistry::new();
        test_slot(&registry, HarnessId::ClaudeCode, true);
        test_slot(&registry, HarnessId::Codex, true);
        test_slot(&registry, HarnessId::Grok, true);
        registry.load_prefs(dir.path());
        assert_eq!(registry.enabled_set(), vec![HarnessId::ClaudeCode]);

        // The rewritten file is the new shape, and the legacy key is gone.
        let text = std::fs::read_to_string(dir.path().join("harness-prefs.json")).unwrap();
        assert!(!text.contains("enabled"), "{text}");
        assert!(text.contains("codex") && text.contains("grok"), "{text}");
        // An agent registered after the migration is new, not a past "no".
        test_slot(&registry, HarnessId::Cursor, true);
        assert_eq!(
            registry.enabled_set(),
            vec![HarnessId::ClaudeCode, HarnessId::Cursor]
        );
    }

    /// The upgrade path the allow-list test above cannot cover: a file written
    /// before the built-in agent existed, loaded by a registry that registers
    /// it alone. The old list could not name that agent, so its absence is not
    /// the user's "no" — and honoring it as one would leave the composer with
    /// nothing runnable at all.
    #[test]
    fn legacy_allow_list_keeps_the_builtin_agent_enabled() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("harness-prefs.json"),
            r#"{ "enabled": ["claude-code"] }"#,
        )
        .unwrap();
        let registry = HarnessRegistry::new();
        registry.register(Arc::new(rcode_harness::native::NativeHarness::default()));
        registry.load_prefs(dir.path());
        assert_eq!(registry.enabled_set(), vec![HarnessId::Native]);

        let text = std::fs::read_to_string(dir.path().join("harness-prefs.json")).unwrap();
        let prefs: HarnessPrefsFile = serde_json::from_str(&text).unwrap();
        assert!(prefs.disabled.is_empty(), "{:?}", prefs.disabled);
    }

    /// The same invariant on the new file shape: a prefs file naming every
    /// runnable agent as disabled — hand-edited, or written by a build that
    /// registered more of them — must not strand the composer either.
    #[test]
    fn prefs_disabling_every_runnable_agent_re_enables_one() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("harness-prefs.json"),
            r#"{ "disabled": ["native"] }"#,
        )
        .unwrap();
        let registry = HarnessRegistry::new();
        registry.register(Arc::new(rcode_harness::native::NativeHarness::default()));
        registry.load_prefs(dir.path());
        assert_eq!(registry.enabled_set(), vec![HarnessId::Native]);

        // The healed file agrees, so the next load has nothing to repair.
        let reloaded = HarnessRegistry::new();
        reloaded.register(Arc::new(rcode_harness::native::NativeHarness::default()));
        reloaded.load_prefs(dir.path());
        assert_eq!(reloaded.enabled_set(), vec![HarnessId::Native]);
    }

    /// The fresh-machine shape (#128): no CLIs installed at all. Under
    /// detection-based enablement nothing is enabled to begin with — no
    /// dimmed default toggles to dismiss — and switching an uninstalled
    /// harness "off" is a clean no-op, not an error.
    #[test]
    fn machine_without_clis_enables_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let registry = HarnessRegistry::new();
        registry.load_prefs(dir.path());
        test_slot(&registry, HarnessId::ClaudeCode, false);
        test_slot(&registry, HarnessId::Codex, false);

        assert_eq!(registry.enabled_set(), Vec::<HarnessId>::new());
        registry.set_enabled(HarnessId::Codex, false).unwrap();
        registry.set_enabled(HarnessId::ClaudeCode, false).unwrap();
        assert_eq!(registry.enabled_set(), Vec::<HarnessId>::new());

        let reloaded = HarnessRegistry::new();
        reloaded.load_prefs(dir.path());
        assert_eq!(reloaded.enabled_set(), Vec::<HarnessId>::new());
    }
}

#[cfg(test)]
mod title_tests {
    use super::*;

    #[test]
    fn title_preferences_persist_and_validate_harness_model_pairs() {
        let dir = tempfile::tempdir().unwrap();
        let registry = HarnessRegistry::new();
        registry.load_prefs(dir.path());
        // Native is the title driver: it supports titles and is auto-enabled
        // (Mock is title-capable too but `auto_enabled` excludes it, so it can
        // never satisfy the enabled-set guard below).
        registry.register(Arc::new(rcode_harness::native::NativeHarness::default()));
        let settings = TitleSettings {
            harness: Some(HarnessId::Native),
            model: Some("haiku-test".into()),
        };
        registry.set_title_settings(settings.clone()).unwrap();
        let reloaded = HarnessRegistry::new();
        reloaded.load_prefs(dir.path());
        assert_eq!(reloaded.title_settings(), settings);
        assert!(
            registry
                .set_title_settings(TitleSettings {
                    harness: None,
                    model: Some("orphan".into())
                })
                .is_err()
        );
        assert!(
            registry
                .set_title_settings(TitleSettings {
                    harness: Some(HarnessId::Cursor),
                    model: None
                })
                .is_err()
        );
        assert_eq!(registry.title_settings(), settings);
        registry
            .set_title_settings(TitleSettings::default())
            .unwrap();
        reloaded.load_prefs(dir.path());
        assert_eq!(reloaded.title_settings(), TitleSettings::default());
    }

    #[test]
    fn old_harness_preferences_default_to_automatic_titles() {
        let prefs: HarnessPrefsFile = serde_json::from_str(r#"{"disabled":["codex"]}"#).unwrap();
        assert_eq!(prefs.titles, TitleSettings::default());
        assert_eq!(prefs.disabled, vec![HarnessId::Codex]);
    }
}
