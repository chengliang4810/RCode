//! Device-local remote engine connections. Only the selected host's own rows
//! enter the local view, so two mutually connected engines cannot relay their
//! inventories back and forth.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, MutexGuard, Weak,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use futures::future::join_all;
use rcode_rpc::{RpcClient, RpcError, connect_ws_authenticated, methods};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;

const CATALOGS: [&str; 4] = [
    methods::WATCH_DEVICES,
    methods::WATCH_CHATS,
    methods::WATCH_SPACES,
    methods::WATCH_SESSIONS,
];
const MAX_CONFIG_BYTES: u64 = 512 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostEdit {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostView {
    pub id: String,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub device_id: Option<String>,
    pub connected: bool,
    pub error: Option<String>,
}

// This is a credential file: neither it nor its records implement Debug.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct HostRecord {
    id: String,
    name: String,
    url: String,
    api_key: String,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
    #[serde(default)]
    device_id: Option<String>,
}

fn enabled_by_default() -> bool {
    true
}

#[derive(Default, Deserialize, Serialize)]
struct Config {
    hosts: Vec<HostRecord>,
}

struct HostState {
    record: HostRecord,
    generation: u64,
    client: Option<Arc<RpcClient>>,
    error: Option<String>,
    catalogs: HashMap<&'static str, Vec<Value>>,
    // Include accepted creates before their first catalog notification, and
    // retain ownership after deletion to prevent stale calls running locally.
    chats: HashSet<String>,
    spaces: HashSet<String>,
}

impl HostState {
    fn new(record: HostRecord) -> Self {
        Self {
            record,
            generation: 0,
            client: None,
            error: None,
            catalogs: HashMap::new(),
            chats: HashSet::new(),
            spaces: HashSet::new(),
        }
    }

    fn view(&self) -> HostView {
        HostView {
            id: self.record.id.clone(),
            name: self.record.name.clone(),
            url: self.record.url.clone(),
            enabled: self.record.enabled,
            device_id: self.record.device_id.clone(),
            connected: self.record.enabled
                && self
                    .client
                    .as_ref()
                    .is_some_and(|client| client.is_connected()),
            error: self.error.clone(),
        }
    }
}

pub struct RemoteHosts {
    path: PathBuf,
    local_device_id: String,
    state: Mutex<BTreeMap<String, HostState>>,
    tasks: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    persistence: Mutex<()>,
    edits: tokio::sync::Mutex<()>,
    changes: watch::Sender<u64>,
    stopped: AtomicBool,
}

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|error| error.into_inner())
}

impl RemoteHosts {
    pub fn open(root: &Path, local_device_id: &str) -> anyhow::Result<Arc<Self>> {
        let path = root.join("remote-hosts.json");
        let config = read_config(&path)?;
        let mut state = BTreeMap::new();
        let mut urls = HashSet::new();
        let mut devices = HashSet::new();
        for mut record in config.hosts {
            validate_id(&record.id).map_err(|_| anyhow::anyhow!("invalid remote host id"))?;
            record.url = normalize_url(&record.url)
                .map_err(|_| anyhow::anyhow!("invalid remote host address"))?;
            validate_name(&record.name).map_err(|_| anyhow::anyhow!("invalid remote host name"))?;
            rcode_rpc::validate_api_key(&record.api_key)
                .map_err(|_| anyhow::anyhow!("invalid remote host credential"))?;
            anyhow::ensure!(
                urls.insert(record.url.clone()),
                "duplicate remote host address"
            );
            if let Some(device) = &record.device_id {
                anyhow::ensure!(device != local_device_id, "remote host is this device");
                anyhow::ensure!(
                    devices.insert(device.clone()),
                    "duplicate remote host device"
                );
            }
            anyhow::ensure!(!state.contains_key(&record.id), "duplicate remote host id");
            state.insert(record.id.clone(), HostState::new(record));
        }
        let (changes, _) = watch::channel(0);
        Ok(Arc::new(Self {
            path,
            local_device_id: local_device_id.into(),
            state: Mutex::new(state),
            tasks: Mutex::new(HashMap::new()),
            persistence: Mutex::new(()),
            edits: tokio::sync::Mutex::new(()),
            changes,
            stopped: AtomicBool::new(false),
        }))
    }

