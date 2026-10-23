//! `WebFetch` — retrieve a URL and hand the model readable text.
//!
//! HTML is stripped to text rather than returned raw: a page's markup is
//! mostly navigation and styling, and pasting it into the next request costs
//! tokens while telling the model nothing. The conversion is deliberately
//! crude — a real reader-mode extractor is a project of its own — but it
//! removes the tags, scripts and styles, which is where the waste is.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::truncate;
use crate::tool::{Tool, ToolContext, ToolError, ToolResult};

/// Response bytes read before giving up. A page larger than this is not
/// something the model should be reading through a tool call.
const MAX_BYTES: usize = 2 * 1024 * 1024;
/// Characters returned before truncation.
const MAX_CHARS: usize = 100_000;
/// How long a fetch may take.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "WebFetch"
    }

    fn description(&self) -> &str {
        "Fetch a URL and return its text. Use WebFetch to read a page you have \
         a reason to trust — documentation, a changelog, an issue. HTML is \
         reduced to text; very long pages are truncated."
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool { true }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch. Must be http or https."
                },
                "prompt": {
                    "type": "string",
                    "description": "What you are looking for, so the result can be summarised around it."
                }
            },
            "required": ["url"]
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let url = input
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("`url` is required".to_owned()))?;
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ToolError::InvalidInput(format!(
                "`{url}` is not an http(s) URL"
            )));
        }

        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("rcode-agent/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| ToolError::Execution(error.to_string()))?;

        let mut response = tokio::select! {
            result = client.get(url).send() => {
                result.map_err(|error| ToolError::Execution(format!("cannot fetch {url}: {error}")))?
            }
            _ = ctx.cancel.cancelled() => return Err(ToolError::Cancelled),
        };

        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();

        if response.content_length().is_some_and(|length| length > MAX_BYTES as u64) {
            return Err(ToolError::Execution(format!(
                "{url} exceeds the {MAX_BYTES}-byte limit"
            )));
        }
        let mut bytes = Vec::new();
        loop {
            let chunk = tokio::select! {
                biased;
                _ = ctx.cancel.cancelled() => return Err(ToolError::Cancelled),
                result = response.chunk() => result.map_err(|error| ToolError::Execution(format!("cannot read {url}: {error}")))?,
            };
            let Some(chunk) = chunk else { break; };
            if chunk.len() > MAX_BYTES.saturating_sub(bytes.len()) {
                return Err(ToolError::Execution(format!("{url} exceeds the {MAX_BYTES}-byte limit")));
            }
            bytes.extend_from_slice(&chunk);
        }

        let body = String::from_utf8_lossy(&bytes);
        let text = if content_type.contains("html") {
            html_to_text(&body)
        } else {
            body.into_owned()
        };

        if !status.is_success() {
            // The body often explains a 4xx better than the code does, so it is
            // reported rather than replaced by the status alone.
            let (text, _) = truncate(&text, 4_000);
            return Ok(ToolResult {
                output: format!("{url} returned HTTP {status}\n\n{text}"),
                is_error: true,
                diff: None,
            });
        }

        let (text, truncated) = truncate(&text, MAX_CHARS);
        let note = if truncated { "\n\n… truncated" } else { "" };
        Ok(ToolResult::ok(format!("{url}\n\n{text}{note}")))
    }
}

/// Reduce HTML to readable text.
///
/// Removes `script`/`style`/`noscript` bodies and comments, turns block-level
/// tags into newlines so paragraphs do not run together, then drops the
/// remaining tags and decodes the entities that actually appear in prose.
pub(crate) fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    // ASCII folding preserves byte offsets. Compute it once rather than
    // copying the remaining document for every script or style block.
    let lower = html.to_ascii_lowercase();
    let mut rest = html;

    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            // An unterminated tag: the rest is text as far as we can tell.
            out.push_str(&rest[start..]);
            return collapse_blank_lines(&decode_entities(&out));
        };
        let tag = &rest[start + 1..start + end];
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();

        // Skip the contents of these entirely — they are not prose.
        if matches!(name.as_str(), "script" | "style" | "noscript" | "template") {
            let closing = format!("</{name}");
            let consumed = html.len() - rest.len();
            match lower[consumed + start..].find(&closing) {
                Some(offset) => {
                    let after = start + offset;
                    rest = match rest[after..].find('>') {
                        Some(end) => &rest[after + end + 1..],
                        None => "",
                    };
                    continue;
                }
                None => return collapse_blank_lines(&decode_entities(&out)),
            }
        }

        if is_block_tag(&name) {
            out.push('\n');
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    collapse_blank_lines(&decode_entities(&out))
}

/// Tags whose boundaries separate text.
fn is_block_tag(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "br"
            | "li"
            | "tr"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "blockquote"
            | "pre"
            | "table"
            | "ul"
            | "ol"
    )
}

