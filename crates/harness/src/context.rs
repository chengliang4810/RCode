//! Context delivery for a native run. No model-specific routing capability is required.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rcode_agent::ToolRegistry;
use rcode_agent::context::{ContextSource, Domain, Snapshot, Status};
use sha2::{Digest, Sha256};

pub(crate) struct RuntimeContext {
    memory_root: Option<PathBuf>,
    memory_enabled: bool,
    mcp: Snapshot,
}

impl RuntimeContext {
    pub(crate) fn new(
        cwd: &Path,
        data_dir: Option<&Path>,
        memory_enabled: bool,
        tools: &ToolRegistry,
        instructions: Option<&str>,
    ) -> Arc<dyn ContextSource> {
        let mut entries = BTreeMap::new();
        for tool in tools.all() {
            if tool.name().starts_with("mcp__") {
                // Use the exact tool identity: splitting __ cannot unambiguously recover server names.
                let bytes =
                    serde_json::to_vec(&(tool.description(), tool.input_schema(), instructions))
                        .expect("MCP metadata serializes");
                entries.insert(
                    tool.name().to_owned(),
                    format!("{:x}", Sha256::digest(bytes)),
                );
            }
        }
        // Hash metadata for comparison; do not re-emit instruction bodies at a lower role.
        // The current MCP API exposes combined instructions, so a change can notify all MCP tools.
        Arc::new(Self {
            memory_root: data_dir.map(|dir| rcode_agent::memory::directory(cwd, dir)),
            memory_enabled,
            mcp: Snapshot {
                domain: Domain::McpStatus,
                scope: "current-run-tools".into(),
                status: Status::Ready,
                entries,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(&'static str);
    #[async_trait::async_trait]
    impl rcode_agent::Tool for Fixture {
        fn name(&self) -> &str {
            "mcp__server__with__underscores__lookup"
        }
        fn description(&self) -> &str {
            "fixture"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object", "description":self.0})
        }
        async fn call(
            &self,
            _: serde_json::Value,
            _: &rcode_agent::ToolContext,
        ) -> Result<rcode_agent::ToolResult, rcode_agent::ToolError> {
            unreachable!()
        }
    }
    #[tokio::test]
    async fn mcp_content_schema_and_disconnect_changes_preserve_exact_tool_identities() {
        let root = tempfile::tempdir().unwrap();
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(Fixture("schema-one")));
        let current = RuntimeContext::new(
            root.path(),
            None,
            false,
            &tools,
            Some("SYSTEM INSTRUCTION ONE"),
        )
        .snapshots()
        .await
        .remove(0);
        assert_eq!(current.entries.len(), 1);
        assert!(
            current
                .entries
                .contains_key("mcp__server__with__underscores__lookup")
        );
        let message = rcode_agent::context::next_record(&[], current.clone())
            .unwrap()
            .message();
        assert!(
            !message
                .content
                .text_content()
                .contains("SYSTEM INSTRUCTION ONE")
        );
        let changed = RuntimeContext::new(
            root.path(),
            None,
            false,
            &tools,
            Some("SYSTEM INSTRUCTION TWO"),
        )
        .snapshots()
        .await
        .remove(0);
        assert_ne!(current.entries, changed.entries);
        tools.register(Arc::new(Fixture("schema-two")));
        let schema = RuntimeContext::new(
            root.path(),
            None,
            false,
            &tools,
            Some("SYSTEM INSTRUCTION ONE"),
        )
        .snapshots()
        .await
        .remove(0);
        assert_ne!(current.entries, schema.entries);
        let empty = RuntimeContext::new(root.path(), None, false, &ToolRegistry::new(), None)
            .snapshots()
            .await
            .remove(0);
        let removed = rcode_agent::context::next_record(&[message], empty).unwrap();
        assert_eq!(removed.remove, ["mcp__server__with__underscores__lookup"]);
    }
    #[tokio::test]
    async fn disabled_memory_does_not_create_or_read_its_directory() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let source = RuntimeContext::new(
            root.path(),
            Some(data.path()),
            false,
            &ToolRegistry::new(),
            None,
        );
        let states = source.snapshots().await;
        assert_eq!(states.last().unwrap().status, Status::Disabled);
        assert!(!rcode_agent::memory::directory(root.path(), data.path()).exists());
    }
}

#[async_trait::async_trait]
impl ContextSource for RuntimeContext {
    async fn snapshots(&self) -> Vec<Snapshot> {
        let mut snapshots = vec![self.mcp.clone()];
        if let Some(root) = &self.memory_root {
            if self.memory_enabled {
                let root = root.clone();
                let fallback_scope = root
                    .join(rcode_agent::memory::INDEX_FILE)
                    .to_string_lossy()
                    .into_owned();
                let facts = tokio::task::spawn_blocking(move || {
                    rcode_agent::memory::index::load(&root).facts
                })
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "memory context reader failed");
                    Snapshot {
                        domain: Domain::MemoryIndex,
                        scope: fallback_scope,
                        status: Status::Unavailable,
                        entries: BTreeMap::new(),
                    }
                });
                snapshots.push(facts);
            } else {
                // In particular, do not read/create the memory directory while disabled.
                snapshots.push(Snapshot {
                    domain: Domain::MemoryIndex,
                    scope: root.to_string_lossy().into_owned(),
                    status: Status::Disabled,
                    entries: BTreeMap::new(),
                });
            }
        }
        snapshots
    }
}
