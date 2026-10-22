use super::*;
use crate::MessageContent;
use crate::tool::{Tool, ToolError, ToolResult};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Barrier;

struct UnusedHost;
#[async_trait::async_trait]
impl AgentHost for UnusedHost {
    async fn reason(
        &self,
        _: &[Message],
        _: &[Arc<dyn Tool>],
    ) -> Result<BoxStream<'static, Result<ReasoningChunk, AgentError>>, AgentError> {
        unreachable!()
    }
    async fn ask(&self, _: InputRequest) -> Result<InputAnswer, AgentError> {
        unreachable!()
    }
}

struct ProbeTool {
    name: &'static str,
    safe: bool,
    barrier: Option<Arc<Barrier>>,
    active: Arc<AtomicUsize>,
    events: Arc<Mutex<Vec<String>>>,
}
#[async_trait::async_trait]
impl Tool for ProbeTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "probe"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    fn is_concurrency_safe(&self, _: &serde_json::Value) -> bool {
        self.safe
    }
    async fn call(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        if !self.safe {
            assert_eq!(
                self.active.load(Ordering::SeqCst),
                0,
                "write crossed a read barrier"
            );
        }
        self.active.fetch_add(1, Ordering::SeqCst);
        self.events
            .lock()
            .unwrap()
            .push(format!("{}-start", self.name));
        if let Some(barrier) = &self.barrier {
            tokio::select! { _ = barrier.wait() => {}, _ = ctx.cancel.cancelled() => {} }
        }
        self.events
            .lock()
            .unwrap()
            .push(format!("{}-end", self.name));
        self.active.fetch_sub(1, Ordering::SeqCst);
        if input["fail"] == true {
            Ok(ToolResult::error("diagnostic"))
        } else {
            Ok(ToolResult::ok(input["value"].as_str().unwrap_or("ok")))
        }
    }
}

fn call(id: &str, tool: &str, value: &str) -> (String, String, serde_json::Value) {
    (id.into(), tool.into(), serde_json::json!({"value":value}))
}

#[tokio::test]
async fn safe_calls_overlap_and_writes_wait_for_the_whole_batch() {
    let active = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut tools = ToolRegistry::new();
    for (name, safe, barrier) in [
        ("Read", true, Some(Arc::new(Barrier::new(2)))),
        ("Write", false, None),
    ] {
        tools.register(Arc::new(ProbeTool {
            name,
            safe,
            barrier,
            active: active.clone(),
            events: events.clone(),
        }));
    }
    let runner = AgentRunner::new(
        Arc::new(UnusedHost),
        Arc::new(tools),
        AgentConfig::default(),
    );
    let (tx, _rx) = mpsc::channel(64);
    let calls = vec![
        call("w0", "Write", "before"),
        call("r1", "Read", "first"),
        call("r2", "Read", "second"),
        call("w1", "Write", "after"),
    ];
    let results = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        runner.run_tools(&calls, &CancellationToken::new(), &tx),
    )
    .await
    .expect("reads must overlap")
    .unwrap();
    let ids: Vec<_> = results
        .iter()
        .map(|b| match b {
            ContentBlock::ToolResult { tool_use_id, .. } => tool_use_id.as_str(),
            _ => panic!(),
        })
        .collect();
    assert_eq!(ids, ["w0", "r1", "r2", "w1"]);
    let events = events.lock().unwrap();
    assert_eq!(&events[..2], ["Write-start", "Write-end"]);
    assert_eq!(&events[6..], ["Write-start", "Write-end"]);
}

#[tokio::test]
async fn cancellation_drains_started_tools_and_never_starts_the_write() {
    let active = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut tools = ToolRegistry::new();
    for (name, safe) in [("Read", true), ("Write", false)] {
        tools.register(Arc::new(ProbeTool {
            name,
            safe,
            barrier: safe.then(|| Arc::new(Barrier::new(99))),
            active: active.clone(),
            events: events.clone(),
        }));
    }
    let runner = AgentRunner::new(
        Arc::new(UnusedHost),
        Arc::new(tools),
        AgentConfig::default(),
    );
    let (tx, _rx) = mpsc::channel(64);
    let calls = vec![
        call("r1", "Read", "one"),
        call("r2", "Read", "two"),
        call("w", "Write", "write"),
    ];
    let cancel = CancellationToken::new();
    let work = runner.run_tools(&calls, &cancel, &tx);
    let stop = async {
        while active.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
        cancel.cancel();
    };
    let (results, _) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(work, stop)
    })
    .await
    .unwrap();
    assert_eq!(results.unwrap().len(), 2);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .all(|event| event.starts_with("Read"))
    );
}

struct FailedRecorder;
#[async_trait::async_trait]
impl crate::HistoryRecorder for FailedRecorder {
    async fn record(&self, _: &Message) -> Result<(), AgentError> {
        Err(AgentError::Host("disk full".into()))
    }
}

#[tokio::test]
async fn persistence_failure_keeps_completed_side_effects_in_memory_and_stops_later_writes() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let history = Arc::new(Mutex::new(Vec::new()));
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(ProbeTool {
        name: "Write",
        safe: false,
        barrier: None,
        active: Arc::new(AtomicUsize::new(0)),
        events: events.clone(),
    }));
    let runner = AgentRunner::new(
        Arc::new(UnusedHost),
        Arc::new(tools),
        AgentConfig::default(),
    )
    .with_history_sink(history.clone())
    .with_history_recorder(Arc::new(FailedRecorder));
    let (tx, _rx) = mpsc::channel(64);
    let result = runner
        .run_tools(
            &[
                call("one", "Write", "written"),
                call("two", "Write", "later"),
            ],
            &CancellationToken::new(),
            &tx,
        )
        .await;
    assert!(result.unwrap_err().to_string().contains("disk full"));
    assert_eq!(events.lock().unwrap().len(), 2, "only one write ran");
    let history = history.lock().unwrap();
    let MessageContent::Blocks(blocks) = &history[0].content else {
        panic!()
    };
    assert!(
        matches!(&blocks[0],ContentBlock::ToolResult{tool_use_id,content,..} if tool_use_id=="one" && content=="written")
    );
}