/// The entities that appear in ordinary prose. A full table would be longer
/// than it is worth; anything unrecognised is left as written, which is
/// visible and honest rather than silently wrong.
fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&hellip;", "…")
        .replace("&rsquo;", "\u{2019}")
        .replace("&lsquo;", "\u{2018}")
        .replace("&ldquo;", "\u{201C}")
        .replace("&rdquo;", "\u{201D}")
}

/// Collapse runs of blank lines and strip trailing whitespace per line.
fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(trimmed);
        out.push('\n');
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    fn context() -> ToolContext {
        ToolContext {
            cwd: std::env::temp_dir(),
            cancel: CancellationToken::new(),
            nested: None,
        }
    }

    #[test]
    fn html_reduces_to_its_prose() {
        let text = html_to_text(
            "<html><head><title>T</title><style>body{color:red}</style></head>\
             <body><h1>Heading</h1><p>First para.</p><p>Second para.</p></body></html>",
        );
        assert!(text.contains("Heading"));
        assert!(text.contains("First para."));
        assert!(text.contains("Second para."));
        // The stylesheet is not prose.
        assert!(!text.contains("color:red"));
        assert!(!text.contains("<p>"));
    }

    #[test]
    fn script_bodies_are_dropped() {
        let text = html_to_text("<p>before</p><script>var x = '<p>fake</p>';</script><p>after</p>");
        assert!(text.contains("before"));
        assert!(text.contains("after"));
        assert!(!text.contains("fake"), "script body leaked: {text}");
    }

    #[test]
    fn mixed_case_hidden_blocks_preserve_unicode_prose() {
        let html = "<p>前文</p><ScRiPt>ignored</sCrIpT><STYLE>ignored</STYLE><p>后文</p>";
        assert_eq!(html_to_text(html), "前文\n\n后文");
        let many = format!("{}<p>kept</p>", "<STYLE>ignored</STYLE>".repeat(10_000));
        assert_eq!(html_to_text(&many), "kept");
    }

    #[tokio::test]
    async fn an_oversized_chunked_body_is_refused_before_eof() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).await.unwrap();
            socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
            let chunk = vec![b'x'; 64 * 1024];
            for _ in 0..=MAX_BYTES / chunk.len() {
                if socket.write_all(b"10000\r\n").await.is_err() { return; }
                if socket.write_all(&chunk).await.is_err() { return; }
                if socket.write_all(b"\r\n").await.is_err() { return; }
            }
            // Do not send the terminating chunk: the limit must stop reading.
            let mut eof = [0];
            let _ = socket.read(&mut eof).await;
        });
        let error = tokio::time::timeout(std::time::Duration::from_secs(3), WebFetchTool.call(json!({"url":url}), &context()))
            .await.expect("body limit must not wait for EOF").unwrap_err();
        assert!(error.to_string().contains("byte limit"), "{error}");
        server.abort();
    }

    #[tokio::test]
    async fn reading_a_stalled_body_remains_cancellable() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (ready, wait) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).await.unwrap();
            socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\n").await.unwrap();
            let _ = ready.send(());
            let _ = socket.read(&mut request).await;
        });
        let ctx = context();
        let cancel = ctx.cancel.clone();
        let fetch = tokio::spawn(async move { WebFetchTool.call(json!({"url":url}), &ctx).await });
        wait.await.unwrap();
        cancel.cancel();
        let error = tokio::time::timeout(std::time::Duration::from_secs(1), fetch).await.unwrap().unwrap().unwrap_err();
        assert!(matches!(error, ToolError::Cancelled));
        server.abort();
    }

    #[test]
    fn entities_in_prose_are_decoded() {
        let text = html_to_text("<p>a &amp; b &lt;tag&gt; &mdash; done</p>");
        assert!(text.contains("a & b <tag> — done"));
    }

    #[test]
    fn block_tags_separate_paragraphs() {
        let text = html_to_text("<p>one</p><p>two</p>");
        // A blank line between paragraphs: the closing and opening tags each
        // contribute a break, which reads the way the page does.
        assert_eq!(text, "one\n\ntwo");
    }

    #[test]
    fn blank_line_runs_are_collapsed() {
        let text = html_to_text("<p>one</p><br><br><br><p>two</p>");
        assert!(!text.contains("\n\n\n"), "got: {text:?}");
    }

    #[test]
    fn an_unterminated_tag_does_not_lose_the_tail() {
        let text = html_to_text("<p>kept</p><p>unclosed");
        assert!(text.contains("kept"));
        assert!(text.contains("unclosed"));
    }

    #[tokio::test]
    async fn a_non_http_url_is_refused() {
        let error = WebFetchTool
            .call(json!({ "url": "file:///etc/passwd" }), &context())
            .await
            .expect_err("refused");
        assert!(error.to_string().contains("not an http(s) URL"));
    }

    #[tokio::test]
    async fn a_missing_url_is_refused() {
        let error = WebFetchTool
            .call(json!({}), &context())
            .await
            .expect_err("refused");
        assert!(matches!(error, ToolError::InvalidInput(_)));
    }
}
