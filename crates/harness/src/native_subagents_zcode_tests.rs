#[tokio::test]
async fn zcode_roles_send_the_selected_model_effort_tools_and_instruction_snapshot() {
    use rcode_proto::ReasoningLevel::{High, Low};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut requests = vec![];
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![];
            let mut buffer = [0; 8192];
            loop {
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
                    continue;
                };
                let length = String::from_utf8_lossy(&request[..end])
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                if request.len() >= end + 4 + length {
                    requests.push(
                        serde_json::from_slice::<serde_json::Value>(
                            &request[end + 4..end + 4 + length],
                        )
                        .unwrap(),
                    );
                    break;
                }
            }
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"done\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    let mut config = providers();
    config.providers[0].protocol = crate::providers::Protocol::OpenAiChat;
    config.providers[0].base_url = Some(format!("http://{address}/v1"));
    config.providers[0].api_key = Some("local-test".into());
    for model in &mut config.providers[0].models {
        model.reasoning_levels = vec![Low, High];
    }
    let mut base = ToolRegistry::new();
    rcode_agent::tools::register_core_tools(&mut base);
    base.register(Arc::new(CountingTool(AGENT.into())));
    let instructions = Arc::new(std::sync::OnceLock::new());
    instructions.set("FROZEN AGENTS RULES".into()).unwrap();
    let runtime = NativeSubagents {
        deps: SubagentDeps {
            managed: None,
            goal: None,
            providers: config,
            data_dir: None,
            settings: AgentSettings::default(),
            request_input: None,
            parent_model: "claude-opus-4".into(),
            parent_reasoning: Some(Low),
            user_instructions: instructions,
            base_tools: Arc::new(base),
            limits: Arc::new(tokio::sync::Semaphore::new(MAX_ACTIVE_SUBAGENTS)),
            active: Arc::new(ActiveSubagents::default()),
        },
    };
    let mut definitions = crate::builtin_subagents::with_builtins(vec![]);
    assert_eq!(
        definitions
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        ["general-purpose", "Explore"]
    );
    let mut explicit = crate::providers::ModelSelection {
        provider_id: "anthropic".into(),
        model_id: "claude-sonnet-4-5".into(),
        reasoning: Some(High),
    };
    definitions[1].model = ModelAlias::parse(&explicit.selection()).unwrap();
    explicit.reasoning = None;
    let mut model_only = definitions[0].clone();
    model_only.model = ModelAlias::parse(&explicit.selection()).unwrap();
    definitions.push(model_only);
    let cwd = tempfile::tempdir().unwrap();
    for definition in definitions {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            runtime.run(SubagentSpawn {
                history: Vec::new(),
                inbox: None,
                history_sink: None,
                definition,
                prompt: "Report the result".into(),
                cwd: cwd.path().to_path_buf(),
                events: rcode_agent::tools::NestedEventSender::null(),
                cancel: tokio_util::sync::CancellationToken::new(),
                tool_use_id: String::new(),
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.output, "done");
    }
    let requests = server.await.unwrap();
    assert_eq!(requests[0]["model"], "claude-opus-4");
    assert_eq!(requests[0]["reasoning_effort"], "low");
    assert!(
        requests[0]["messages"]
            .to_string()
            .contains("FROZEN AGENTS RULES")
    );
    assert_eq!(requests[1]["model"], "claude-sonnet-4-5");
    assert_eq!(requests[1]["reasoning_effort"], "high");
    assert!(
        !requests[1]["messages"]
            .to_string()
            .contains("FROZEN AGENTS RULES")
    );
    let mut tools = requests[1]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    tools.sort();
    assert_eq!(
        tools,
        [
            "Bash",
            "Glob",
            "Grep",
            "Read",
            "TodoWrite",
            "WebFetch",
            "WebSearch"
        ]
    );
    assert!(
        !requests[0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["function"]["name"] == AGENT)
    );
    assert!(
        requests[2].get("reasoning_effort").is_none(),
        "model-only selection must not inherit the parent effort"
    );
    let mut settings = AgentSettings::default();
    explicit.reasoning = Some(High);
    settings
        .subagent_models
        .insert("builtin:Explore".into(), explicit.selection());
    settings.save(cwd.path()).unwrap();
    assert_eq!(AgentSettings::load(cwd.path()), settings);
}
