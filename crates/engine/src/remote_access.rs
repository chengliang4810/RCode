//! The optional authenticated listener. The desktop's own RPC stays in memory.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use rcode_rpc::{RpcError, RpcService, generate_api_key, validate_api_key};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, watch};

pub const DEFAULT_REMOTE_PORT: u16 = 27655;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default)]
    api_key: Option<String>,
}

fn default_port() -> u16 {
    DEFAULT_REMOTE_PORT
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_port(),
            api_key: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAccessView {
    pub enabled: bool,
    pub port: u16,
    pub has_api_key: bool,
    pub error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAccessEdit {
    pub enabled: bool,
    pub port: u16,
    pub api_key: Option<String>,
}

struct Listener {
    port: u16,
    key: watch::Sender<Option<String>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.key.send_replace(None);
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl Listener {
    async fn stop(mut self) {
        self.key.send_replace(None);
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

pub struct RemoteAccess {
    path: PathBuf,
    config: Mutex<Config>,
    error: Mutex<Option<String>>,
    listener: AsyncMutex<Option<Listener>>,
    service: OnceLock<Arc<dyn RpcService>>,
    stopped: AtomicBool,
}

impl RemoteAccess {
    pub fn open(root: &Path) -> anyhow::Result<Arc<Self>> {
        let path = root.join("remote-access.json");
        let config = match std::fs::symlink_metadata(&path) {
            Ok(meta) if !meta.file_type().is_file() => {
                anyhow::bail!("remote-access.json must be a regular file")
            }
            Ok(_) => serde_json::from_slice(&std::fs::read(&path)?)
                .map_err(|_| anyhow::anyhow!("remote-access.json is invalid"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(error) => return Err(error.into()),
        };
        Ok(Arc::new(Self {
            path,
            config: Mutex::new(config),
            error: Mutex::new(None),
            listener: AsyncMutex::new(None),
            service: OnceLock::new(),
            stopped: AtomicBool::new(false),
        }))
    }

    pub fn view(&self) -> RemoteAccessView {
        let config = self.config.lock().unwrap_or_else(|e| e.into_inner());
        RemoteAccessView {
            enabled: config.enabled,
            port: config.port,
            has_api_key: config.api_key.is_some(),
            error: self.error.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        }
    }

    pub fn api_key(&self) -> Option<String> {
        self.config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .api_key
            .clone()
    }

    pub async fn start(&self, service: Arc<dyn RpcService>) {
        let _ = self.service.set(service);
        let config = self
            .config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Err(error) = self
            .apply(
                RemoteAccessEdit {
                    enabled: config.enabled,
                    port: config.port,
                    api_key: config.api_key,
                },
                false,
            )
            .await
        {
            *self.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(error.to_string());
            tracing::warn!("external access listener could not start");
        }
    }

    pub async fn start_configured(
        &self,
        service: Arc<dyn RpcService>,
        api_key: String,
        port: u16,
    ) -> Result<(), RpcError> {
        let _ = self.service.set(service);
        self.configure_runtime(api_key, port).await
    }

    pub async fn set(&self, edit: RemoteAccessEdit) -> Result<RemoteAccessView, RpcError> {
        self.apply(edit, true).await
    }

    /// Command-line credentials belong to this process; do not copy them to settings.
    pub async fn configure_runtime(&self, api_key: String, port: u16) -> Result<(), RpcError> {
        self.apply(
            RemoteAccessEdit {
                enabled: true,
                port,
                api_key: Some(api_key),
            },
            false,
        )
        .await
        .map(|_| ())
    }

    async fn apply(
        &self,
        edit: RemoteAccessEdit,
        persist: bool,
    ) -> Result<RemoteAccessView, RpcError> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(RpcError::Closed);
        }
        if edit.port == 0 {
            return Err(RpcError::BadParams(
                "port must be between 1 and 65535".into(),
            ));
        }
        if let Some(key) = &edit.api_key {
            validate_api_key(key)?;
        }
        let mut running = self.listener.lock().await;
        // Shutdown marks this before taking the lock, so a queued settings RPC
        // cannot recreate the listener after the engine has begun stopping.
        if self.stopped.load(Ordering::Acquire) {
            return Err(RpcError::Closed);
        }
        let mut config = self
            .config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        config.enabled = edit.enabled;
        config.port = edit.port;
        if let Some(key) = edit.api_key {
            config.api_key = Some(key);
        }
        if config.enabled && config.api_key.is_none() {
            config.api_key = Some(generate_api_key());
        }
        let new_listener = if config.enabled
            && running
                .as_ref()
                .is_none_or(|listener| listener.port != config.port)
        {
            let service = self
                .service
                .get()
                .cloned()
                .ok_or_else(|| RpcError::Failed("external access is not ready".into()))?;
            let socket = tokio::net::TcpListener::bind(("0.0.0.0", config.port))
                .await
                .map_err(|_| RpcError::Failed(format!("cannot listen on port {}", config.port)))?;
            Some((socket, service))
        } else {
            None
        };
        if persist {
            save_private_json(&self.path, &config)
                .map_err(|_| RpcError::Failed("could not save external access settings".into()))?;
        }
        if let Some((socket, service)) = new_listener {
            let (key, rx) = watch::channel(config.api_key.clone());
            let task = tokio::spawn(rcode_rpc::serve_authenticated_listener(socket, service, rx));
            if let Some(old) = running.replace(Listener {
                port: config.port,
                key,
                task: Some(task),
            }) {
                old.stop().await;
            }
        } else if !config.enabled {
            if let Some(old) = running.take() {
                old.stop().await;
            }
        } else if let Some(listener) = running.as_ref() {
            if *listener.key.borrow() != config.api_key {
                listener.key.send_replace(config.api_key.clone());
            }
        }
        *self.config.lock().unwrap_or_else(|e| e.into_inner()) = config;
        *self.error.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(self.view())
    }

    pub async fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(listener) = self.listener.lock().await.take() {
            listener.stop().await;
        }
    }
}

pub(crate) fn save_private_json(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("missing config directory"))?;
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temp, value)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rcode_rpc::RpcReply;

    struct Echo;
    #[async_trait]
    impl RpcService for Echo {
        async fn handle(&self, _: &str, params: serde_json::Value) -> Result<RpcReply, RpcError> {
            Ok(RpcReply::Value(params))
        }
    }

    #[tokio::test]
    async fn shutdown_rejects_queued_and_later_listener_updates() {
        let dir = tempfile::tempdir().unwrap();
        let access = RemoteAccess::open(dir.path()).unwrap();
        access.start(Arc::new(Echo)).await;
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let guard = access.listener.lock().await;
        let pending_access = Arc::clone(&access);
        let pending = tokio::spawn(async move {
            pending_access
                .set(RemoteAccessEdit {
                    enabled: true,
                    port,
                    api_key: None,
                })
                .await
        });
        tokio::task::yield_now().await;
        let stopping_access = Arc::clone(&access);
        let stopping = tokio::spawn(async move { stopping_access.shutdown().await });
        while !access.stopped.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
        drop(guard);
        assert!(matches!(pending.await.unwrap(), Err(RpcError::Closed)));
        stopping.await.unwrap();
        assert!(matches!(
            access.configure_runtime("Abcd1234".into(), port).await,
            Err(RpcError::Closed)
        ));
        assert!(!dir.path().join("remote-access.json").exists());
        assert!(
            tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn enabling_generates_twelve_character_key_and_failed_edits_keep_the_listener() {
        let dir = tempfile::tempdir().unwrap();
        let access = RemoteAccess::open(dir.path()).unwrap();
        access.start(Arc::new(Echo)).await;
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        access
            .set(RemoteAccessEdit {
                enabled: true,
                port,
                api_key: None,
            })
            .await
            .unwrap();
        let original = access.api_key().unwrap();
        assert_eq!(original.len(), 12);
        assert!(
            access
                .set(RemoteAccessEdit {
                    enabled: true,
                    port,
                    api_key: Some("invalid".into())
                })
                .await
                .is_err()
        );
        assert_eq!(access.api_key().unwrap(), original);
        let client =
            rcode_rpc::connect_ws_authenticated(&format!("ws://127.0.0.1:{port}"), &original)
                .await
                .unwrap();
        assert_eq!(client.call("Echo", serde_json::json!(7)).await.unwrap(), 7);
        access
            .set(RemoteAccessEdit {
                enabled: false,
                port,
                api_key: None,
            })
            .await
            .unwrap();
        assert!(
            tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_err()
        );
        let loaded = RemoteAccess::open(dir.path()).unwrap();
        assert!(!loaded.view().enabled);
        assert_eq!(loaded.api_key().unwrap(), original);
    }
}
