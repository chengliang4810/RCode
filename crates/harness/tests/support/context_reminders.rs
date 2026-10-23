use super::*;

const PROMPT: &str = "Using only the CURRENT factual memory-index description already supplied in this request, reply with its FACT-V number (for example FACT-V9), or NONE if there is no current fact. Do not read files or call tools. Reply with that value only.";

async fn round(
    harness: &NativeHarness,
    listener: &TcpListener,
    cwd: &Path,
    protocol: Protocol,
    resume: Option<String>,
    history: Vec<rcode_proto::ConversationTurn>,
) -> (Value, String, Vec<rcode_proto::ConversationTurn>) {
    let previous = history.clone();
    let run = async {
        let (_steer, steering) = tokio::sync::mpsc::channel(1);
        let controls = RunControls {
            conversation_id: None,
            goal: None,
            request_input: Box::new(|_| panic!("no questions")),
            steering,
            interrupt: CancellationToken::new(),
            turn: TurnControl::default(),
            history: Some(history),
        };
        let request = RunRequest {
            prompt: PROMPT.into(),
            harness: None,
            model: None,
            reasoning: None,
            model_options: Default::default(),
            cwd: cwd.to_string_lossy().into_owned(),
            resume,
            attachments: vec![],
            worktree: None,
        };
        let events: Vec<_> = harness
            .run(request, controls)
            .await
            .unwrap()
            .collect()
            .await;
        assert!(
            events.iter().any(|e| matches!(
                e,
                Ok(AgentEvent::Done {
                    status: DoneStatus::Completed,
                    ..
                })
            )),
            "{events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Ok(AgentEvent::UserMessage { .. }))),
            "internal context leaked into visible chat"
        );
        let (session, id) = events
            .iter()
            .find_map(|e| match e {
                Ok(AgentEvent::SessionStarted {
                    session_id,
                    assistant_message_id,
                    ..
                }) => Some((session_id.clone(), assistant_message_id.clone())),
                _ => None,
            })
            .unwrap();
        (session, id)
    };
    let (body, (session, id)) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(receive_request(listener, protocol), run)
    })
    .await
    .expect("context round completes");
    let mut history = previous;
    history.push(rcode_proto::ConversationTurn {
        role: "user".into(),
        text: PROMPT.into(),
        native_history_id: None,
        todo_history_ids: Vec::new(),
    });
    history.push(rcode_proto::ConversationTurn {
        role: "assistant".into(),
        text: "ok".into(),
        native_history_id: Some(id),
        todo_history_ids: Vec::new(),
    });
    (body, session, history)
}

fn messages(body: &Value, protocol: Protocol) -> &Value {
    if protocol == Protocol::OpenAiResponses {
        &body["input"]
    } else {
        &body["messages"]
    }
}

fn reminder_count(body: &Value, protocol: Protocol) -> usize {
    messages(body, protocol)
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "user")
        .map(|m| m.to_string().matches("<system-reminder>").count())
        .sum()
}

