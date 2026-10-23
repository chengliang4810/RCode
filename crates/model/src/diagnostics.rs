//! Opt-in diagnostics of the actual wire request. Authentication and content never leave here.
use std::{
    collections::BTreeMap,
    io::Write,
    sync::{Arc, Mutex},
    time::Instant,
};

use futures::StreamExt;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::{ModelError, ModelResult, ModelStream, ModelStreamEvent, TokenUsage};

const MAX_MESSAGES: usize = 4096;
const MAX_METADATA_UPDATES: usize = 32;

pub trait DiagnosticsObserver: Send + Sync {
    fn on_event(&self, event: DiagnosticEvent);
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticEvent {
    pub attempt_id: String,
    #[serde(flatten)]
    pub data: DiagnosticData,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DiagnosticData {
    Request {
        protocol: String,
        origin: String,
        endpoint_sha256: String,
        fingerprint: RequestFingerprint,
    },
    Headers {
        status: u16,
        request_id: Option<String>,
    },
    Metadata {
        source: String,
        model: Option<String>,
        response_id: Option<String>,
        raw_usage: BTreeMap<String, Value>,
    },
    Finished {
        outcome: String,
        duration_ms: u64,
        usage: Option<TokenUsage>,
        request_id: Option<String>,
        error_kind: Option<crate::ModelErrorCategory>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageFingerprint {
    pub role: String,
    #[serde(flatten)]
    pub content: Fingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestFingerprint {
    pub wire: Fingerprint,
    pub system: Fingerprint,
    pub tools: Fingerprint,
    pub parameters: Fingerprint,
    pub message_count: usize,
    pub messages_truncated: bool,
    pub messages: Vec<MessageFingerprint>,
}

struct HashWriter {
    digest: Sha256,
    bytes: u64,
}
impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.digest.update(bytes);
        self.bytes += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn hash(value: &impl Serialize) -> Fingerprint {
    let mut writer = HashWriter {
        digest: Sha256::new(),
        bytes: 0,
    };
    serde_json::to_writer(&mut writer, value).expect("diagnostic JSON values serialize");
    Fingerprint {
        sha256: format!("{:x}", writer.digest.finalize()),
        bytes: writer.bytes,
    }
}

impl RequestFingerprint {
    /// Hash unredacted JSON before any observation truncation, preserving array/wire order.
    /// Only digest/length pairs and fixed role labels are retained.
    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let body: Value = serde_json::from_slice(bytes).ok()?;
        let fields = body.as_object()?;
        let input: Vec<&Value> = match body.get("messages").or_else(|| body.get("input")) {
            Some(Value::Array(items)) => items.iter().collect(),
            Some(value) => vec![value],
            None => vec![],
        };
        let mut system = Vec::new();
        if let Some(value) = body.get("system").or_else(|| body.get("instructions")) {
            system.push(value);
        }
        let mut messages = Vec::new();
        for item in input {
            if matches!(
                item.get("role").and_then(Value::as_str),
                Some("system" | "developer")
            ) {
                system.push(item);
            }
            messages.push(item);
        }
        let parameters: BTreeMap<_, _> = fields
            .iter()
            .filter(|(key, _)| {
                !matches!(
                    key.as_str(),
                    "messages" | "input" | "system" | "instructions" | "tools"
                )
            })
            .collect();
        Some(Self {
            wire: Fingerprint {
                sha256: format!("{:x}", Sha256::digest(bytes)),
                bytes: bytes.len() as u64,
            },
            system: hash(&system),
            tools: hash(&body.get("tools")),
            parameters: hash(&parameters),
            message_count: messages.len(),
            messages_truncated: messages.len() > MAX_MESSAGES,
            messages: messages
                .into_iter()
                .take(MAX_MESSAGES)
                .map(|item| MessageFingerprint {
                    role: match item
                        .get("role")
                        .or_else(|| item.get("type"))
                        .and_then(Value::as_str)
                    {
                        Some(
                            role @ ("system"
                            | "developer"
                            | "user"
                            | "assistant"
                            | "tool"
                            | "function_call"
                            | "function_call_output"
                            | "reasoning"),
                        ) => role,
                        _ => "other",
                    }
                    .to_owned(),
                    content: hash(item),
                })
                .collect(),
        })
    }
}

// These identifiers are provider metadata, not arbitrary free text. Reject credential-like values.
fn identifier(value: Option<&str>) -> Option<String> {
    let value = value?;
    if value.is_empty()
        || value.len() > 256
        || value.to_ascii_lowercase().contains("sk-")
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
    {
        return None;
    }
    Some(value.to_owned())
}

fn usage_fields(value: &Value) -> BTreeMap<String, Value> {
    let mut result = BTreeMap::new();
    for key in [
        "prompt_tokens",
        "completion_tokens",
        "input_tokens",
        "output_tokens",
        "total_tokens",
        "prompt_cache_hit_tokens",
        "prompt_cache_miss_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
        "cached_tokens",
        "reasoning_tokens",
        "thinking_tokens",
    ] {
        if let Some(v) = value.get(key).filter(|v| v.is_number() || v.is_null()) {
            result.insert(key.into(), v.clone());
        }
    }
    for parent in [
        "prompt_tokens_details",
        "completion_tokens_details",
        "input_tokens_details",
        "output_tokens_details",
        "cache_creation",
    ] {
        let Some(details) = value.get(parent) else {
            continue;
        };
        let selected: BTreeMap<_, _> = [
            "cached_tokens",
            "reasoning_tokens",
            "audio_tokens",
            "text_tokens",
            "accepted_prediction_tokens",
            "rejected_prediction_tokens",
            "ephemeral_5m_input_tokens",
            "ephemeral_1h_input_tokens",
        ]
        .into_iter()
        .filter_map(|key| {
            details
                .get(key)
                .filter(|v| v.is_number() || v.is_null())
                .map(|v| (key, v))
        })
        .collect();
        if !selected.is_empty() {
            result.insert(parent.into(), serde_json::to_value(selected).unwrap());
        }
    }
    result
}

struct TraceState {
    finished: bool,
    metadata: BTreeMap<String, Value>,
    updates: usize,
}
struct TraceInner {
    observer: Arc<dyn DiagnosticsObserver>,
    id: String,
    started: Instant,
    cancellation: CancellationToken,
    state: Mutex<TraceState>,
}
impl TraceInner {
    fn emit(&self, data: DiagnosticData) {
        // An optional observer must never change provider execution, even if it panics.
        let event = DiagnosticEvent {
            attempt_id: self.id.clone(),
            data,
        };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.observer.on_event(event)
        }));
    }
    fn finish(
        &self,
        outcome: &str,
        usage: Option<TokenUsage>,
        request_id: Option<&str>,
        error_kind: Option<crate::ModelErrorCategory>,
    ) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.finished {
            return;
        }
        state.finished = true;
        drop(state);
        self.emit(DiagnosticData::Finished {
            outcome: outcome.into(),
            duration_ms: self.started.elapsed().as_millis() as u64,
            usage,
            request_id: identifier(request_id),
            error_kind,
        });
    }
}
impl Drop for TraceInner {
    fn drop(&mut self) {
        self.finish(
            if self.cancellation.is_cancelled() {
                "cancelled"
            } else {
                "dropped"
            },
            None,
            None,
            None,
        );
    }
}

