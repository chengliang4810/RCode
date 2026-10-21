//! Fetch vendor models without exposing configured credentials to the UI.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use rcode_harness::provider_store::DiscoveredModel;
use rcode_harness::providers::{Protocol, ProviderConfig};
use reqwest::Url;
use serde::Deserialize;

const BODY_LIMIT: usize = 8 * 1024 * 1024;

fn models_url(base: &str) -> Result<Url, String> {
    let base = base.trim();
    let exact = base.ends_with('#');
    let mut url = Url::parse(base.strip_suffix('#').unwrap_or(base))
        .map_err(|_| "invalid provider URL".to_owned())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("invalid provider URL".into());
    }
    let path = url.path().trim_end_matches('/').to_owned();
    if path.ends_with("/models") {
        return Ok(url);
    }
    let request_suffix = ["/chat/completions", "/responses", "/messages"]
        .into_iter()
        .find(|suffix| path.ends_with(suffix));
    let prefix = match request_suffix {
        Some(suffix) => &path[..path.len() - suffix.len()],
        // An arbitrary exact generation URL has no discoverable base path.
        // Use the origin's standard models endpoint rather than its POST URL.
        None if exact && !path.ends_with("/v1") => "",
        None => &path,
    };
    let prefix = if prefix.is_empty() { "/v1" } else { prefix };
    url.set_path(&format!("{prefix}/models"));
    Ok(url)
}

#[derive(Deserialize)]
struct Page {
    data: Vec<VendorModel>,
    #[serde(default)]
    has_more: bool,
    last_id: Option<String>,
}

#[derive(Deserialize)]
struct VendorModel {
    id: String,
    display_name: Option<String>,
    owned_by: Option<String>,
}

pub(crate) async fn fetch(provider: &ProviderConfig) -> Result<Vec<DiscoveredModel>, String> {
    let key = provider.resolve_api_key(&std::env::vars().collect());
    let url = models_url(provider.endpoint())?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "could not initialize model discovery".to_owned())?;
    tokio::time::timeout(
        Duration::from_secs(60),
        fetch_pages(&client, url, provider.protocol, key.as_deref()),
    )
    .await
    .map_err(|_| "model discovery timed out".to_owned())?
}

