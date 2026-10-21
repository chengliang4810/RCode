use rcode_presentation::TextKey;
use serde_json::{Map, Value, json};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EditorMode {
    Form,
    Json,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Transport {
    #[default]
    Stdio,
    Http,
}

#[derive(Clone, Default)]
pub(super) struct ServerForm {
    pub name: String,
    pub transport: Transport,
    pub command: String,
    pub args: String,
    pub env: String,
    pub url: String,
    pub headers: String,
    // Preserve fields without a form control when switching from JSON and back.
    extra: Map<String, Value>,
}

#[derive(Debug)]
pub(super) struct FormError(pub TextKey, pub Option<String>);

impl FormError {
    pub fn message(&self) -> String {
        let localizer = crate::l10n::localizer();
        match &self.1 {
            Some(error) => localizer.format_with(self.0, &[("error", error.as_str())]),
            None => localizer.text(self.0),
        }
    }
}

fn string_map(text: &str, key: TextKey) -> Result<Value, FormError> {
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    let parsed: std::collections::BTreeMap<String, String> =
        serde_json::from_str(text).map_err(|_| FormError(key, None))?;
    Ok(json!(parsed))
}

impl ServerForm {
    /// Permit incomplete required fields while switching modes, but never discard malformed JSON.
    pub fn config(&self) -> Result<Value, FormError> {
        let mut entry = self.extra.clone();
        match self.transport {
            Transport::Stdio => {
                let args = parse_args(&self.args)?;
                entry.insert("type".into(), json!("stdio"));
                entry.insert("command".into(), json!(self.command.trim()));
                entry.insert("args".into(), json!(args));
                entry.insert(
                    "env".into(),
                    string_map(&self.env, TextKey::SettingsMcpEnvInvalid)?,
                );
            }
            Transport::Http => {
                entry.insert("type".into(), json!("http"));
                entry.insert("url".into(), json!(self.url.trim()));
                entry.insert(
                    "headers".into(),
                    string_map(&self.headers, TextKey::SettingsMcpHeadersInvalid)?,
                );
            }
        }
        Ok(Value::Object(entry))
    }

    pub fn json_draft(&self) -> Result<String, FormError> {
        let name = if self.name.trim().is_empty() {
            "my-mcp-server"
        } else {
            self.name.trim()
        };
        Ok(serde_json::to_string_pretty(&json!({ name: self.config()? })).unwrap())
    }

    pub fn submission(&self) -> Result<(String, Value), FormError> {
        let name = self.name.trim().to_owned();
        let config = self.config()?;
        validate(&name, &config)?;
        Ok((name, config))
    }

    pub fn from_entry(name: String, entry: Value) -> Self {
        let mut extra = entry.as_object().cloned().unwrap_or_default();
        let command = extra
            .remove("command")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        let url = extra
            .remove("url")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        let args = pretty(extra.remove("args"));
        let env = pretty(extra.remove("env"));
        let headers = pretty(extra.remove("headers"));
        extra.remove("type");
        Self {
            name,
            transport: if command.trim().is_empty() {
                Transport::Http
            } else {
                Transport::Stdio
            },
            command,
            args,
            env,
            url,
            headers,
            extra,
        }
    }
}

fn pretty(value: Option<Value>) -> String {
    value
        .map(|v| serde_json::to_string_pretty(&v).unwrap())
        .unwrap_or_default()
}

/// Support ZCode-style command-line arguments and lossless JSON arrays.
fn parse_args(text: &str) -> Result<Vec<String>, FormError> {
    let invalid = || FormError(TextKey::SettingsMcpArgsInvalid, None);
    if text.trim_start().starts_with('[') {
        return serde_json::from_str(text).map_err(|_| invalid());
    }
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut started = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\'' | '"' if quote.is_none() => {
                quote = Some(ch);
                started = true;
            }
            ch if Some(ch) == quote => quote = None,
            '\\' if quote != Some('\'')
                && chars.peek().is_some_and(|next| {
                    Some(*next) == quote
                        || (quote.is_none() && (next.is_whitespace() || matches!(next, '\'' | '"')))
                }) =>
            {
                current.push(chars.next().unwrap());
                started = true;
            }
            ch if ch.is_whitespace() && quote.is_none() => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            ch => {
                current.push(ch);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err(invalid());
    }
    if started {
        args.push(current);
    }
    Ok(args)
}

fn validate(name: &str, config: &Value) -> Result<(), FormError> {
    if name.trim().is_empty() {
        return Err(FormError(TextKey::SettingsMcpNameRequired, None));
    }
    rcode_mcp::config::validate_server(name, config)
        .map_err(|error| FormError(TextKey::SettingsMcpConfigInvalid, Some(error.to_string())))
}

/// Accept the same three single-server shapes as ZCode: wrapped, named, or a bare entry.
pub(super) fn parse_json(text: &str, fallback_name: &str) -> Result<(String, Value), FormError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| FormError(TextKey::SettingsMcpJsonInvalid, Some(error.to_string())))?;
    let object = value
        .as_object()
        .ok_or(FormError(TextKey::SettingsMcpObjectRequired, None))?;
    let (name, entry) = if let Some(servers) = object.get("mcpServers") {
        named_entry(servers)?
    } else if object.contains_key("command")
        || object.contains_key("url")
        || object.contains_key("type")
    {
        (fallback_name.trim().to_owned(), value)
    } else {
        named_entry(&value)?
    };
    validate(&name, &entry)?;
    Ok((name, entry))
}

