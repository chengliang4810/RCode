use async_trait::async_trait;
use rcode_agent::agent_tasks::{AgentTaskStore, AgentTasks};
use rcode_agent::subagents::SubagentDefinition;
use rcode_agent::tools::{SubagentOutcome, SubagentRuntime, SubagentSpawn};
use rcode_agent::{Message, ToolContext, ToolRegistry};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Runtime {
    gate: Option<Arc<tokio::sync::Semaphore>>,
    running: AtomicUsize,
    seen: Mutex<Vec<(String, usize, String)>>,
}
#[async_trait]
impl SubagentRuntime for Runtime {
    fn definitions(&self, _: &std::path::Path) -> Vec<SubagentDefinition> {
        vec![]
    }
    async fn run(&self, spawn: SubagentSpawn) -> Result<SubagentOutcome, String> {
        self.seen.lock().unwrap().push((
            spawn.prompt.clone(),
            spawn.history.len(),
            spawn.definition.name.clone(),
        ));
        self.running.fetch_add(1, Ordering::SeqCst);
        let ready = if let Some(gate) = &self.gate {
            tokio::select! {
                _ = spawn.cancel.cancelled() => Err("cancelled".to_owned()),
                permit = gate.acquire() => { permit.unwrap().forget(); Ok(()) }
            }
        } else {
            Ok(())
        };
        self.running.fetch_sub(1, Ordering::SeqCst);
        ready?;
        let queued = spawn
            .inbox
            .as_ref()
            .map(|inbox| inbox.drain())
            .unwrap_or_default();
        let output = format!("{} {}", spawn.prompt, queued.join(" "));
        let mut history = spawn.history;
        history.push(Message::user(spawn.prompt));
        history.push(Message::assistant(output.clone()));
        Ok(SubagentOutcome { output, history })
    }
}
fn setup(runtime: Arc<dyn SubagentRuntime>) -> (Arc<AgentTasks>, ToolRegistry, ToolContext) {
    let manager = Arc::new(AgentTasks::new(
        Arc::new(AgentTaskStore::default()),
        runtime,
    ));
    let mut tools = ToolRegistry::new();
    tools.set_managed_agents(manager.clone());
    let ctx = ToolContext {
        cwd: std::env::temp_dir(),
        cancel: CancellationToken::new(),
        nested: None,
    };
    (manager, tools, ctx)
}
async fn call(tools: &ToolRegistry, name: &str, input: Value, ctx: &ToolContext) -> Value {
    serde_json::from_str(&tools.call(name, input, ctx).await.unwrap().output).unwrap()
}
async fn start(tools: &ToolRegistry, ctx: &ToolContext, background: bool) -> Value {
    call(
        tools,
        "Agent",
        json!({"description":"Review code","prompt":"initial","run_in_background":background}),
        ctx,
    )
    .await
}

#[tokio::test]
async fn sidebar_stop_suppresses_background_model_notifications() {
    let dir = tempfile::tempdir().unwrap();
    let processes = Arc::new(rcode_agent::processes::ProcessRegistry::open(Some(dir.path())));
    let manager = Arc::new(AgentTasks::new(
        Arc::new(AgentTaskStore::with_processes("chat".into(), processes.clone())),
        Arc::new(Runtime::default()),
    ));
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(rcode_agent::tools::BashTool));
    tools.set_managed_agents(manager.clone());
    let ctx = ToolContext { cwd: dir.path().into(), cancel: CancellationToken::new(), nested: None };
    let command = if cfg!(windows) { "Start-Sleep -Seconds 120" } else { "sleep 120" };
    let task = call(&tools, "Bash", json!({"command":command,"run_in_background":true}), &ctx).await;
    let id = task["backgroundTaskId"].as_str().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while processes.list("chat").is_empty() { tokio::time::sleep(std::time::Duration::from_millis(20)).await; }
    }).await.unwrap();
    assert!(processes.stop("chat", id).unwrap());
    tokio::time::timeout(std::time::Duration::from_secs(10), manager.wait_for_idle()).await.unwrap();
    assert!(!manager.has_notifications(), "manual stop must not wake the parent model");
    assert!(manager.notifications().is_empty());
    assert_eq!(call(&tools, "TaskOutput", json!({"task_id":id,"block":false}), &ctx).await["task"]["status"], "stopped");
}

