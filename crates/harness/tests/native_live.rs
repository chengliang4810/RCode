//! End-to-end: a real conversation through the whole native stack.
//!
//! Ignored by default — it needs network access and a credential:
//!
//! ```text
//! RCODE_LIVE_KEY=… RCODE_LIVE_URL=… \
//!   cargo test -p rcode-harness --test native_live -- --ignored --nocapture
//! ```
//!
//! What this covers that the unit tests cannot: the *seam*. The loop, the tool
//! set, the provider client and the event mapping are each tested alone, but
//! only a real run shows whether they fit — whether a tool call the model makes
//! actually reaches a tool, whether the tool's output reaches the model, and
//! whether the events the engine folds are the ones it expects.
//!
//! It also exercises the paths that need a real model to reach at all: a tool
//! call only happens if the model decides to make one.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use futures::StreamExt;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, timeout, timeout_at};
use tokio_util::sync::CancellationToken;

use rcode_harness::native::NativeHarness;
use rcode_harness::{Harness, RunControls, TurnControl};
use rcode_proto::{AgentEvent, DoneStatus, HarnessId, RunRequest};

/// A provider configuration written for this test, from the environment.
///
/// Written to a temp dir rather than read from the real one so a run here
/// cannot touch the user's own setup.
fn write_config() -> Option<tempfile::TempDir> {
    let key = std::env::var("RCODE_LIVE_KEY").ok()?;
    let endpoint = std::env::var("RCODE_LIVE_URL").ok()?;
    let model =
        std::env::var("RCODE_LIVE_MODEL").unwrap_or_else(|_| "claude-sonnet-5-5".to_owned());

    let dir = tempfile::tempdir().expect("a temp dir");
    let config = serde_json::json!({
        "providers": [{
            "id": "live",
            "label": "Live test provider",
            "protocol": "anthropic_messages",
            "baseUrl": endpoint,
            "apiKey": key,
            "models": [{
                "id": model,
                "model": model,
                "label": "Live model",
                "maxTokens": 4096,
                "contextWindow": 200000
            }]
        }],
        "defaultModel": model
    });
    std::fs::write(
        dir.path().join("providers.json"),
        serde_json::to_string_pretty(&config).expect("serialises"),
    )
    .expect("writes the config");
    Some(dir)
}

/// Controls that optionally answer model-authored questions, and take no steering.
fn controls(allow: bool) -> (RunControls, mpsc::Sender<rcode_harness::SteerMessage>) {
    let (steer_tx, steering) = mpsc::channel(1);
    let request_input: Box<
        dyn Fn(
                rcode_proto::InteractionRequest,
            ) -> oneshot::Receiver<rcode_proto::InteractionResponse>
            + Send
            + Sync,
    > = Box::new(move |request| {
        let (tx, rx) = oneshot::channel();
        let answer = if !allow {
            rcode_proto::InteractionResponse::Dismissed
        } else {
            match request {
                // A question is answered with the first choice it offered.
                rcode_proto::InteractionRequest::Questions { questions } => {
                    rcode_proto::InteractionResponse::Questions {
                        answers: questions
                            .iter()
                            .map(|question| rcode_proto::UserInputAnswer {
                                question_id: question.id.clone(),
                                selected_option_ids: question
                                    .options
                                    .first()
                                    .map(|option| option.id.clone())
                                    .into_iter()
                                    .collect(),
                                text: None,
                            })
                            .collect(),
                    }
                }
            }
        };
        let _ = tx.send(answer);
        rx
    });
    let controls = RunControls {
        conversation_id: None,
        goal: None,
        request_input,
        steering,
        interrupt: CancellationToken::new(),
        turn: TurnControl::default(),
        history: None,
    };
    (controls, steer_tx)
}

const RUN_DEADLINE: Duration = Duration::from_secs(180);
const CANCEL_GRACE: Duration = Duration::from_secs(10);

fn request(prompt: &str, cwd: &std::path::Path, model: &str) -> RunRequest {
    RunRequest {
        prompt: prompt.to_owned(),
        harness: Some(HarnessId::Native),
        model: Some(model.to_owned()),
        reasoning: None,
        model_options: Default::default(),
        cwd: cwd.to_string_lossy().into_owned(),
        resume: None,
        attachments: Vec::new(),
        worktree: None,
    }
}

