//! MCP servers this device can call tools on.
//!
//! The file is `mcp.json`, shaped the way Claude Code and Cursor spell it, so
//! a config written for another tool can be pasted in unchanged:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "files": {
//!       "command": "npx",
//!       "args": ["-y", "@modelcontextprotocol/server-filesystem", "/work"]
//!     },
//!     "docs": { "url": "https://example.invalid/mcp", "headers": { "Authorization": "Bearer …" } }
//!   }
//! }
//! ```
//!
//! Two transports, and the shape says which: `command` starts a local process
//! and talks over its stdio; `url` dials a streamable-HTTP endpoint. A server
//! that names both, or neither, is refused rather than guessed at — starting
//! the wrong transport fails later and less clearly.
//!
//! Project and user files merge by server name, the project's entry winning for
//! the same reason a project's skill wins: it is the more specific context.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where a server's configuration came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum McpSource {
    /// `<workspace>/.rcode/mcp.json`
    Project,
    /// `<config root>/mcp.json`
    User,
    /// A file an installed plugin contributes. Read-only, and named after the
    /// plugin: the settings page shows it, but the plugin owns it.
    Plugin,
}

impl McpSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::User => "user",
            Self::Plugin => "plugin",
        }
    }
}

/// One configured server, after parsing and validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    pub transport: McpTransport,
    /// Switched off servers stay in the file but never start.
    pub enabled: bool,
    pub source: McpSource,
}

/// How to reach a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpTransport {
    /// A local process, spoken to over stdin/stdout.
    Stdio {
        command: String,
        args: Vec<String>,
        /// Extra environment variables, on top of the process's own.
        env: BTreeMap<String, String>,
    },
    /// A streamable-HTTP endpoint.
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

impl McpTransport {
    /// A short line for the settings page: what this server is, where it lives.
    pub fn describe(&self) -> String {
        match self {
            Self::Stdio { command, args, .. } => {
                if args.is_empty() {
                    command.clone()
                } else {
                    format!("{command} {}", args.join(" "))
                }
            }
            Self::Http { url, .. } => url.clone(),
        }
    }
}

/// Why a file could not be read as configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {error}")]
    Unreadable { path: PathBuf, error: String },
    #[error("{path} is not valid JSON: {error}")]
    Malformed { path: PathBuf, error: String },
    #[error("`{name}`: {problem}")]
    Invalid { name: String, problem: String },
}

/// The on-disk shape.
///
/// `#[serde(default)]` everywhere so a file that only names servers — the
/// common case, and what another tool's config looks like — reads without
/// every field spelled out.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct ConfigFile {
    #[serde(alias = "mcp_servers", alias = "servers")]
    mcp_servers: BTreeMap<String, ServerFile>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct ServerFile {
    command: Option<String>,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    url: Option<String>,
    headers: BTreeMap<String, String>,
    /// Absent means on: the switches exist to turn things off.
    enabled: Option<bool>,
}

/// Validate a settings entry before persisting it, using the runtime's transport parser.
pub fn validate_server(name: &str, value: &serde_json::Value) -> Result<(), ConfigError> {
    let invalid = |problem: String| ConfigError::Invalid {
        name: name.to_owned(),
        problem,
    };
    if name.trim().is_empty() || name.chars().any(char::is_control) {
        return Err(invalid(
            "needs a non-empty server name without control characters".into(),
        ));
    }
    if !value.is_object() {
        return Err(invalid("the server configuration must be an object".into()));
    }
    let entry: ServerFile = serde_json::from_value(value.clone()).map_err(|_| {
        invalid(
            "command/url must be strings, args a string array, and env/headers string maps".into(),
        )
    })?;
    let transport = transport_for(name, entry)?;
    if let Some(kind) = value.get("type") {
        let valid = match &transport {
            McpTransport::Stdio { .. } => kind.as_str() == Some("stdio"),
            McpTransport::Http { .. } => matches!(
                kind.as_str(),
                Some("http" | "streamableHttp" | "streamable-http")
            ),
        };
        if !valid {
            return Err(invalid(
                "supported transports are stdio and streamable HTTP; type must match command/url"
                    .into(),
            ));
        }
    }
    if let McpTransport::Http { url, headers } = transport {
        let uri: http::Uri = url
            .parse()
            .map_err(|_| invalid("needs an absolute HTTP or HTTPS URL".into()))?;
        if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.authority().is_none() {
            return Err(invalid("needs an absolute HTTP or HTTPS URL".into()));
        }
        for (key, value) in headers {
            if key.parse::<http::header::HeaderName>().is_err()
                || value.parse::<http::header::HeaderValue>().is_err()
            {
                return Err(invalid("contains an invalid HTTP header".into()));
            }
        }
    }
    Ok(())
}