#[tokio::test]
async fn agent_is_the_only_spawn_entry_and_task_is_not_resolved_or_executed() {
    let runtime = Arc::new(Runtime::default());
    let (_, tools, ctx) = setup(runtime.clone());
    let names: Vec<_> = tools
        .definitions()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        names.iter().filter(|name| name.as_str() == "Agent").count(),
        1
    );
    assert!(!names.iter().any(|name| name == "Task"));
    for old_name in ["Task", "task"] {
        assert!(tools.get(old_name).is_none());
        assert!(matches!(
            tools
                .call(
                    old_name,
                    json!({"description":"Old entry", "prompt":"must not execute"}),
                    &ctx
                )
                .await,
            Err(rcode_agent::ToolError::NotFound(_))
        ));
    }
    assert!(runtime.seen.lock().unwrap().is_empty());
    let output = start(&tools, &ctx, false).await;
    assert_eq!(output["task"]["status"], "completed");
}

#[tokio::test]
async fn background_agents_overlap_and_output_waits_for_the_selected_agent() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let runtime = Arc::new(Runtime {
        gate: Some(gate.clone()),
        ..Default::default()
    });
    let (manager, tools, ctx) = setup(runtime.clone());
    let mut ids = vec![];
    for _ in 0..3 {
        ids.push(
            start(&tools, &ctx, true).await["agentId"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while runtime.running.load(Ordering::SeqCst) != 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(manager.has_running());
    let pending = call(
        &tools,
        "TaskOutput",
        json!({"task_id":ids[0],"block":false}),
        &ctx,
    )
    .await;
    assert_eq!(pending["retrieval_status"], "not_ready");
    let timed = call(
        &tools,
        "TaskOutput",
        json!({"task_id":ids[0],"timeout":0}),
        &ctx,
    )
    .await;
    assert_eq!(timed["retrieval_status"], "timeout");
    gate.add_permits(3);
    for id in ids {
        let output = call(&tools, "TaskOutput", json!({"task_id":id}), &ctx).await;
        assert_eq!(output["task"]["status"], "completed");
    }
    assert!(!manager.has_running());
    assert_eq!(manager.notifications().len(), 3);
    assert!(
        manager.notifications().is_empty(),
        "completion is delivered once per generation"
    );
}
#[tokio::test]
async fn send_message_queues_for_a_running_agent_and_resumes_a_finished_one_with_history() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let runtime = Arc::new(Runtime {
        gate: Some(gate.clone()),
        ..Default::default()
    });
    let (_, tools, ctx) = setup(runtime.clone());
    let id = start(&tools, &ctx, true).await["agentId"]
        .as_str()
        .unwrap()
        .to_owned();
    let delivered = call(
        &tools,
        "SendMessage",
        json!({"to":id,"summary":"Additional requirement","message":"check errors"}),
        &ctx,
    )
    .await;
    assert_eq!(delivered["delivery"], "queued");
    gate.add_permits(1);
    let first = call(&tools, "TaskOutput", json!({"task_id":id}), &ctx).await;
    assert!(
        first["task"]["output"]
            .as_str()
            .unwrap()
            .contains("check errors")
    );
    let resumed = call(
        &tools,
        "SendMessage",
        json!({"to":id,"summary":"Continue review","message":"verify the fix"}),
        &ctx,
    )
    .await;
    assert_eq!(resumed["delivery"], "resumed_background");
    gate.add_permits(1);
    let second = call(&tools, "TaskOutput", json!({"task_id":id}), &ctx).await;
    assert_eq!(second["task"]["status"], "completed");
    let seen = runtime.seen.lock().unwrap();
    assert_eq!(seen[1].0, "verify the fix");
    assert_eq!(
        seen[1].1, 2,
        "the original child conversation survives continuation"
    );
    assert_eq!(seen[0].2, seen[1].2, "continuation uses the original role");
}
#[tokio::test]
async fn management_is_conversation_scoped_and_model_overrides_are_rejected() {
    let runtime = Arc::new(Runtime::default());
    let (_, a, ctx) = setup(runtime.clone());
    let (_, b, other_ctx) = setup(runtime);
    let completed = start(&a, &ctx, false).await;
    let id = completed["task"]["agentId"].as_str().unwrap();
    for (name, input) in [
        ("TaskOutput", json!({"task_id":id,"block":false})),
        ("TaskStop", json!({"task_id":id})),
        (
            "SendMessage",
            json!({"to":id,"summary":"Hi","message":"Hello"}),
        ),
    ] {
        assert!(b.call(name, input, &other_ctx).await.is_err());
    }
    assert!(
        a.call(
            "Agent",
            json!({"description":"Test","prompt":"Test","model":"other"}),
            &ctx
        )
        .await
        .is_err()
    );
    assert!(
        call(&b, "TaskList", json!({}), &other_ctx).await["tasks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn stopping_one_agent_and_parent_turn_leave_other_background_agents_running() {
    let runtime = Arc::new(Runtime {
        gate: Some(Arc::new(tokio::sync::Semaphore::new(0))),
        ..Default::default()
    });
    let (manager, tools, ctx) = setup(runtime);
    let first = start(&tools, &ctx, true).await["agentId"]
        .as_str()
        .unwrap()
        .to_owned();
    let second = start(&tools, &ctx, true).await["agentId"]
        .as_str()
        .unwrap()
        .to_owned();
    call(&tools, "TaskStop", json!({"task_id":first}), &ctx).await;
    assert_eq!(
        call(&tools, "TaskOutput", json!({"task_id":first}), &ctx).await["task"]["status"],
        "stopped"
    );
    assert_eq!(
        call(
            &tools,
            "TaskOutput",
            json!({"task_id":second,"block":false}),
            &ctx
        )
        .await["task"]["status"],
        "running"
    );
    ctx.cancel.cancel();
    let next_ctx = ToolContext {
        cancel: CancellationToken::new(),
        ..ctx
    };
    assert_eq!(
        call(
            &tools,
            "TaskOutput",
            json!({"task_id":second,"block":false}),
            &next_ctx
        )
        .await["task"]["status"],
        "running"
    );
    manager.cancel_running();
    assert_eq!(
        call(&tools, "TaskOutput", json!({"task_id":second}), &next_ctx).await["task"]["status"],
        "stopped"
    );
}

struct FailingRuntime;
#[async_trait]
impl SubagentRuntime for FailingRuntime {
    fn definitions(&self, _: &std::path::Path) -> Vec<SubagentDefinition> {
        vec![]
    }
    async fn run(&self, _: SubagentSpawn) -> Result<SubagentOutcome, String> {
        panic!("fixture panic");
    }
}
#[tokio::test]
async fn background_panics_are_reported_and_foreground_failures_are_tool_errors() {
    let (manager, tools, ctx) = setup(Arc::new(FailingRuntime));
    let result = tools
        .call(
            "Agent",
            json!({"description":"Fail safely","prompt":"initial"}),
            &ctx,
        )
        .await
        .unwrap();
    assert!(result.is_error);
    let value: Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(value["task"]["status"], "failed");
    assert!(
        value["task"]["error"]
            .as_str()
            .unwrap()
            .contains("panicked")
    );
    assert!(!manager.has_running());
    assert!(
        manager.notifications().is_empty(),
        "foreground failures were already returned to the caller"
    );
}

#[tokio::test]
async fn manual_and_timed_promotion_keep_the_original_agent_alive() {
    for timed in [false, true] {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let runtime = Arc::new(Runtime {
            gate: Some(gate.clone()),
            ..Default::default()
        });
        let (manager, mut tools, ctx) = setup(runtime.clone());
        tools.register(Arc::new(rcode_agent::tools::BashTool));
        let mut input = json!({"description":"Promote","prompt":"initial"});
        if timed {
            input["auto_background_ms"] = json!(10);
        }
        let launch = tools.call("Agent", input, &ctx);
        let promote = async {
            while runtime.running.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            if !timed {
                let list = call(&tools, "TaskList", json!({}), &ctx).await;
                let id = list["tasks"][0]["task_id"].as_str().unwrap();
                assert_eq!(
                    call(&tools, "TaskBackground", json!({"task_id":id}), &ctx).await["backgrounded"],
                    true
                );
            }
        };
        let (result, _) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(launch, promote)
        })
        .await
        .unwrap();
        let result: Value = serde_json::from_str(&result.unwrap().output).unwrap();
        let id = result["agentId"].as_str().unwrap();
        assert_eq!(result["isAsync"], true);
        ctx.cancel.cancel();
        manager.cancel_foreground();
        let next = ToolContext {
            cancel: CancellationToken::new(),
            ..ctx.clone()
        };
        assert_eq!(
            call(
                &tools,
                "TaskOutput",
                json!({"task_id":id,"block":false}),
                &next
            )
            .await["task"]["status"],
            "running"
        );
        gate.add_permits(1);
        assert_eq!(
            call(&tools, "TaskOutput", json!({"task_id":id}), &next).await["task"]["status"],
            "completed"
        );
        assert_eq!(
            runtime.seen.lock().unwrap().len(),
            1,
            "promotion must not relaunch the agent"
        );
        assert_eq!(manager.notifications().len(), 1);
        assert!(manager.notifications().is_empty());
    }
}

#[tokio::test]
async fn managed_bash_caps_excessive_timeouts_and_shares_the_timeout_schema() {
    let (manager, mut tools, ctx) = setup(Arc::new(Runtime::default()));
    let plain = rcode_agent::tools::BashTool;
    let schema = rcode_agent::Tool::input_schema(&plain);
    tools.register(Arc::new(plain));
    tools.set_managed_agents(manager);
    let managed = tools.get("Bash").unwrap();
    assert_eq!(
        managed.input_schema()["properties"]["timeout"],
        schema["properties"]["timeout"]
    );
    let result = tools
        .call(
            "Bash",
            json!({"command":"echo capped", "timeout":900}),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output);
    assert!(result.output.contains("capped"));
    let tasks = call(&tools, "TaskList", json!({}), &ctx).await;
    assert_eq!(tasks["tasks"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn managed_bash_records_pipeline_failure_in_task_output() {
    let (manager, mut tools, ctx) = setup(Arc::new(Runtime::default()));
    tools.register(Arc::new(rcode_agent::tools::BashTool));
    tools.set_managed_agents(manager);
    let result = tools
        .call(
            "Bash",
            json!({
                "command":if cfg!(windows) { "Write-Output build-failed; cmd.exe /d /c exit 23 | Select-Object -Last 5" } else { "(printf build-failed; exit 23) | cat" }, "timeout":10
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(result.is_error, "{}", result.output);
    assert!(result.output.starts_with("exit 23\n"), "{}", result.output);
    let tasks = call(&tools, "TaskList", json!({}), &ctx).await;
    let task = &tasks["tasks"][0];
    assert_eq!(task["status"], "failed");
    let output = call(
        &tools,
        "TaskOutput",
        json!({"task_id":task["task_id"]}),
        &ctx,
    )
    .await;
    assert_eq!(output["task"]["status"], "failed");
    assert!(
        output["task"]["output"]
            .as_str()
            .unwrap()
            .contains("build-failed")
    );
}

#[tokio::test]
async fn bash_timeout_promotes_the_same_process_and_retains_output_after_parent_stop() {
    let (manager, mut tools, mut ctx) = setup(Arc::new(Runtime::default()));
    tools.register(Arc::new(rcode_agent::tools::BashTool));
    tools.set_managed_agents(manager.clone());
    let dir = tempfile::tempdir().unwrap();
    ctx.cwd = dir.path().into();
    let command = if cfg!(windows) {
        "Add-Content executions.txt once; Write-Output started; Start-Sleep -Seconds 3; Write-Output finished"
    } else {
        "echo once >> executions.txt; echo started; sleep 3; echo finished"
    };
    let result = call(&tools, "Bash", json!({"command":command,"timeout":1}), &ctx).await;
    assert_eq!(result["isAsync"], true);
    let id = result["backgroundTaskId"].as_str().unwrap();
    ctx.cancel.cancel();
    manager.cancel_foreground();
    let next = ToolContext {
        cancel: CancellationToken::new(),
        ..ctx
    };
    let pending = call(
        &tools,
        "TaskOutput",
        json!({"task_id":id,"block":false}),
        &next,
    )
    .await;
    assert_eq!(pending["task"]["task_type"], "local_bash");
    assert!(
        pending["task"]["output"]
            .as_str()
            .unwrap()
            .contains("started")
    );
    let finished = call(
        &tools,
        "TaskOutput",
        json!({"task_id":id,"timeout":10000}),
        &next,
    )
    .await;
    assert_eq!(finished["task"]["status"], "completed");
    assert!(
        finished["task"]["output"]
            .as_str()
            .unwrap()
            .contains("finished")
    );
    let log = finished["task"]["output_file"].as_str().unwrap();
    assert!(std::fs::read_to_string(log).unwrap().contains("finished"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("executions.txt"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert_eq!(manager.notifications().len(), 1);
    assert!(
        tools
            .call(
                "SendMessage",
                json!({"to":id,"summary":"No","message":"No"}),
                &next
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn explicit_background_shell_is_stopped_with_its_process_tree_on_runtime_exit() {
    let (manager, mut tools, mut ctx) = setup(Arc::new(Runtime::default()));
    tools.register(Arc::new(rcode_agent::tools::BashTool));
    tools.set_managed_agents(manager.clone());
    let dir = tempfile::tempdir().unwrap();
    ctx.cwd = dir.path().into();
    let root = CancellationToken::new();
    manager.bind_runtime(root.clone(), Arc::new(|_| {}));
    let command = if cfg!(windows) {
        "Write-Output started; Start-Sleep -Seconds 5; Set-Content leaked.txt leaked"
    } else {
        "echo started; sleep 5; echo leaked > leaked.txt"
    };
    let launched = call(
        &tools,
        "Bash",
        json!({"command":command,"run_in_background":true}),
        &ctx,
    )
    .await;
    let id = launched["backgroundTaskId"].as_str().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if call(
                &tools,
                "TaskOutput",
                json!({"task_id":id,"block":false}),
                &ctx,
            )
            .await["task"]["output"]
                .as_str()
                .unwrap()
                .contains("started")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    root.cancel();
    let stopped = call(
        &tools,
        "TaskOutput",
        json!({"task_id":id,"timeout":10000}),
        &ctx,
    )
    .await;
    assert_eq!(stopped["task"]["status"], "stopped");
    assert!(!dir.path().join("leaked.txt").exists());
}

#[tokio::test]
async fn manual_shell_promotion_disarms_the_original_foreground_timeout() {
    let (manager, mut tools, mut ctx) = setup(Arc::new(Runtime::default()));
    tools.register(Arc::new(rcode_agent::tools::BashTool));
    tools.set_managed_agents(manager.clone());
    let dir = tempfile::tempdir().unwrap();
    ctx.cwd = dir.path().into();
    let command = if cfg!(windows) {
        "Write-Output started; Start-Sleep -Seconds 3; Write-Output finished"
    } else {
        "echo started; sleep 3; echo finished"
    };
    let foreground = tools.call(
        "Bash",
        json!({"command":command,"timeout":1,"auto_background":false}),
        &ctx,
    );
    let promote = async {
        loop {
            let list = call(&tools, "TaskList", json!({}), &ctx).await;
            if let Some(task) = list["tasks"].as_array().unwrap().first() {
                assert_eq!(
                    call(
                        &tools,
                        "TaskBackground",
                        json!({"task_id":task["task_id"]}),
                        &ctx
                    )
                    .await["backgrounded"],
                    true
                );
                break;
            }
            tokio::task::yield_now().await;
        }
    };
    let (result, _) = tokio::join!(foreground, promote);
    let launched: Value = serde_json::from_str(&result.unwrap().output).unwrap();
    let id = launched["backgroundTaskId"].as_str().unwrap();
    let result = call(
        &tools,
        "TaskOutput",
        json!({"task_id":id,"timeout":10000}),
        &ctx,
    )
    .await;
    assert_eq!(result["task"]["status"], "completed");
    assert!(
        result["task"]["output"]
            .as_str()
            .unwrap()
            .contains("finished")
    );
}

#[tokio::test]
async fn reported_task_history_is_bounded_without_exhausting_future_task_capacity() {
    let (_, tools, ctx) = setup(Arc::new(Runtime::default()));
    let first = start(&tools, &ctx, false).await["task"]["task_id"]
        .as_str()
        .unwrap()
        .to_owned();
    for _ in 0..260 {
        assert_eq!(
            start(&tools, &ctx, false).await["task"]["status"],
            "completed"
        );
    }
    assert_eq!(
        call(&tools, "TaskList", json!({}), &ctx).await["tasks"]
            .as_array()
            .unwrap()
            .len(),
        256
    );
    assert!(
        tools
            .call("TaskOutput", json!({"task_id":first}), &ctx)
            .await
            .is_err()
    );
}