#[derive(Clone)]
pub(crate) struct AttemptTrace(Arc<TraceInner>);
impl AttemptTrace {
    pub(crate) fn start(
        observer: Arc<dyn DiagnosticsObserver>,
        request: &reqwest::Request,
        protocol: &str,
        cancellation: CancellationToken,
    ) -> Option<Self> {
        let fingerprint = RequestFingerprint::from_bytes(request.body()?.as_bytes()?)?;
        let mut endpoint = request.url().clone();
        let _ = endpoint.set_username("");
        let _ = endpoint.set_password(None);
        endpoint.set_query(None);
        endpoint.set_fragment(None);
        let trace = Self(Arc::new(TraceInner {
            observer,
            id: uuid::Uuid::new_v4().to_string(),
            started: Instant::now(),
            cancellation,
            state: Mutex::new(TraceState {
                finished: false,
                metadata: BTreeMap::new(),
                updates: 0,
            }),
        }));
        trace.0.emit(DiagnosticData::Request {
            protocol: protocol.into(),
            origin: request.url().origin().ascii_serialization(),
            endpoint_sha256: hash(&endpoint.as_str()).sha256,
            fingerprint,
        });
        Some(trace)
    }
    pub(crate) fn headers(&self, status: u16, request_id: Option<&str>) {
        self.0.emit(DiagnosticData::Headers {
            status,
            request_id: identifier(request_id),
        });
    }
    pub(crate) fn payload(&self, body: &str) {
        let Ok(value) = serde_json::from_str::<Value>(body) else {
            return;
        };
        for (source, object) in [
            ("root", Some(&value)),
            ("message", value.get("message")),
            ("response", value.get("response")),
            ("data", value.get("data")),
        ] {
            let Some(object) = object else {
                continue;
            };
            let model = identifier(object.get("model").and_then(Value::as_str));
            let response_id = identifier(object.get("id").and_then(Value::as_str));
            let raw_usage = object.get("usage").map(usage_fields).unwrap_or_default();
            if model.is_none() && response_id.is_none() && raw_usage.is_empty() {
                continue;
            }
            let data = DiagnosticData::Metadata {
                source: source.into(),
                model,
                response_id,
                raw_usage,
            };
            let serialized = serde_json::to_value(&data).unwrap();
            let Ok(mut state) = self.0.state.lock() else {
                return;
            };
            if state.updates >= MAX_METADATA_UPDATES
                || state.metadata.get(source) == Some(&serialized)
            {
                continue;
            }
            state.metadata.insert(source.into(), serialized);
            state.updates += 1;
            drop(state);
            self.0.emit(data);
        }
    }
    pub(crate) fn failed(&self, error: &ModelError) {
        self.0.finish(
            if error.is_cancelled() {
                "cancelled"
            } else {
                "error"
            },
            None,
            error.request_id(),
            Some(error.diagnostic().category()),
        );
    }
    pub(crate) fn stream(
        &self,
        stream: ModelStream,
        cancellation: CancellationToken,
    ) -> ModelStream {
        let trace = self.clone();
        ModelStream::with_cancellation(
            stream.map(move |event: ModelResult<ModelStreamEvent>| {
                match &event {
                    Ok(ModelStreamEvent::Completed(response)) => trace.0.finish(
                        "completed",
                        response.usage().cloned(),
                        response.request_id(),
                        None,
                    ),
                    Ok(ModelStreamEvent::Interrupted { error, .. }) | Err(error) => {
                        trace.failed(error)
                    }
                    _ => {}
                }
                event
            }),
            cancellation.child_token(),
        )
    }
}

#[cfg(test)]
mod tests;
