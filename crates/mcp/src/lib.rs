//! MCP client support: configuration, server connections, and the tool bridge.
//!
//! `rcode-agent`'s `Tool` trait is the seam (decision D22): an MCP tool is
//! bridged into an ordinary tool, so the ReAct loop never learns that MCP
//! exists. This crate owns everything that would leak if it did — transports,
//! handshakes, reconnects, and the name mangling that keeps two servers' tools
//! apart.

mod client;
pub mod config;
mod pool;
mod tool;

pub use client::{McpClient, McpToolInfo, ServerReport, result_is_error, result_text, tool_counts};
pub use config::{ConfigError, McpServer, McpSource, McpTransport, load};
pub use pool::McpClientPool;
pub use tool::McpTool;

use std::path::Path;
use std::sync::Arc;

/// Connect to every server configured for `cwd`.
///
/// A configuration that cannot be read yields no client and no tools, with a
/// log line: the agent still runs, it just cannot reach those servers — the
/// same posture as a skill scan that meets a malformed file. A server that
/// fails to start is recorded on the client rather than failing this call.
pub async fn connect(cwd: &Path, config_root: &Path) -> Option<Arc<McpClient>> {
    let servers = match load(cwd, config_root) {
        Ok(servers) => servers,
        Err(error) => {
            tracing::warn!(%error, "MCP configuration could not be read; no servers are available");
            return None;
        }
    };
    if servers.is_empty() {
        return None;
    }

    let client = Arc::new(McpClient::connect_in(servers, Some(cwd.to_path_buf())).await);
    tracing::info!(servers = ?tool_counts(&client), "MCP servers connected");
    Some(client)
}
