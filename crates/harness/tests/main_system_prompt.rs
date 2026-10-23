//! Verify the production main loop's prompt at the HTTP boundary, without a model service.

use std::path::Path;
use std::time::Duration;

use futures::StreamExt;
use rcode_harness::native::NativeHarness;
use rcode_harness::prompts::IDENTITY;
use rcode_harness::providers::{Protocol, ProvidersConfig};
use rcode_harness::{CancellationToken, Harness, RunControls, TurnControl};
use rcode_model::prompt_cache::SYSTEM_PROMPT_DYNAMIC_BOUNDARY;
use rcode_proto::{AgentEvent, DoneStatus, RunRequest};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[path = "support/context_reminders.rs"]
mod context_reminders;

fn config(protocol: Protocol, endpoint: &str) -> ProvidersConfig {
    serde_json::from_value(json!({
        "providers": [{
            "id": "fixture", "label": "Fixture", "protocol": protocol,
            "baseUrl": endpoint, "apiKey": "fixture-key",
            "models": [{"id": "test-choice", "model": "vendor-model", "maxTokens": 1024}]
        }],
        "defaultModel": "test-choice"
    }))
    .unwrap()
}

fn response(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenAiChat => concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        ),
        Protocol::OpenAiResponses => concat!(
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n"
        ),
        Protocol::AnthropicMessages => concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"fixture\",\"role\":\"assistant\",\"content\":[]}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        ),
    }
}

/// Exports credential-free production wire requests for an explicitly invoked live study.
#[tokio::test]
#[ignore = "set RCODE_CACHE_STUDY_DIR to export local mock-server captures; no live API calls"]
async fn export_prompt_cache_probe_requests() {
    let destination = std::path::PathBuf::from(
        std::env::var_os("RCODE_CACHE_STUDY_DIR").expect("RCODE_CACHE_STUDY_DIR is required"),
    );
    assert!(destination.is_absolute());
    std::fs::create_dir_all(&destination).unwrap();
    for (name, protocol) in [
        ("chat", Protocol::OpenAiChat),
        ("messages", Protocol::AnthropicMessages),
        ("responses", Protocol::OpenAiResponses),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let harness = NativeHarness::new(config(protocol, &endpoint));
        let project = tempfile::tempdir().unwrap();
        let (body, _) = capture_turn_with_prompt(
            &harness, &listener, project.path(), protocol, None, Default::default(),
            "This is an API compatibility probe. Reply only with OK and do not call tools.",
        ).await;
        std::fs::write(destination.join(format!("rcode-{name}.json")), serde_json::to_vec_pretty(&body).unwrap()).unwrap();
    }
}

async fn receive_request(listener: &TcpListener, protocol: Protocol) -> Value {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut request = Vec::new();
    let mut buffer = [0; 4096];
    let body = loop {
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0, "request ended before its body");
        request.extend_from_slice(&buffer[..count]);
        assert!(request.len() < 1024 * 1024, "fixture request is too large");
        let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..end]);
        let length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap();
        if request.len() >= end + 4 + length {
            break serde_json::from_slice(&request[end + 4..end + 4 + length]).unwrap();
        }
    };
    let response = response(protocol);
    socket.write_all(format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
        response.len()
    ).as_bytes()).await.unwrap();
    body
}

async fn capture_turn(
    harness: &NativeHarness,
    listener: &TcpListener,
    cwd: &Path,
    protocol: Protocol,
    resume: Option<String>,
    options: serde_json::Map<String, Value>,
) -> (Value, String) {
    capture_turn_with_prompt(harness, listener, cwd, protocol, resume, options, "hello").await
}

