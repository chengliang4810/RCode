//! These are correctness regressions, not probes that expect the old bugs.
//! Python is available on the CI runners; no provider or remote MCP is used.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rcode_mcp::{McpClient, McpClientPool, McpServer, McpSource, McpToolInfo, McpTransport};
use serde_json::{Value, json};

fn python() -> String {
    ["python", "python3"]
        .into_iter()
        .find(|candidate| {
            std::process::Command::new(candidate)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
        })
        .expect("MCP regression fixtures require Python on PATH")
        .to_owned()
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshot_server.py")
}

fn configuration(
    script: &Path,
    label: &str,
    marker: Option<&Path>,
    mode: &str,
    starts: Option<&Path>,
) -> Value {
    json!({"mcpServers": {"fixture": {"command": python(), "args": [
        "-u", script.to_str().unwrap(), label,
        marker.map_or("-", |path| path.to_str().unwrap()), mode,
        starts.map_or("-", |path| path.to_str().unwrap())
    ]}}})
}

fn write_config(root: &Path, config: &Value) {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join("mcp.json"), config.to_string()).unwrap();
}

async fn acquire(pool: &McpClientPool, cwd: &Path, root: &Path) -> Arc<McpClient> {
    let client = tokio::time::timeout(Duration::from_secs(10), pool.client(cwd, root))
        .await
        .expect("local initialize completes")
        .expect("configured client");
    assert_eq!(client.tools().len(), 1, "{:?}", client.servers());
    client
}

async fn call(client: &McpClient) -> Result<Value, String> {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        client.call(
            &McpToolInfo::wire_name("fixture", "record"),
            Some(json!({})),
        ),
    )
    .await
    .expect("local tool settles")?;
    serde_json::from_str(&rcode_mcp::result_text(&result)).map_err(|error| error.to_string())
}

fn lines(path: &Path) -> usize {
    std::fs::read_to_string(path).unwrap().lines().count()
}

fn catalog_server(name: &str, label: &str, tools: &[&str], marker: &Path) -> McpServer {
    McpServer {
        name: name.into(),
        enabled: true,
        source: McpSource::User,
        transport: McpTransport::Stdio {
            command: python(),
            args: vec![
                "-u".into(),
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/catalog_server.py")
                    .to_string_lossy()
                    .into_owned(),
                label.into(),
                marker.to_string_lossy().into_owned(),
                serde_json::to_string(tools).unwrap(),
            ],
            env: Default::default(),
        },
    }
}

#[tokio::test]
async fn ambiguous_wire_names_are_rejected_and_unique_tools_keep_their_schema_route() {
    for (server_a, tool_a, server_b, tool_b) in [
        ("a.b", "record", "a_b", "record"),
        ("kit:docs", "record", "plugin_kit_docs", "record"),
        ("a__b", "record", "a", "b__record"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let marker_a = root.path().join("a.calls");
        let marker_b = root.path().join("b.calls");
        let client = McpClient::connect(vec![
            catalog_server(server_a, "SERVER_A", &[tool_a, "only_a"], &marker_a),
            catalog_server(server_b, "SERVER_B", &[tool_b, "only_b"], &marker_b),
        ])
        .await;
        let ambiguous = McpToolInfo::wire_name(server_a, tool_a);
        assert_eq!(ambiguous, McpToolInfo::wire_name(server_b, tool_b));
        assert_eq!(client.tools().len(), 2, "{:?}", client.servers());
        assert!(
            client
                .tools()
                .iter()
                .all(|info| info.wire_name != ambiguous)
        );
        assert!(
            client
                .call(&ambiguous, Some(json!({"label": "SERVER_B"})))
                .await
                .is_err()
        );
        assert!(
            !marker_a.exists() && !marker_b.exists(),
            "ambiguous tools must never execute"
        );
        for report in client.servers() {
            assert!(report.connected, "{report:?}");
            assert_eq!(report.tool_count, 1);
            assert!(
                report
                    .problem
                    .as_deref()
                    .is_some_and(|problem| problem.contains("ambiguous"))
            );
        }
        for info in client.tools() {
            let label = info.input_schema["properties"]["label"]["const"]
                .as_str()
                .unwrap();
            assert_eq!(info.description, label);
            let result = client
                .call(&info.wire_name, Some(json!({"label": label})))
                .await
                .unwrap();
            let executed: Value = serde_json::from_str(&rcode_mcp::result_text(&result)).unwrap();
            assert_eq!(
                executed["label"], label,
                "schema and execution must have one owner"
            );
            assert_eq!(executed["name"], info.name);
        }
        assert_eq!(lines(&marker_a), 1);
        assert_eq!(lines(&marker_b), 1);
    }
}

#[tokio::test]
async fn tool_name_sanitization_collisions_are_all_rejected_within_one_server() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("calls");
    let client = McpClient::connect(vec![catalog_server(
        "fixture",
        "SERVER",
        &["read.file", "read_file", "safe"],
        &marker,
    )])
    .await;
    assert_eq!(client.tools().len(), 1, "{:?}", client.servers());
    assert_eq!(client.tools()[0].name, "safe");
    assert!(
        client
            .call("mcp__fixture__read_file", Some(json!({"label": "SERVER"})))
            .await
            .is_err()
    );
    assert!(!marker.exists());
}