/// Every configured server at `cwd`, project entries shadowing the user's.
///
/// A file that does not exist contributes nothing. A file that cannot be parsed
/// is an error rather than an empty list: silently starting with no servers is
/// indistinguishable from "the user configured none", and the two need
/// different fixes.
pub fn load(cwd: &Path, config_root: &Path) -> Result<Vec<McpServer>, ConfigError> {
    load_roots(Some(cwd), config_root)
}

/// Personal settings never read a project's configuration.
pub fn load_personal(config_root: &Path) -> Result<Vec<McpServer>, ConfigError> {
    load_roots(None, config_root)
}

fn load_roots(cwd: Option<&Path>, config_root: &Path) -> Result<Vec<McpServer>, ConfigError> {
    let mut servers: BTreeMap<String, McpServer> = BTreeMap::new();
    // Plugin entries first, then the user's file, then the project's: the more
    // specific the configuration, the later it lands, and the last write for a
    // name wins. A plugin's servers are also namespaced, so an unqualified name
    // always means the user's own entry.
    for plugin in rcode_plugins::enabled(config_root) {
        for path in &plugin.contributions.mcp {
            for mut server in read_file(path, McpSource::Plugin)? {
                server.name = format!("{}:{}", plugin.name, server.name);
                servers.insert(server.name.clone(), server);
            }
        }
    }
    let mut roots = vec![(config_root.join(FILE_NAME), McpSource::User)];
    if let Some(cwd) = cwd {
        roots.push((cwd.join(PROJECT_DIR).join(FILE_NAME), McpSource::Project));
    }
    for (path, source) in roots {
        for server in read_file(&path, source)? {
            servers.insert(server.name.clone(), server);
        }
    }
    Ok(servers.into_values().collect())
}

/// The file name both roots use.
pub const FILE_NAME: &str = "mcp.json";
/// The project-level directory, matching memory and agents.
pub const PROJECT_DIR: &str = ".rcode";

fn read_file(path: &Path, source: McpSource) -> Result<Vec<McpServer>, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ConfigError::Unreadable {
                path: path.to_path_buf(),
                error: error.to_string(),
            });
        }
    };
    let file: ConfigFile = serde_json::from_str(&text).map_err(|error| ConfigError::Malformed {
        path: path.to_path_buf(),
        error: error.to_string(),
    })?;

    let mut servers = Vec::with_capacity(file.mcp_servers.len());
    for (name, entry) in file.mcp_servers {
        let transport = transport_for(&name, entry)?;
        servers.push(McpServer {
            name,
            transport,
            enabled: true,
            source,
        });
    }
    Ok(servers)
}

