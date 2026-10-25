//! Authenticated engine RPC for viewers that connect directly to one host.
//!
//! The host owns the registry and session documents. Viewers only import its
//! replicas; mutations use typed RPCs and commands retain their original ids.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use rcode_doc::{RegistryDoc, SessionCommandEntry, WorkspaceState};
use rcode_proto::{
    Chat, Device, Session, SidebarPinChange, SidebarPreferences, SidebarPreferencesState,
    SidebarSectionChange, Space,
};
use rcode_rpc::{RpcClient, methods};
use rcode_sync::DocsStore;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::client::ClientInner;
use crate::error::{ClientError, Result};
use crate::events::ClientEvent;
use crate::live::b64;
use crate::session::SessionCore;
use crate::{lock, now_ms, read, write};

const RETRY: Duration = Duration::from_secs(1);
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const WATCHES: &[&str] = &[
    methods::WATCH_DEVICES,
    methods::WATCH_SPACES,
    methods::WATCH_CHATS,
    methods::WATCH_SESSIONS,
    methods::WATCH_SIDEBAR_PREFERENCES,
];

#[derive(Default)]
struct Mirror {
    devices: Vec<Device>,
    spaces: Vec<Space>,
    chats: Vec<Chat>,
    sessions: Vec<Session>,
    preferences: Option<SidebarPreferences>,
    pending_chats: HashMap<String, Chat>,
}

struct Request {
    method: &'static str,
    params: Value,
    chat_id: Option<String>,
    command_id: Option<String>,
}

pub(crate) struct DirectBackend {
    client: RwLock<Option<Arc<RpcClient>>>,
    requests: mpsc::Sender<Request>,
    incoming: Mutex<Option<mpsc::Receiver<Request>>>,
    mirror: Mutex<Mirror>,
    watched: Mutex<HashMap<String, std::sync::Weak<SessionCore>>>,
    host_id: Mutex<Option<String>>,
    staged: Mutex<HashMap<String, (String, Vec<u8>)>>,
    pub(crate) store: Arc<DocsStore>,
    cancel: CancellationToken,
}

pub(crate) fn websocket_url(value: &str) -> Result<String> {
    let value = value.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(value)
        .map_err(|_| ClientError::InvalidArgument("A server URL is required".into()))?;
    if parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ClientError::InvalidArgument(
            "The server URL must not contain credentials or query parameters".into(),
        ));
    }
    match parsed.scheme() {
        "http" => Ok(format!("ws://{}", &value[7..])),
        "https" => Ok(format!("wss://{}", &value[8..])),
        "ws" | "wss" => Ok(value.into()),
        _ => Err(ClientError::InvalidArgument(
            "Use a ws, wss, http or https server URL".into(),
        )),
    }
}

impl DirectBackend {
    pub(crate) fn new(inner: &Arc<ClientInner>, store: DocsStore) -> Self {
        let (requests, incoming) = mpsc::channel(128);
        Self {
            client: RwLock::new(None),
            requests,
            incoming: Mutex::new(Some(incoming)),
            mirror: Mutex::default(),
            watched: Mutex::default(),
            host_id: Mutex::default(),
            staged: Mutex::default(),
            store: Arc::new(store),
            cancel: inner.cancel.child_token(),
        }
    }

    pub(crate) fn connected(&self) -> bool {
        read(&self.client)
            .as_ref()
            .is_some_and(|c| c.is_connected())
    }

    pub(crate) fn reconnect_now(&self) {
        if let Some(client) = read(&self.client).as_ref() {
            client.reconnect_now();
        }
    }

