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
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use rmcp::service::{RoleClient, RunningService, ServiceError};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::config::{McpServer, McpTransport};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CALL_TIMEOUT: Duration = Duration::from_secs(600);

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
    servers: Vec<Arc<ServerState>>,
    tools: Vec<Arc<McpToolInfo>>,
}

struct ServerState {
    config: McpServer,
    cwd: Option<PathBuf>,
    connection: Mutex<Connection>,
    /// Single-flight reconnects without holding the connection state lock.
    reconnect: Mutex<()>,
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
        Self::connect_in(servers, None).await
    }

    pub(crate) async fn connect_in(servers: Vec<McpServer>, cwd: Option<PathBuf>) -> Self {
        Self::connect_reusing(servers, cwd, None).await
    }

    pub(crate) async fn connect_reusing(
        servers: Vec<McpServer>,
        cwd: Option<PathBuf>,
        previous: Option<Arc<Self>>,
    ) -> Self {
        let mut states = Vec::with_capacity(servers.len());
        let mut tools = Vec::new();

        for config in servers {
            if let Some(state) = previous.as_ref().and_then(|client| client.servers.iter().find(|state| {
                state.config == config && state.cwd == cwd && state.problem.is_none()
            })) {
                tools.extend(state.tools.iter().cloned().map(Arc::new));
                states.push(state.clone());
                continue;
            }
            if !config.enabled {
                states.push(Arc::new(ServerState {
                    config,
                    cwd: cwd.clone(),
                    connection: Mutex::new(Connection::Failed),
                    reconnect: Mutex::new(()),
                    tools: Vec::new(),
                    problem: Some("switched off".to_owned()),
                }));
                continue;
            }
            let (connection, listed, problem) = match open(&config, cwd.as_deref()).await {
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
            states.push(Arc::new(ServerState {
                config,
                cwd: cwd.clone(),
                connection: Mutex::new(connection),
                reconnect: Mutex::new(()),
                tools: listed,
                problem,
            }));
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

    pub(crate) fn has_startup_failures(&self) -> bool {
        self.servers.iter().any(|state| state.config.enabled && state.problem.is_some())
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
    /// A failed connection is rebuilt before the next independent call. Once
    /// a request has been sent, its outcome can be uncertain: never replay it.
    pub async fn call(
        &self,
        wire_name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<CallToolResult, String> {
        self.call_with_cancel(wire_name, arguments, &CancellationToken::new())
            .await
    }

    /// Stop the current request and retire its connection on cancellation.
    /// Its effects may already have happened, so it is never replayed.
    pub async fn call_with_cancel(
        &self,
        wire_name: &str,
        arguments: Option<serde_json::Value>,
        cancel: &CancellationToken,
    ) -> Result<CallToolResult, String> {
        self.call_inner(wire_name, arguments, cancel, CALL_TIMEOUT)
            .await
    }

    async fn call_inner(
        &self,
        wire_name: &str,
        arguments: Option<serde_json::Value>,
        cancel: &CancellationToken,
        timeout: Duration,
    ) -> Result<CallToolResult, String> {
        let (server_index, tool) = self
            .resolve(wire_name)
            .ok_or_else(|| format!("no MCP tool named `{wire_name}`"))?;

        let state = &self.servers[server_index];
        let service = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err("cancelled before the MCP call was sent".into()),
            result = tokio::time::timeout(CONNECT_TIMEOUT, state.service()) => {
                result.map_err(|_| "MCP reconnection timed out after 30 seconds".to_owned())??
            }
        };
        // A slow tool must not block other calls or a new run's instructions.
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err("cancelled".to_owned()),
            result = tokio::time::timeout(timeout, call_once(&service, &tool.name, arguments)) => {
                match result {
                    Ok(result) => result.map_err(|error| error.to_string()),
                    Err(_) => Err(format!("tool response timed out after {} seconds", timeout.as_secs_f64())),
                }
            }
        };
        match result {
            Ok(result) => Ok(result),
            Err(error) => {
                let mut connection = state.connection.lock().await;
                if matches!(&*connection, Connection::Live(current) if Arc::ptr_eq(current, &service))
                {
                    service.cancellation_token().cancel();
                    *connection = Connection::Failed;
                }
                Err(format!(
                    "MCP tool `{wire_name}` failed; its outcome may be unknown and the call was not retried: {error}"
                ))
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

impl ServerState {
    async fn service(&self) -> Result<Arc<RunningService<RoleClient, ()>>, String> {
        if let Connection::Live(service) = &*self.connection.lock().await {
            return Ok(service.clone());
        }
        let _reconnect = self.reconnect.lock().await;
        // Another caller may have reconnected while this one was waiting.
        if let Connection::Live(service) = &*self.connection.lock().await {
            return Ok(service.clone());
        }
        let (service, _) = open(&self.config, self.cwd.as_deref()).await?;
        let service = Arc::new(service);
        *self.connection.lock().await = Connection::Live(service.clone());
        Ok(service)
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
    cwd: Option<&Path>,
) -> Result<(RunningService<RoleClient, ()>, Vec<McpToolInfo>), String> {
    open_with_timeout(server, cwd, CONNECT_TIMEOUT).await
}

async fn open_with_timeout(
    server: &McpServer,
    cwd: Option<&Path>,
    timeout: Duration,
) -> Result<(RunningService<RoleClient, ()>, Vec<McpToolInfo>), String> {
    tokio::time::timeout(timeout, open_unbounded(server, cwd))
        .await
        .map_err(|_| {
            format!(
                "MCP server `{}` initialization or tool discovery timed out after {} seconds",
                server.name,
                timeout.as_secs_f64()
            )
        })?
}

async fn open_unbounded(
    server: &McpServer,
    cwd: Option<&Path>,
) -> Result<(RunningService<RoleClient, ()>, Vec<McpToolInfo>), String> {
    let service = match &server.transport {
        McpTransport::Stdio { command, args, env } => {
            let mut cmd = tokio::process::Command::new(command);
            cmd.args(args);
            cmd.kill_on_drop(true);
            if let Some(cwd) = cwd {
                cmd.current_dir(cwd);
            }
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
            // Session recovery may replay the original POST inside the SDK.
            // An expired session must fail this call, not repeat its effect.
            let mut config = StreamableHttpClientTransportConfig::with_uri(url.clone())
                .reinit_on_expired_session(false);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(mode: &str, marker: &Path) -> McpServer {
        let python = ["python", "python3"]
            .into_iter()
            .find(|command| {
                std::process::Command::new(command)
                    .arg("--version")
                    .output()
                    .is_ok_and(|output| output.status.success())
            })
            .expect("MCP fixtures require Python on PATH");
        McpServer {
            name: "fixture".into(),
            enabled: true,
            source: crate::config::McpSource::User,
            transport: McpTransport::Stdio {
                command: python.into(),
                args: vec![
                    "-u".into(),
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/snapshot_server.py")
                        .to_string_lossy()
                        .into_owned(),
                    "test".into(),
                    marker.to_string_lossy().into_owned(),
                    mode.into(),
                    "-".into(),
                ],
                env: BTreeMap::new(),
            },
        }
    }

    #[tokio::test]
    async fn initialization_and_tool_discovery_have_deadlines() {
        let directory = tempfile::tempdir().unwrap();
        for mode in ["held_initialize", "held_list"] {
            let config = fixture(mode, &directory.path().join(mode));
            let result = tokio::time::timeout(
                Duration::from_secs(3),
                open_with_timeout(&config, None, Duration::from_millis(500)),
            )
            .await
            .expect("initialization hung");
            assert!(result.err().unwrap().contains("timed out"));
        }
    }

    #[tokio::test]
    async fn cancellation_and_timeout_retire_the_connection_without_replaying_effects() {
        let directory = tempfile::tempdir().unwrap();
        for cancelled in [true, false] {
            let marker = directory
                .path()
                .join(if cancelled { "cancel" } else { "timeout" });
            let client =
                Arc::new(McpClient::connect(vec![fixture("held_after_commit", &marker)]).await);
            assert_eq!(client.tools().len(), 1, "{:?}", client.servers());
            let cancel = CancellationToken::new();
            let task = {
                let client = client.clone();
                let cancel = cancel.clone();
                tokio::spawn(async move {
                    client
                        .call_inner(
                            "mcp__fixture__record",
                            None,
                            &cancel,
                            Duration::from_millis(500),
                        )
                        .await
                })
            };
            tokio::time::timeout(Duration::from_secs(3), async {
                while !marker.with_extension("ready").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("fixture did not commit its effect");
            if cancelled {
                cancel.cancel();
            }
            let error = tokio::time::timeout(Duration::from_secs(3), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert!(
                error.contains(if cancelled { "cancelled" } else { "timed out" }),
                "{error}"
            );
            assert!(error.contains("not retried"));
            assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 1);
            // A new explicit call reconnects, rather than keeping a hung peer.
            let second = tokio::time::timeout(
                Duration::from_secs(3),
                client.call("mcp__fixture__record", None),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(result_text(&second).contains("\"count\": 2"));
            assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 2);
        }
    }
}