    pub fn start(self: &Arc<Self>) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        let records: Vec<_> = lock(&self.state)
            .values()
            .filter(|host| host.record.enabled)
            .map(|host| (host.record.clone(), host.generation))
            .collect();
        for (record, generation) in records {
            self.spawn(record, generation);
        }
    }

    pub fn list(&self) -> Vec<HostView> {
        let mut hosts: Vec<_> = lock(&self.state).values().map(HostState::view).collect();
        hosts.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        hosts
    }

    pub fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub fn api_key(&self, id: &str) -> Result<String, RpcError> {
        lock(&self.state)
            .get(id)
            .map(|host| host.record.api_key.clone())
            .ok_or_else(|| RpcError::BadParams("remote host does not exist".into()))
    }

    pub async fn save(self: &Arc<Self>, edit: HostEdit) -> Result<HostView, RpcError> {
        let _edit = self.edits.lock().await;
        if self.stopped.load(Ordering::Acquire) {
            return Err(RpcError::Closed);
        }
        validate_name(&edit.name)?;
        let url = normalize_url(&edit.url)?;
        let id = edit.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        validate_id(&id)?;
        let (record, generation, previous, view) = {
            let _write = lock(&self.persistence);
            let mut state = lock(&self.state);
            let previous = state.get(&id);
            let api_key = edit
                .api_key
                .or_else(|| previous.map(|host| host.record.api_key.clone()))
                .ok_or_else(|| RpcError::BadParams("API key is required".into()))?;
            rcode_rpc::validate_api_key(&api_key)?;
            if state
                .values()
                .any(|host| host.record.id != id && host.record.url == url)
            {
                return Err(RpcError::BadParams(
                    "remote host address is already configured".into(),
                ));
            }
            let same_address = previous.is_some_and(|host| host.record.url == url);
            let record = HostRecord {
                id: id.clone(),
                name: edit.name.trim().into(),
                url,
                api_key,
                enabled: edit
                    .enabled
                    .unwrap_or_else(|| previous.is_none_or(|host| host.record.enabled)),
                device_id: previous
                    .filter(|_| same_address)
                    .and_then(|host| host.record.device_id.clone()),
            };
            let generation = previous.map_or(1, |host| host.generation.wrapping_add(1));
            let mut records: Vec<_> = state
                .values()
                .filter(|host| host.record.id != id)
                .map(|host| host.record.clone())
                .collect();
            records.push(record.clone());
            write_config(&self.path, records)?;
            let mut replacement = HostState::new(record.clone());
            replacement.generation = generation;
            let previous = state.remove(&id);
            if same_address && let Some(previous) = &previous {
                replacement.catalogs = previous.catalogs.clone();
                replacement.chats = previous.chats.clone();
                replacement.spaces = previous.spaces.clone();
            }
            let view = replacement.view();
            state.insert(id.clone(), replacement);
            (
                record,
                generation,
                previous.and_then(|host| host.client),
                view,
            )
        };
        if let Some(previous) = previous {
            previous.close();
        }
        self.stop_task(&id).await;
        self.bump();
        if record.enabled && !self.stopped.load(Ordering::Acquire) {
            self.spawn(record, generation);
        }
        Ok(view)
    }

    pub async fn remove(&self, id: &str) -> Result<(), RpcError> {
        let _edit = self.edits.lock().await;
        let previous = {
            let _write = lock(&self.persistence);
            let mut state = lock(&self.state);
            if !state.contains_key(id) {
                return Ok(());
            }
            let records = state
                .values()
                .filter(|host| host.record.id != id)
                .map(|host| host.record.clone())
                .collect();
            write_config(&self.path, records)?;
            state.remove(id).and_then(|host| host.client)
        };
        if let Some(client) = previous {
            client.close();
        }
        self.stop_task(id).await;
        self.bump();
        Ok(())
    }

    pub fn client(&self, target: &str) -> Option<Result<Arc<RpcClient>, RpcError>> {
        let state = lock(&self.state);
        let host = state.values().find(|host| {
            host.record.device_id.as_deref() == Some(target) || host.record.id == target
        })?;
        Some(match &host.client {
            Some(client) if host.record.enabled && client.is_connected() => Ok(client.clone()),
            _ => Err(RpcError::Failed("remote device is disconnected".into())),
        })
    }

    pub fn merge(&self, method: &str, local: Value) -> Value {
        let Some(mut rows) = local.as_array().cloned() else {
            return local;
        };
        if !CATALOGS.contains(&method) {
            return Value::Array(rows);
        }
        let id_field = if method == methods::WATCH_SESSIONS {
            "chatId"
        } else {
            "id"
        };
        let mut ids: HashSet<String> = rows
            .iter()
            .filter_map(|row| row[id_field].as_str().map(str::to_owned))
            .collect();
        for host in lock(&self.state).values() {
            if method == methods::WATCH_DEVICES {
                let Some(device_id) = &host.record.device_id else {
                    continue;
                };
                let mut row = host.catalogs.get(method).and_then(|rows| rows.first()).cloned().unwrap_or_else(|| json!({
                    "id":device_id, "name":host.record.name, "platform":"unknown", "lastSeenAt":null, "createdAt":null
                }));
                row["name"] = json!(host.record.name);
                row["lastSeenAt"] = if host.record.enabled
                    && host
                        .client
                        .as_ref()
                        .is_some_and(|client| client.is_connected())
                {
                    json!(chrono::Utc::now())
                } else {
                    Value::Null
                };
                if ids.insert(device_id.clone()) {
                    rows.push(row);
                }
            } else if let Some(remote) = host.catalogs.get(method) {
                for row in remote {
                    if let Some(id) = row[id_field].as_str()
                        && ids.insert(id.into())
                    {
                        rows.push(row.clone());
                    }
                }
            }
        }
        Value::Array(rows)
    }

    pub fn target_for(&self, method: &str, params: &Value) -> Option<String> {
        let mut state = lock(&self.state);
        let explicit = params
            .get("targetDeviceId")
            .and_then(Value::as_str)
            .or_else(|| params.get("deviceId").and_then(Value::as_str));
        if let Some(target) = explicit {
            if target == self.local_device_id {
                if method == methods::MUTATE {
                    for host in state.values_mut() {
                        if let Some(id) = params["chatId"].as_str() {
                            host.chats.remove(id);
                        }
                        if let Some(id) = params["spaceId"].as_str() {
                            host.spaces.remove(id);
                        }
                    }
                }
                return None;
            }
            if let Some(host) = state
                .values_mut()
                .find(|host| host.record.device_id.as_deref() == Some(target))
            {
                if method == methods::MUTATE {
                    if params["op"] == "createChat"
                        && let Some(id) = params["chatId"].as_str()
                    {
                        host.chats.insert(id.into());
                    }
                    if params["op"] == "createSpace"
                        && let Some(id) = params["spaceId"].as_str()
                    {
                        host.spaces.insert(id.into());
                    }
                }
                return Some(target.into());
            }
        }
        target_from_fields(&state, params, 0)
    }

    pub async fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        let _edit = self.edits.lock().await;
        let clients: Vec<_> = lock(&self.state)
            .values_mut()
            .filter_map(|host| host.client.take())
            .collect();
        for client in clients {
            client.close();
        }
        let tasks: Vec<_> = lock(&self.tasks).drain().map(|(_, task)| task).collect();
        for task in &tasks {
            task.abort();
        }
        for task in tasks {
            let _ = task.await;
        }
        self.bump();
    }

    fn bump(&self) {
        self.changes
            .send_modify(|value| *value = value.wrapping_add(1));
    }

    async fn stop_task(&self, id: &str) {
        let task = lock(&self.tasks).remove(id);
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
    }

    fn spawn(self: &Arc<Self>, record: HostRecord, generation: u64) {
        let mut tasks = lock(&self.tasks);
        if tasks.contains_key(&record.id) {
            return;
        }
        let weak = Arc::downgrade(self);
        tasks.insert(
            record.id.clone(),
            tokio::spawn(run_host(weak, record, generation)),
        );
    }

    fn update_error(&self, id: &str, generation: u64, error: &str) {
        let changed = {
            let mut state = lock(&self.state);
            let Some(host) = state
                .get_mut(id)
                .filter(|host| host.generation == generation)
            else {
                return;
            };
            let changed = host.error.as_deref() != Some(error);
            host.error = Some(error.into());
            changed
        };
        if changed {
            self.bump();
        }
    }

    fn attach(
        &self,
        id: &str,
        generation: u64,
        device: &str,
        client: Arc<RpcClient>,
    ) -> Result<(), RpcError> {
        if device.is_empty() || device == self.local_device_id {
            return Err(RpcError::BadParams("remote host is this device".into()));
        }
        let _write = lock(&self.persistence);
        let mut state = lock(&self.state);
        if state
            .values()
            .any(|host| host.record.id != id && host.record.device_id.as_deref() == Some(device))
        {
            return Err(RpcError::BadParams(
                "remote device is already configured".into(),
            ));
        }
        let host = state
            .get(id)
            .filter(|host| host.generation == generation)
            .ok_or(RpcError::Closed)?;
        if host.record.device_id.as_deref() != Some(device) {
            let records = state
                .values()
                .map(|host| {
                    let mut record = host.record.clone();
                    if record.id == id {
                        record.device_id = Some(device.into());
                    }
                    record
                })
                .collect();
            write_config(&self.path, records)?;
        }
        let host = state.get_mut(id).unwrap();
        if host.record.device_id.as_deref() != Some(device) {
            host.catalogs.clear();
            host.chats.clear();
            host.spaces.clear();
        }
        host.record.device_id = Some(device.into());
        host.client = Some(client);
        host.error = None;
        drop(state);
        self.bump();
        Ok(())
    }

    fn cache(&self, id: &str, generation: u64, device: &str, method: &'static str, value: Value) {
        let Some(rows) = value.as_array() else {
            return;
        };
        let rows: Vec<_> = rows
            .iter()
            .filter(|row| {
                if method == methods::WATCH_DEVICES {
                    row["id"].as_str() == Some(device)
                } else {
                    row["deviceId"].as_str() == Some(device)
                }
            })
            .cloned()
            .collect();
        let changed = {
            let mut state = lock(&self.state);
            let Some(host) = state
                .get_mut(id)
                .filter(|host| host.generation == generation)
            else {
                return;
            };
            if host.catalogs.get(method) == Some(&rows) {
                return;
            }
            if method == methods::WATCH_CHATS {
                host.chats.extend(
                    rows.iter()
                        .filter_map(|row| row["id"].as_str().map(str::to_owned)),
                );
            }
            if method == methods::WATCH_SPACES {
                host.spaces.extend(
                    rows.iter()
                        .filter_map(|row| row["id"].as_str().map(str::to_owned)),
                );
            }
            host.catalogs.insert(method, rows);
            true
        };
        if changed {
            self.bump();
        }
    }
}

