use super::*;

#[tokio::test]
async fn one_harness_reloads_mcp_for_projects_and_settings_changes() {
    let root = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let script = root.path().join("snapshot_server.py");
    std::fs::write(&script, include_str!("../../mcp/tests/snapshot_server.py")).unwrap();
    let python = ["python", "python3"]
        .into_iter()
        .find(|candidate| {
            std::process::Command::new(candidate)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
        })
        .expect("MCP regression fixture requires Python");
    let configure = |label: &str| {
        std::fs::write(
            root.path().join("mcp.json"),
            serde_json::json!({
                "mcpServers": {"fixture": {"command": python, "args": [
                    "-u", script.to_str().unwrap(), label, "-", "normal", "-"
                ]}}
            })
            .to_string(),
        )
        .unwrap();
    };
    let harness = NativeHarness::from_data_dir(root.path());
    assert!(harness.mcp(a.path()).await.is_none());
    configure("first");
    let first = harness.mcp(a.path()).await.unwrap();
    assert_eq!(first.tools().len(), 1);
    let other = harness.mcp(b.path()).await.unwrap();
    assert!(
        !Arc::ptr_eq(&first, &other),
        "different projects have independent connections"
    );
    configure("second");
    let updated = harness.mcp(a.path()).await.unwrap();
    let tool = rcode_mcp::McpToolInfo::wire_name("fixture", "record");
    let read = |result| {
        serde_json::from_str::<serde_json::Value>(&rcode_mcp::result_text(&result)).unwrap()
    };
    let updated = read(updated.call(&tool, None).await.unwrap());
    let retained = read(first.call(&tool, None).await.unwrap());
    assert_eq!(updated["label"], "second");
    assert_eq!(retained["label"], "first");
    let other = read(other.call(&tool, None).await.unwrap());
    assert_eq!(
        std::path::Path::new(other["cwd"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        b.path().canonicalize().unwrap()
    );
}
