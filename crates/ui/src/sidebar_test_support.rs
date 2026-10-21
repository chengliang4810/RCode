//! Shared fixtures for the sidebar's pin and section tests.
//!
//! Pins and sections live in the workspace registry, and every write goes to
//! the engine — so a test that exercises them needs an engine, an identity (the
//! profile key is derived from one), and a way to put registry state in place
//! without a live engine answering. That is what this module is for.
//!
//! `None` semantics matter here and are easy to get wrong: the profile key is
//! `None` until an identity exists, and callers treat that as "not ready"
//! rather than "no profile". [`auth`] is what makes the key exist.

use rcode_proto::{AuthState, SidebarSection, UserProfile};

use crate::state::{AppState, EngineHandle};

/// The one identity this build has. Tests that need a profile key take it from
/// here rather than inventing one, so a key computed in a test is the key the
/// shell computes.
pub(crate) fn auth() -> AuthState {
    AuthState::SignedIn {
        user: UserProfile {
            id: "user".into(),
            email: "user@example.test".into(),
            name: None,
        },
        org_id: Some("org".into()),
    }
}

/// The profile key for [`auth`] with no configured org override.
pub(crate) fn profile_key() -> String {
    crate::settings::sidebar_pin_profile_key(Some(&auth()), None).expect("a profile key")
}

/// An engine whose writes are accepted and never answered.
///
/// The optimistic path is what these tests inspect: a pending write projects
/// onto the state immediately and stays projected while the engine is silent,
/// which is exactly the window the sidebar renders.
///
/// The runtime is leaked on purpose. `RpcClient::new` spawns a reader task onto
/// the current runtime, and that task has to outlive this call or the client
/// stops reading; a `Handle` alone cannot be kept, because dropping the
/// `Runtime` it belongs to shuts it down. The leak is bounded — the process
/// ends with the test. Both channel ends stay open for the same reason: a
/// closed receiver fails the send, which marks the write unconfirmed and
/// erases the projection under test.
pub(crate) fn silent_engine_handle() -> EngineHandle {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    // The client is built with the runtime entered — `RpcClient::new` spawns
    // its reader task — and the runtime is leaked afterwards, because dropping
    // it would stop that task and with it the client. The `EnterGuard` borrows
    // the runtime, so it closes before the leak.
    let handle = {
        let _guard = runtime.enter();
        let (out, requests) = tokio::sync::mpsc::channel(64);
        let (replies, inbound) = tokio::sync::mpsc::channel(64);
        std::mem::forget(requests);
        std::mem::forget(replies);
        EngineHandle::from_test_client(rcode_rpc::RpcClient::new(out, inbound))
    };
    std::mem::forget(runtime);
    handle
}

/// Put registry state in place: identity, pins, sections, and the two flags
/// that say the app may edit them.
///
/// `synced` and `initialized` are what `SidebarPreferencesState::can_edit`
/// reads, so a test that forgets them gets a sidebar that refuses writes — the
/// state a real client is in before its first registry frame, and not what
/// these tests are about.
pub(crate) fn seed_preferences(
    state: &mut AppState,
    pins: &[&str],
    sections: Vec<SidebarSection>,
    editable: bool,
) {
    state.auth = Some(auth());
    state.sidebar_preferences.pinned_session_ids =
        pins.iter().map(|id| (*id).to_string()).collect();
    state.sidebar_preferences.sections = sections;
    state.sidebar_preferences.synced = editable;
    state.sidebar_preferences.initialized = editable;
}

/// A section with no members, for fixtures that only need the group to exist.
pub(crate) fn section(id: &str, name: &str) -> SidebarSection {
    SidebarSection {
        id: id.into(),
        name: name.into(),
        session_ids: Vec::new(),
        collapsed: false,
    }
}

/// The default two-section fixture (`Focus`, `Later`) several tests expect.
pub(crate) fn default_sections() -> Vec<SidebarSection> {
    vec![section("a", "Focus"), section("b", "Later")]
}