/// Drive one run to completion, collecting the events and returning a summary.
#[derive(Debug, Default)]
struct RunSummary {
    text: String,
    tool_calls: Vec<(String, rcode_proto::ToolCall)>,
    tool_results: Vec<(String, bool, String)>,
    done: Option<DoneStatus>,
    error: Option<String>,
    started: bool,
}

async fn drive(harness: &NativeHarness, request: RunRequest) -> RunSummary {
    let (controls, _steer_tx) = controls(true);
    drive_with_controls(harness, request, controls).await
}

async fn drive_with_controls(
    harness: &NativeHarness,
    request: RunRequest,
    controls: RunControls,
) -> RunSummary {
    let deadline = Instant::now() + RUN_DEADLINE;
    let interrupt = controls.interrupt.clone();
    let mut stream = timeout_at(deadline, harness.run(request, controls))
        .await
        .expect("the run started within its total deadline")
        .expect("the run started");
    let mut summary = RunSummary::default();
    let mut timed_out = false;
    let mut cancel_deadline = None;
    let mut eof_deadline = None;
    loop {
        // Neither a trickle of deltas nor a Done with an unclosed stream may
        // extend the overall deadline. A timed-out run is cancelled, then must
        // deliver its terminal event and EOF within one bounded grace period.
        let next_deadline = cancel_deadline.or(eof_deadline).unwrap_or(deadline);
        let next = match if Instant::now() >= next_deadline {
            // A perpetually ready stream must not defeat timeout_at by
            // yielding another item before its timer is polled.
            Err(())
        } else {
            timeout_at(next_deadline, stream.next())
                .await
                .map_err(|_| ())
        } {
            Ok(next) => next,
            Err(_) if !timed_out && summary.done.is_none() => {
                timed_out = true;
                interrupt.cancel();
                cancel_deadline = Some(Instant::now() + CANCEL_GRACE);
                continue;
            }
            Err(_) => {
                panic!("the run did not reach terminal EOF within its deadline/grace: {summary:?}")
            }
        };
        let Some(event) = next else { break };
        assert!(
            summary.done.is_none(),
            "an event followed the unique terminal Done: {event:?}"
        );
        match event.expect("the event stream is readable") {
            AgentEvent::SessionStarted { .. } => summary.started = true,
            AgentEvent::TextDelta { text } => summary.text.push_str(&text),
            AgentEvent::ToolCall { id, call } => summary.tool_calls.push((id, call)),
            AgentEvent::ToolResult {
                id,
                is_error,
                output,
                ..
            } => summary
                .tool_results
                .push((id, is_error, output.unwrap_or_default())),
            AgentEvent::Done { status, error, .. } => {
                summary.done = Some(status);
                summary.error = error;
                eof_deadline = Some((Instant::now() + CANCEL_GRACE).min(deadline));
            }
            AgentEvent::InputRequested { .. } => {
                panic!("the loop's duplicate InputRequested was not filtered")
            }
            _ => {}
        }
    }
    assert!(
        summary.done.is_some(),
        "EOF arrived without a Done: {summary:?}"
    );
    if timed_out {
        assert_eq!(
            summary.done,
            Some(DoneStatus::Interrupted),
            "timeout cancellation was not reported as Interrupted: {summary:?}"
        );
        panic!(
            "the live run exceeded its total deadline (cancellation reached unique Done and EOF): {summary:?}"
        );
    }
    summary
}

#[tokio::test]
#[ignore = "needs network and a credential; run with --ignored"]
async fn a_plain_conversation_reaches_a_model_and_finishes() {
    let Some(config_dir) = write_config() else {
        panic!("set RCODE_LIVE_KEY and RCODE_LIVE_URL");
    };
    let model =
        std::env::var("RCODE_LIVE_MODEL").unwrap_or_else(|_| "claude-sonnet-5-5".to_owned());
    let harness = NativeHarness::from_data_dir(config_dir.path());

    let cwd = tempfile::tempdir().expect("a working directory");
    let summary = drive(
        &harness,
        request("Say the single word: ready", cwd.path(), &model),
    )
    .await;

    assert!(summary.started, "the run never reported SessionStarted");
    assert_eq!(
        summary.done,
        Some(DoneStatus::Completed),
        "the run did not complete; error: {:?}",
        summary.error
    );
    assert!(
        summary.text.to_lowercase().contains("ready"),
        "expected the model's answer, got: {:?}",
        summary.text
    );
}

