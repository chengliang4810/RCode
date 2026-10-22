//! Developer-only Android/iOS smoke fixture. No provider is contacted.
//!
//! Run: cargo run -p rcode-engine --example mobile_smoke
//! Android emulator host: ws://10.0.2.2:27657 (fixture key: MobileFixture123).
//! Use `question` for a typed user question or `slow` for a 15-second reply.
//! Every response contains Markdown math and a real interactive Luau visual.
//! The temporary profile, chats and uploaded scripts are removed on clean exit.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};
use rcode_engine::{EngineCore, HarnessRegistry, Uploads};
use rcode_harness::{Harness, HarnessError, RunControls};
use rcode_proto::{
    AgentEvent, ChatConfig, DoneStatus, HarnessId, InteractionRequest, InteractionResponse, Model,
    ReasoningLevel, RunRequest, SteeringMode, UserInputQuestion,
};
use rcode_rpc::methods;
use serde_json::json;

const FIXTURE_KEY: &str = "MobileFixture123";
const MODEL: &str = "mobile-qa-mock";
const CHAT: &str = "mobile-qa-chat";
const VISUAL: &str = r##"
local state = ui.state { count = 0 }
return function()
  return ui.column { gap = 10,
    ui.text("Native Luau · Count " .. state.count),
    ui.button { key = "add", label = "Add", on_click = function()
      state.count = state.count + 1
    end },
    ui.canvas { height = 48, draw = function(g)
      g:rect(0, 8, 120, 24, { fill = "success" })
    end },
    ui.scene { height = 120, draw = function(scene)
      scene:box { at = { 0, 0.5, 0 }, size = 1, color = "accent" }
    end },
    ui.shader { height = 48, code = [[
      void mainImage(out vec4 color, in vec2 point) {
        color = vec4(0.25, 0.5, 1.0, 1.0);
      }
    ]] },
  }
end
"##;

/// A deterministic test driver. Native is only its catalog ID: the production
/// registry intentionally excludes Mock from enabled agents, so using Mock
/// would make this fixture's model inaccessible to the shipping picker.
struct MobileQaMockHarness {
    uploads: Arc<OnceLock<Uploads>>,
}

