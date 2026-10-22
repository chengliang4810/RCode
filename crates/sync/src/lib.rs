//! rcode-sync — local SQLite persistence and optional transport clients.
//!
//! - [`ChatClient`] exchanges chat2 checkpoint/row frames over WebSocket.
//! - [`RegistryClient`] exchanges workspace registry rows and presence.
//! - [`DocsStore`] persists snapshots, pending work and processed commands.
//!
//! The clients require a separately configured protocol endpoint. The mock-server
//! feature supplies local test infrastructure.

pub mod budget;
pub mod chat_client;
pub mod chat_frames;
pub mod dial;
pub mod net_path;
pub mod registry;
pub mod socket;
mod store;
mod sync_jobs;
mod types;
pub mod wake;

pub use chat_client::{
    ChatClient, ChatDocSink, ChatEvent, ChatStatsSnapshot, ChatTuning, CheckpointFetcher,
};
pub use registry::{
    ReconnectState, RegistryClient, RegistryEvent, RegistryTransport, RegistryTuning,
};
pub use store::{DocsStore, StoreError};
pub use types::{RoomStatsSnapshot, StaticUrl, SyncError, UrlProvider};