async fn capture_turn_with_prompt(
    harness: &NativeHarness,
    listener: &TcpListener,
    cwd: &Path,
    protocol: Protocol,
    resume: Option<String>,
    options: serde_json::Map<String, Value>,
    prompt: &str,
) -> (Value, String) {
    let run = async {
        let (_steer_tx, steering) = tokio::sync::mpsc::channel(1);
        let controls = RunControls {
            conversation_id: None,
            goal: None,
            request_input: Box::new(|_| panic!("fixture must not ask questions")),
            steering,
            interrupt: CancellationToken::new(),
            turn: TurnControl::default(),
            history: None,
        };
        let request = RunRequest {
            prompt: prompt.into(),
            harness: None,
            model: None,
            reasoning: None,
            model_options: options,
            cwd: cwd.to_string_lossy().into_owned(),
            resume,
            attachments: Vec::new(),
            worktree: None,
        };
        let events: Vec<_> = harness
            .run(request, controls)
            .await
            .unwrap()
            .collect()
            .await;
        assert!(
            events.iter().any(|event| matches!(
                event,
                Ok(AgentEvent::Done {
                    status: DoneStatus::Completed,
                    ..
                })
            )),
            "{events:?}"
        );
        events
            .iter()
            .find_map(|event| match event {
                Ok(AgentEvent::SessionStarted { session_id, .. }) => Some(session_id.clone()),
                _ => None,
            })
            .unwrap()
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        let (body, session_id) = tokio::join!(receive_request(listener, protocol), run);
        (body, session_id)
    })
    .await
    .expect("fixture completes within its deadline")
}

fn system_text(body: &Value, protocol: Protocol) -> String {
    assert!(!body.to_string().contains(SYSTEM_PROMPT_DYNAMIC_BOUNDARY));
    assert!(!body.to_string().contains("x-rcode-client:"));
    match protocol {
        Protocol::OpenAiChat => {
            let messages = body["messages"].as_array().unwrap();
            assert_eq!(messages[0]["role"], "system");
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["role"] == "system")
                    .count(),
                1
            );
            messages[0]["content"].as_str().unwrap().to_owned()
        }
        Protocol::OpenAiResponses => {
            assert!(
                body["input"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|item| item["role"] != "system")
            );
            body["instructions"].as_str().unwrap().to_owned()
        }
        Protocol::AnthropicMessages => {
            let blocks = body["system"].as_array().unwrap();
            assert!((2..=4).contains(&blocks.len()));
            assert_eq!(blocks[0]["text"], IDENTITY);
            if blocks[1]["cache_control"]["scope"] == "global" {
                assert!(blocks[0].get("cache_control").is_none());
                assert_eq!(blocks[2]["cache_control"]["type"], "ephemeral");
                if let Some(dynamic) = blocks.get(3) {
                    assert!(dynamic.get("cache_control").is_none());
                }
            } else {
                assert_eq!(blocks[0]["cache_control"]["type"], "ephemeral");
                assert_eq!(blocks[1]["cache_control"]["type"], "ephemeral");
                if let Some(dynamic) = blocks.get(2) {
                    assert!(dynamic.get("cache_control").is_none());
                }
            }
            blocks
                .iter()
                .map(|block| block["text"].as_str().unwrap())
                .collect::<Vec<_>>()
                .join("\n\n")
        }
    }
}

