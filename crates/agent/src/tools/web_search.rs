//! `WebSearch` 使用 Tavily 协议：`POST {endpoint}/search`，JSON 请求包含
//! `query` 和 `max_results`，可选 Bearer token 只来自独立的搜索配置。
//!
//! 空配置在调用时使用内置的免费公共端点，不把端点或凭据写入默认配置。
//! 自定义后端必须提供 Tavily 兼容接口；SearXNG、Brave 等原生 API 并不兼容。
//! 公共服务不可用时明确返回失败，不把失败解释成没有搜索结果。

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::tool::{Tool, ToolContext, ToolError, ToolResult};

/// 内置的免费兜底端点，不承诺第三方服务的可用性。
const DEFAULT_ENDPOINT: &str = "https://tavily.claude-code-best.win";
const DEFAULT_RESULTS: usize = 10;
const MAX_RESULTS: usize = 20;
/// 输入和输出都按字符截断，避免切断 UTF-8 多字节序列。
const MAX_QUERY_CHARS: usize = 2_000;
const MAX_TITLE_CHARS: usize = 300;
const MAX_URL_CHARS: usize = 2_048;
const MAX_SNIPPET_CHARS: usize = 500;
/// 流读取过程中检查上限，而不是整包下载后才检查大小。
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
/// 覆盖发送请求与读取完整响应的总超时。
const TIMEOUT: Duration = Duration::from_secs(20);

/// 独立的搜索配置，绝不从模型 provider 继承密钥，也不读取进程环境。
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebSearchConfig {
    /// Tavily 后端根地址；空字符串表示使用免费兜底端点。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub endpoint: String,
    /// 可选搜索 Bearer token；缺省或空白时不发送 Authorization。
    #[serde(skip_serializing_if = "Option::is_none", alias = "apiKey")]
    pub token: Option<String>,
}

impl fmt::Debug for WebSearchConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebSearchConfig")
            // 地址也不打印，避免误配置在 URL 中的凭据进入日志。
            .field(
                "endpoint",
                &if self.endpoint.trim().is_empty() {
                    "<default>"
                } else {
                    "<configured>"
                },
            )
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl WebSearchConfig {
    fn resolved_endpoint(&self) -> &str {
        let trimmed = self.endpoint.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            DEFAULT_ENDPOINT
        } else {
            trimmed
        }
    }
}

pub struct WebSearchTool {
    config: Option<Arc<WebSearchConfig>>,
    timeout: Duration,
}

impl WebSearchTool {
    /// 明示禁用，不等同于空配置；宿主可用此实例覆盖默认注册。
    pub fn disabled() -> Self {
        Self {
            config: None,
            timeout: TIMEOUT,
        }
    }

    /// 只接收搜索配置，不接受模型 provider 或其凭据。
    pub fn new(config: WebSearchConfig) -> Self {
        Self {
            config: Some(Arc::new(config)),
            timeout: TIMEOUT,
        }
    }

    /// 免费兜底端点，无凭据。
    pub fn with_default_backend() -> Self {
        Self::new(WebSearchConfig::default())
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
    }