impl Drop for RemoteHosts {
    fn drop(&mut self) {
        for task in lock(&self.tasks).values() {
            task.abort();
        }
        for host in lock(&self.state).values() {
            if let Some(client) = &host.client {
                client.close();
            }
        }
    }
}

fn target_from_fields(
    state: &BTreeMap<String, HostState>,
    params: &Value,
    depth: usize,
) -> Option<String> {
    if depth > 3 {
        return None;
    }
    for host in state.values() {
        let Some(device) = host.record.device_id.as_ref() else {
            continue;
        };
        if ["targetDeviceId", "deviceId"]
            .iter()
            .any(|field| params[*field].as_str() == Some(device))
        {
            return Some(device.clone());
        }
        if ["chatId", "sourceChatId", "targetChatId"]
            .iter()
            .any(|field| {
                params[*field]
                    .as_str()
                    .is_some_and(|id| host.chats.contains(id))
            })
        {
            return Some(device.clone());
        }
        if params["spaceId"]
            .as_str()
            .is_some_and(|id| host.spaces.contains(id))
        {
            return Some(device.clone());
        }
    }
    for field in ["target", "request"] {
        if params[field].is_object()
            && let Some(device) = target_from_fields(state, &params[field], depth + 1)
        {
            return Some(device);
        }
    }
    None
}

