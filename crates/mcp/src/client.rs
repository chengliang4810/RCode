//! Running MCP servers, and calling their tools.
//!
//! One connection per configured server, kept alive for the life of the
//! harness: starting `npx` or dialing an HTTP endpoint on every tool call would
//! pay the handshake per call and lose whatever state the server keeps between
//! them.
//!
//! A server that fails to start is not fatal. It is recorded with the reason
//! and skipped, the same way a malformed `SKILL.md` is skipped by the scan: one
//! broken server must not take the other tools — or the conversation — down
//! with it. The settings page shows the failure.
//!
//! Failure on a *later* call is handled differently: the call returns an error
//! result to the model, the connection is dropped, and the next call dials
//! again. A server that crashed and was restarted by its supervisor should
//! recover without the user touching anything.

use std::collections::BTreeMap;
use std::sync::Arc;

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use rmcp::service::{RoleClient, RunningService, ServiceError};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use tokio::sync::Mutex;

use crate::config::{McpServer, McpTransport};

/// One tool a server advertises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolInfo {
    pub server: String,
    /// The server's name for it, as the server spells it.
    pub name: String,
    /// The name the model sees: `mcp__<server>__<tool>`.
    ///
    /// Namespaced because two servers may both define `search`, and the model
    /// has one flat tool list. The prefix is also the rule the rewrite doc
    /// reserves (§5) and the transcript decoder already recognises.
    pub wire_name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

impl McpToolInfo {
    /// The wire name for a server's tool.
    pub fn wire_name(server: &str, tool: &str) -> String {
        let server = if let Some((plugin, name)) = server.split_once(':') { format!("plugin_{plugin}_{name}") } else { server.to_owned() };
        let safe = |name: &str| name.chars().map(|character| if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') { character } else { '_' }).collect::<String>();
        format!("mcp__{}__{}", safe(&server), safe(tool))
    }
}

/// A live connection, or why there is none.
enum Connection {
    Live(Arc<RunningService<RoleClient, ()>>),
    /// The last attempt failed. The reason travels on the server's `problem`
    /// so the settings page can show it; the next call retries.
    Failed,
}

/// Every configured server, and the tools each one advertises.
pub struct McpClient {
    servers: Vec<ServerState>,
    tools: Vec<Arc<McpToolInfo>>,
}

struct ServerState {
    config: McpServer,
    connection: Mutex<Connection>,
    /// The tools listed at startup, so a call can be routed without asking the
    /// server which endpoint it belongs to.
    tools: Vec<McpToolInfo>,
    /// Why the server did not start (or listed no tools).
    problem: Option<String>,
}

impl McpClient {
    /// Connect to every enabled server and list its tools.
    ///
    /// Never fails: an unreachable server contributes no tools and a recorded
    /// reason. Startup must not depend on somebody else's process.
    pub async fn connect(servers: Vec<McpServer>) -> Self {
        let mut states = Vec::with_capacity(servers.len());
        let mut tools = Vec::new();

        for config in servers {
            if !config.enabled {
                states.push(ServerState {
                    config,
                    connection: Mutex::new(Connection::Failed),
                    tools: Vec::new(),
                    problem: Some("switched off".to_owned()),
                });
                continue;
            }
            let (connection, listed, problem) = match open(&config).await {
                Ok((service, listed)) => (Connection::Live(Arc::new(service)), listed, None),
                Err(error) => {
                    tracing::warn!(
                        server = %config.name,
                        %error,
                        "an MCP server did not start; its tools are unavailable"
                    );
                    (Connection::Failed, Vec::new(), Some(error))
                }
            };
            for tool in &listed {
                tools.push(Arc::new(tool.clone()));
            }
            states.push(ServerState {
                config,
                connection: Mutex::new(connection),
                tools: listed,
                problem,
            });
        }

        Self {
            servers: states,
            tools,
        }
    }

    /// Every tool every server advertises, ready to register.
    pub fn tools(&self) -> &[Arc<McpToolInfo>] {
        &self.tools
    }