async fn fetch_pages(
    client: &reqwest::Client,
    base: Url,
    protocol: Protocol,
    key: Option<&str>,
) -> Result<Vec<DiscoveredModel>, String> {
    let mut models = BTreeMap::new();
    let mut cursors = BTreeSet::new();
    let mut cursor: Option<String> = None;
    for _ in 0..100 {
        let mut url = base.clone();
        if let Some(cursor) = &cursor {
            let query = url
                .query_pairs()
                .filter(|(name, _)| name != "after_id")
                .map(|(name, value)| (name.into_owned(), value.into_owned()))
                .collect::<Vec<_>>();
            url.set_query(None);
            url.query_pairs_mut()
                .extend_pairs(query)
                .append_pair("after_id", cursor);
        }
        let mut request = client.get(url).header("accept", "application/json");
        if protocol == Protocol::AnthropicMessages {
            request = request.header("anthropic-version", "2023-06-01");
            if let Some(key) = key {
                request = request.header("x-api-key", key);
            }
        } else if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        let mut response = request
            .send()
            .await
            .map_err(|error| error.without_url().to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "model discovery returned HTTP {}",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > BODY_LIMIT as u64)
        {
            return Err("model list response is too large".into());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| error.without_url().to_string())?
        {
            if body.len().saturating_add(chunk.len()) > BODY_LIMIT {
                return Err("model list response is too large".into());
            }
            body.extend_from_slice(&chunk);
        }
        let page: Page =
            serde_json::from_slice(&body).map_err(|_| "invalid model list response".to_owned())?;
        for model in page.data {
            if model.id.trim().is_empty() || model.id.chars().any(char::is_control) {
                continue;
            }
            models.entry(model.id.clone()).or_insert(DiscoveredModel {
                model: model.id,
                display_name: model.display_name,
                owned_by: model.owned_by,
            });
        }
        if models.len() > 10_000 {
            return Err("model list has too many entries".into());
        }
        if !page.has_more {
            return Ok(models.into_values().collect());
        }
        let next = page
            .last_id
            .filter(|id| !id.is_empty())
            .ok_or("missing model pagination cursor")?;
        if !cursors.insert(next.clone()) {
            return Err("repeated model pagination cursor".into());
        }
        cursor = Some(next);
    }
    Err("model list has too many pages".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn discovery_uses_the_models_resource_for_bases_and_complete_request_urls() {
        for (input, expected) in [
            (
                "https://www.aidawan.fun",
                "https://www.aidawan.fun/v1/models",
            ),
            (
                "https://www.aidawan.fun/v1/",
                "https://www.aidawan.fun/v1/models",
            ),
            (
                "https://gateway.test/proxy/v1/chat/completions#",
                "https://gateway.test/proxy/v1/models",
            ),
            (
                "https://gateway.test/proxy/v1/responses?x=a%2Fb#",
                "https://gateway.test/proxy/v1/models?x=a%2Fb",
            ),
            (
                "https://gateway.test/v1/messages#",
                "https://gateway.test/v1/models",
            ),
            (
                "https://gateway.test/custom/invoke#",
                "https://gateway.test/v1/models",
            ),
            (
                "https://gateway.test/v1/models#",
                "https://gateway.test/v1/models",
            ),
        ] {
            assert_eq!(models_url(input).unwrap().as_str(), expected);
        }
        assert!(models_url("https://gateway.test/v1#fragment").is_err());
        assert!(models_url("https://user:key@gateway.test").is_err());
    }

    async fn server(responses: Vec<(u16, String)>) -> (Url, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!(
            "http://{}/v1/models",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                    assert_eq!(stream.read(&mut byte).await.unwrap(), 1);
                    request.push(byte[0]);
                }
                requests.push(String::from_utf8(request).unwrap());
                let response = format!(
                    "HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });
        (url, task)
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    #[tokio::test]
    async fn openai_discovery_reads_vendor_names_and_deduplicates_the_response() {
        let (url, server) =
            server(vec![(200, serde_json::json!({"data":[
            {"id":"z-model", "owned_by":"vendor"}, {"id":"a-model"}, {"id":"z-model"}, {"id":""}
        ]}).to_string())])
            .await;
        let models = fetch_pages(
            &client(),
            url,
            Protocol::OpenAiResponses,
            Some("synthetic-key"),
        )
        .await
        .unwrap();
        assert_eq!(
            models.iter().map(|m| m.model.as_str()).collect::<Vec<_>>(),
            ["a-model", "z-model"]
        );
        assert_eq!(models[1].owned_by.as_deref(), Some("vendor"));
        let requests = server.await.unwrap();
        assert!(requests[0].starts_with("GET /v1/models HTTP/1.1"));
        assert!(
            requests[0]
                .to_lowercase()
                .contains("authorization: bearer synthetic-key")
        );
    }

    #[tokio::test]
    async fn anthropic_discovery_uses_its_auth_headers_and_follows_pagination() {
        let (url, server) = server(vec![
            (200, serde_json::json!({"data":[{"id":"first", "display_name":"First model"}], "has_more":true, "last_id":"first"}).to_string()),
            (200, serde_json::json!({"data":[{"id":"second"}], "has_more":false}).to_string()),
        ]).await;
        let models = fetch_pages(
            &client(),
            url,
            Protocol::AnthropicMessages,
            Some("synthetic-key"),
        )
        .await
        .unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].display_name.as_deref(), Some("First model"));
        let requests = server.await.unwrap();
        assert!(
            requests[0]
                .to_lowercase()
                .contains("x-api-key: synthetic-key")
        );
        assert!(requests[0].contains("anthropic-version: 2023-06-01"));
        assert!(requests[1].starts_with("GET /v1/models?after_id=first HTTP/1.1"));
    }

    #[tokio::test]
    async fn discovery_reports_http_and_format_errors_without_echoing_response_bodies() {
        for (status, body, expected) in [
            (401, "private-server-details", "HTTP 401"),
            (200, "not json", "invalid model list response"),
        ] {
            let (url, server) = server(vec![(status, body.into())]).await;
            let error = fetch_pages(&client(), url, Protocol::OpenAiChat, None)
                .await
                .unwrap_err();
            assert!(error.contains(expected));
            assert!(!error.contains(body));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn discovery_rejects_a_repeated_pagination_cursor() {
        let page = serde_json::json!({"data":[], "has_more":true, "last_id":"same"}).to_string();
        let (url, server) = server(vec![(200, page.clone()), (200, page)]).await;
        assert!(
            fetch_pages(&client(), url, Protocol::AnthropicMessages, None)
                .await
                .unwrap_err()
                .contains("repeated")
        );
        server.await.unwrap();
    }
}