async fn assert_settings_refresh(protocol: Protocol) {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    config(protocol, &endpoint).save(data.path()).unwrap();
    let memory = rcode_agent::memory::directory(project.path(), data.path());
    std::fs::create_dir_all(&memory).unwrap();
    std::fs::write(
        memory.join("MEMORY.md"),
        "- [Preference](preference.md) — Index memory fixture.",
    )
    .unwrap();
    std::fs::write(
        memory.join("preference.md"),
        "Fact body must be read on demand.",
    )
    .unwrap();
    std::fs::create_dir_all(project.path().join(".rcode/memory")).unwrap();
    std::fs::write(
        project.path().join(".rcode/memory/project.md"),
        "Project memory fixture.",
    )
    .unwrap();
    let harness = NativeHarness::from_data_dir(data.path());

    let mut resume = None;
    for (turn, enabled) in [false, true, true, false].into_iter().enumerate() {
        if turn == 2 {
            std::fs::write(
                memory.join("MEMORY.md"),
                "- [Preference](preference.md) — Updated memory fixture.",
            )
            .unwrap();
        }
        std::fs::write(
            data.path().join("agent-settings.json"),
            json!({
                "memory": enabled, "subagents": enabled, "mcp": false
            })
            .to_string(),
        )
        .unwrap();
        let (body, id) = capture_turn(
            &harness,
            &listener,
            project.path(),
            protocol,
            resume.clone(),
            Default::default(),
        )
        .await;
        if let Some(previous) = &resume {
            assert_eq!(&id, previous);
        }
        resume = Some(id);
        let system = system_text(&body, protocol);
        assert!(system.contains(IDENTITY));
        assert!(system.contains("# Doing tasks"));
        assert!(system.contains("# Formatting rules"));
        assert!(system.contains("# Environment"));
        assert_eq!(system.contains("# Windows shell"), cfg!(windows));
        if cfg!(windows) {
            assert!(system.contains("the `Bash` tool executes PowerShell"));
        }
        for excluded in [
            "# Using your tools",
            "# Session-specific guidance",
            "# Tool results",
            "# Language",
            "# Output Style",
            "Model ID:",
            "gitStatus:",
            "Recent commits:",
        ] {
            assert!(
                !system.contains(excluded),
                "unexpected base content: {excluded}"
            );
        }
        assert!(!system.contains("fixture-key"));
        assert_eq!(system.contains("# Auto memory"), enabled);
        assert_eq!(
            system.contains("Index memory fixture."),
            enabled && turn == 1
        );
        assert_eq!(
            system.contains("Updated memory fixture."),
            enabled && turn == 2
        );
        assert_eq!(system.contains(memory.to_string_lossy().as_ref()), enabled);
        assert!(!system.contains("Fact body must be read on demand."));
        assert!(!system.contains("Project memory fixture."));
        assert!(!system.contains("MemoryWrite"));
        assert!(!system.contains("# Team memory"));
        let tool_names: Vec<_> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| {
                tool["name"]
                    .as_str()
                    .or_else(|| tool["function"]["name"].as_str())
                    .unwrap()
            })
            .collect();
        // 内置子智能体没有总启停开关；memory 开关只控制记忆上下文。
        assert!(tool_names.contains(&"Agent"));
        assert!(!tool_names.contains(&"Task"));
        assert!(!tool_names.contains(&"MemoryWrite"));
        for name in ["Read", "Write", "Edit", "Glob", "Grep"] {
            assert!(tool_names.contains(&name));
        }
        assert!(!system.contains("SearchExtraTools"));
    }
}

#[tokio::test]
async fn disabled_memory_does_not_create_a_project_memory_directory() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    config(Protocol::OpenAiChat, &endpoint)
        .save(data.path())
        .unwrap();
    std::fs::write(
        data.path().join("agent-settings.json"),
        r#"{"memory":false,"subagents":false,"mcp":false}"#,
    )
    .unwrap();
    let harness = NativeHarness::from_data_dir(data.path());
    let (body, _) = capture_turn(
        &harness,
        &listener,
        project.path(),
        Protocol::OpenAiChat,
        None,
        Default::default(),
    )
    .await;
    assert!(!system_text(&body, Protocol::OpenAiChat).contains("# Auto memory"));
    assert!(!data.path().join("projects").exists());
}

#[tokio::test]
async fn chat_completions_receives_main_prompt_and_live_capabilities() {
    assert_settings_refresh(Protocol::OpenAiChat).await;
}

#[tokio::test]
async fn responses_receives_main_prompt_as_instructions() {
    assert_settings_refresh(Protocol::OpenAiResponses).await;
}

#[tokio::test]
async fn anthropic_uses_the_default_organization_cache_blocks() {
    assert_settings_refresh(Protocol::AnthropicMessages).await;
}

