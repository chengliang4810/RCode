use super::*;
use crate::transport::{HttpRequest, HttpResponse, HttpTransport};
use crate::{
    AnthropicConfig, AnthropicModel, Model, ModelMessage, ModelRequest, ModelRuntimeConfig,
    OpenAiConfig, OpenAiModel, OpenAiResponsesConfig, OpenAiResponsesModel, RetryConfig,
};
use async_trait::async_trait;
use serde_json::json;

#[derive(Default)]
struct Observer(Mutex<Vec<DiagnosticEvent>>);
impl DiagnosticsObserver for Observer {
    fn on_event(&self, event: DiagnosticEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[test]
fn wire_fingerprints_detect_long_content_schema_role_and_parameter_changes_without_retaining_them()
{
    let text = format!("{}private-tail", "x".repeat(5000));
    let original = json!({"model":"model", "messages":[{"role":"system","content":"private-system"},{"role":"user","content":text}],
        "tools":[{"name":"secret-tool","schema":{"type":"string"}}], "temperature":0.2});
    let fingerprint =
        |v: &Value| RequestFingerprint::from_bytes(&serde_json::to_vec(v).unwrap()).unwrap();
    let first = fingerprint(&original);
    let serialized = serde_json::to_string(&first).unwrap();
    for value in ["private", "secret-tool", "xxxxx"] {
        assert!(!serialized.contains(value));
    }
    let mut edited = original.clone();
    edited["messages"][1]["content"] = json!(format!("{}changed-tail", "x".repeat(5000)));
    let changed = fingerprint(&edited);
    assert_ne!(first.messages[1], changed.messages[1]);
    assert_eq!(first.system, changed.system);
    assert_eq!(first.tools, changed.tools);
    assert_eq!(first.parameters, changed.parameters);
    edited = original.clone();
    edited["messages"][1]["role"] = json!("assistant");
    assert_ne!(first.messages[1], fingerprint(&edited).messages[1]);
    edited = original.clone();
    edited["tools"][0]["schema"]["type"] = json!("number");
    assert_ne!(first.tools, fingerprint(&edited).tools);
    edited = original.clone();
    edited["temperature"] = json!(0.3);
    assert_ne!(first.parameters, fingerprint(&edited).parameters);
    edited = original.clone();
    edited["messages"]
        .as_array_mut()
        .unwrap()
        .push(json!({"role":"assistant","content":"new"}));
    assert_eq!(
        &fingerprint(&edited).messages[..first.messages.len()],
        &first.messages
    );
}

#[test]
fn response_projection_preserves_conflicting_zero_counters_and_rejects_arbitrary_fields() {
    let observer = Arc::new(Observer::default());
    let request = reqwest::Client::new()
        .post("http://user:password@localhost/private?api_key=secret")
        .bearer_auth("credential-header")
        .json(&json!({"messages":[]}))
        .build()
        .unwrap();
    let trace =
        AttemptTrace::start(observer.clone(), &request, "test", CancellationToken::new()).unwrap();
    let frame = json!({"model":"upstream-model", "id":"response-1", "usage": {
        "prompt_tokens":1000, "cache_read_input_tokens":0, "prompt_tokens_details":{"cached_tokens":800,"api_key":"hidden"},
        "cached_tokens":0, "api_key":"hidden", "secret_numeric":123456, "completion_tokens":"not-a-number" }, "content":"private"});
    trace.headers(200, Some("header-1"));
    trace.payload(&frame.to_string());
    trace.payload(&frame.to_string());
    trace.payload(
        &json!({"model":"sk-do-not-store-this-secret", "id":"contains spaces"}).to_string(),
    );
    drop(trace);
    let events = observer.0.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e.data, DiagnosticData::Metadata { .. }))
            .count(),
        1
    );
    let encoded = serde_json::to_string(&*events).unwrap();
    for value in [
        "password",
        "private",
        "secret",
        "api_key",
        "credential",
        "not-a-number",
        "hidden",
    ] {
        assert!(!encoded.contains(value), "{value}");
    }
    let DiagnosticData::Metadata { raw_usage, .. } = &events[2].data else {
        panic!()
    };
    assert_eq!(raw_usage["cache_read_input_tokens"], 0);
    assert_eq!(raw_usage["prompt_tokens_details"]["cached_tokens"], 800);
    assert_eq!(raw_usage["cached_tokens"], 0);
}