#[tokio::test]
#[ignore = "needs network and a credential; run with --ignored"]
async fn the_model_can_read_a_file_through_the_tool() {
    let Some(config_dir) = write_config() else {
        panic!("set RCODE_LIVE_KEY and RCODE_LIVE_URL");
    };
    let model =
        std::env::var("RCODE_LIVE_MODEL").unwrap_or_else(|_| "claude-sonnet-5-5".to_owned());
    let harness = NativeHarness::from_data_dir(config_dir.path());

    // A file whose content the model cannot guess, so answering correctly
    // proves it actually read it.
    let cwd = tempfile::tempdir().expect("a working directory");
    std::fs::write(
        cwd.path().join("secret.txt"),
        "the passphrase is ORANGE-42\n",
    )
    .expect("writes the fixture");

    let summary = drive(
        &harness,
        request(
            "Read secret.txt and tell me the passphrase it contains.",
            cwd.path(),
            &model,
        ),
    )
    .await;

    assert_eq!(
        summary.done,
        Some(DoneStatus::Completed),
        "the run did not complete; error: {:?}",
        summary.error
    );
    assert!(
        !summary.tool_calls.is_empty(),
        "the model never called a tool; text: {:?}",
        summary.text
    );
    // Deliberately *not* asserting that every tool succeeded. A model probes —
    // it tries a command, sees it fail, and adjusts, which is correct
    // behaviour and not a defect in the tools. What matters is that it got the
    // answer, which the next assertion checks. The failures are printed so a
    // run that needed many attempts is visible.
    let failures: Vec<&(String, bool, String)> = summary
        .tool_results
        .iter()
        .filter(|(_, is_error, _)| *is_error)
        .collect();
    if !failures.is_empty() {
        eprintln!(
            "note: {} of {} tool calls failed (the model recovered); first: {:?}",
            failures.len(),
            summary.tool_results.len(),
            failures.first().map(|(_, _, output)| output)
        );
    }
    assert!(
        summary.text.contains("ORANGE-42"),
        "the model did not report the file's content; text: {:?}",
        summary.text
    );
}

#[tokio::test]
#[ignore = "needs network and a credential; run with --ignored"]
async fn the_model_can_write_a_file_and_run_it() {
    let Some(config_dir) = write_config() else {
        panic!("set RCODE_LIVE_KEY and RCODE_LIVE_URL");
    };
    let model =
        std::env::var("RCODE_LIVE_MODEL").unwrap_or_else(|_| "claude-sonnet-5-5".to_owned());
    let harness = NativeHarness::from_data_dir(config_dir.path());

    let cwd = tempfile::tempdir().expect("a working directory");
    let summary = drive(
        &harness,
        request(
            "Write a file named hello.txt containing exactly the text `hi there`, \
             then read it back and confirm its contents.",
            cwd.path(),
            &model,
        ),
    )
    .await;

    assert_eq!(
        summary.done,
        Some(DoneStatus::Completed),
        "the run did not complete; error: {:?}",
        summary.error
    );
    // The file on disk is the real evidence — a model can claim to have
    // written something without doing it.
    let written = std::fs::read_to_string(cwd.path().join("hello.txt"))
        .expect("the model did not create hello.txt");
    assert!(
        written.contains("hi there"),
        "hello.txt has unexpected contents: {written:?}"
    );
}

