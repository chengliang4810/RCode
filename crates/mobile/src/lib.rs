//! Portable native rendering shared by Flutter's Android and iOS clients.
//!
//! Flutter owns application state and remote RPC. The narrow C ABI exposes
//! Luau visuals, GPU frames and six bundled math fonts, while the internal
//! layout and wallpaper algorithms remain available to Rust and its tests.

mod flutter_visual;
mod l10n;
pub mod layout;
pub mod visual;
pub mod wallpaper;
