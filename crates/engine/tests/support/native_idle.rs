use rcode_engine::EngineCore;
use rcode_proto::{AgentEvent, DoneStatus, HarnessId, RunRequest};
use std::{path::Path, time::Duration};

pub const FILL_MARKER: &str = "warm-slot-fixture";

pub async fn fill(core: &EngineCore, project: &Path) {
    for index in 0..12 {
        let chat_id = format!("{FILL_MARKER}-{index}");
        core.workspace
            .create_chat(&chat_id, None, Some(&core.device_id), None, None)
            .unwrap();
        core.workspace
            .rename_chat(&chat_id, "Warm fixture")
            .unwrap();
        core.workspace
            .set_chat_cwd(&chat_id, &project.to_string_lossy())
            .unwrap();
        let (_, mut events) = core.sessions.subscribe(&chat_id, 0).unwrap();
        core.sessions
            .dispatch(
                &chat_id,
                HarnessId::Native,
                RunRequest {
                    prompt: chat_id.clone(),
                    harness: Some(HarnessId::Native),
                    model: Some("fixture".into()),
                    reasoning: None,
                    model_options: Default::default(),
                    cwd: project.to_string_lossy().into(),
                    resume: None,
                    attachments: Vec::new(),
                    worktree: None,
                },
                None,
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let AgentEvent::Done { status, .. } = events.recv().await.unwrap().event {
                    assert_eq!(status, DoneStatus::Completed);
                    break;
                }
            }
        })
        .await
        .unwrap();
    }
    assert_eq!(core.doc_host.sync_resources()["retainedBy"]["writers"], 12);
    for index in 0..12 {
        assert!(
            core.sessions
                .live_run_steerable(&format!("{FILL_MARKER}-{index}"))
        );
    }
}
