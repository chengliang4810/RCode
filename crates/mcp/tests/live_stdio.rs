//! A real stdio handshake against a tiny scripted server.
//!
//! The only test that exercises the transport, the initialize handshake and
//! `tools/call` together; the unit tests cover each piece in isolation. Ignored
//! by default because it needs a Python interpreter on the machine — the same
//! reason the live model tests are ignored.

use std::sync::Arc;

use rcode_agent::{Tool, ToolContext};
use rcode_mcp::{McpClient, McpServer, McpSource, McpTool, McpTransport};

fn python() -> Option<String> {
    for candidate in ["python", "python3"] {
        if std::process::Command::new(candidate)
            .arg("--version")
            .output()
            .is_ok()
        {
            return Some(candidate.to_owned());
        }
    }
    None
}

fn script() -> std::path::PathBuf {
    let path = std::env::temp_dir().join("rcode-fake-mcp.py");
    std::fs::write(&path, include_str!("fake_server.py")).unwrap();
    path
}

#[tokio::test]
#[ignore = "needs a Python interpreter; run with --ignored"]
async fn a_stdio_server_lists_and_answers_a_tool_call() {
    let interpreter = python().expect("python on PATH");
    let client = Arc::new(
        McpClient::connect(vec![McpServer {
            name: "fake".to_owned(),
            transport: McpTransport::Stdio {
                command: interpreter,
                args: vec![script().to_string_lossy().into_owned()],
                env: Default::default(),
            },
            enabled: true,
            source: McpSource::User,
        }])
        .await,
    );

    let report = &client.servers()[0];
    assert!(report.connected, "{report:?}");
    assert_eq!(report.tool_count, 1);
    let instructions = client.instructions().await.expect("initialize instructions");
    assert!(instructions.contains("## fake"));
    assert!(instructions.contains("Use the echo tool"));

    let info = client.tools()[0].clone();
    assert_eq!(info.wire_name, "mcp__fake__echo");

    let tool = McpTool::new(info, Arc::clone(&client));
    let result = tool
        .call(
            serde_json::json!({ "text": "hello" }),
            &ToolContext {
                cwd: std::env::temp_dir(),
                cancel: tokio_util::sync::CancellationToken::new(),
                nested: None,
            },
        )
        .await
        .expect("the call runs");

    assert!(!result.is_error);
    assert!(result.output.contains("echo: hello"), "{}", result.output);
}

/// The whole path a run takes: `mcp.json` on disk → loader → connection →
/// tool list. A failure here means a configured server silently contributed
/// no tools, which is the one outcome the user cannot diagnose from the UI.
#[tokio::test]
#[ignore = "needs a Python interpreter; run with --ignored"]
async fn a_configured_server_reaches_the_tool_list_through_the_loader() {
    let config = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let interpreter = python().expect("python on PATH");
    std::fs::write(
        config.path().join("mcp.json"),
        serde_json::json!({
            "mcpServers": {
                "fake": {
                    "command": interpreter,
                    "args": [script().to_string_lossy()],
                }
            }
        })
        .to_string(),
    )
    .unwrap();

    let client = rcode_mcp::connect(project.path(), config.path())
        .await
        .expect("a configured server yields a client");
    let names: Vec<&str> = client
        .tools()
        .iter()
        .map(|tool| tool.wire_name.as_str())
        .collect();
    assert_eq!(names, ["mcp__fake__echo"]);
}

#[tokio::test]
#[ignore = "needs a Python interpreter; run with --ignored"]
async fn an_unreachable_server_is_reported_not_fatal() {
    let client = McpClient::connect(vec![McpServer {
        name: "missing".to_owned(),
        transport: McpTransport::Stdio {
            command: "definitely-not-a-real-command-rcode".to_owned(),
            args: Vec::new(),
            env: Default::default(),
        },
        enabled: true,
        source: McpSource::User,
    }])
    .await;

    let report = &client.servers()[0];
    assert!(!report.connected);
    assert!(report.problem.is_some());
    assert!(client.tools().is_empty());
}
