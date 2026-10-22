//! RCode mobile core — the UniFFI surface shared by the iOS and Android apps.
//!
//! - [`client_ffi`]: account, workspace and session state (wraps `rcode-client`).
//! - [`layout`]: analytic transcript layout — markdown → measured display lists
//!   (wraps `rcode-markdown` + `rcode-text`).

uniffi::setup_scaffolding!("rcode_core");

mod client_ffi;
mod flutter_visual;
mod l10n;
pub mod layout;
pub mod visual;
pub mod wallpaper;

/// Version handshake: the Swift/Kotlin bindings must match the linked library.
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}
