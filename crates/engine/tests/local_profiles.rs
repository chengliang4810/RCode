//! Integrated regressions for local-first profile privacy and lifecycle boundaries.

use std::path::Path;
use std::sync::{Arc, Barrier};

use rcode_engine::{
    DataRoots, Engine, EngineConfig, EngineCore, EngineProfile, HarnessId, default_registry,
};

fn config(data_dir: &Path, edge_url: String, edge_token: Option<&str>) -> EngineConfig {
    EngineConfig {
        roots: DataRoots::same(data_dir),
        edge_url,
        edge_token: edge_token.map(str::to_string),
        ipc_port: 0,
        default_harness: HarnessId::Mock,
        org_id: None,
    }
}

fn assemble(profile: EngineProfile) -> EngineCore {
    EngineCore::assemble_with_profile(profile, Arc::new(default_registry()), HarnessId::Mock, None)
        .expect("assemble profile")
}

async fn shutdown(core: EngineCore) {
    core.shutdown().await;
    drop(core);
}

fn concurrent_engine_info(
    config: Arc<EngineConfig>,
    workers: usize,
) -> std::collections::HashSet<String> {
    let barrier = Arc::new(Barrier::new(workers));
    (0..workers)
        .map(|_| {
            let config = config.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                Engine::engine_info(&config)
                    .expect("resolve concurrent engine info")
                    .device_id
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|call| call.join().expect("engine-info worker"))
        .collect()
}

#[tokio::test]
async fn concurrent_engine_info_and_runtime_share_one_device_identity() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = Arc::new(config(dir.path(), "http://127.0.0.1:1".into(), None));
    let announced = concurrent_engine_info(config, 32);

    assert_eq!(announced.len(), 1, "every viewport announces one identity");
    let announced = announced.into_iter().next().expect("announced id");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("device-id"))
            .expect("persisted device id")
            .trim(),
        announced
    );

    let core = assemble(EngineProfile::new(
        &DataRoots::same(dir.path()),
        "test-org",
        "test-user",
    ));
    assert_eq!(
        core.device_id, announced,
        "the assembled runtime must use the identity already announced"
    );
    shutdown(core).await;
}

#[tokio::test]
async fn empty_legacy_device_identity_is_repaired_once_for_all_boots() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("device-id"), b"").expect("seed truncated identity");
    let config = Arc::new(config(dir.path(), "http://127.0.0.1:1".into(), None));

    let announced = concurrent_engine_info(config, 32);
    assert_eq!(
        announced.len(),
        1,
        "legacy repair must publish one identity"
    );
    let announced = announced.into_iter().next().expect("repaired id");
    assert!(!announced.trim().is_empty());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("device-id"))
            .expect("repaired device id")
            .trim(),
        announced
    );

    let core = assemble(EngineProfile::new(
        &DataRoots::same(dir.path()),
        "test-org",
        "test-user",
    ));
    assert_eq!(core.device_id, announced);
    shutdown(core).await;
}

/// A `location` pointer moves the growth and leaves the identity behind.
///
/// The two roots are what makes "put the data on another drive" possible
/// without moving the API keys off the system one; this pins which side each
/// artefact lands on, because getting it wrong is silent.
#[tokio::test]
async fn a_split_moves_the_stores_and_keeps_the_identity() {
    let config_root = tempfile::tempdir().expect("config root");
    let data_root = tempfile::tempdir().expect("data root");
    let roots = DataRoots::split(config_root.path(), data_root.path());

    let profile = EngineProfile::new(&roots, "dev-org", "dev-user");
    assert_eq!(profile.config_root(), config_root.path());
    assert_eq!(profile.data_root(), data_root.path());
    assert!(
        profile.store_root().starts_with(data_root.path()),
        "the store root belongs to the data root: {}",
        profile.store_root().display()
    );

    let core = assemble(profile);
    assert!(
        data_root
            .path()
            .join("orgs/dev-org/dev-user/docs.sqlite3")
            .is_file(),
        "documents land in the data root"
    );
    assert!(
        !config_root.path().join("orgs").exists(),
        "nothing store-shaped may be created beside the identity"
    );
    assert!(
        config_root.path().join("device-id").is_file(),
        "the device id stays with the config"
    );
    assert!(
        config_root.path().join("engine.lock").exists(),
        "the single-instance lock stays with the config"
    );
    shutdown(core).await;
}