    fn description(&self) -> &str {
        "Search the web for current information. Use it when the answer depends \
         on something after your training data — a release, an error message, a \
         version's behaviour. Prefer WebFetch when you already have the URL."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_QUERY_CHARS,
                    "description": "What to search for."
                },
                "num_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_RESULTS,
                    "default": DEFAULT_RESULTS,
                    "description": "Number of results to return (1–20, default 10)."
                }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        if ctx.cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let query = input
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .ok_or_else(|| ToolError::InvalidInput("`query` is required".to_owned()))?;
        if query.chars().count() > MAX_QUERY_CHARS {
            return Err(ToolError::InvalidInput(format!(
                "`query` exceeds {MAX_QUERY_CHARS} characters"
            )));
        }
        let num_results = match input.get("num_results") {
            None => DEFAULT_RESULTS,
            Some(value) => value
                .as_u64()
                .filter(|n| (1..=MAX_RESULTS as u64).contains(n))
                .ok_or_else(|| {
                    ToolError::InvalidInput(format!(
                        "`num_results` must be an integer from 1 to {MAX_RESULTS}"
                    ))
                })? as usize,
        };

        let Some(config) = &self.config else {
            return Ok(ToolResult::ok(
                "Web search is turned off on this device. Answer without \
                 searching, or ask the user to enable it.",
            ));
        };

        let client = search_client(self.timeout)?;
        let request = search_request(&client, config, query, num_results)?;
        match search_response(&client, request, ctx, self.timeout).await? {
            SearchResponse::Results(value) => render_results(&value, query, num_results),
            SearchResponse::HttpError(status) => Ok(ToolResult::error(format!(
                "The search endpoint returned HTTP {status}."
            ))),
        }
    }
}

fn search_client(timeout: Duration) -> Result<reqwest::Client, ToolError> {
    let builder = reqwest::Client::builder()
        .timeout(timeout)
        // 不跟随重定向，避免把查询或搜索凭据发给另一个主机。
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("rcode-agent/", env!("CARGO_PKG_VERSION")));
    // 单元测试仅访问回环桩，不借用宿主代理或其凭据。
    #[cfg(test)]
    let builder = builder.no_proxy();
    builder.build().map_err(search_error)
}

fn search_request(
    client: &reqwest::Client,
    config: &WebSearchConfig,
    query: &str,
    num_results: usize,
) -> Result<reqwest::Request, ToolError> {
    let url =
        reqwest::Url::parse(&format!("{}/search", config.resolved_endpoint())).map_err(|_| {
            ToolError::InvalidInput("search endpoint must be an http(s) base URL".into())
        })?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ToolError::InvalidInput(
            "search endpoint must be an http(s) base URL without credentials, query or fragment"
                .into(),
        ));
    }
    let mut request = client
        .post(url)
        .json(&json!({ "query": query, "max_results": num_results }));
    if let Some(token) = config
        .token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        request = request.bearer_auth(token);
    }
    request.build().map_err(search_error)
}

fn search_error(error: reqwest::Error) -> ToolError {
    if error.is_timeout() {
        ToolError::Timeout("web search request".into())
    } else {
        // reqwest 的默认错误包含 URL；配置错误可能把凭据放在 URL 中。
        ToolError::Execution(format!("search failed: {}", error.without_url()))
    }
}

enum SearchResponse {
    Results(Value),
    HttpError(reqwest::StatusCode),
}

async fn search_response(
    client: &reqwest::Client,
    request: reqwest::Request,
    ctx: &ToolContext,
    timeout: Duration,
) -> Result<SearchResponse, ToolError> {
    let fetch = async {
        let mut response = client.execute(request).await.map_err(search_error)?;
        let status = response.status();
        if !status.is_success() {
            // 无需读取错误正文，它可能无限等待或回显凭据。
            return Ok(SearchResponse::HttpError(status));
        }
        if response
            .content_length()
            .is_some_and(|len| len > MAX_RESPONSE_BYTES as u64)
        {
            return Err(response_too_large());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(search_error)? {
            if chunk.len() > MAX_RESPONSE_BYTES - body.len() {
                return Err(response_too_large());
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body)
            .map(SearchResponse::Results)
            .map_err(|error| {
                ToolError::Execution(format!("the search endpoint did not return JSON: {error}"))
            })
    };
    tokio::select! {
        biased;
        _ = ctx.cancel.cancelled() => Err(ToolError::Cancelled),
        result = tokio::time::timeout(timeout, fetch) => {
            result.map_err(|_| ToolError::Timeout("web search request".into()))?
        }
    }
}

fn response_too_large() -> ToolError {
    ToolError::Execution(format!(
        "search response exceeds the {MAX_RESPONSE_BYTES}-byte limit"
    ))
}

fn result_items(value: &Value) -> Option<&Vec<Value>> {
    value
        .as_array()
        .or_else(|| value.get("results").and_then(Value::as_array))
        .or_else(|| value.get("items").and_then(Value::as_array))
        .or_else(|| value.get("data").and_then(Value::as_array))
}

/// 保留已有的响应字段宽容性；这不代表兼容其他搜索服务的请求协议。
fn extract_results(value: &Value, num_results: usize) -> Vec<(String, String, String)> {
    let Some(items) = result_items(value) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let url = item
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| !url.trim().is_empty())?;
            let title = item.get("title").and_then(Value::as_str).unwrap_or(url);
            let snippet = ["snippet", "content", "description"]
                .iter()
                .find_map(|key| item.get(*key).and_then(Value::as_str))
                .unwrap_or_default();
            Some((
                title.chars().take(MAX_TITLE_CHARS).collect(),
                url.chars().take(MAX_URL_CHARS).collect(),
                snippet.chars().take(MAX_SNIPPET_CHARS).collect(),
            ))
        })
        .take(num_results.min(MAX_RESULTS))
        .collect()
}