/// Resolve one entry's transport, refusing the ambiguous shapes.
fn transport_for(name: &str, entry: ServerFile) -> Result<McpTransport, ConfigError> {
    let command = entry.command.filter(|c| !c.trim().is_empty());
    let url = entry.url.filter(|u| !u.trim().is_empty());
    match (command, url) {
        (Some(command), None) => Ok(McpTransport::Stdio {
            command,
            args: entry.args,
            env: entry.env,
        }),
        (None, Some(url)) => Ok(McpTransport::Http {
            url,
            headers: entry.headers,
        }),
        (Some(_), Some(_)) => Err(ConfigError::Invalid {
            name: name.to_owned(),
            problem: "names both `command` and `url`; a server uses one transport".to_owned(),
        }),
        (None, None) => Err(ConfigError::Invalid {
            name: name.to_owned(),
            problem: "needs either `command` (stdio) or `url` (http)".to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, body: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn a_missing_file_configures_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path(), dir.path()).unwrap().is_empty());
    }

    #[test]
    fn a_stdio_server_carries_its_command_args_and_env() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            r#"{"mcpServers":{"files":{
                "command":"npx",
                "args":["-y","server-filesystem","/work"],
                "env":{"TOKEN":"abc"}
            }}}"#,
        );
        let servers = load(dir.path(), dir.path()).unwrap();
        assert_eq!(servers.len(), 1);
        match &servers[0].transport {
            McpTransport::Stdio { command, args, env } => {
                assert_eq!(command, "npx");
                assert_eq!(args, &["-y", "server-filesystem", "/work"]);
                assert_eq!(env.get("TOKEN").map(String::as_str), Some("abc"));
            }
            other => panic!("expected stdio, got {other:?}"),
        }
        assert!(servers[0].enabled, "a server starts on unless turned off");
    }

    #[test]
    fn an_http_server_carries_its_url_and_headers() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            r#"{"mcpServers":{"docs":{"url":"https://example.invalid/mcp",
                "headers":{"Authorization":"Bearer t"}}}}"#,
        );
        let servers = load(dir.path(), dir.path()).unwrap();
        match &servers[0].transport {
            McpTransport::Http { url, headers } => {
                assert_eq!(url, "https://example.invalid/mcp");
                assert_eq!(
                    headers.get("Authorization").map(String::as_str),
                    Some("Bearer t")
                );
            }
            other => panic!("expected http, got {other:?}"),
        }
    }

    #[test]
    fn a_server_naming_both_transports_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            r#"{"mcpServers":{"both":{"command":"x","url":"https://y"}}}"#,
        );
        let error = load(dir.path(), dir.path()).unwrap_err();
        assert!(
            error.to_string().contains("both `command` and `url`"),
            "{error}"
        );
    }

    #[test]
    fn a_server_naming_neither_transport_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), r#"{"mcpServers":{"empty":{}}}"#);
        let error = load(dir.path(), dir.path()).unwrap_err();
        assert!(error.to_string().contains("either `command`"), "{error}");
    }

    #[test]
    fn a_malformed_file_is_an_error_not_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "{ not json");
        let error = load(dir.path(), dir.path()).unwrap_err();
        assert!(matches!(error, ConfigError::Malformed { .. }), "{error:?}");
    }

    #[test]
    fn the_project_entry_wins_over_the_users() {
        let config = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write(
            config.path(),
            r#"{"mcpServers":{"files":{"command":"user-cmd"},"docs":{"command":"docs"}}}"#,
        );
        write(
            &project.path().join(PROJECT_DIR),
            r#"{"mcpServers":{"files":{"command":"project-cmd"}}}"#,
        );

        let servers = load(project.path(), config.path()).unwrap();
        let files = servers.iter().find(|s| s.name == "files").unwrap();
        match &files.transport {
            McpTransport::Stdio { command, .. } => assert_eq!(command, "project-cmd"),
            other => panic!("{other:?}"),
        }
        assert_eq!(files.source, McpSource::Project);
        // The user's unshadowed entry survives.
        let docs = servers.iter().find(|s| s.name == "docs").unwrap();
        assert_eq!(docs.source, McpSource::User);
    }

    #[test]
    fn other_tools_spellings_are_accepted() {
        // `mcp_servers` and a bare `servers` map both appear in the wild.
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), r#"{"mcp_servers":{"a":{"command":"x"}}}"#);
        assert_eq!(load(dir.path(), dir.path()).unwrap().len(), 1);
    }
}