async fn identify(client: &RpcClient) -> Result<String, RpcError> {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        client.call(methods::ENGINE_INFO, json!({})),
    )
    .await
    .map_err(|_| RpcError::Transport("remote engine did not identify itself".into()))?;
    let value = match result {
        Ok(value) => value,
        Err(RpcError::UnknownMethod(_)) => tokio::time::timeout(
            Duration::from_secs(5),
            client.call(methods::LOCAL_DEVICE, json!({})),
        )
        .await
        .map_err(|_| RpcError::Transport("remote engine did not identify itself".into()))??,
        Err(error) => return Err(error),
    };
    value["deviceId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| RpcError::Failed("remote engine identity is missing".into()))
}

async fn run_host(owner: Weak<RemoteHosts>, record: HostRecord, generation: u64) {
    let mut delay = Duration::from_millis(250);
    loop {
        let client = match connect_ws_authenticated(&record.url, &record.api_key).await {
            Ok(client) => Arc::new(client),
            Err(_) => {
                let Some(manager) = owner.upgrade() else {
                    return;
                };
                manager.update_error(
                    &record.id,
                    generation,
                    "Could not connect or authenticate to remote engine",
                );
                drop(manager);
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(5));
                continue;
            }
        };
        delay = Duration::from_millis(250);
        loop {
            let device = match identify(&client).await {
                Ok(device) => device,
                Err(_) => break,
            };
            let Some(manager) = owner.upgrade() else {
                client.close();
                return;
            };
            if let Err(error) = manager.attach(&record.id, generation, &device, client.clone()) {
                manager.update_error(&record.id, generation, &error.to_string());
                client.close();
                break;
            }
            drop(manager);
            let watches = CATALOGS.map(|method| {
                watch_catalog(
                    owner.clone(),
                    record.id.clone(),
                    generation,
                    device.clone(),
                    client.clone(),
                    method,
                )
            });
            tokio::select! {
                _ = join_all(watches) => {},
                _ = async {
                    let mut ticks = 0_u32;
                    while client.is_connected() {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        ticks += 1;
                        if ticks % 100 == 0 {
                            let Some(manager) = owner.upgrade() else { return; };
                            manager.bump();
                        }
                    }
                } => {},
            }
            let Some(manager) = owner.upgrade() else {
                client.close();
                return;
            };
            manager.update_error(
                &record.id,
                generation,
                "Remote connection lost; reconnecting",
            );
            drop(manager);
            while !client.is_connected() {
                if owner.strong_count() == 0 {
                    client.close();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        client.close();
        let Some(manager) = owner.upgrade() else {
            return;
        };
        manager.update_error(
            &record.id,
            generation,
            "Remote engine identity is unavailable or already configured",
        );
        drop(manager);
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(5));
    }
}

async fn watch_catalog(
    owner: Weak<RemoteHosts>,
    id: String,
    generation: u64,
    device: String,
    client: Arc<RpcClient>,
    method: &'static str,
) {
    loop {
        if !client.is_connected() {
            return;
        }
        if let Ok(mut stream) = client.subscribe_scoped(method, json!({})).await {
            while let Some(value) = stream.recv().await {
                let Some(manager) = owner.upgrade() else {
                    return;
                };
                manager.cache(&id, generation, &device, method, value);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

pub fn normalize_url(value: &str) -> Result<String, RpcError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(RpcError::BadParams(
            "remote host address is required".into(),
        ));
    }
    let bare = !value.contains("://");
    let candidate = if bare {
        format!("ws://{value}")
    } else {
        value.into()
    };
    let mut url = reqwest::Url::parse(&candidate)
        .map_err(|_| RpcError::BadParams("invalid remote host address".into()))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err(RpcError::BadParams(
            "remote address must not contain credentials, query parameters, or fragments".into(),
        ));
    }
    let scheme = match url.scheme() {
        "http" | "ws" => "ws",
        "https" | "wss" => "wss",
        _ => {
            return Err(RpcError::BadParams(
                "remote address requires ws, wss, http, or https".into(),
            ));
        }
    };
    url.set_scheme(scheme)
        .map_err(|_| RpcError::BadParams("invalid remote host scheme".into()))?;
    let authority = value.split('/').next().unwrap_or_default();
    let explicit_port = if authority.starts_with('[') {
        authority
            .split_once(']')
            .is_some_and(|(_, tail)| tail.starts_with(':'))
    } else {
        authority.contains(':')
    };
    if bare && !explicit_port {
        url.set_port(Some(27655))
            .map_err(|_| RpcError::BadParams("invalid remote host port".into()))?;
    }
    Ok(url.to_string())
}

fn validate_id(value: &str) -> Result<(), RpcError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(RpcError::BadParams("invalid remote host id".into()));
    }
    Ok(())
}