fn render_results(value: &Value, query: &str, num_results: usize) -> Result<ToolResult, ToolError> {
    let items = result_items(value).ok_or_else(|| {
        ToolError::Execution(
            "the search endpoint returned no results array; expected Tavily-compatible JSON".into(),
        )
    })?;
    let results = extract_results(value, num_results);
    if results.is_empty() {
        if !items.is_empty() {
            return Err(ToolError::Execution(
                "the search endpoint returned no readable result URLs".into(),
            ));
        }
        return Ok(ToolResult::ok(format!("No results for `{query}`.")));
    }
    let rendered = results
        .iter()
        .enumerate()
        .map(|(index, (title, url, snippet))| {
            let mut entry = format!("{}. {title}\n   {url}", index + 1);
            if !snippet.is_empty() {
                entry.push_str(&format!("\n   {snippet}"));
            }
            entry
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(ToolResult::ok(format!(
        "{} result(s) for `{query}`:\n\n{rendered}",
        results.len()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio::task::JoinHandle;
    use tokio_util::sync::CancellationToken;

    #[derive(Debug)]
    struct CapturedRequest {
        first_line: String,
        headers: Vec<(String, String)>,
        body: Value,
    }

    impl CapturedRequest {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        }
    }

    struct Stub {
        endpoint: String,
        captured: oneshot::Receiver<CapturedRequest>,
        task: JoinHandle<()>,
    }

    impl Drop for Stub {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    impl Stub {
        async fn request(&mut self) -> CapturedRequest {
            tokio::time::timeout(Duration::from_secs(3), &mut self.captured)
                .await
                .expect("request reached stub")
                .expect("request captured")
        }

        fn tool(&self, token: Option<&str>) -> WebSearchTool {
            WebSearchTool::new(WebSearchConfig {
                endpoint: format!(" {}/ ", self.endpoint),
                token: token.map(str::to_owned),
            })
        }
    }

    /// 只监听回环，读取一条完整 HTTP 请求；不访问真实端点或宿主凭据。
    async fn stub(response: String, hold_open: bool) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let endpoint = format!("http://{}", listener.local_addr().expect("local address"));
        let (tx, captured) = oneshot::channel();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut buffer = [0; 1024];
                let read = socket.read(&mut buffer).await.expect("read headers");
                assert!(read > 0, "request ended before headers");
                bytes.extend_from_slice(&buffer[..read]);
                assert!(bytes.len() <= 16 * 1024, "request headers too large");
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let header = String::from_utf8(bytes[..header_end].to_vec()).expect("UTF-8 headers");
            let mut lines = header.lines();
            let first_line = lines.next().expect("request line").to_owned();
            let headers: Vec<_> = lines
                .filter_map(|line| line.split_once(':'))
                .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
                .collect();
            let length: usize = headers
                .iter()
                .find(|(key, _)| key == "content-length")
                .expect("content length")
                .1
                .parse()
                .expect("numeric length");
            assert!(length <= 16 * 1024, "request body too large");
            while bytes.len() < header_end + length {
                let mut buffer = [0; 1024];
                let read = socket.read(&mut buffer).await.expect("read body");
                assert!(read > 0, "request ended before body");
                bytes.extend_from_slice(&buffer[..read]);
            }
            let body = serde_json::from_slice(&bytes[header_end..header_end + length])
                .expect("JSON request");
            // 部分正文发送完成后通知测试，取消用例不依赖定时 sleep。
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = tx.send(CapturedRequest {
                first_line,
                headers,
                body,
            });
            if hold_open {
                std::future::pending::<()>().await;
            }
        });
        Stub {
            endpoint,
            captured,
            task,
        }
    }

    async fn json_stub(body: Value) -> Stub {
        let body = body.to_string();
        stub(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()), false).await
    }

    fn context() -> ToolContext {
        ToolContext {
            cwd: std::env::temp_dir(),
            cancel: CancellationToken::new(),
            nested: None,
        }
    }

    #[test]
    fn results_are_read_from_a_bare_array() {
        let value = json!([
            { "title": "One", "url": "https://a.test", "snippet": "first" },
            { "title": "Two", "url": "https://b.test", "content": "second" }
        ]);
        let results = extract_results(&value, DEFAULT_RESULTS);
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0],
            ("One".into(), "https://a.test".into(), "first".into())
        );
        assert_eq!(results[1].2, "second");
    }

    #[test]
    fn results_are_read_from_a_wrapped_object() {
        for key in ["results", "items", "data"] {
            let value = json!({ key: [{ "title": "T", "url": "https://x.test" }] });
            assert_eq!(
                extract_results(&value, DEFAULT_RESULTS).len(),
                1,
                "key {key} was not read"
            );
        }
    }

    #[test]
    fn an_entry_without_a_url_is_skipped() {
        let value = json!([{ "title": "no url" }, { "url": "https://x.test" }]);
        let results = extract_results(&value, DEFAULT_RESULTS);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "https://x.test");
    }

    #[test]
    fn an_unrecognisable_shape_yields_nothing() {
        assert!(extract_results(&json!({ "unexpected": true }), DEFAULT_RESULTS).is_empty());
        assert!(extract_results(&json!("a string"), DEFAULT_RESULTS).is_empty());
    }

    #[test]
    fn results_are_capped() {
        let items: Vec<Value> = (0..MAX_RESULTS + 5)
            .map(|index| json!({ "url": format!("https://x.test/{index}") }))
            .collect();
        assert_eq!(
            extract_results(&json!(items), MAX_RESULTS).len(),
            MAX_RESULTS
        );
        assert_eq!(extract_results(&json!(items), 1).len(), 1);
    }

    #[tokio::test]
    async fn a_disabled_tool_says_so_without_failing() {
        let result = WebSearchTool::disabled()
            .call(json!({ "query": "anything" }), &context())
            .await
            .expect("does not fail");
        assert!(!result.is_error);
        assert!(result.output.contains("turned off"));
    }

    #[tokio::test]
    async fn an_empty_query_is_refused() {
        let error = WebSearchTool::disabled()
            .call(json!({ "query": "   " }), &context())
            .await
            .expect_err("refused");
        assert!(matches!(error, ToolError::InvalidInput(_)));
    }

    #[test]
    fn empty_configuration_stays_empty_and_debug_is_redacted() {
        let config: WebSearchConfig = serde_json::from_value(json!({})).expect("empty config");
        assert_eq!(config, WebSearchConfig::default());
        assert_eq!(serde_json::to_value(&config).expect("serialize"), json!({}));
        assert_eq!(config.resolved_endpoint(), DEFAULT_ENDPOINT);
        let config = WebSearchConfig {
            endpoint: "https://user:endpoint-secret@example.test".into(),
            token: Some("search-secret".into()),
        };
        let debug = format!("{config:?}");
        assert!(!debug.contains("search-secret"));
        assert!(!debug.contains("endpoint-secret"));
        assert!(debug.contains("<redacted>"));
    }

    #[tokio::test]
    async fn empty_config_falls_back_and_posts_without_auth() {
        let client = search_client(TIMEOUT).expect("client");
        for endpoint in ["", "   ", " /// "] {
            let config = WebSearchConfig {
                endpoint: endpoint.into(),
                token: None,
            };
            let mut request =
                search_request(&client, &config, "rust 搜索", DEFAULT_RESULTS).expect("request");
            assert_eq!(request.url().as_str(), format!("{DEFAULT_ENDPOINT}/search"));
            assert_eq!(request.method(), reqwest::Method::POST);
            assert!(
                !request
                    .headers()
                    .contains_key(reqwest::header::AUTHORIZATION)
            );
            let mut server = json_stub(json!({"results": [{"title": "Rust", "url": "https://rust.test", "content": "摘要"}]})).await;
            // 仅替换已断言的请求 URL，将真实默认请求送到回环桩；不访问免费服务。
            *request.url_mut() =
                reqwest::Url::parse(&format!("{}/search", server.endpoint)).expect("loopback URL");
            let response = search_response(&client, request, &context(), TIMEOUT)
                .await
                .expect("search response");
            let SearchResponse::Results(value) = response else {
                panic!("successful JSON response");
            };
            let result = render_results(&value, "rust 搜索", DEFAULT_RESULTS).expect("render");
            assert!(result.output.contains("摘要"));
            let captured = server.request().await;
            assert_eq!(captured.first_line, "POST /search HTTP/1.1");
            assert_eq!(captured.header("authorization"), None);
            assert_eq!(captured.header("content-type"), Some("application/json"));
            assert_eq!(
                captured.body,
                json!({"query": "rust 搜索", "max_results": 10})
            );
        }
    }

    #[tokio::test]
    async fn custom_endpoint_token_and_result_count_are_used() {
        let mut server = json_stub(json!({"results": [
            {"title": "One", "url": "https://one.test", "content": "first"},
            {"title": "Two", "url": "https://two.test", "content": "second"}
        ]}))
        .await;
        let result = server
            .tool(Some(" test-search-token "))
            .call(json!({"query": "  custom  ", "num_results": 1}), &context())
            .await
            .expect("custom search");
        assert!(!result.is_error);
        assert!(result.output.contains("1 result(s)"));
        assert!(!result.output.contains("two.test"));
        let captured = server.request().await;
        assert_eq!(captured.first_line, "POST /search HTTP/1.1");
        assert_eq!(
            captured.header("authorization"),
            Some("Bearer test-search-token")
        );
        assert_eq!(captured.body, json!({"query": "custom", "max_results": 1}));
    }

    #[tokio::test]
    async fn empty_tokens_do_not_send_authorization_and_twenty_is_valid() {
        for token in [None, Some(""), Some("   ")] {
            let mut server = json_stub(json!({"results": []})).await;
            let result = server
                .tool(token)
                .call(json!({"query": "missing", "num_results": 20}), &context())
                .await
                .expect("empty results");
            assert!(!result.is_error);
            assert_eq!(result.output, "No results for `missing`.");
            let captured = server.request().await;
            assert_eq!(captured.header("authorization"), None);
            assert_eq!(captured.body["max_results"], 20);
        }
    }

    #[tokio::test]
    async fn invalid_result_counts_and_oversized_queries_are_refused() {
        let tool = WebSearchTool::with_default_backend();
        for num_results in [
            json!(0),
            json!(21),
            json!(-1),
            json!(1.5),
            json!("2"),
            json!(null),
            json!(true),
            json!(u64::MAX),
        ] {
            let error = tool
                .call(
                    json!({"query": "test", "num_results": num_results}),
                    &context(),
                )
                .await
                .expect_err("invalid count does not request network");
            assert!(matches!(error, ToolError::InvalidInput(_)));
        }
        let error = tool
            .call(
                json!({"query": "界".repeat(MAX_QUERY_CHARS + 1)}),
                &context(),
            )
            .await
            .expect_err("oversized query refused");
        assert!(matches!(error, ToolError::InvalidInput(_)));
        let schema = tool.input_schema();
        assert_eq!(schema["properties"]["num_results"]["minimum"], 1);
        assert_eq!(schema["properties"]["num_results"]["maximum"], 20);
    }

    #[test]
    fn invalid_endpoint_and_token_are_refused_without_disclosing_secrets() {
        let client = search_client(TIMEOUT).expect("client");
        for endpoint in [
            "not a URL",
            "file:///tmp",
            "https://user:secret@host.test",
            "https://host.test?token=secret",
            "https://host.test#secret",
        ] {
            let config = WebSearchConfig {
                endpoint: endpoint.into(),
                token: None,
            };
            let error = search_request(&client, &config, "test", 10).expect_err("invalid endpoint");
            assert!(matches!(error, ToolError::InvalidInput(_)));
            assert!(!error.to_string().contains("secret"));
        }
        let config = WebSearchConfig {
            endpoint: String::new(),
            token: Some("secret\r\nHeader: bad".into()),
        };
        let error = search_request(&client, &config, "test", 10).expect_err("invalid token");
        assert!(!error.to_string().contains("secret"));
    }

    #[tokio::test]
    async fn non_success_status_is_an_error_without_waiting_for_body() {
        for status in [
            "429 Too Many Requests",
            "503 Service Unavailable",
            "302 Found",
        ] {
            let mut server = stub(format!("HTTP/1.1 {status}\r\nLocation: https://do-not-follow.test\r\nContent-Length: 99999\r\n\r\n"), true).await;
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                server.tool(None).call(json!({"query": "test"}), &context()),
            )
            .await
            .expect("does not wait for error body")
            .expect("HTTP error result");
            assert!(result.is_error);
            assert!(result.output.contains(&status[..3]));
            server.request().await;
        }
    }

    #[tokio::test]
    async fn malformed_json_and_wrong_shapes_fail_not_no_results() {
        for body in [
            "{ not json",
            "{\"results\":{}}",
            "{\"unexpected\":true}",
            "{\"results\":[{\"title\":\"no URL\"}]}",
        ] {
            let mut server = stub(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                ),
                false,
            )
            .await;
            let error = server
                .tool(None)
                .call(json!({"query": "test"}), &context())
                .await
                .expect_err("malformed response");
            assert!(matches!(error, ToolError::Execution(_)));
            server.request().await;
        }
    }

    #[tokio::test]
    async fn cancellation_reaches_header_and_body_waits() {
        for response in [
            String::new(),
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{".into(),
        ] {
            let mut server = stub(response, true).await;
            let ctx = context();
            let run_ctx = ctx.clone();
            let tool = server.tool(None);
            let call =
                tokio::spawn(async move { tool.call(json!({"query": "test"}), &run_ctx).await });
            server.request().await;
            ctx.cancel.cancel();
            let error = tokio::time::timeout(Duration::from_secs(2), call)
                .await
                .expect("cancel completes")
                .expect("call joins")
                .expect_err("cancelled search");
            assert!(matches!(error, ToolError::Cancelled));
        }
    }

    #[tokio::test]
    async fn already_cancelled_default_tool_does_not_start_a_request() {
        let ctx = context();
        ctx.cancel.cancel();
        let error = WebSearchTool::with_default_backend()
            .call(json!({"query": "test"}), &ctx)
            .await
            .expect_err("cancelled");
        assert!(matches!(error, ToolError::Cancelled));
    }

    #[tokio::test]
    async fn timeouts_cover_header_and_body_waits() {
        for response in [
            String::new(),
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{".into(),
        ] {
            let mut server = stub(response, true).await;
            let mut tool = server.tool(None);
            tool.timeout = Duration::from_millis(100);
            let error = tokio::time::timeout(
                Duration::from_secs(2),
                tool.call(json!({"query": "test"}), &context()),
            )
            .await
            .expect("total timeout completes")
            .expect_err("timed out");
            assert!(matches!(error, ToolError::Timeout(_)));
            server.request().await;
        }
    }

    #[tokio::test]
    async fn response_size_is_bounded_before_eof_with_and_without_content_length() {
        let huge = "x".repeat(MAX_RESPONSE_BYTES + 1);
        let chunked = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{huge}\r\n",
            huge.len()
        );
        for response in [
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                MAX_RESPONSE_BYTES + 1
            ),
            chunked,
        ] {
            let mut server = stub(response, true).await;
            let error = tokio::time::timeout(
                Duration::from_secs(2),
                server.tool(None).call(json!({"query": "test"}), &context()),
            )
            .await
            .expect("size limit does not wait for EOF")
            .expect_err("too large");
            assert!(matches!(error, ToolError::Execution(_)));
            assert!(error.to_string().contains("byte limit"));
            server.request().await;
        }
    }

    #[tokio::test]
    async fn result_text_and_count_are_bounded_for_unicode() {
        let item = json!({
            "title": "题".repeat(MAX_TITLE_CHARS + 100),
            "url": format!("https://x.test/{}", "界".repeat(MAX_URL_CHARS)),
            "content": "文".repeat(MAX_SNIPPET_CHARS + 100),
        });
        let value = json!({"results": vec![item; MAX_RESULTS + 5]});
        let results = extract_results(&value, MAX_RESULTS);
        assert_eq!(results.len(), MAX_RESULTS);
        for (title, url, snippet) in &results {
            assert_eq!(title.chars().count(), MAX_TITLE_CHARS);
            assert_eq!(url.chars().count(), MAX_URL_CHARS);
            assert_eq!(snippet.chars().count(), MAX_SNIPPET_CHARS);
        }
        let mut server = json_stub(value).await;
        let result = server
            .tool(None)
            .call(json!({"query": "test"}), &context())
            .await
            .expect("bounded results");
        assert!(result.output.starts_with("10 result(s)"));
        assert!(
            result.output.chars().count()
                < DEFAULT_RESULTS * (MAX_TITLE_CHARS + MAX_URL_CHARS + MAX_SNIPPET_CHARS + 20)
                    + MAX_QUERY_CHARS
        );
        assert_eq!(server.request().await.body["max_results"], DEFAULT_RESULTS);
    }

    #[tokio::test]
    async fn core_tools_register_search_and_custom_config_replaces_it_in_place() {
        let mut registry = crate::tool::ToolRegistry::new();
        super::super::register_core_tools(&mut registry);
        let names: Vec<String> = registry
            .all()
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect();
        assert_eq!(names.iter().filter(|name| *name == "WebSearch").count(), 1);
        let mut server = json_stub(json!({"results": []})).await;
        super::super::register_core_tools_with_search(
            &mut registry,
            WebSearchConfig {
                endpoint: server.endpoint.clone(),
                token: Some("only-search-token".into()),
            },
        );
        let after: Vec<String> = registry
            .all()
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect();
        assert_eq!(names, after);
        registry
            .call("WebSearch", json!({"query": "override"}), &context())
            .await
            .expect("custom override");
        assert_eq!(
            server.request().await.header("authorization"),
            Some("Bearer only-search-token")
        );
        registry.register(Arc::new(WebSearchTool::disabled()));
        let result = registry
            .call("WebSearch", json!({"query": "disabled"}), &context())
            .await
            .expect("disabled override");
        assert!(result.output.contains("turned off"));
        assert_eq!(registry.len(), names.len());
    }
}
