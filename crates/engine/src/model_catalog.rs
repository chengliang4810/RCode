//! Refresh the complete public catalog without delaying desktop startup.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use rcode_harness::model_catalog::{self, BODY_LIMIT, ModelCatalog};

const URL: &str = "https://models.dev/api.json";

/// Schedule at most one catalog refresh per config root in this process.
/// Both a desktop viewport and a standalone engine can initiate startup.
pub fn start(root: PathBuf) {
    static STARTED: OnceLock<Mutex<std::collections::HashSet<PathBuf>>> = OnceLock::new();
    if !STARTED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(root.clone())
    {
        return;
    }
    tokio::spawn(async move {
        let warm_root = root.clone();
        let _ = tokio::task::spawn_blocking(move || model_catalog::cached(&warm_root)).await;
        if let Err(error) = refresh(&root, URL).await {
            tracing::warn!(%error, "models.dev refresh failed; retaining the local catalog");
        }
    });
}

async fn refresh(root: &Path, url: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|_| "could not initialize model catalog download".to_owned())?;
    let mut response = client
        .get(url)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "model catalog returned HTTP {}",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > BODY_LIMIT as u64)
    {
        return Err("model catalog is too large".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| error.without_url().to_string())?
    {
        if body.len().saturating_add(chunk.len()) > BODY_LIMIT {
            return Err("model catalog is too large".into());
        }
        body.extend_from_slice(&chunk);
    }
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || publish(&root, &body))
        .await
        .map_err(|_| "model catalog cache worker failed".to_owned())?
}

fn publish(root: &Path, body: &[u8]) -> Result<(), String> {
    ModelCatalog::parse(body)?;
    let path = model_catalog::cache_path(root);
    let directory = path.parent().unwrap();
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(directory).map_err(|error| error.to_string())?;
    temporary
        .write_all(body)
        .map_err(|error| error.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary
        .persist(&path)
        .map_err(|error| error.error.to_string())?;
    model_catalog::cached(root);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    #[ignore = "downloads the live public models.dev catalog"]
    async fn live_catalog_download_and_lookup() {
        let root = tempfile::tempdir().unwrap();
        refresh(root.path(), URL).await.unwrap();
        let catalog = model_catalog::cached(root.path());
        let gpt = catalog.lookup(
            "gpt-5",
            "openai",
            "https://api.openai.com/v1",
            rcode_harness::providers::Protocol::OpenAiResponses,
        );
        assert_eq!(gpt.context_window, 400_000);
        assert_eq!(gpt.max_tokens, 128_000);
        assert!(gpt.input_format.accepts_images());
        assert!(!gpt.reasoning_levels.is_empty());
        let sonnet = catalog.lookup(
            "claude-sonnet-4-5-20250929",
            "custom",
            "https://custom.test/v1",
            rcode_harness::providers::Protocol::AnthropicMessages,
        );
        assert_eq!(sonnet.context_window, 200_000);
        assert_eq!(sonnet.max_tokens, 64_000);
        assert!(sonnet.input_format.accepts_images());
        assert!(!sonnet.reasoning_levels.is_empty());
        let length = std::fs::metadata(model_catalog::cache_path(root.path()))
            .unwrap()
            .len();
        eprintln!("validated full models.dev cache: {length} bytes; gpt-5: {gpt:?}");
    }

    #[tokio::test]
    async fn refresh_persists_the_entire_response_and_failed_updates_keep_the_cache() {
        let root = tempfile::tempdir().unwrap();
        publish(root.path(), br#"{"old":{"models":{"old":{"id":"old"}}}}"#).unwrap();
        let body = br#"{"test":{"models":{"model":{"id":"model","limit":{"context":1000000,"output":64000},"extra":"preserved"}}}}"#;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/api.json", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for (status, body) in [
                (200, body.as_slice()),
                (200, b"{}".as_slice()),
                (503, b"offline".as_slice()),
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    assert_eq!(stream.read(&mut byte).await.unwrap(), 1);
                    request.push(byte[0]);
                }
                assert!(
                    String::from_utf8(request)
                        .unwrap()
                        .starts_with("GET /api.json ")
                );
                stream.write_all(format!("HTTP/1.1 {status} Result\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).await.unwrap();
                stream.write_all(body).await.unwrap();
            }
        });
        refresh(root.path(), &url).await.unwrap();
        assert_eq!(
            std::fs::read(model_catalog::cache_path(root.path())).unwrap(),
            body
        );
        assert!(refresh(root.path(), &url).await.is_err());
        assert!(refresh(root.path(), &url).await.is_err());
        assert_eq!(
            std::fs::read(model_catalog::cache_path(root.path())).unwrap(),
            body
        );
        server.await.unwrap();
    }
}