fn validate_name(value: &str) -> Result<(), RpcError> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(RpcError::BadParams(
            "remote host name is required and must be at most 256 bytes".into(),
        ));
    }
    Ok(())
}

fn read_config(path: &Path) -> anyhow::Result<Config> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(error) => return Err(error.into()),
        Ok(metadata) => anyhow::ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "remote host configuration must be a regular file"
        ),
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_CONFIG_BYTES,
        "remote host configuration exceeds limit"
    );
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid remote host configuration"))
}

fn write_config(path: &Path, hosts: Vec<HostRecord>) -> Result<(), RpcError> {
    if let Ok(metadata) = std::fs::symlink_metadata(path)
        && (!metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err(RpcError::Failed(
            "remote host configuration must be a regular file".into(),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| RpcError::Failed("remote host configuration has no directory".into()))?;
    std::fs::create_dir_all(parent)
        .map_err(|_| RpcError::Failed("could not create remote host settings directory".into()))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| RpcError::Failed("could not stage remote host settings".into()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| RpcError::Failed("could not protect remote host settings".into()))?;
    }
    let data = serde_json::to_vec_pretty(&Config { hosts })
        .map_err(|_| RpcError::Failed("could not serialize remote host settings".into()))?;
    if data.len() as u64 > MAX_CONFIG_BYTES {
        return Err(RpcError::BadParams(
            "remote host configuration exceeds limit".into(),
        ));
    }
    file.write_all(&data)
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| RpcError::Failed("could not save remote host settings".into()))?;
    file.persist(path)
        .map_err(|_| RpcError::Failed("could not replace remote host settings".into()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::StreamExt;
    use rcode_rpc::{RpcReply, RpcService, serve_authenticated_listener};

    const KEY: &str = "FixtureKey12";

    fn edit(name: &str, url: &str, enabled: bool) -> HostEdit {
        HostEdit {
            id: None,
            name: name.into(),
            url: url.into(),
            api_key: Some(KEY.into()),
            enabled: Some(enabled),
        }
    }

    #[test]
    fn addresses_normalize_without_accepting_embedded_credentials() {
        assert_eq!(
            normalize_url("server.example").unwrap(),
            "ws://server.example:27655/"
        );
        assert_eq!(normalize_url("127.0.0.1:80").unwrap(), "ws://127.0.0.1/");
        assert_eq!(normalize_url("[::1]").unwrap(), "ws://[::1]:27655/");
        assert_eq!(
            normalize_url("https://server.example/rpc").unwrap(),
            "wss://server.example/rpc"
        );
        assert_eq!(
            normalize_url("http://server.example:27655").unwrap(),
            "ws://server.example:27655/"
        );
        for url in [
            "ws://user:PrivateKey12@host",
            "ws://host/?key=PrivateKey12",
            "ws://host/#PrivateKey12",
            "file:///tmp/server",
            "",
        ] {
            let error = normalize_url(url).unwrap_err().to_string();
            assert!(!error.contains("PrivateKey12"));
        }
    }

    #[tokio::test]
    async fn host_settings_preserve_saved_keys_and_reject_invalid_edits() {
        let dir = tempfile::tempdir().unwrap();
        let manager = RemoteHosts::open(dir.path(), "local").unwrap();
        let host = manager
            .save(edit("Remote", "server.example", false))
            .await
            .unwrap();
        assert_eq!(host.url, "ws://server.example:27655/");
        assert!(!serde_json::to_string(&host).unwrap().contains(KEY));
        let updated = manager
            .save(HostEdit {
                id: Some(host.id.clone()),
                name: "Renamed".into(),
                url: host.url.clone(),
                api_key: None,
                enabled: None,
            })
            .await
            .unwrap();
        assert_eq!(manager.api_key(&updated.id).unwrap(), KEY);
        assert!(!updated.enabled);
        let path = dir.path().join("remote-hosts.json");
        let before = std::fs::read(&path).unwrap();
        let mut invalid = edit("Invalid", "other.example", false);
        invalid.api_key = Some("short".into());
        assert!(manager.save(invalid).await.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(
            manager
                .save(edit("Duplicate", &host.url, false))
                .await
                .is_err()
        );
        let reopened = RemoteHosts::open(dir.path(), "local").unwrap();
        assert_eq!(reopened.list()[0].name, "Renamed");
        assert_eq!(reopened.api_key(&host.id).unwrap(), KEY);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        manager.remove(&host.id).await.unwrap();
        assert!(manager.list().is_empty());
        assert!(
            RemoteHosts::open(dir.path(), "local")
                .unwrap()
                .list()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn catalogs_filter_foreign_devices_and_route_creates_before_watch_delivery() {
        let dir = tempfile::tempdir().unwrap();
        let manager = RemoteHosts::open(dir.path(), "local").unwrap();
        let unknown = manager
            .save(edit("Unknown", "unknown.example", false))
            .await
            .unwrap();
        let host = manager
            .save(edit("Remote", "server.example", false))
            .await
            .unwrap();
        {
            let mut state = lock(&manager.state);
            state.get_mut(&host.id).unwrap().record.device_id = Some("peer".into());
            // Ensure an unconnected host sorting first cannot suppress routing
            // for a later host whose identity is already known.
            assert!(state.contains_key(&unknown.id));
        }
        manager.cache(&host.id, 1, "peer", methods::WATCH_DEVICES, json!([
            {"id":"peer", "name":"Peer", "platform":"linux", "lastSeenAt":null, "createdAt":null},
            {"id":"third", "name":"Third", "platform":"linux"}
        ]));
        manager.cache(
            &host.id,
            1,
            "peer",
            methods::WATCH_CHATS,
            json!([
                {"id":"peer-chat", "deviceId":"peer"}, {"id":"third-chat", "deviceId":"third"}
            ]),
        );
        manager.cache(
            &host.id,
            1,
            "peer",
            methods::WATCH_SPACES,
            json!([
                {"id":"peer-space", "deviceId":"peer"}, {"id":"third-space", "deviceId":"third"}
            ]),
        );
        assert_eq!(
            manager.merge(methods::WATCH_CHATS, json!([])),
            json!([{"id":"peer-chat", "deviceId":"peer"}])
        );
        let devices = manager.merge(methods::WATCH_DEVICES, json!([{"id":"local"}]));
        assert_eq!(devices.as_array().unwrap().len(), 2);
        assert!(
            devices
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["id"] != "third")
        );
        assert_eq!(
            manager
                .target_for("ReadFile", &json!({"chatId":"peer-chat"}))
                .as_deref(),
            Some("peer")
        );
        assert_eq!(
            manager
                .target_for("ListFiles", &json!({"target":{"spaceId":"peer-space"}}))
                .as_deref(),
            Some("peer")
        );
        assert_eq!(
            manager
                .target_for(
                    methods::MUTATE,
                    &json!({"op":"createChat", "chatId":"new-chat", "deviceId":"peer"})
                )
                .as_deref(),
            Some("peer")
        );
        assert_eq!(
            manager
                .target_for(methods::QUEUE_COMMAND, &json!({"chatId":"new-chat"}))
                .as_deref(),
            Some("peer")
        );
        assert!(manager.client("peer").unwrap().is_err());
        assert!(
            manager
                .target_for(
                    methods::MUTATE,
                    &json!({"op":"createChat", "chatId":"new-chat", "deviceId":"local"})
                )
                .is_none()
        );
        assert!(
            manager
                .target_for(methods::QUEUE_COMMAND, &json!({"chatId":"new-chat"}))
                .is_none()
        );
        manager.remove(&host.id).await.unwrap();
        assert!(
            manager
                .target_for("ReadFile", &json!({"chatId":"peer-chat"}))
                .is_none()
        );
    }

    struct Peer;
    #[async_trait]
    impl RpcService for Peer {
        async fn handle(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
            let value = match method {
                methods::ENGINE_INFO | methods::LOCAL_DEVICE => {
                    return RpcReply::value(&json!({"deviceId":"peer"}));
                }
                methods::WATCH_DEVICES => {
                    json!([{"id":"peer", "name":"Server", "platform":"linux", "lastSeenAt":null, "createdAt":null}, {"id":"local", "name":"Do not mirror"}])
                }
                methods::WATCH_CHATS => {
                    json!([{"id":"chat", "deviceId":"peer"}, {"id":"foreign", "deviceId":"local"}])
                }
                methods::WATCH_SPACES => json!([{"id":"space", "deviceId":"peer"}]),
                methods::WATCH_SESSIONS => {
                    json!([{"chatId":"chat", "deviceId":"peer", "status":"idle"}])
                }
                "Echo" => return RpcReply::value(&params),
                _ => return Err(RpcError::UnknownMethod(method.into())),
            };
            Ok(RpcReply::Stream(
                futures::stream::once(async move { value })
                    .chain(futures::stream::pending())
                    .boxed(),
            ))
        }
    }

    async fn settled(mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(8), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn authenticated_hosts_recover_then_disconnect_without_local_fallback() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (key, receiver) = watch::channel(Some(KEY.into()));
        let server = tokio::spawn(serve_authenticated_listener(
            listener,
            Arc::new(Peer),
            receiver,
        ));
        let dir = tempfile::tempdir().unwrap();
        let manager = RemoteHosts::open(dir.path(), "local").unwrap();
        let host = manager.save(edit("My server", &url, true)).await.unwrap();
        settled(|| {
            manager.list()[0].connected
                && manager
                    .merge(methods::WATCH_CHATS, json!([]))
                    .as_array()
                    .unwrap()
                    .len()
                    == 1
        })
        .await;
        assert_eq!(
            manager
                .target_for("ReadFile", &json!({"chatId":"chat"}))
                .as_deref(),
            Some("peer")
        );
        let client = manager.client("peer").unwrap().unwrap();
        assert_eq!(
            client.call("Echo", json!({"remote":true})).await.unwrap(),
            json!({"remote":true})
        );
        key.send_replace(None);
        settled(|| !manager.list()[0].connected).await;
        assert!(manager.client("peer").unwrap().is_err());
        assert_eq!(
            manager
                .target_for("ReadFile", &json!({"chatId":"chat"}))
                .as_deref(),
            Some("peer")
        );
        key.send_replace(Some(KEY.into()));
        settled(|| manager.list()[0].connected).await;
        let disabled = manager
            .save(HostEdit {
                id: Some(host.id.clone()),
                name: "My server".into(),
                url,
                api_key: None,
                enabled: Some(false),
            })
            .await
            .unwrap();
        assert!(!disabled.connected);
        assert!(manager.client("peer").unwrap().is_err());
        assert!(
            !client.is_connected(),
            "disabling must close even retained RPC clients"
        );
        manager.shutdown().await;
        server.abort();
    }

    #[tokio::test]
    async fn connecting_to_self_or_to_the_same_device_twice_is_rejected() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (_key, receiver) = watch::channel(Some(KEY.into()));
        let server = tokio::spawn(serve_authenticated_listener(
            listener,
            Arc::new(Peer),
            receiver,
        ));
        let dir = tempfile::tempdir().unwrap();
        let manager = RemoteHosts::open(dir.path(), "local").unwrap();
        manager.save(edit("One", &url, true)).await.unwrap();
        settled(|| manager.list()[0].connected).await;
        let duplicate = manager
            .save(edit("Two", &format!("{url}/other"), true))
            .await
            .unwrap();
        settled(|| {
            manager
                .list()
                .iter()
                .find(|host| host.id == duplicate.id)
                .unwrap()
                .error
                .is_some()
        })
        .await;
        assert_eq!(
            manager.list().iter().filter(|host| host.connected).count(),
            1
        );
        let self_dir = tempfile::tempdir().unwrap();
        let local = RemoteHosts::open(self_dir.path(), "peer").unwrap();
        local.save(edit("Self", &url, true)).await.unwrap();
        settled(|| local.list()[0].error.is_some()).await;
        assert!(!local.list()[0].connected);
        local.shutdown().await;
        manager.shutdown().await;
        server.abort();
    }
}