#[tokio::test]
async fn bare_harness_also_receives_the_main_prompt() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let harness = NativeHarness::new(config(Protocol::OpenAiChat, &endpoint));
    let project = tempfile::tempdir().unwrap();
    let (body, _) = capture_turn(
        &harness,
        &listener,
        project.path(),
        Protocol::OpenAiChat,
        None,
        Default::default(),
    )
    .await;
    assert!(system_text(&body, Protocol::OpenAiChat).contains(IDENTITY));
}

fn cache_markers(value: &Value) -> usize {
    match value {
        Value::Object(map) => {
            usize::from(map.contains_key("cache_control"))
                + map.values().map(cache_markers).sum::<usize>()
        }
        Value::Array(values) => values.iter().map(cache_markers).sum(),
        _ => 0,
    }
}

#[tokio::test]
async fn restored_sessions_keep_the_system_prefix_when_user_input_and_agents_change() {
    for protocol in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
    ] {
        let project = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        config(protocol, &endpoint).save(data.path()).unwrap();
        std::fs::write(
            data.path().join("agent-settings.json"),
            r#"{"memory":false,"subagents":false,"mcp":false}"#,
        )
        .unwrap();
        std::fs::write(project.path().join("AGENTS.md"), "FROZEN AGENTS POLICY").unwrap();
        std::fs::create_dir_all(project.path().join("src")).unwrap();
        std::fs::write(project.path().join("src/AGENTS.md"), "CHILD MUST NOT LOAD").unwrap();
        std::fs::create_dir_all(data.path().join("policy")).unwrap();
        std::fs::write(data.path().join("policy/AGENTS.md"), "POLICY MUST NOT LOAD").unwrap();
        std::fs::write(
            data.path().join("AGENTS.md"),
            "DATA DIRECTORY POLICY MUST NOT LOAD",
        )
        .unwrap();
        std::fs::write(project.path().join("CLAUDE.md"), "CLAUDE MUST NOT LOAD").unwrap();
        let first_host = NativeHarness::from_data_dir(data.path());
        let (first, id) = capture_turn_with_prompt(
            &first_host,
            &listener,
            project.path(),
            protocol,
            None,
            Default::default(),
            "USER INPUT ONE",
        )
        .await;
        std::fs::write(project.path().join("AGENTS.md"), "NEW AGENTS POLICY").unwrap();
        std::fs::write(data.path().join("AGENTS.md"), "NEW USER POLICY").unwrap();
        let restored_host = NativeHarness::from_data_dir(data.path());
        restored_host.reset_prompt_cache(&id).await;
        let (restored, _) = capture_turn_with_prompt(
            &restored_host,
            &listener,
            project.path(),
            protocol,
            Some(id),
            Default::default(),
            "USER INPUT TWO",
        )
        .await;
        let prefix = system_text(&first, protocol);
        assert_eq!(prefix, system_text(&restored, protocol));
        assert!(prefix.contains("FROZEN AGENTS POLICY"));
        assert!(prefix.contains("Today's date"));
        assert!(!prefix.contains("NEW AGENTS POLICY"));
        assert!(!prefix.contains("NEW USER POLICY"));
        assert!(!prefix.contains("CLAUDE MUST NOT LOAD"));
        assert!(!prefix.contains("CHILD MUST NOT LOAD"));
        assert!(!prefix.contains("POLICY MUST NOT LOAD"));
        assert!(!prefix.contains("DATA DIRECTORY POLICY MUST NOT LOAD"));
        assert!(!prefix.contains("<project-instructions>"));
        assert!(!prefix.contains("Instructions from"));
        assert!(first.to_string().contains("USER INPUT ONE"));
        assert!(restored.to_string().contains("USER INPUT TWO"));
        let (fresh, _) = capture_turn(
            &restored_host,
            &listener,
            project.path(),
            protocol,
            None,
            Default::default(),
        )
        .await;
        assert!(system_text(&fresh, protocol).contains("NEW AGENTS POLICY"));
        assert!(!system_text(&fresh, protocol).contains("NEW USER POLICY"));
    }
}

