//! Connections belong to an immutable project/configuration snapshot. New
//! runs reload the effective configuration; existing runs retain their Arc.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{Mutex, OnceCell};

use crate::{McpClient, McpServer};

const MAX_CACHED_PROJECTS: usize = 8;

#[derive(PartialEq, Eq)]
struct Scope {
    cwd: PathBuf,
    config_root: PathBuf,
}

struct Entry {
    scope: Scope,
    servers: Vec<McpServer>,
    client: Arc<OnceCell<Arc<McpClient>>>,
}

/// Reuse healthy connections without freezing the first project's settings.
/// Idle scopes are bounded; eviction never changes an in-flight run's client.
#[derive(Default)]
pub struct McpClientPool {
    entries: Mutex<VecDeque<Entry>>,
}

impl McpClientPool {
    /// Acquire the current effective snapshot, including plugin and project
    /// overrides. Equality compares the actual configuration, not file mtimes.
    pub async fn client(&self, cwd: &Path, config_root: &Path) -> Option<Arc<McpClient>> {
        let scope = Scope {
            cwd: normalize(cwd),
            config_root: normalize(config_root),
        };
        let servers = match crate::load(&scope.cwd, &scope.config_root) {
            Ok(servers) => servers,
            Err(error) => {
                self.remove(&scope).await;
                tracing::warn!(%error, "MCP configuration could not be read; no servers are available");
                return None;
            }
        };
        if !servers.iter().any(|server| server.enabled) {
            self.remove(&scope).await;
            return None;
        }

        let (cell, previous) = {
            let mut entries = self.entries.lock().await;
            let existing = entries
                .iter()
                .position(|entry| entry.scope == scope)
                .and_then(|index| entries.remove(index));
            let previous = existing
                .as_ref()
                .and_then(|entry| entry.client.get())
                .cloned();
            let entry = match existing {
                Some(entry)
                    if entry.servers == servers
                        && !previous
                            .as_ref()
                            .is_some_and(|client| client.has_startup_failures()) =>
                {
                    entry
                }
                _ => Entry {
                    scope: Scope {
                        cwd: scope.cwd.clone(),
                        config_root: scope.config_root.clone(),
                    },
                    servers: servers.clone(),
                    client: Arc::new(OnceCell::new()),
                },
            };
            let cell = entry.client.clone();
            entries.push_back(entry);
            while entries.len() > MAX_CACHED_PROJECTS {
                entries.pop_front();
            }
            (cell, previous)
        };

        // Single flight per snapshot, never hold the pool lock while dialing.
        let client = cell
            .get_or_init(|| async {
                Arc::new(
                    McpClient::connect_reusing(servers, Some(scope.cwd.clone()), previous).await,
                )
            })
            .await
            .clone();
        Some(client)
    }

    async fn remove(&self, scope: &Scope) {
        self.entries
            .lock()
            .await
            .retain(|entry| entry.scope != *scope);
    }
}

fn normalize(path: &Path) -> PathBuf {
    path.canonicalize()
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}