#[tokio::test]
async fn configuration_changes_take_effect_without_changing_an_existing_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let pool = McpClientPool::default();
    assert!(pool.client(project.path(), root.path()).await.is_none());
    write_config(
        root.path(),
        &configuration(&fixture(), "old", None, "normal", None),
    );
    let old = acquire(&pool, project.path(), root.path()).await;
    let reused = acquire(&pool, project.path(), root.path()).await;
    assert!(
        Arc::ptr_eq(&old, &reused),
        "unchanged configuration reuses its connection"
    );
    // Same-length edit: file size or modification-time heuristics are insufficient.
    write_config(
        root.path(),
        &configuration(&fixture(), "new", None, "normal", None),
    );
    let new = acquire(&pool, project.path(), root.path()).await;
    assert!(!Arc::ptr_eq(&old, &new));
    assert_eq!(call(&new).await.unwrap()["label"], "new");
    assert_eq!(
        call(&old).await.unwrap()["label"],
        "old",
        "active runs retain their snapshot"
    );
    write_config(root.path(), &json!({"mcpServers": {}}));
    assert!(pool.client(project.path(), root.path()).await.is_none());
}

#[tokio::test]
async fn projects_have_independent_configuration_and_execution_directories() {
    let root = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    write_config(
        root.path(),
        &configuration(&fixture(), "personal", None, "normal", None),
    );
    write_config(
        &a.path().join(".rcode"),
        &configuration(&fixture(), "project-a", None, "normal", None),
    );
    let pool = McpClientPool::default();
    let first = acquire(&pool, a.path(), root.path()).await;
    let second = acquire(&pool, b.path(), root.path()).await;
    assert!(!Arc::ptr_eq(&first, &second));
    let first_result = call(&first).await.unwrap();
    let second_result = call(&second).await.unwrap();
    assert_eq!(first_result["label"], "project-a");
    assert_eq!(second_result["label"], "personal");
    assert_eq!(
        Path::new(first_result["cwd"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        a.path().canonicalize().unwrap()
    );
    assert_eq!(
        Path::new(second_result["cwd"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        b.path().canonicalize().unwrap()
    );
    assert!(Arc::ptr_eq(
        &first,
        &acquire(&pool, &a.path().join("."), root.path()).await
    ));
}

#[tokio::test]
async fn concurrent_runs_share_one_handshake_for_the_same_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let starts = root.path().join("starts");
    write_config(
        root.path(),
        &configuration(&fixture(), "shared", None, "normal", Some(&starts)),
    );
    let pool = McpClientPool::default();
    let (a, b, c) = tokio::join!(
        acquire(&pool, project.path(), root.path()),
        acquire(&pool, project.path(), root.path()),
        acquire(&pool, project.path(), root.path()),
    );
    assert!(Arc::ptr_eq(&a, &b) && Arc::ptr_eq(&b, &c));
    assert_eq!(lines(&starts), 1);
}

#[tokio::test]
async fn a_failed_handshake_does_not_hide_a_server_that_recovers() {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let script = root.path().join("initially-missing.py");
    write_config(
        root.path(),
        &configuration(&script, "recovered", None, "normal", None),
    );
    let pool = McpClientPool::default();
    let failed = tokio::time::timeout(
        Duration::from_secs(5),
        pool.client(project.path(), root.path()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(failed.tools().is_empty());
    std::fs::write(script, include_str!("snapshot_server.py")).unwrap();
    let recovered = acquire(&pool, project.path(), root.path()).await;
    assert_eq!(call(&recovered).await.unwrap()["label"], "recovered");
}

#[tokio::test]
async fn malformed_or_disabled_configuration_does_not_reuse_stale_tools() {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let pool = McpClientPool::default();
    let mut config = configuration(&fixture(), "valid", None, "normal", None);
    write_config(root.path(), &config);
    let active = acquire(&pool, project.path(), root.path()).await;
    std::fs::write(root.path().join("mcp.json"), "{broken").unwrap();
    assert!(pool.client(project.path(), root.path()).await.is_none());
    config["mcpServers"]["fixture"]["enabled"] = json!(false);
    write_config(root.path(), &config);
    assert!(pool.client(project.path(), root.path()).await.is_none());
    assert_eq!(call(&active).await.unwrap()["label"], "valid");
}

#[tokio::test]
async fn retrying_an_offline_server_preserves_healthy_server_sessions() {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let starts = root.path().join("healthy.starts");
    let missing = root.path().join("recoverable.py");
    let mut config = configuration(&fixture(), "healthy", None, "normal", Some(&starts));
    config["mcpServers"]["other"] =
        configuration(&missing, "other", None, "normal", None)["mcpServers"]["fixture"].clone();
    write_config(root.path(), &config);
    let pool = McpClientPool::default();
    let first = acquire(&pool, project.path(), root.path()).await;
    let retry = acquire(&pool, project.path(), root.path()).await;
    assert_eq!(
        lines(&starts),
        1,
        "an unavailable server must not restart a healthy peer"
    );
    std::fs::write(missing, include_str!("snapshot_server.py")).unwrap();
    let recovered = tokio::time::timeout(
        Duration::from_secs(10),
        pool.client(project.path(), root.path()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(recovered.tools().len(), 2);
    assert_eq!(
        first.tools().len(),
        1,
        "old run's catalog remains a snapshot"
    );
    assert_eq!(retry.tools().len(), 1);
    assert_eq!(lines(&starts), 1);
    assert_eq!(call(&first).await.unwrap()["label"], "healthy");
    assert_eq!(call(&recovered).await.unwrap()["label"], "healthy");
}

#[tokio::test]
async fn a_slow_tool_does_not_block_another_runs_instructions() {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let release = root.path().join("release");
    write_config(
        root.path(),
        &configuration(&fixture(), "slow", Some(&release), "held_call", None),
    );
    let pool = McpClientPool::default();
    let client = acquire(&pool, project.path(), root.path()).await;
    let pending = {
        let client = client.clone();
        tokio::spawn(async move { call(&client).await })
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !release.with_extension("ready").exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "server did not receive the tool call"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let instructions = tokio::time::timeout(Duration::from_secs(1), client.instructions())
        .await
        .expect("tool call held the connection lock")
        .unwrap();
    assert!(instructions.contains("slow"));
    let report = {
        let client = client.clone();
        tokio::time::timeout(
            Duration::from_secs(1),
            tokio::task::spawn_blocking(move || client.servers()),
        )
        .await
        .expect("a slow call blocked server reports")
        .unwrap()
    };
    assert!(report[0].connected, "{report:?}");
    std::fs::write(release, "release").unwrap();
    assert!(pending.await.unwrap().is_ok());
}

async fn no_replay(mode: &str) {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let marker = root.path().join("effects");
    let starts = root.path().join("starts");
    write_config(
        root.path(),
        &configuration(&fixture(), mode, Some(&marker), mode, Some(&starts)),
    );
    let pool = McpClientPool::default();
    let client = acquire(&pool, project.path(), root.path()).await;
    assert!(client.servers()[0].connected);
    let error = call(&client).await.unwrap_err();
    assert!(error.contains("not retried"), "{error}");
    let report = &client.servers()[0];
    assert!(!report.connected, "{report:?}");
    assert!(report.problem.is_some(), "{report:?}");
    assert_eq!(
        lines(&marker),
        1,
        "one public call must commit at most one effect"
    );
    assert_eq!(
        lines(&starts),
        1,
        "the failed call itself must not reconnect"
    );
    // This is a new explicit public call. It may reconnect and execute once.
    let result = call(&client).await.unwrap();
    assert_eq!(result["count"], 2);
    let report = &client.servers()[0];
    assert!(report.connected, "{report:?}");
    assert!(report.problem.is_none(), "{report:?}");
    assert_eq!(lines(&marker), 2);
    assert_eq!(lines(&starts), 2);
}

#[tokio::test]
async fn an_rpc_error_after_commit_is_not_automatically_replayed() {
    no_replay("error_once").await;
}

#[tokio::test]
async fn a_disconnect_after_commit_is_not_automatically_replayed() {
    no_replay("disconnect_once").await;
}

#[tokio::test]
async fn failed_reconnect_updates_health_and_successful_reconnect_clears_the_error() {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let script = root.path().join("server.py");
    let marker = root.path().join("effects");
    let source = include_str!("snapshot_server.py");
    std::fs::write(&script, source).unwrap();
    write_config(
        root.path(),
        &configuration(&script, "health", Some(&marker), "error_once", None),
    );
    let client = acquire(&McpClientPool::default(), project.path(), root.path()).await;
    assert!(
        call(&client)
            .await
            .unwrap_err()
            .contains("Action committed")
    );
    let first_problem = client.servers()[0].problem.clone().unwrap();
    std::fs::write(
        &script,
        "raise SystemExit('reconnect fixture unavailable')\n",
    )
    .unwrap();
    assert!(call(&client).await.is_err());
    let report = &client.servers()[0];
    assert!(!report.connected, "{report:?}");
    assert!(report.problem.is_some(), "{report:?}");
    assert_ne!(
        report.problem.as_ref().unwrap(),
        &first_problem,
        "report must show the latest failure"
    );
    std::fs::write(&script, source).unwrap();
    assert_eq!(call(&client).await.unwrap()["count"], 2);
    let report = &client.servers()[0];
    assert!(report.connected, "{report:?}");
    assert!(report.problem.is_none(), "{report:?}");
    assert_eq!(
        lines(&marker),
        2,
        "a reconnect failure must not execute the tool"
    );
}

#[tokio::test]
async fn reports_remain_responsive_during_reconnect_and_show_its_cancellation() {
    let root = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let script = root.path().join("server.py");
    let marker = root.path().join("effects");
    let starts = root.path().join("starts");
    let source = include_str!("snapshot_server.py");
    std::fs::write(&script, source).unwrap();
    write_config(
        root.path(),
        &configuration(
            &script,
            "health",
            Some(&marker),
            "error_once",
            Some(&starts),
        ),
    );
    let client = acquire(&McpClientPool::default(), project.path(), root.path()).await;
    assert!(call(&client).await.is_err());
    std::fs::write(
        &script,
        source.replace("if mode == \"held_initialize\":", "if True:"),
    )
    .unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    let pending = {
        let client = client.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            client
                .call_with_cancel("mcp__fixture__record", None, &cancel)
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        while lines(&starts) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("reconnect did not start");
    let report = {
        let client = client.clone();
        tokio::time::timeout(
            Duration::from_secs(1),
            tokio::task::spawn_blocking(move || client.servers()),
        )
        .await
        .expect("reconnect blocked server reports")
        .unwrap()
    };
    assert!(!report[0].connected, "{report:?}");
    cancel.cancel();
    assert!(
        tokio::time::timeout(Duration::from_secs(3), pending)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .contains("cancelled before")
    );
    let report = &client.servers()[0];
    assert!(!report.connected, "{report:?}");
    assert!(
        report
            .problem
            .as_deref()
            .unwrap()
            .contains("cancelled before")
    );
    std::fs::write(&script, source).unwrap();
    assert_eq!(call(&client).await.unwrap()["count"], 2);
    assert!(client.servers()[0].connected);
    assert!(client.servers()[0].problem.is_none());
}