    fn client(&self) -> Result<Arc<RpcClient>> {
        read(&self.client)
            .clone()
            .filter(|c| c.is_connected())
            .ok_or_else(|| ClientError::HostUnavailable("The server is disconnected".into()))
    }

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let client = self.client()?;
        tokio::select! {
            _ = self.cancel.cancelled() => Err(ClientError::Closed),
            result = tokio::time::timeout(CALL_TIMEOUT, client.call(method, params)) => {
                match result {
                    Ok(Ok(value)) => Ok(value),
                    Ok(Err(rcode_rpc::RpcError::UnknownMethod(name))) => Err(ClientError::Unsupported(name)),
                    Ok(Err(rcode_rpc::RpcError::Failed(reason) | rcode_rpc::RpcError::BadParams(reason) | rcode_rpc::RpcError::Capability(reason))) => Err(ClientError::HostError(reason)),
                    _ => Err(ClientError::HostUnavailable("The server did not confirm the operation; check its state before retrying".into())),
                }
            }
        }
    }

    pub(crate) fn enqueue(
        &self,
        method: &'static str,
        params: Value,
        chat_id: Option<String>,
        command_id: Option<String>,
    ) -> Result<()> {
        self.client()?;
        self.requests
            .try_send(Request {
                method,
                params,
                chat_id,
                command_id,
            })
            .map_err(|_| ClientError::HostUnavailable("The server request queue is busy".into()))
    }

    pub(crate) fn command(&self, chat_id: &str, entry: &SessionCommandEntry) -> Result<()> {
        self.enqueue(
            methods::RELAY_COMMAND,
            json!({"chatId":chat_id,"entry":entry}),
            Some(chat_id.into()),
            Some(entry.id.clone()),
        )
    }

    pub(crate) fn stage(&self, reference: &str, name: &str, data: &[u8]) -> Result<()> {
        self.client()?;
        let mut staged = lock(&self.staged);
        let used: usize = staged.values().map(|(_, bytes)| bytes.len()).sum();
        if used.saturating_add(data.len()) > 48 * 1024 * 1024 {
            return Err(ClientError::Storage(
                "Staged attachments exceed 48 MB; wait for pending sends to finish".into(),
            ));
        }
        staged.insert(reference.into(), (name.into(), data.into()));
        Ok(())
    }

    async fn upload_staged(&self, params: &mut Value) -> Result<()> {
        let mut refs = HashSet::new();
        collect_pending(params, &mut refs);
        for reference in refs {
            let item = lock(&self.staged).get(&reference).cloned();
            let Some((name, bytes)) = item else {
                return Err(ClientError::Storage(
                    "The staged attachment is no longer available".into(),
                ));
            };
            let upload_id = reference
                .trim_start_matches("pending://")
                .split('/')
                .next()
                .unwrap_or("");
            let chunks: Vec<&[u8]> = if bytes.is_empty() {
                vec![&[]]
            } else {
                bytes.chunks(510_000).collect()
            };
            for (seq, data) in chunks.into_iter().enumerate() {
                self.call(
                    methods::UPLOAD_CHUNK,
                    json!({"uploadId":upload_id,"seq":seq,"data":b64::encode(data)}),
                )
                .await?;
            }
            let result = self
                .call(
                    methods::UPLOAD_COMMIT,
                    json!({"uploadId":upload_id,"fileName":name}),
                )
                .await?;
            let path = result.get("path").and_then(Value::as_str).ok_or_else(|| {
                ClientError::HostError("The server did not return an attachment path".into())
            })?;
            replace_reference(params, &reference, path);
            lock(&self.staged).remove(&reference);
        }
        Ok(())
    }

    pub(crate) async fn upload(
        &self,
        name: &str,
        bytes: &[u8],
        progress: Option<crate::rpc::ProgressFn>,
    ) -> Result<String> {
        let id = crate::new_upload_id();
        let parts: Vec<&[u8]> = if bytes.is_empty() {
            vec![&[]]
        } else {
            bytes.chunks(510_000).collect()
        };
        for (seq, part) in parts.iter().enumerate() {
            self.call(
                methods::UPLOAD_CHUNK,
                json!({"uploadId":id,"seq":seq,"data":b64::encode(part)}),
            )
            .await?;
            if let Some(f) = &progress {
                f((seq + 1) as f64 / parts.len() as f64 * 0.99);
            }
        }
        let result = self
            .call(
                methods::UPLOAD_COMMIT,
                json!({"uploadId":id,"fileName":name}),
            )
            .await?;
        let path = result.get("path").and_then(Value::as_str).ok_or_else(|| {
            ClientError::HostError("The server did not return an attachment path".into())
        })?;
        if let Some(f) = progress {
            f(1.0);
        }
        Ok(path.into())
    }

    pub(crate) async fn read_attachment(&self, path: &str) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut offset = 0;
        loop {
            let value = self
                .call(
                    methods::READ_ATTACHMENT_CHUNK,
                    json!({"path":path,"offset":offset}),
                )
                .await?;
            bytes.extend(
                b64::decode(
                    value
                        .get("data")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )
                .ok_or_else(|| ClientError::HostError("Invalid attachment data".into()))?,
            );
            if bytes.len() > 24 * 1024 * 1024 {
                return Err(ClientError::HostError("Attachment exceeds 24 MB".into()));
            }
            if value.get("done").and_then(Value::as_bool).unwrap_or(true) {
                return Ok(bytes);
            }
            let next = value.get("nextOffset").and_then(Value::as_u64).unwrap_or(0);
            if next <= offset {
                return Err(ClientError::HostError(
                    "Attachment read made no progress".into(),
                ));
            }
            offset = next;
        }
    }

    pub(crate) fn start(&self, inner: &Arc<ClientInner>) {
        let weak = Arc::downgrade(inner);
        let cancel = self.cancel.clone();
        crate::runtime::shared().spawn(async move {
            loop {
                let Some(inner) = weak.upgrade() else { return };
                let crate::config::Credentials::Direct { api_key } = &inner.credentials else { return };
                let api_key = api_key.clone();
                let Ok(url) = websocket_url(&inner.config.edge_url) else { return };
                drop(inner);
                let result = tokio::select! {
                    _ = cancel.cancelled() => return,
                    value = rcode_rpc::connect_ws_authenticated(&url, &api_key) => value,
                };
                let Some(inner) = weak.upgrade() else { return };
                let Some(direct) = inner.direct() else { return };
                match result {
                    Ok(client) => {
                        *write(&direct.client) = Some(Arc::new(client));
                        if let Ok(value) = direct.call(methods::LOCAL_DEVICE, json!({})).await {
                            *lock(&direct.host_id) = value.get("deviceId").and_then(Value::as_str).map(str::to_owned);
                        }
                        inner.recompute_connectivity();
                        return;
                    }
                    Err(error) => {
                        let reason = error.to_string();
                        if reason.contains("authentication rejected") {
                            inner.events.ordered(ClientEvent::AuthExpired { reason: "The server rejected this API key".into() });
                            return;
                        }
                    }
                }
                drop(inner);
                tokio::select! { _ = cancel.cancelled() => return, _ = tokio::time::sleep(RETRY) => {} }
            }
        });
        for method in WATCHES {
            self.watch_registry(inner, method);
        }
        let Some(mut requests) = lock(&self.incoming).take() else {
            return;
        };
        let weak = Arc::downgrade(inner);
        let cancel = self.cancel.clone();
        crate::runtime::shared().spawn(async move {
            loop {
                let request = tokio::select! { _ = cancel.cancelled() => return, value = requests.recv() => value };
                let Some(mut request) = request else { return };
                let Some(inner) = weak.upgrade() else { return };
                let Some(direct) = inner.direct() else { return };
                // Each submitted mutation is sent once. A lost acknowledgement is
                // resolved by the authoritative watches, never by blind replay.
                let result = match direct.upload_staged(&mut request.params).await {
                    Ok(()) => direct.call(request.method, request.params).await,
                    Err(error) => Err(error),
                };
                if let Err(error) = result {
                    if let Some(core) = request.chat_id.and_then(|id| inner.session_core(&id)) {
                        core.direct_failure(request.command_id.as_deref(), &error.to_string());
                    }
                    tracing::warn!(method = request.method, "direct host operation was not confirmed");
                }
            }
        });
    }

    fn watch_registry(&self, inner: &Arc<ClientInner>, method: &'static str) {
        let weak = Arc::downgrade(inner);
        let cancel = self.cancel.clone();
        crate::runtime::shared().spawn(async move {
            loop {
                let client = weak.upgrade().and_then(|i| i.direct().and_then(|d| d.client().ok()));
                if let Some(client) = client {
                    if let Ok(mut stream) = client.subscribe(method, json!({})).await {
                        loop {
                            let next = tokio::select! { _ = cancel.cancelled() => return, value = stream.recv() => value };
                            let Some(value) = next else { break };
                            let Some(inner) = weak.upgrade() else { return };
                            let Some(direct) = inner.direct() else { return };
                            if direct.apply_registry(&inner, method, value).is_err() {
                                tracing::warn!(method, "invalid direct registry frame");
                            }
                        }
                    }
                }
                if let Some(inner) = weak.upgrade() { inner.recompute_connectivity(); }
                tokio::select! { _ = cancel.cancelled() => return, _ = tokio::time::sleep(RETRY) => {} }
            }
        });
    }

    fn apply_registry(&self, inner: &Arc<ClientInner>, method: &str, value: Value) -> Result<()> {
        let invalid =
            |_: serde_json::Error| ClientError::HostError("Invalid registry frame".into());
        let mut mirror = lock(&self.mirror);
        match method {
            methods::WATCH_DEVICES => {
                mirror.devices = serde_json::from_value(value).map_err(invalid)?
            }
            methods::WATCH_SPACES => {
                mirror.spaces = serde_json::from_value(value).map_err(invalid)?
            }
            methods::WATCH_CHATS => {
                mirror.chats = serde_json::from_value(value).map_err(invalid)?;
                let received: HashSet<_> = mirror.chats.iter().map(|row| row.id.clone()).collect();
                mirror.pending_chats.retain(|id, _| !received.contains(id));
            }
            methods::WATCH_SESSIONS => {
                mirror.sessions = serde_json::from_value(value).map_err(invalid)?
            }
            methods::WATCH_SIDEBAR_PREFERENCES => {
                let state: SidebarPreferencesState =
                    serde_json::from_value(value).map_err(invalid)?;
                mirror.preferences = Some(SidebarPreferences {
                    sections: state.sections,
                    pinned_session_ids: state.pinned_session_ids,
                });
            }
            _ => return Ok(()),
        }
        let mut doc = RegistryDoc::new(inner.config.device_id.clone());
        doc.set_local_only(true);
        for row in &mirror.devices {
            doc.upsert_device(row)?;
        }
        for row in &mirror.spaces {
            doc.upsert_space(row)?;
        }
        for row in &mirror.chats {
            doc.upsert_chat(row)?;
        }
        for row in mirror.pending_chats.values() {
            if !mirror.chats.iter().any(|existing| existing.id == row.id) {
                doc.upsert_chat(row)?;
            }
        }
        for row in &mirror.sessions {
            doc.upsert_session(row)?;
        }
        doc.reconcile_sidebar_pins(true)?;
        if let Some(prefs) = &mirror.preferences {
            for section in &prefs.sections {
                doc.change_sidebar_pin(&SidebarPinChange::Section {
                    change: SidebarSectionChange::Create {
                        id: section.id.clone(),
                        name: section.name.clone(),
                    },
                })?;
                doc.change_sidebar_pin(&SidebarPinChange::Section {
                    change: SidebarSectionChange::Collapse {
                        id: section.id.clone(),
                        collapsed: section.collapsed,
                    },
                })?;
                let mut previous = None;
                for session_id in &section.session_ids {
                    doc.change_sidebar_pin(&SidebarPinChange::Section {
                        change: SidebarSectionChange::Assign {
                            session_id: session_id.clone(),
                            section_id: Some(section.id.clone()),
                            after: previous.clone(),
                            before: None,
                        },
                    })?;
                    previous = Some(session_id.clone());
                }
            }
            let mut previous = None;
            for id in &prefs.pinned_session_ids {
                doc.change_sidebar_pin(&SidebarPinChange::Pin {
                    session_id: id.clone(),
                    after: previous.clone(),
                    before: None,
                })?;
                previous = Some(id.clone());
            }
        }
        let bytes = doc.to_bytes()?;
        inner.workspace.replace(doc);
        drop(mirror);
        let _ = self.store.save_snapshot(rcode_doc::REGISTRY_DOC_ID, &bytes);
        inner.synced.store(true, Ordering::Release);
        self.update_presence(inner);
        inner.recompute_connectivity();
        inner.recompute_workspace();
        Ok(())
    }

    pub(crate) fn update_presence(&self, inner: &Arc<ClientInner>) {
        if self.connected()
            && let Some(id) = lock(&self.host_id).as_ref()
        {
            inner.workspace.set_presence(id, now_ms());
        }
    }

    pub(crate) fn watch_session(&self, inner: &Arc<ClientInner>, core: &Arc<SessionCore>) {
        {
            let mut watched = lock(&self.watched);
            if watched
                .get(&core.chat_id)
                .and_then(std::sync::Weak::upgrade)
                .is_some_and(|existing| Arc::ptr_eq(&existing, core))
            {
                return;
            }
            watched.insert(core.chat_id.clone(), Arc::downgrade(core));
        }
        let weak = Arc::downgrade(core);
        let owner = Arc::downgrade(inner);
        let cancel = core.direct_cancel();
        crate::runtime::shared().spawn(async move {
            loop {
                let client = owner.upgrade().and_then(|i| i.direct().and_then(|d| d.client().ok()));
                let Some(core) = weak.upgrade() else { return };
                let chat_id = core.chat_id.clone();
                drop(core);
                if let Some(client) = client {
                    if let Ok(mut stream) = client.subscribe("WatchSessionReplica", json!({"chatId":chat_id})).await {
                        loop {
                            let value = tokio::select! { _ = cancel.cancelled() => return, value = stream.recv() => value };
                            let Some(value) = value else { break };
                            let Some(core) = weak.upgrade() else { return };
                            let encoded = value.get("snapshot").or_else(|| value.get("update")).and_then(Value::as_str);
                            if let Some(bytes) = encoded.and_then(b64::decode) {
                                if core.doc().doc().import(&bytes).is_ok() {
                                    core.set_hydrated();
                                    core.schedule_refresh();
                                }
                            }
                        }
                    }
                }
                tokio::select! { _ = cancel.cancelled() => return, _ = tokio::time::sleep(RETRY) => {} }
            }
        });
    }

    pub(crate) fn registry_written(
        &self,
        before: &WorkspaceState,
        after: &WorkspaceState,
    ) -> Result<()> {
        {
            let mut mirror = lock(&self.mirror);
            for chat in &after.chats {
                if !before.chats.iter().any(|row| row.id == chat.id) {
                    mirror.pending_chats.insert(chat.id.clone(), chat.clone());
                }
            }
        }
        for params in registry_mutations(before, after) {
            self.enqueue(methods::MUTATE, params, None, None)?;
        }
        Ok(())
    }

    pub(crate) fn persist_session(&self, core: &SessionCore) {
        if let Ok(bytes) = core.doc().export_snapshot() {
            let _ = self.store.save_snapshot(&core.chat_id, &bytes);
        }
    }

    pub(crate) fn stop(&self) {
        self.cancel.cancel();
        if let Some(client) = write(&self.client).take() {
            client.close();
        }
        lock(&self.staged).clear();
    }
}

