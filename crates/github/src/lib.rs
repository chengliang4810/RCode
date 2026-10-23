//! Public GitHub resources: ordered proxy failover, then direct access.
//! Use a public, unauthenticated HTTP client. Credential-bearing URLs and private API paths bypass mirrors.

use anyhow::{Context as _, bail, ensure};
use futures::StreamExt as _;
use sha2::{Digest, Sha256};
use std::{future::Future, path::Path, time::Duration};
use tokio::io::AsyncWriteExt as _;

pub const PROXIES: [&str; 2] = ["https://gh-proxy.com/", "https://ghfast.top/"];

pub fn routes(url: &str) -> Vec<String> {
    let eligible = reqwest::Url::parse(url).ok().is_some_and(|url| {
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && (matches!(
                url.host_str(),
                Some("github.com" | "raw.githubusercontent.com" | "codeload.github.com")
            ) || url.host_str() == Some("api.github.com") && url.path().starts_with("/repos/"))
    });
    let mut routes = Vec::new();
    if eligible {
        routes.extend(PROXIES.iter().map(|proxy| format!("{proxy}{url}")));
    }
    routes.push(url.to_owned());
    routes
}

pub fn client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .read_timeout(Duration::from_secs(20))
        .user_agent(concat!("rcode/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("too many GitHub redirects");
            }
            if attempt.previous().iter().any(|url| url.scheme() == "https")
                && attempt.url().scheme() != "https"
            {
                return attempt.error("GitHub redirect would downgrade HTTPS");
            }
            attempt.follow()
        }))
        .build()?)
}

pub async fn with_fallback<T, F, Fut>(url: &str, request: F) -> anyhow::Result<T>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    try_routes(routes(url), request).await
}

async fn try_routes<T, F, Fut>(routes: Vec<String>, mut request: F) -> anyhow::Result<T>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    let mut failures = Vec::new();
    for route in routes {
        match request(route.clone()).await {
            Ok(value) => return Ok(value),
            Err(error) => {
                tracing::debug!(route, %error, "GitHub request failed; trying next route");
                failures.push(format!("{route}: {error:#}"));
            }
        }
    }
    bail!("GitHub resource unavailable: {}", failures.join("; "))
}

/// Validation is part of each attempt: HTTP 200 HTML is not valid JSON/ZIP.
pub async fn fetch<T>(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
    timeout: Duration,
    validate: impl Fn(&[u8]) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    fetch_routes(client, routes(url), max_bytes, timeout, validate).await
}

/// Fetch trust-bearing metadata or executable content from its HTTPS origin.
/// Mirrors are appropriate only when a separately trusted digest authenticates
/// the response; checking JSON or ZIP syntax alone does not authenticate it.
pub async fn fetch_origin<T>(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
    timeout: Duration,
    validate: impl Fn(&[u8]) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    fetch_routes(client, vec![url.to_owned()], max_bytes, timeout, validate).await
}

async fn fetch_routes<T>(
    client: &reqwest::Client,
    routes: Vec<String>,
    max_bytes: usize,
    timeout: Duration,
    validate: impl Fn(&[u8]) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    try_routes(routes, |route| {
        let validate = &validate;
        async move {
            tokio::time::timeout(timeout, async {
                let response = client
                    .get(&route)
                    .header(reqwest::header::CACHE_CONTROL, "no-cache")
                    .send()
                    .await?
                    .error_for_status()?;
                ensure!(
                    response
                        .content_length()
                        .is_none_or(|n| n <= max_bytes as u64),
                    "resource exceeds {max_bytes} bytes"
                );
                let mut body = Vec::new();
                let mut stream = response.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk?;
                    ensure!(
                        body.len().saturating_add(chunk.len()) <= max_bytes,
                        "resource exceeds {max_bytes} bytes"
                    );
                    body.extend_from_slice(&chunk);
                }
                validate(&body)
            })
            .await
            .context("GitHub request deadline exceeded")?
        }
    })
    .await
}

