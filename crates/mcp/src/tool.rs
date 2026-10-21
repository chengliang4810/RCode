//! One MCP tool, as a tool the ReAct loop can call.
//!
//! The bridge is deliberately thin: the name, description and schema come from
//! the server (a proxy that restated them would drift from what it proxies),
//! and the call forwards the model's JSON unchanged. Everything this side adds
//! is the namespaced tool name and
//! turning a transport failure into a result the model can read.

use std::sync::Arc;

use async_trait::async_trait;
use rcode_agent::tool::{Tool, ToolContext, ToolError, ToolResult};

use crate::client::{McpClient, McpToolInfo, result_is_error, result_text};

pub struct McpTool {
    info: Arc<McpToolInfo>,
    client: Arc<McpClient>,
}

impl McpTool {
    pub fn new(info: Arc<McpToolInfo>, client: Arc<McpClient>) -> Self {
        Self { info, client }
    }

    pub fn info(&self) -> &McpToolInfo {
        &self.info
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.info.wire_name
    }

    fn description(&self) -> &str {
        // A server is free to leave the description out; the model still needs
        // to know what namespace it is calling into.
        if self.info.description.is_empty() {
            "A tool from an MCP server."
        } else {
            &self.info.description
        }
    }

    fn input_schema(&self) -> serde_json::Value {
        self.info.input_schema.clone()
    }

    async fn call(
        &self,
        input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let arguments = (!input.is_null()).then_some(input);
        match self.client.call(&self.info.wire_name, arguments).await {
            Ok(result) => {
                let text = result_text(&result);
                // A tool-level failure is a *result*, not a tool error: the
                // model should read the server's reason and decide what to do.
                Ok(if result_is_error(&result) {
                    ToolResult::error(if text.is_empty() {
                        "the MCP server reported a failure without a message".to_owned()
                    } else {
                        text
                    })
                } else {
                    ToolResult::ok(text)
                })
            }
            // A server that could not be reached is also a result: the model
            // can report it to the user, and the run continues with the rest
            // of the tools.
            Err(error) => Ok(ToolResult::error(format!(
                "MCP server `{}` is unavailable: {error}",
                self.info.server
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> Arc<McpToolInfo> {
        Arc::new(McpToolInfo {
            server: "files".to_owned(),
            name: "read_file".to_owned(),
            wire_name: McpToolInfo::wire_name("files", "read_file"),
            description: "Read a file.".to_owned(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } }
            }),
        })
    }

    #[test]
    fn the_wire_name_namespaces_the_server() {
        // Two servers may both define `search`; the model has one flat list.
        assert_eq!(
            McpToolInfo::wire_name("files", "read_file"),
            "mcp__files__read_file"
        );
        assert_eq!(McpToolInfo::wire_name("kit:docs", "get_widget"), "mcp__plugin_kit_docs__get_widget");
        assert_ne!(
            McpToolInfo::wire_name("a", "search"),
            McpToolInfo::wire_name("b", "search")
        );
    }

    #[test]
    fn the_schema_comes_from_the_server_unchanged() {
        let tool = McpTool::new(info(), unreachable_client());
        let schema = tool.input_schema();
        assert_eq!(schema["properties"]["path"]["type"], "string");
    }

    #[test]
    fn an_empty_description_still_says_what_it_is() {
        let mut bare = (*info()).clone();
        bare.description = String::new();
        let tool = McpTool::new(Arc::new(bare), unreachable_client());
        assert!(tool.description().contains("MCP"));
    }

    /// A client with one server that will not start: enough to exercise the
    /// tool's own behaviour without spawning anything.
    fn unreachable_client() -> Arc<McpClient> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let client = runtime.block_on(McpClient::connect(vec![crate::config::McpServer {
            name: "files".to_owned(),
            transport: crate::config::McpTransport::Stdio {
                command: "definitely-not-a-real-command-rcode".to_owned(),
                args: Vec::new(),
                env: Default::default(),
            },
            enabled: true,
            source: crate::config::McpSource::User,
        }]));
        std::mem::forget(runtime);
        Arc::new(client)
    }
}