fn collect_pending(value: &Value, out: &mut HashSet<String>) {
    match value {
        Value::String(text) if text.starts_with("pending://") => {
            out.insert(text.clone());
        }
        Value::Array(items) => {
            for item in items {
                collect_pending(item, out);
            }
        }
        Value::Object(items) => {
            for item in items.values() {
                collect_pending(item, out);
            }
        }
        _ => {}
    }
}

fn replace_reference(value: &mut Value, from: &str, to: &str) {
    match value {
        Value::String(text) => *text = text.replace(from, to),
        Value::Array(items) => {
            for item in items {
                replace_reference(item, from, to);
            }
        }
        Value::Object(items) => {
            for item in items.values_mut() {
                replace_reference(item, from, to);
            }
        }
        _ => {}
    }
}

fn registry_mutations(before: &WorkspaceState, after: &WorkspaceState) -> Vec<Value> {
    let mut operations = Vec::new();
    for chat in &after.chats {
        let old = before.chats.iter().find(|row| row.id == chat.id);
        let Some(old) = old else {
            operations.push(
                json!({"op":"createChat","chatId":chat.id,"deviceId":chat.device_id,
                "spaceId":chat.space_id,"config":chat.config,"cwd":chat.cwd,"branch":chat.branch}),
            );
            if let Some(title) = &chat.title {
                operations.push(json!({"op":"renameChat","chatId":chat.id,"title":title}));
            }
            continue;
        };
        for (changed, params) in [
            (
                old.title != chat.title,
                json!({"op":"renameChat","chatId":chat.id,"title":chat.title.clone().unwrap_or_default()}),
            ),
            (
                old.archived != chat.archived,
                json!({"op":"setChatArchived","chatId":chat.id,"archived":chat.archived}),
            ),
            (
                old.config != chat.config && chat.config.is_some(),
                json!({"op":"setChatConfig","chatId":chat.id,"config":chat.config}),
            ),
            (
                old.cwd != chat.cwd && chat.cwd.is_some(),
                json!({"op":"setChatCwd","chatId":chat.id,"cwd":chat.cwd}),
            ),
            (
                old.branch != chat.branch && chat.branch.is_some(),
                json!({"op":"setChatBranch","chatId":chat.id,"branch":chat.branch}),
            ),
            (
                old.last_seen_at != chat.last_seen_at,
                json!({"op":"markChatSeen","chatId":chat.id,"at":chat.last_seen_at.map(|t|t.timestamp_millis())}),
            ),
        ] {
            if changed {
                operations.push(params);
            }
        }
    }
    for chat in &before.chats {
        if !after.chats.iter().any(|row| row.id == chat.id) {
            operations.push(json!({"op":"deleteChat","chatId":chat.id}));
        }
    }
    for space in &after.spaces {
        match before.spaces.iter().find(|row| row.id == space.id) {
            None => operations.push(json!({"op":"createSpace","spaceId":space.id,"deviceId":space.device_id,"path":space.path,"name":space.name,"gitDetected":space.git_detected})),
            Some(old) if old.name != space.name => operations.push(json!({"op":"renameSpace","spaceId":space.id,"name":space.name})),
            _ => {}
        }
    }
    for space in &before.spaces {
        if !after.spaces.iter().any(|row| row.id == space.id) {
            operations.push(json!({"op":"deleteSpace","spaceId":space.id}));
        }
    }
    for device in &after.devices {
        if before
            .devices
            .iter()
            .any(|row| row.id == device.id && row.name != device.name)
        {
            operations.push(json!({"op":"renameDevice","deviceId":device.id,"name":device.name}));
        }
    }
    operations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_normalized_without_putting_credentials_in_the_url() {
        assert_eq!(
            websocket_url("https://host.example:27655/").unwrap(),
            "wss://host.example:27655"
        );
        assert_eq!(
            websocket_url("ws://127.0.0.1:27655").unwrap(),
            "ws://127.0.0.1:27655"
        );
        for url in [
            "https://user:secret@host",
            "https://host?token=secret",
            "ftp://host",
            "host",
        ] {
            assert!(websocket_url(url).is_err());
        }
    }
}