#[tokio::test]
async fn project_context_cache_reset_and_all_cache_modes_reach_the_wire() {
    let protocol = Protocol::AnthropicMessages;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let harness = NativeHarness::new(config(protocol, &endpoint));
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("AGENTS.md"), "INITIAL PROJECT POLICY").unwrap();
    let mut resume = None;
    for (index, mode) in ["organization", "global", "tool-based"]
        .into_iter()
        .enumerate()
    {
        let options = serde_json::from_value(json!({"prompt": {"cacheMode": mode}})).unwrap();
        let (body, id) = capture_turn(
            &harness,
            &listener,
            project.path(),
            protocol,
            resume,
            options,
        )
        .await;
        assert!(system_text(&body, protocol).contains("# Doing tasks"));
        assert!(
            cache_markers(&body) <= 4,
            "too many Anthropic cache breakpoints: {body}"
        );
        assert_eq!(
            body["system"].as_array().unwrap().len(),
            (if index == 1 { 3 } else { 2 }) + usize::from(cfg!(windows))
        );
        if cfg!(windows) {
            let dynamic = body["system"].as_array().unwrap().last().unwrap();
            assert!(dynamic["text"].as_str().unwrap().contains("# Windows shell"));
            assert!(dynamic.get("cache_control").is_none());
        }
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "user");
        assert!(!system_text(&body, protocol).contains("<project-instructions>"));
        assert!(!system_text(&body, protocol).contains("Instructions from"));
        assert!(system_text(&body, protocol).contains("INITIAL PROJECT POLICY"));
        assert!(system_text(&body, protocol).contains("Today's date"));
        assert!(
            !body["messages"]
                .to_string()
                .contains("INITIAL PROJECT POLICY")
        );
        assert!(!body["messages"].to_string().contains("Today's date"));
        resume = Some(id);
    }
    let options = serde_json::from_value(json!({"prompt": {"cacheMode": "tool-based"}})).unwrap();
    std::fs::write(project.path().join("AGENTS.md"), "CHANGED PROJECT POLICY").unwrap();
    let (cached, id) = capture_turn(
        &harness,
        &listener,
        project.path(),
        protocol,
        resume,
        options,
    )
    .await;
    assert!(system_text(&cached, protocol).contains("INITIAL PROJECT POLICY"));
    harness.reset_prompt_cache(&id).await;
    let options = serde_json::from_value(json!({"prompt": {"cacheMode": "tool-based"}})).unwrap();
    let (refreshed, _) = capture_turn(
        &harness,
        &listener,
        project.path(),
        protocol,
        Some(id),
        options,
    )
    .await;
    assert!(system_text(&refreshed, protocol).contains("INITIAL PROJECT POLICY"));
}

#[tokio::test]
async fn project_context_can_be_disabled_without_replacing_the_base_in_each_protocol() {
    for protocol in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let harness = NativeHarness::new(config(protocol, &endpoint));
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("AGENTS.md"), "PROJECT CONTEXT FIXTURE").unwrap();
        for disabled in [false, true] {
            let options =
                serde_json::from_value(json!({"prompt": {"disableProjectInstructions": disabled}}))
                    .unwrap();
            let (body, _) =
                capture_turn(&harness, &listener, project.path(), protocol, None, options).await;
            let text = system_text(&body, protocol);
            assert!(text.starts_with(IDENTITY));
            assert!(text.contains("# Doing tasks"));
            assert!(text.contains("# Environment"));
            assert_eq!(text.contains("PROJECT CONTEXT FIXTURE"), !disabled);
            assert_eq!(
                body.to_string().contains("PROJECT CONTEXT FIXTURE"),
                !disabled
            );
            assert!(body.to_string().contains("Today's date"));
        }
    }
}
