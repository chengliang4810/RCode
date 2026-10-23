//! Optional local metadata log. Bounded background I/O never gates model execution.
use std::{
    collections::{HashMap, VecDeque},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::{SystemTime, UNIX_EPOCH},
};

use rcode_model::diagnostics::{DiagnosticData, DiagnosticEvent, DiagnosticsObserver};
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 1024 * 1024;
const QUEUE_CAPACITY: usize = 64;

#[derive(Default, Deserialize)]
#[serde(default)]
struct Config {
    enabled: bool,
}

enum Command {
    Record(Vec<u8>),
    #[cfg(test)]
    Flush(mpsc::Sender<()>),
}
struct Sink {
    sender: mpsc::SyncSender<Command>,
}
impl Sink {
    fn open(root: &Path) -> Option<Arc<Self>> {
        let root = root.canonicalize().ok()?;
        // One process-lifetime writer per data root, including between consecutive runs.
        static SINKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Sink>>>> = OnceLock::new();
        let mut sinks = SINKS.get_or_init(Mutex::default).lock().ok()?;
        if let Some(sink) = sinks.get(&root) {
            return Some(sink.clone());
        }
        let directory = root.join("request-diagnostics");
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        std::thread::Builder::new().name("rcode-request-diagnostics".into()).spawn(move || {
            let mut warned = false;
            while let Ok(command) = receiver.recv() {
                match command {
                    Command::Record(bytes) => if let Err(error) = append(&directory, &bytes, MAX_FILE_BYTES) {
                        if !warned { tracing::warn!(%error, "request diagnostics write failed; model execution continues"); warned = true; }
                    },
                    #[cfg(test)]
                    Command::Flush(done) => { let _ = done.send(()); }
                }
            }
        }).ok()?;
        let sink = Arc::new(Self { sender });
        sinks.insert(root, sink.clone());
        Some(sink)
    }

    fn record(&self, value: Value) {
        let Ok(mut bytes) = serde_json::to_vec(&value) else {
            return;
        };
        if bytes.len() > MAX_RECORD_BYTES {
            tracing::warn!("oversized request diagnostic skipped");
            return;
        }
        bytes.push(b'\n');
        if self.sender.try_send(Command::Record(bytes)).is_err() {
            tracing::warn!("request diagnostics queue unavailable; record skipped");
        }
    }

    #[cfg(test)]
    fn flush(&self) {
        let (tx, rx) = mpsc::channel();
        self.sender.send(Command::Flush(tx)).unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    }
}

fn append(directory: &Path, bytes: &[u8], limit: u64) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join("requests.jsonl");
    if std::fs::metadata(&path)
        .map(|m| m.len() + bytes.len() as u64 > limit)
        .unwrap_or(false)
    {
        let oldest = directory.join("requests.2.jsonl");
        match std::fs::remove_file(&oldest) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let recent = directory.join("requests.1.jsonl");
        if recent.exists() {
            std::fs::rename(&recent, &oldest)?;
        }
        std::fs::rename(&path, recent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.flush()
}

pub(crate) struct SessionTrace {
    sink: Arc<Sink>,
    session_id: String,
    assistant_message_id: String,
    completed: Arc<Mutex<VecDeque<String>>>,
}
impl SessionTrace {
    pub fn open(root: &Path, session_id: &str, assistant_message_id: &str) -> Option<Arc<Self>> {
        let config: Config =
            serde_json::from_slice(&std::fs::read(root.join("request-diagnostics.json")).ok()?)
                .ok()?;
        if !config.enabled {
            return None;
        }
        Some(Arc::new(Self {
            sink: Sink::open(root)?,
            session_id: session_id.into(),
            assistant_message_id: assistant_message_id.into(),
            completed: Arc::default(),
        }))
    }
    pub fn begin(&self, purpose: &str) -> Arc<dyn DiagnosticsObserver> {
        Arc::new(CallObserver {
            sink: self.sink.clone(),
            session_id: self.session_id.clone(),
            assistant_message_id: self.assistant_message_id.clone(),
            call_id: uuid::Uuid::new_v4().to_string(),
            purpose: purpose.into(),
            attempts: Mutex::default(),
            completed: self.completed.clone(),
        })
    }
    /// Completed events can queue ahead of transcript consumption. Preserve their order.
    pub fn link_model_call(&self, id: &str) {
        let call_id = self
            .completed
            .lock()
            .ok()
            .and_then(|mut queue| queue.pop_front());
        if let Some(call_id) = call_id {
            self.sink.record(
                json!({ "version": 1, "timestampMs": timestamp(), "sessionId": self.session_id,
                "assistantMessageId": self.assistant_message_id, "callId": call_id,
                "kind": "model_call_link", "modelCallId": id }),
            );
        }
    }
}

struct CallObserver {
    sink: Arc<Sink>,
    session_id: String,
    assistant_message_id: String,
    call_id: String,
    purpose: String,
    attempts: Mutex<HashMap<String, usize>>,
    completed: Arc<Mutex<VecDeque<String>>>,
}
impl DiagnosticsObserver for CallObserver {
    fn on_event(&self, event: DiagnosticEvent) {
        let number = if let Ok(mut attempts) = self.attempts.lock() {
            let next = attempts.len() + 1;
            *attempts.entry(event.attempt_id.clone()).or_insert(next)
        } else {
            return;
        };
        if matches!(&event.data, DiagnosticData::Finished { outcome, .. } if outcome == "completed")
        {
            if let Ok(mut completed) = self.completed.lock() {
                completed.push_back(self.call_id.clone());
            }
        }
        let Ok(mut record) = serde_json::to_value(event) else {
            return;
        };
        let Some(record) = record.as_object_mut() else {
            return;
        };
        record.insert("version".into(), json!(1));
        record.insert("timestampMs".into(), json!(timestamp()));
        record.insert("sessionId".into(), json!(self.session_id));
        record.insert(
            "assistantMessageId".into(),
            json!(self.assistant_message_id),
        );
        record.insert("callId".into(), json!(self.call_id));
        record.insert("purpose".into(), json!(self.purpose));
        record.insert("attempt".into(), json!(number));
        self.sink.record(Value::Object(record.clone()));
    }
}
fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests;
