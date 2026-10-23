use super::*;

#[tokio::test]
async fn request_diagnostics_records_real_wire_and_links_transcript_usage_in_all_protocols() {
    use sha2::{Digest, Sha256};
    for protocol in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("request-diagnostics.json"),
            r#"{"enabled":true}"#,
        )
        .unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            serde_json::to_vec(&config(protocol, &endpoint)).unwrap(),
        )
        .unwrap();
        let harness = NativeHarness::from_data_dir(data.path());
        let project = tempfile::tempdir().unwrap();
        let run = async {
            let (_steer, steering) = tokio::sync::mpsc::channel(1);
            let controls = RunControls {
                conversation_id: None,
                goal: None,
                request_input: Box::new(|_| panic!("no questions")),
                steering,
                interrupt: CancellationToken::new(),
                turn: TurnControl::default(),
                history: None,
            };
            harness
                .run(
                    RunRequest {
                        prompt: "private-content-for-diagnostic-test".into(),
                        harness: None,
                        model: None,
                        reasoning: None,
                        model_options: Default::default(),
                        cwd: project.path().to_string_lossy().into_owned(),
                        resume: None,
                        attachments: vec![],
                        worktree: None,
                    },
                    controls,
                )
                .await
                .unwrap()
                .collect::<Vec<_>>()
                .await
        };
        let (body, events) = tokio::time::timeout(Duration::from_secs(15), async {
            tokio::join!(receive_request(&listener, protocol), run)
        })
        .await
        .unwrap();
        let id = events
            .iter()
            .find_map(|e| match e {
                Ok(AgentEvent::ModelCall { id, .. }) => Some(id),
                _ => None,
            })
            .unwrap();
        let path = data.path().join("request-diagnostics/requests.jsonl");
        let (text, rows) = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let rows: Vec<Value> = text
                    .lines()
                    .filter_map(|s| serde_json::from_str(s).ok())
                    .collect();
                if rows.iter().any(|r| r["kind"] == "model_call_link") {
                    break (text, rows);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(!text.contains("private-content-for-diagnostic-test"));
        assert!(!text.contains("fixture-key"));
        let start = rows.iter().find(|r| r["kind"] == "request").unwrap();
        assert_eq!(
            start["fingerprint"]["wire"]["sha256"],
            format!("{:x}", Sha256::digest(serde_json::to_vec(&body).unwrap()))
        );
        let link = rows
            .iter()
            .find(|r| r["kind"] == "model_call_link")
            .unwrap();
        assert_eq!(link["modelCallId"], id.as_str());
        assert_eq!(link["callId"], start["callId"]);
        assert!(
            rows.iter()
                .any(|r| r["kind"] == "finished" && r["outcome"] == "completed")
        );
        // Turning the opt-in off applies at the next run; existing records remain intact.
        std::fs::write(
            data.path().join("request-diagnostics.json"),
            r#"{"enabled":false}"#,
        )
        .unwrap();
        capture_turn(
            &harness,
            &listener,
            project.path(),
            protocol,
            None,
            Default::default(),
        )
        .await;
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
}