#[async_trait]
impl Harness for MobileQaMockHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Native
    }
    fn display_name(&self) -> &str {
        "Mobile QA Mock (no provider)"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[]
    }
    fn deterministic_turn_end(&self) -> bool {
        true
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![Model {
            id: MODEL.into(),
            label: "Mobile QA Mock".into(),
            description: Some("Local test fixture; no provider requests".into()),
            reasoning_levels: vec![],
            options: vec![],
        }])
    }
    async fn run(
        &self,
        request: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        let uploads = self
            .uploads
            .get()
            .ok_or_else(|| HarnessError::Protocol("fixture is not initialized".into()))?;
        let id = format!("v:{}", uuid::Uuid::new_v4());
        let path = uploads
            .store_visual(&id, VISUAL)
            .map_err(|error| HarnessError::Protocol(error.to_string()))?;
        let (send, receive) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            let prompt = request.prompt.to_lowercase();
            let text = format!("Mobile QA mock received: {}\n\n", request.prompt);
            if send.send(Ok(AgentEvent::TextDelta { text })).await.is_err() {
                return;
            }
            if prompt.contains("question") || prompt.contains("问题") {
                let answer = (controls.request_input)(InteractionRequest::Questions {
                    questions: vec![UserInputQuestion {
                        id: "qa-choice".into(),
                        header: "QA choice".into(),
                        question: "Choose a fixture answer".into(),
                        options: vec!["Alpha".into(), "Beta".into()],
                        prefill: None,
                        multiline: false,
                        multi_select: false,
                    }],
                });
                let answer = tokio::select! {
                    _ = send.closed() => return,
                    _ = controls.interrupt.cancelled() => None,
                    result = answer => result.ok(),
                };
                let text = match answer {
                    Some(InteractionResponse::Questions { answers }) => {
                        format!("Typed answer received: {}\n\n", json!(answers))
                    }
                    _ => "Question cancelled.\n\n".into(),
                };
                if send.send(Ok(AgentEvent::TextDelta { text })).await.is_err() {
                    return;
                }
            }
            let delay = if prompt.contains("slow") || prompt.contains("慢") {
                Duration::from_secs(15)
            } else {
                Duration::from_millis(250)
            };
            let interrupted = tokio::select! {
                _ = send.closed() => return,
                _ = controls.interrupt.cancelled() => true,
                _ = tokio::time::sleep(delay) => false,
            };
            if !interrupted {
                // Formulas use the same Markdown TextDelta as real agents;
                // Visual uses the production AgentEvent -> doc part fold.
                let events = [
                    AgentEvent::TextDelta {
                        text:
                            "Inline math: $E = mc^2$.\n\n$$\\int_0^1 x^2\\,dx = \\frac{1}{3}$$\n\n"
                                .into(),
                    },
                    AgentEvent::Visual {
                        id,
                        path,
                        title: "Mobile QA native visual".into(),
                    },
                    AgentEvent::TextDelta {
                        text: "Fixture response completed.".into(),
                    },
                ];
                for event in events {
                    if send.send(Ok(event)).await.is_err() {
                        return;
                    }
                }
            }
            let _ = send
                .send(Ok(AgentEvent::Done {
                    status: if interrupted {
                        DoneStatus::Interrupted
                    } else {
                        DoneStatus::Completed
                    },
                    result: None,
                    error: None,
                    session_id: None,
                }))
                .await;
        });
        Ok(futures::stream::unfold(receive, |mut receive| async move {
            receive.recv().await.map(|event| (event, receive))
        })
        .boxed())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rcode_visual::check(VISUAL, rcode_visual::seed_for("mobile-qa"))?;
    let profile = tempfile::tempdir()?;
    let project = profile.path().join("qa-project");
    std::fs::create_dir(&project)?;
    let uploads = Arc::new(OnceLock::new());
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(MobileQaMockHarness {
        uploads: uploads.clone(),
    }));
    let core = EngineCore::assemble(profile.path(), Arc::new(registry), HarnessId::Native, None)?;
    uploads
        .set(core.uploads.clone())
        .map_err(|_| anyhow::anyhow!("fixture uploads already initialized"))?;
    let cwd = project.to_string_lossy().into_owned();
    core.workspace.create_space(
        "mobile-qa-space",
        &core.device_id,
        &cwd,
        Some("Mobile QA fixture".into()),
        false,
    )?;
    core.workspace.create_chat(
        CHAT,
        Some("mobile-qa-space"),
        Some(&core.device_id),
        Some(ChatConfig {
            harness: HarnessId::Native,
            model: Some(MODEL.into()),
            reasoning: None,
            model_options: Default::default(),
        }),
        Some(cwd.clone()),
    )?;
    core.workspace
        .rename_chat(CHAT, "Mobile QA · real engine")?;
    core.sessions
        .dispatch(
            CHAT,
            HarnessId::Native,
            RunRequest {
                prompt: "Mobile fixture ready".into(),
                harness: Some(HarnessId::Native),
                model: Some(MODEL.into()),
                reasoning: None,
                model_options: Default::default(),
                cwd,
                resume: None,
                attachments: vec![],
                worktree: None,
            },
            None,
        )
        .await?;

    // This is the real authenticated listener, restricted to host loopback.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:27657").await?;
    let (_access, key) = tokio::sync::watch::channel(Some(FIXTURE_KEY.to_owned()));
    let service = Arc::new(core.rpc_service().as_ref().clone().without_remote_access());
    let server = tokio::spawn(rcode_rpc::serve_authenticated_listener(
        listener, service, key,
    ));

    // A startup probe verifies the actual socket and model catalog before QA.
    let client = rcode_rpc::connect_ws_authenticated("ws://127.0.0.1:27657", FIXTURE_KEY).await?;
    let models: Vec<Model> = client
        .call_as(methods::LIST_MODELS, json!({ "harness": "native" }))
        .await?;
    anyhow::ensure!(models.iter().any(|model| model.id == MODEL));
    client.close();
    println!("Mobile QA mock ready (no provider requests)");
    println!("Host: ws://127.0.0.1:27657; emulator: ws://10.0.2.2:27657");
    println!("Public fixture key: {FIXTURE_KEY}");
    println!("deviceId={}; chatId={CHAT}; model={MODEL}", core.device_id);
    println!("Temporary profile: {}", profile.path().display());
    println!("Prompts: any text / question / slow. Ctrl+C shuts down and removes the profile.");
    tokio::signal::ctrl_c().await?;
    server.abort();
    core.shutdown().await;
    Ok(())
}