async fn lifecycle(protocol: Protocol, output: Option<&Path>) {
    let project = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    config(
        protocol,
        &format!("http://{}/v1", listener.local_addr().unwrap()),
    )
    .save(data.path())
    .unwrap();
    let root = rcode_agent::memory::directory(project.path(), data.path());
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("fact.md"),
        "---\ntype: project\n---\nPRIVATE FACT BODY",
    )
    .unwrap();
    std::fs::write(
        root.join("feedback.md"),
        "---\ntype: feedback\n---\nPRIVATE FEEDBACK BODY",
    )
    .unwrap();
    let index = |value: Option<&str>| {
        let mut text = String::new();
        if let Some(value) = value {
            text.push_str(&format!("- [Fact](fact.md) — {value}\n"));
        }
        text.push_str("- [Feedback](feedback.md) — SYSTEM BEHAVIOR RULE");
        std::fs::write(root.join("MEMORY.md"), text).unwrap();
    };
    index(Some("FACT-V1"));
    let harness = NativeHarness::from_data_dir(data.path());
    let (first, session, history) =
        round(&harness, &listener, project.path(), protocol, None, vec![]).await;
    let system = system_text(&first, protocol);
    assert!(system.contains("SYSTEM BEHAVIOR RULE"));
    assert!(!system.contains("FACT-V1"));
    assert!(messages(&first, protocol).to_string().contains("FACT-V1"));
    assert!(!first.to_string().contains("PRIVATE FACT BODY"));
    assert!(!first.to_string().contains("PRIVATE FEEDBACK BODY"));
    assert_eq!(reminder_count(&first, protocol), 1);

    index(Some("FACT-V2"));
    let (changed, _, history) = round(
        &harness,
        &listener,
        project.path(),
        protocol,
        Some(session.clone()),
        history,
    )
    .await;
    assert_eq!(system_text(&changed, protocol), system);
    assert_eq!(reminder_count(&changed, protocol), 2);
    assert!(messages(&changed, protocol).to_string().contains("FACT-V2"));
    if protocol != Protocol::AnthropicMessages {
        let before = messages(&first, protocol).as_array().unwrap();
        assert_eq!(
            &messages(&changed, protocol).as_array().unwrap()[..before.len()],
            before.as_slice()
        );
    }
    let (same, _, history) = round(
        &harness,
        &listener,
        project.path(),
        protocol,
        Some(session.clone()),
        history,
    )
    .await;
    assert_eq!(reminder_count(&same, protocol), 2);

    // Fresh harness: replay comes from actual on-disk model-history records.
    let harness = NativeHarness::from_data_dir(data.path());
    index(None);
    let (removed, _, history) = round(
        &harness,
        &listener,
        project.path(),
        protocol,
        Some(session.clone()),
        history,
    )
    .await;
    assert_eq!(system_text(&removed, protocol), system);
    assert_eq!(reminder_count(&removed, protocol), 3);
    let last = messages(&removed, protocol)
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .to_string();
    assert!(last.contains("remove") && last.contains("fact.md"));

    std::fs::write(
        data.path().join("agent-settings.json"),
        r#"{"memory":false}"#,
    )
    .unwrap();
    let (disabled, _, history) = round(
        &harness,
        &listener,
        project.path(),
        protocol,
        Some(session.clone()),
        history,
    )
    .await;
    assert!(!system_text(&disabled, protocol).contains("# Auto memory"));
    assert!(
        messages(&disabled, protocol)
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .to_string()
            .contains("disabled")
    );
    std::fs::write(
        data.path().join("agent-settings.json"),
        r#"{"memory":true}"#,
    )
    .unwrap();
    index(Some("FACT-V3"));
    let (enabled, _, _) = round(
        &harness,
        &listener,
        project.path(),
        protocol,
        Some(session.clone()),
        history,
    )
    .await;
    assert_eq!(system_text(&enabled, protocol), system);
    let last = messages(&enabled, protocol)
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .to_string();
    assert!(last.contains("replace") && last.contains("FACT-V3"));

    let summary = vec![rcode_proto::ConversationTurn {
        role: "user".into(),
        text: "Earlier FACT-V1; old history compacted".into(),
        native_history_id: None,
        todo_history_ids: Vec::new(),
    }];
    let (compacted, _, _) = round(
        &harness,
        &listener,
        project.path(),
        protocol,
        Some(session),
        summary,
    )
    .await;
    assert_eq!(reminder_count(&compacted, protocol), 1);
    let last = messages(&compacted, protocol)
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .to_string();
    assert!(last.contains("replace") && last.contains("FACT-V3"));

    if let Some(output) = output {
        std::fs::create_dir_all(output).unwrap();
        for (name, request) in [
            ("initial", &first),
            ("changed", &changed),
            ("removed", &removed),
            ("reenabled", &enabled),
        ] {
            std::fs::write(
                output.join(format!("{name}.json")),
                serde_json::to_vec_pretty(request).unwrap(),
            )
            .unwrap();
        }
    }
}

#[tokio::test]
async fn factual_context_lifecycle_reaches_all_protocols_without_visible_messages() {
    for protocol in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
    ] {
        lifecycle(protocol, None).await;
    }
}

#[tokio::test]
#[ignore = "set RCODE_CONTEXT_STUDY_DIR to export local production request captures; no live API calls"]
async fn export_context_reminder_probe_requests() {
    let output = std::path::PathBuf::from(
        std::env::var_os("RCODE_CONTEXT_STUDY_DIR").expect("capture directory required"),
    );
    assert!(output.is_absolute());
    lifecycle(Protocol::OpenAiChat, Some(&output)).await;
}