/// Stream into a sidecar and retry the whole transfer on truncation/checksum errors.
pub async fn download(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    max_bytes: u64,
) -> anyhow::Result<()> {
    let partial = dest.with_extension("partial");
    let result = with_fallback(url, |route| {
        let partial = &partial;
        async move {
            let response = client.get(route).send().await?.error_for_status()?;
            ensure!(
                response.content_length().is_none_or(|n| n <= max_bytes),
                "download too large"
            );
            let mut output = tokio::fs::File::create(partial).await?;
            let mut stream = response.bytes_stream();
            let mut hash = Sha256::new();
            let mut length = 0u64;
            let mut prefix = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                if prefix.len() < 256 {
                    prefix.extend_from_slice(&chunk[..chunk.len().min(256 - prefix.len())]);
                }
                length = length.saturating_add(chunk.len() as u64);
                ensure!(length <= max_bytes, "download too large");
                hash.update(&chunk);
                output.write_all(&chunk).await?;
            }
            ensure!(length > 0, "empty download");
            let prefix = String::from_utf8_lossy(&prefix)
                .trim_start()
                .to_ascii_lowercase();
            ensure!(
                !prefix.starts_with("<!doctype html") && !prefix.starts_with("<html"),
                "received HTML instead of a download"
            );
            if let Some(expected) = expected_sha256 {
                let actual = format!("{:x}", hash.finalize());
                ensure!(
                    actual.eq_ignore_ascii_case(expected.trim()),
                    "checksum mismatch: expected {expected}, got {actual}"
                );
            }
            output.flush().await?;
            drop(output);
            tokio::fs::rename(partial, dest).await?;
            Ok(())
        }
    })
    .await;
    if result.is_err() {
        tokio::fs::remove_file(&partial).await.ok();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_order_and_private_requests() {
        let url = "https://raw.githubusercontent.com/a/b/main/file.json";
        assert_eq!(
            routes(url),
            vec![
                format!("{}{url}", PROXIES[0]),
                format!("{}{url}", PROXIES[1]),
                url.into()
            ]
        );
        assert_eq!(
            routes("https://api.github.com/repos/owner/repo/releases/latest").len(),
            3
        );
        for url in [
            "https://example.com/file",
            "https://api.github.com/user",
            "https://token@github.com/a/b",
            "https://github.com/a/b?token=secret",
            "http://github.com/a/b",
        ] {
            assert_eq!(routes(url), vec![url.to_owned()]);
        }
    }
    #[tokio::test]
    async fn failures_advance_and_success_stops() {
        let mut seen = Vec::new();
        let result = try_routes(
            vec!["first".into(), "second".into(), "direct".into()],
            |route| {
                seen.push(route.clone());
                async move {
                    if route == "second" {
                        Ok(42)
                    } else {
                        bail!("invalid response")
                    }
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(result, 42);
        assert_eq!(seen, vec!["first", "second"]);
    }

    async fn serve(body: &'static [u8], advertised: usize) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).await.unwrap();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {advertised}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
        });
        format!("http://{address}/resource")
    }

    #[tokio::test]
    async fn origin_fetch_does_not_fall_back_to_mirrors_on_failure() {
        use std::sync::{Arc, Mutex};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let length = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..length]);
                observed
                    .lock()
                    .unwrap()
                    .push(request.lines().next().unwrap().to_owned());
                socket.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
            }
        });
        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(proxy).unwrap())
            .build()
            .unwrap();
        let error = fetch_origin(
            &client,
            "https://github.com/owner/repo/manifest.json",
            100,
            Duration::from_secs(1),
            |bytes| Ok(bytes.to_vec()),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("GitHub resource unavailable"));
        assert_eq!(
            *requests.lock().unwrap(),
            ["CONNECT github.com:443 HTTP/1.1"]
        );
        server.abort();
    }

    #[tokio::test]
    async fn html_and_truncated_bodies_fall_back_to_a_valid_response() {
        let html = b"<html>proxy error</html>";
        let routes = vec![
            serve(html, html.len()).await,
            serve(b"truncated", 40).await,
            serve(b"valid", 5).await,
        ];
        let client = client().unwrap();
        let value = try_routes(routes, |route| {
            let client = &client;
            async move {
                fetch(client, &route, 100, Duration::from_secs(1), |bytes| {
                    ensure!(bytes == b"valid", "not the requested resource");
                    Ok(bytes.to_vec())
                })
                .await
            }
        })
        .await
        .unwrap();
        assert_eq!(value, b"valid");
    }

    #[tokio::test]
    async fn corrupted_downloads_retry_without_replacing_an_existing_file() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("artifact.zip");
        std::fs::write(&dest, b"old artifact").unwrap();
        let client = client().unwrap();
        let expected = format!("{:x}", Sha256::digest(b"valid"));
        let corrupt = serve(b"corrupt", 7).await;
        assert!(
            download(&client, &corrupt, &dest, Some(&expected), 100)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"old artifact");
        assert!(!dest.with_extension("partial").exists());
        let routes = vec![serve(b"broken", 20).await, serve(b"valid", 5).await];
        try_routes(routes, |route| {
            let (client, expected, dest) = (&client, &expected, &dest);
            async move { download(client, &route, dest, Some(expected), 100).await }
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(dest).unwrap(), b"valid");
    }
}