#[test]
fn bounded_message_list_still_hashes_the_complete_wire_and_endpoint_does_not_retain_credentials() {
    let body = json!({"messages":(0..MAX_MESSAGES+1).map(|n|json!({"role":"user","content":n})).collect::<Vec<_>>()});
    let first = RequestFingerprint::from_bytes(&serde_json::to_vec(&body).unwrap()).unwrap();
    assert!(first.messages_truncated);
    assert_eq!(first.message_count, MAX_MESSAGES + 1);
    assert_eq!(first.messages.len(), MAX_MESSAGES);
    let mut changed = body;
    changed["messages"][MAX_MESSAGES]["content"] = json!("changed beyond retained list");
    let second = RequestFingerprint::from_bytes(&serde_json::to_vec(&changed).unwrap()).unwrap();
    assert_eq!(first.messages, second.messages);
    assert_ne!(first.wire, second.wire);
    let observer = Arc::new(Observer::default());
    for url in [
        "http://user:password@localhost/v1/chat?api_key=secret",
        "http://localhost/v1/chat?api_key=changed",
        "http://localhost/v2/chat",
    ] {
        let request = reqwest::Client::new()
            .post(url)
            .json(&json!({"messages":[]}))
            .build()
            .unwrap();
        drop(
            AttemptTrace::start(observer.clone(), &request, "test", CancellationToken::new())
                .unwrap(),
        );
    }
    let hashes: Vec<_> = observer
        .0
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match &e.data {
            DiagnosticData::Request {
                endpoint_sha256, ..
            } => Some(endpoint_sha256.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(hashes[0], hashes[1]);
    assert_ne!(hashes[1], hashes[2]);
}

struct Transport {
    body: String,
    requests: Mutex<Vec<Vec<u8>>>,
    failures: std::sync::atomic::AtomicUsize,
}
#[async_trait]
impl HttpTransport for Transport {
    async fn send(
        &self,
        request: HttpRequest,
        cancellation: CancellationToken,
    ) -> ModelResult<HttpResponse> {
        self.requests
            .lock()
            .unwrap()
            .push(request.request.body().unwrap().as_bytes().unwrap().to_vec());
        if self
            .failures
            .fetch_update(
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
                |n| n.checked_sub(1),
            )
            .is_ok()
        {
            return Err(ModelError::transport(
                crate::TransportErrorKind::Connection,
                None::<&str>,
            ));
        }
        Ok(HttpResponse::new(
            200,
            Some("header-1".into()),
            Box::pin(futures::stream::iter(vec![Ok(self
                .body
                .as_bytes()
                .to_vec())])),
            cancellation,
        ))
    }
}
fn sse(value: Value) -> String {
    format!("data: {value}\n\n")
}
fn body(protocol: usize) -> String {
    match protocol {
        0 => format!("{}data: [DONE]\n\n",sse(json!({"id":"response-1","model":"upstream-model","choices":[{"delta":{"content":"answer"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":1000,"completion_tokens":1,"prompt_tokens_details":{"cached_tokens":800},"cache_read_input_tokens":0}}))),
        1 => sse(json!({"type":"response.completed","response":{"id":"response-1","model":"upstream-model","status":"completed","output":[],
            "usage":{"input_tokens":1000,"output_tokens":1,"input_tokens_details":{"cached_tokens":800}}}})),
        _ => [sse(json!({"type":"message_start","message":{"id":"response-1","model":"upstream-model","role":"assistant","content":[],"usage":{"input_tokens":200,"cache_read_input_tokens":800,"output_tokens":0}}})),
            sse(json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}})),
            sse(json!({"type":"message_stop"}))].concat(),
    }
}
fn model(
    protocol: usize,
    runtime: ModelRuntimeConfig,
    transport: Arc<Transport>,
) -> Box<dyn Model> {
    let endpoint = url::Url::parse("http://localhost/v1").unwrap();
    match protocol {
        0 => Box::new(OpenAiModel::with_transport(
            OpenAiConfig::new(endpoint, "credential-header", "configured-model")
                .with_runtime(runtime),
            transport,
        )),
        1 => Box::new(OpenAiResponsesModel::with_transport(
            OpenAiResponsesConfig::new(endpoint, "credential-header", "configured-model")
                .with_runtime(runtime),
            transport,
        )),
        _ => Box::new(AnthropicModel::with_transport(
            AnthropicConfig::new(endpoint, "credential-header", "configured-model")
                .with_runtime(runtime),
            transport,
        )),
    }
}
#[tokio::test]
async fn all_protocols_observe_exact_wire_and_raw_metadata_without_changing_events_or_prepared_execution()
 {
    for protocol in 0..3 {
        let observer = Arc::new(Observer::default());
        let transport = Arc::new(Transport {
            body: body(protocol),
            requests: Mutex::default(),
            failures: 0.into(),
        });
        let request = ModelRequest::new(vec![
            ModelMessage::system_text("private-system"),
            ModelMessage::user_text("private-input"),
        ]);
        let plain = model(protocol, ModelRuntimeConfig::default(), transport.clone());
        let expected: Vec<_> = plain
            .stream(request.clone(), CancellationToken::new())
            .await
            .unwrap()
            .collect()
            .await;
        let observed = model(
            protocol,
            ModelRuntimeConfig::default().with_diagnostics_observer(observer.clone()),
            transport.clone(),
        );
        let prepared = observed.prepare_stream(request.clone()).unwrap();
        assert!(
            observer.0.lock().unwrap().is_empty(),
            "preparing must not produce effects"
        );
        let got: Vec<_> = prepared
            .start(CancellationToken::new())
            .unwrap()
            .collect()
            .await;
        assert_eq!(expected, got);
        let direct: Vec<_> = observed
            .stream(request, CancellationToken::new())
            .await
            .unwrap()
            .collect()
            .await;
        assert_eq!(expected, direct);
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests[0], requests[1]);
        assert_eq!(requests[1], requests[2]);
        let events = observer.0.lock().unwrap();
        let fingerprint = events
            .iter()
            .find_map(|e| match &e.data {
                DiagnosticData::Request { fingerprint, .. } => Some(fingerprint),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            fingerprint.wire.sha256,
            format!("{:x}", Sha256::digest(&requests[0]))
        );
        assert!(events.iter().any(|e| matches!(&e.data,DiagnosticData::Metadata {model:Some(model),raw_usage,..} if model=="upstream-model" && !raw_usage.is_empty())));
        assert_eq!(events.iter().filter(|e| matches!(&e.data,DiagnosticData::Finished {outcome,usage:Some(_),..} if outcome=="completed")).count(),2);
        let encoded = serde_json::to_string(&*events).unwrap();
        for value in ["private-system", "private-input", "credential-header"] {
            assert!(!encoded.contains(value));
        }
    }
}

#[tokio::test]
async fn retries_have_separate_attempts_and_cancellation_still_stops_before_sending() {
    let observer = Arc::new(Observer::default());
    let transport = Arc::new(Transport {
        body: body(0),
        requests: Mutex::default(),
        failures: 1.into(),
    });
    let runtime = ModelRuntimeConfig::default()
        .with_diagnostics_observer(observer.clone())
        .with_retry(
            RetryConfig::default()
                .with_base_delay(std::time::Duration::ZERO)
                .with_jitter(false),
        );
    let model = model(0, runtime, transport.clone());
    let request = ModelRequest::new(vec![ModelMessage::user_text("private-input")]);
    let events: Vec<_> = model
        .stream(request.clone(), CancellationToken::new())
        .await
        .unwrap()
        .collect()
        .await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Ok(ModelStreamEvent::Completed(_))))
    );
    let records = observer.0.lock().unwrap();
    let ids: Vec<_> = records
        .iter()
        .filter(|e| matches!(e.data, DiagnosticData::Request { .. }))
        .map(|e| e.attempt_id.clone())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    assert!(
        records
            .iter()
            .any(|e| matches!(&e.data,DiagnosticData::Finished{outcome,..}if outcome=="error"))
    );
    drop(records);
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(
        model
            .stream(request, cancel)
            .await
            .err()
            .unwrap()
            .is_cancelled()
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn dropped_stream_records_incomplete_attempt_and_observer_panic_does_not_break_model() {
    struct Panics;
    impl DiagnosticsObserver for Panics {
        fn on_event(&self, _: DiagnosticEvent) {
            panic!("observer only")
        }
    }
    let transport = Arc::new(Transport {
        body: body(0),
        requests: Mutex::default(),
        failures: 0.into(),
    });
    let panicking = model(
        0,
        ModelRuntimeConfig::default().with_diagnostics_observer(Arc::new(Panics)),
        transport.clone(),
    );
    assert!(
        panicking
            .complete(
                ModelRequest::new(vec![ModelMessage::user_text("hi")]),
                CancellationToken::new()
            )
            .await
            .is_ok()
    );
    struct Hanging;
    #[async_trait]
    impl HttpTransport for Hanging {
        async fn send(
            &self,
            _: HttpRequest,
            cancellation: CancellationToken,
        ) -> ModelResult<HttpResponse> {
            let chunk = sse(json!({"id":"response-2","choices":[{"delta":{"content":"partial"}}]}));
            let body = futures::stream::iter(vec![Ok(chunk.into_bytes())])
                .chain(futures::stream::pending());
            Ok(HttpResponse::new(200, None, Box::pin(body), cancellation))
        }
    }
    let observer = Arc::new(Observer::default());
    let model = OpenAiModel::with_transport(
        OpenAiConfig::new(
            url::Url::parse("http://localhost/v1").unwrap(),
            "fixture",
            "model",
        )
        .with_runtime(ModelRuntimeConfig::default().with_diagnostics_observer(observer.clone())),
        Arc::new(Hanging),
    );
    let mut stream = model
        .stream(
            ModelRequest::new(vec![ModelMessage::user_text("hi")]),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(stream.next().await.unwrap().is_ok());
    drop(stream);
    tokio::time::timeout(std::time::Duration::from_secs(2),async {
        loop {
            if observer.0.lock().unwrap().iter().any(|e|matches!(&e.data,DiagnosticData::Finished{outcome,..} if outcome=="cancelled"||outcome=="dropped")){break;}
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
}
