//! Local model history. Display summaries never serve as execution history.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rcode_agent::{
    AgentError, ContentBlock, HistoryRecorder, Message, MessageContent, MessageRole,
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

fn path(root: &Path, id: &str) -> PathBuf {
    root.join("model-history")
        .join(format!("{:x}.jsonl", Sha256::digest(id.as_bytes())))
}

pub(super) struct Recorder {
    path: PathBuf,
    model: String,
    writing: tokio::sync::Mutex<()>,
}

impl Recorder {
    pub fn new(root: &Path, id: &str, model: &str) -> Self {
        Self {
            path: path(root, id),
            model: model.to_owned(),
            writing: tokio::sync::Mutex::new(()),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Record {
    version: u8,
    model: String,
    message: Message,
}

#[async_trait::async_trait]
impl HistoryRecorder for Recorder {
    async fn record(&self, message: &Message) -> Result<(), AgentError> {
        let mut bytes = serde_json::to_vec(&Record {
            version: 1,
            model: self.model.clone(),
            message: message.clone(),
        })
        .map_err(|e| AgentError::Host(e.to_string()))?;
        bytes.push(b'\n');
        let _writing = self.writing.lock().await;
        let write = async {
            tokio::fs::create_dir_all(self.path.parent().expect("history parent")).await?;
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .await?;
            file.write_all(&bytes).await?;
            file.flush().await?;
            file.sync_data().await
        }
        .await;
        write.map_err(|e| AgentError::Host(format!("cannot persist model history: {e}")))
    }
}

/// A torn final line is ignored. Completed calls are replayed as data; calls
/// without recorded results get an explicit interruption result, never rerun.
pub(super) async fn load(
    root: &Path,
    id: &str,
    model: &str,
) -> Result<Option<Vec<Message>>, String> {
    let file = match tokio::fs::File::open(path(root, id)).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read model history: {error}")),
    };
    let mut reader = tokio::io::BufReader::new(file);
    let mut records = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        if reader
            .read_line(&mut line)
            .await
            .map_err(|e| e.to_string())?
            == 0
        {
            break;
        }
        if !line.ends_with('\n') {
            break;
        }
        let mut record = serde_json::from_str::<Record>(&line)
            .map_err(|e| format!("invalid model history: {e}"))?;
        if record.version != 1 {
            return Err("unsupported model history version".into());
        }
        // Provider reasoning/signatures belong to the model that produced them.
        // Keep observable work when switching models, not foreign thinking.
        if record.model != model {
            if let MessageContent::Blocks(blocks) = &mut record.message.content {
                blocks.retain(|block| !matches!(block, ContentBlock::Reasoning { .. }));
            }
        }
        records.push(record.message);
    }
    if records.is_empty() {
        return Ok(None);
    }
    complete_pairs(records).map(Some)
}

/// Restore only planning state from excluded groups, never their tool bodies.
pub(super) async fn todo_checkpoint(root: &Path, ids: &[String], model: &str) -> Result<Option<Message>, String> {
    let mut groups = Vec::new();
    for id in ids.iter().rev() {
        if let Some(history) = load(root, id, model).await? {
            let has_list = rcode_agent::todo::State::from_history(&history).current.is_some();
            groups.push(history);
            if has_list { break; }
        }
    }
    let history = groups.into_iter().rev().flatten().collect::<Vec<_>>();
    let state = rcode_agent::todo::State::from_history(&history);
    Ok(state.current.as_ref().map(|_| state.record("checkpoint")))
}

fn complete_pairs(records: Vec<Message>) -> Result<Vec<Message>, String> {
    let mut messages = Vec::new();
    let mut calls = Vec::new();
    let mut results = HashMap::new();
    let flush = |messages: &mut Vec<Message>,
                 calls: &mut Vec<String>,
                 results: &mut HashMap<String, ContentBlock>| {
        if !calls.is_empty() {
            let blocks = calls.drain(..).map(|id| results.remove(&id).unwrap_or_else(|| ContentBlock::ToolResult {
                tool_use_id: id,
                content: "Execution was interrupted before a result was recorded. Inspect current state before retrying any side effect.".into(),
                is_error: true,
            })).collect();
            messages.push(Message::tool_results(blocks));
        }
    };
    for message in records {
        if message.role == MessageRole::Tool {
            let MessageContent::Blocks(blocks) = message.content else {
                return Err("unstructured history tool result".into());
            };
            for block in blocks {
                let ContentBlock::ToolResult { tool_use_id, .. } = &block else {
                    return Err("invalid history tool block".into());
                };
                if !calls.contains(tool_use_id) || results.contains_key(tool_use_id) {
                    return Err("unpaired or duplicate history tool result".into());
                }
                results.insert(tool_use_id.clone(), block);
            }
        } else {
            flush(&mut messages, &mut calls, &mut results);
            calls = message
                .content
                .tool_calls()
                .into_iter()
                .map(|(id, _, _)| id)
                .collect();
            let unique: HashSet<_> = calls.iter().collect();
            if unique.len() != calls.len() {
                return Err("duplicate history tool call".into());
            }
            messages.push(message);
        }
    }
    flush(&mut messages, &mut calls, &mut results);
    Ok(messages)
}

#[cfg(test)]
#[path = "native_history_tests.rs"]
mod tests;