fn named_entry(value: &Value) -> Result<(String, Value), FormError> {
    let servers = value
        .as_object()
        .ok_or(FormError(TextKey::SettingsMcpObjectRequired, None))?;
    if servers.len() != 1 {
        return Err(FormError(TextKey::SettingsMcpSingleServer, None));
    }
    let (name, entry) = servers.iter().next().unwrap();
    if !entry.is_object() {
        return Err(FormError(TextKey::SettingsMcpObjectRequired, None));
    }
    Ok((name.trim().to_owned(), entry.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_args_preserve_quotes_empty_args_and_windows_paths() {
        let parsed = parse_args(r#"-y server "C:\Program Files\data" "" --label='a b'"#).unwrap();
        assert_eq!(
            parsed,
            ["-y", "server", r"C:\Program Files\data", "", "--label=a b"]
        );
        assert!(parse_args("server \"unclosed").is_err());
        assert_eq!(
            parse_args(r#"["a b", "", "C:\\data"]"#).unwrap(),
            ["a b", "", r"C:\data"]
        );
    }

    #[test]
    fn form_and_json_preserve_argument_boundaries_and_extra_fields() {
        let entry = json!({ "command": "node", "args": ["C:\\Program Files\\server.js", "", "--label=a b"], "env": {"KEY":"value"}, "custom": {"option": true} });
        let form = ServerForm::from_entry("files".into(), entry.clone());
        let (name, roundtrip) = parse_json(&form.json_draft().unwrap(), "")
            .unwrap_or_else(|e| panic!("{}", e.message()));
        assert_eq!(name, "files");
        assert_eq!(roundtrip["args"], entry["args"]);
        assert_eq!(roundtrip["env"], entry["env"]);
        assert_eq!(roundtrip["custom"], entry["custom"]);
    }

    #[test]
    fn all_three_json_shapes_resolve_one_server() {
        for text in [
            r#"{"docs":{"url":"https://example.invalid/mcp"}}"#,
            r#"{"mcpServers":{"docs":{"url":"https://example.invalid/mcp"}}}"#,
            r#"{"url":"https://example.invalid/mcp"}"#,
        ] {
            let (name, config) =
                parse_json(text, "docs").unwrap_or_else(|e| panic!("{}", e.message()));
            assert_eq!(name, "docs");
            assert_eq!(config["url"], "https://example.invalid/mcp");
        }
    }

    #[test]
    fn invalid_drafts_are_rejected_without_losing_the_original_text() {
        for text in [
            "null",
            "[]",
            "{",
            r#"{"mcpServers":{}}"#,
            r#"{"one":{"command":"x"},"two":{"command":"y"}}"#,
            r#"{"one":{"command":"x","args":[1]}}"#,
            r#"{"one":{"type":"sse","url":"https://example.invalid"}}"#,
        ] {
            assert!(parse_json(text, "").is_err(), "{text}");
        }
        let form = ServerForm {
            name: "files".into(),
            command: "node".into(),
            env: r#"{"KEY":1}"#.into(),
            ..Default::default()
        };
        assert!(form.json_draft().is_err());
        assert_eq!(form.env, r#"{"KEY":1}"#);
    }
}