    /// Initialize instructions from currently connected servers. Read per run
    /// so a reconnected service does not retain its previous instructions.
    pub async fn instructions(&self) -> Option<String> {
        let mut sections = Vec::new();
        for state in &self.servers {
            let connection = state.connection.lock().await;
            if let Connection::Live(service) = &*connection {
                if let Some(text) = service.peer_info().and_then(|info| info.instructions.clone()) {
                    let text: String = text.chars().take(16_384).collect();
                    if !text.trim().is_empty() {
                        sections.push(format!("## {}\n\n{text}", state.config.name));
                    }
                }
            }
        }
        (!sections.is_empty()).then(|| sections.join("\n\n"))
    }

    /// The servers and what happened to them, for the settings page.
    pub fn servers(&self) -> Vec<ServerReport> {
        self.servers
            .iter()
            .map(|state| ServerReport {
                name: state.config.name.clone(),
                source: state.config.source,
                transport: state.config.transport.describe(),
                enabled: state.config.enabled,
                connected: state.problem.is_none(),
                problem: state.problem.clone(),
                tool_count: state.tools.len(),
            })
            .collect()
    }

    /// Call one tool. `wire_name` is [`McpToolInfo::wire_name`]'s spelling.
    ///
    /// A server that has gone away is dialled again once: the common case is a
    /// crashed local process, and making the user restart the app for it would
    /// be the wrong default.
    pub async fn call(
        &self,
        wire_name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<CallToolResult, String> {
        let (server_index, tool) = self
            .resolve(wire_name)
            .ok_or_else(|| format!("no MCP tool named `{wire_name}`"))?;

        let state = &self.servers[server_index];
        {
            let mut connection = state.connection.lock().await;
            match &*connection {
                Connection::Live(service) => {
                    match call_once(service, &tool.name, arguments.clone()).await {
                        Ok(result) => return Ok(result),
                        Err(error) => {
                            // The connection is suspect; drop it so the retry
                            // dials fresh instead of reusing a dead pipe.
                            tracing::warn!(
                                server = %state.config.name,
                                tool = %tool.name,
                                %error,
                                "an MCP tool call failed; reconnecting"
                            );
                            *connection = Connection::Failed;
                        }
                    }
                }
                Connection::Failed => {}
            }
        }

        // Reconnect, then retry once.
        let mut connection = state.connection.lock().await;
        match open(&state.config).await {
            Ok((service, _)) => {
                let service = Arc::new(service);
                let result = call_once(&service, &tool.name, arguments).await;
                *connection = Connection::Live(service);
                result.map_err(|error| error.to_string())
            }
            Err(error) => {
                *connection = Connection::Failed;
                Err(error)
            }
        }
    }

    /// Which server advertises `wire_name`, and the server's own name for it.
    fn resolve(&self, wire_name: &str) -> Option<(usize, McpToolInfo)> {
        self.servers
            .iter()
            .position(|state| state.tools.iter().any(|tool| tool.wire_name == wire_name))
            .and_then(|index| {
                self.servers[index]
                    .tools
                    .iter()
                    .find(|tool| tool.wire_name == wire_name)
                    .map(|tool| (index, tool.clone()))
            })
    }
}

/// What the settings page shows for one server.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerReport {
    pub name: String,
    pub source: crate::config::McpSource,
    pub transport: String,
    pub enabled: bool,
    pub connected: bool,
    pub problem: Option<String>,
    pub tool_count: usize,
}

/// Start one server and list its tools.
async fn open(
    server: &McpServer,
) -> Result<(RunningService<RoleClient, ()>, Vec<McpToolInfo>), String> {
    let service = match &server.transport {
        McpTransport::Stdio { command, args, env } => {
            let mut cmd = tokio::process::Command::new(command);
            cmd.args(args);
            for (key, value) in env {
                cmd.env(key, value);
            }
            let transport = TokioChildProcess::new(cmd)
                .map_err(|error| format!("cannot start `{command}`: {error}"))?;
            ().serve(transport)
                .await
                .map_err(|error| error.to_string())?
        }
        McpTransport::Http { url, headers } => {
            let mut config = StreamableHttpClientTransportConfig::with_uri(url.clone());
            // A remote server usually authenticates with a bearer token; the
            // header map is the only place the user can put one.
            for (name, value) in headers {
                match (
                    http::HeaderName::try_from(name.as_str()),
                    http::HeaderValue::try_from(value.as_str()),
                ) {
                    (Ok(name), Ok(value)) => {
                        config.custom_headers.insert(name, value);
                    }
                    // A header that cannot be encoded is dropped with a log
                    // line rather than failing the whole server: the rest of
                    // its configuration is still what the user meant.
                    _ => tracing::warn!(
                        server = %server.name,
                        header = %name,
                        "ignoring an MCP header that is not valid HTTP"
                    ),
                }
            }
            let transport = rmcp::transport::StreamableHttpClientTransport::from_config(config);
            ().serve(transport)
                .await
                .map_err(|error| error.to_string())?
        }
    };

    let listed = service
        .list_all_tools()
        .await
        .map_err(|error| error.to_string())?;
    let tools = listed
        .into_iter()
        .map(|tool| McpToolInfo {
            server: server.name.clone(),
            wire_name: McpToolInfo::wire_name(&server.name, &tool.name),
            name: tool.name.to_string(),
            description: tool.description.map(|d| d.to_string()).unwrap_or_default(),
            input_schema: serde_json::Value::Object((*tool.input_schema).clone()),
        })
        .collect();
    Ok((service, tools))
}

async fn call_once(
    service: &RunningService<RoleClient, ()>,
    tool: &str,
    arguments: Option<serde_json::Value>,
) -> Result<CallToolResult, ServiceError> {
    // The struct is `#[non_exhaustive]`; its builder is the supported way in.
    let mut params = CallToolRequestParams::new(tool.to_string());
    match arguments {
        Some(serde_json::Value::Object(map)) => params = params.with_arguments(map),
        // A tool with no declared arguments may be called with nothing at all;
        // a non-object argument is wrapped rather than dropped, so the server
        // sees what the model sent instead of an empty call.
        Some(serde_json::Value::Null) | None => {}
        Some(other) => {
            let mut map = serde_json::Map::new();
            map.insert("value".to_owned(), other);
            params = params.with_arguments(map);
        }
    }
    service.call_tool(params).await
}

/// One result's content as text for the model.
///
/// Images and embedded resources are named rather than inlined: the model
/// reads a text channel, and base64 in a tool result costs more context than it
/// can repay.
pub fn result_text(result: &CallToolResult) -> String {
    let mut parts: Vec<String> = result
        .content
        .iter()
        .map(|block| match block {
            ContentBlock::Text(text) => text.text.clone(),
            ContentBlock::Image(image) => format!("[image: {}]", image.mime_type),
            ContentBlock::Audio(audio) => format!("[audio: {}]", audio.mime_type),
            ContentBlock::Resource(resource) => match &resource.resource {
                rmcp::model::ResourceContents::TextResourceContents { uri, .. } => {
                    format!("[resource: {uri}]")
                }
                rmcp::model::ResourceContents::BlobResourceContents { uri, .. } => {
                    format!("[resource: {uri}]")
                }
                _ => "[resource]".to_owned(),
            },
            ContentBlock::ResourceLink(link) => {
                format!("[resource: {}]", link.uri)
            }
            _ => "[content]".to_owned(),
        })
        .collect();
    if parts.is_empty() {
        if let Some(structured) = &result.structured_content {
            parts.push(structured.to_string());
        }
    }
    parts.join("\n")
}

/// Whether a server result reports a tool-level failure.
pub fn result_is_error(result: &CallToolResult) -> bool {
    result.is_error.unwrap_or(false)
}

/// Server names to tool counts, for a compact log line.
pub fn tool_counts(client: &McpClient) -> BTreeMap<String, usize> {
    client
        .servers()
        .into_iter()
        .map(|server| (server.name, server.tool_count))
        .collect()
}