#[tokio::test]
#[ignore = "needs network and a credential; run with --ignored"]
async fn the_write_tool_can_access_a_path_outside_the_working_directory() {
    let Some(config_dir) = write_config() else {
        panic!("set RCODE_LIVE_KEY and RCODE_LIVE_URL");
    };
    let model =
        std::env::var("RCODE_LIVE_MODEL").unwrap_or_else(|_| "claude-sonnet-5-5".to_owned());
    let harness = NativeHarness::from_data_dir(config_dir.path());

    // Keep the external write inside an isolated fixture while exercising
    // the same trust-all filesystem access as an absolute memory path.
    let fixture = tempfile::tempdir().expect("an enclosing fixture");
    let cwd = fixture.path().join("work");
    std::fs::create_dir(&cwd).expect("a working directory");
    let escaped = fixture.path().join("escaped.txt");
    let request = request(
        "Use the Write tool to write ../escaped.txt with contents path-check. \
         Use that exact relative path. Do not use Bash or Edit. Then report completion.",
        &cwd,
        &model,
    );
    let (controls, _steer_tx) = controls(false);
    let summary = drive_with_controls(&harness, request, controls).await;
    assert_eq!(
        summary.done,
        Some(DoneStatus::Completed),
        "error: {:?}",
        summary.error
    );
    let write_id = summary
        .tool_calls
        .iter()
        .find_map(|(id, call)| match call {
            rcode_proto::ToolCall::WriteFile { path, .. }
                if path == "../escaped.txt" || std::path::Path::new(path) == escaped =>
            {
                Some(id)
            }
            _ => None,
        })
        .expect("the model must actually attempt the escaping Write");
    assert!(
        summary
            .tool_results
            .iter()
            .any(|(id, is_error, output)| id == write_id
                && !*is_error
                && output.contains("escaped.txt")),
        "the external Write must succeed: {summary:?}"
    );
    assert!(
        std::fs::read_to_string(&escaped)
            .unwrap()
            .contains("path-check")
    );
}

#[tokio::test]
#[ignore = "needs network and a credential; run with --ignored"]
async fn a_write_runs_without_requesting_approval() {
    let Some(config_dir) = write_config() else {
        panic!("set RCODE_LIVE_KEY and RCODE_LIVE_URL");
    };
    let model =
        std::env::var("RCODE_LIVE_MODEL").unwrap_or_else(|_| "claude-sonnet-5-5".to_owned());
    let harness = NativeHarness::from_data_dir(config_dir.path());

    let cwd = tempfile::tempdir().expect("a working directory");
    let request = request(
        "Use the Write tool exactly once to create allowed.txt containing the word go, \
         then stop. Do not use Bash or another tool to create the file.",
        cwd.path(),
        &model,
    );

    let asked = Arc::new(AtomicUsize::new(0));
    let (mut controls, _steer_tx) = controls(false);
    controls.request_input = Box::new({
        let asked = asked.clone();
        move |_| {
            asked.fetch_add(1, Ordering::SeqCst);
            let (tx, rx) = oneshot::channel();
            let _ = tx.send(rcode_proto::InteractionResponse::Dismissed);
            rx
        }
    });
    let summary = drive_with_controls(&harness, request, controls).await;
    assert_eq!(asked.load(Ordering::SeqCst), 0, "tools execute directly");
    assert_eq!(
        summary.done,
        Some(DoneStatus::Completed),
        "error: {:?}",
        summary.error
    );
    assert!(
        std::fs::read_to_string(cwd.path().join("allowed.txt"))
            .is_ok_and(|text| text.contains("go")),
        "the direct write did not happen"
    );
}

#[tokio::test]
#[ignore = "run with --ignored"]
async fn an_unknown_model_is_rejected_before_anything_runs() {
    let Some(config_dir) = write_config() else {
        panic!("set RCODE_LIVE_KEY and RCODE_LIVE_URL");
    };
    let harness = NativeHarness::from_data_dir(config_dir.path());
    let cwd = tempfile::tempdir().expect("a working directory");
    let request = request("hello", cwd.path(), "no-such-model");

    let (controls, _steer_tx) = controls(true);
    let error = match timeout(RUN_DEADLINE, harness.run(request, controls))
        .await
        .expect("the invalid model was rejected within the deadline")
    {
        Ok(_) => panic!("an unknown model must be rejected before a run starts"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(
        message.contains("no-such-model"),
        "the error must name the model: {message}"
    );
}
