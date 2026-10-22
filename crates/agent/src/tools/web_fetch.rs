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

        let response = tokio::select! {
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

        let bytes = tokio::select! {
            result = response.bytes() => {
                result.map_err(|error| ToolError::Execution(format!("cannot read {url}: {error}")))?
            }
            _ = ctx.cancel.cancelled() => return Err(ToolError::Cancelled),
        };
        if bytes.len() > MAX_BYTES {
            return Err(ToolError::Execution(format!(
                "{url} returned {} bytes, over the {MAX_BYTES}-byte limit",
                bytes.len()
            )));
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
            match rest[start..].to_ascii_lowercase().find(&closing) {
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
