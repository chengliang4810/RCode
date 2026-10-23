// Modified for RCode.
use crate::{ModelError, ModelResult};

/// 已完成的 SSE event。`data` 保持 provider 原文，JSON 解码由 adapter 负责。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseEvent {
    pub(crate) event: Option<String>,
    pub(crate) data: String,
}

/// 字节级 SSE parser，安全保留跨 chunk 的 UTF-8 与未完成行。
pub(crate) struct SseParser {
    pending_bytes: Vec<u8>,
    event: Option<String>,
    data_lines: Vec<String>,
    saw_data_field: bool,
    done: bool,
}

impl SseParser {
    pub(crate) fn new() -> Self {
        Self {
            pending_bytes: Vec::new(),
            event: None,
            data_lines: Vec::new(),
            saw_data_field: false,
            done: false,
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<ModelResult<SseEvent>> {
        if self.done {
            return Vec::new();
        }
        self.pending_bytes.extend_from_slice(bytes);
        let Some(complete_end) = self
            .pending_bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map(|index| index + 1)
        else {
            return Vec::new();
        };

        let remaining = self.pending_bytes.split_off(complete_end);
        let complete = std::mem::replace(&mut self.pending_bytes, remaining);
        let mut events = Vec::new();
        // 按行解码，后续坏字节不能撤销已经收齐的事件；HTTP 分块不改变事件顺序。
        for raw_line in complete.split_inclusive(|byte| *byte == b'\n') {
            let raw_line = match std::str::from_utf8(raw_line) {
                Ok(line) => line,
                Err(error) => {
                    events.push(Err(ModelError::protocol(
                        crate::ProtocolErrorKind::Provider,
                    )
                    .with_error(&error)
                    .with_body(String::from_utf8_lossy(raw_line))));
                    break;
                }
            };
            let line = raw_line
                .strip_suffix('\n')
                .expect("split_inclusive always includes newline")
                .strip_suffix('\r')
                .unwrap_or(raw_line.strip_suffix('\n').expect("newline removed"));
            if line.is_empty() {
                if self.saw_data_field {
                    events.push(Ok(SseEvent {
                        event: self.event.take(),
                        data: std::mem::take(&mut self.data_lines).join("\n"),
                    }));
                    self.saw_data_field = false;
                } else {
                    self.event = None;
                }
                continue;
            }
            if let Some(value) = line.strip_prefix("data:") {
                let value = value.strip_prefix(' ').unwrap_or(value);
                if value == "[DONE]" {
                    self.done = true;
                    break;
                }
                self.saw_data_field = true;
                self.data_lines.push(value.to_owned());
            } else if let Some(value) = line.strip_prefix("event:") {
                self.event = Some(value.strip_prefix(' ').unwrap_or(value).to_owned());
            }
        }
        events
    }

    pub(crate) fn is_done(&self) -> bool {
        self.done
    }

    /// Some providers omit the final line/event separator. Decode any trailing
    /// data at EOF, but leave completion to the provider's explicit terminal
    /// event or [DONE]; an ordinary delta does not make a truncated stream valid.
    pub(crate) fn finish(&mut self) -> Vec<ModelResult<SseEvent>> {
        self.push(b"\n\n")
    }
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eof_recognizes_done_without_a_final_newline() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"data: [DONE]").is_empty());
        assert!(parser.finish().is_empty());
        assert!(parser.is_done());
    }

    #[test]
    fn eof_flushes_data_without_inventing_completion() {
        let mut parser = SseParser::new();
        let bytes = "event: delta\ndata: {\"text\":\"你好\"}".as_bytes();
        for chunk in bytes.chunks(1) {
            assert!(parser.push(chunk).is_empty());
        }
        assert_eq!(
            parser.finish().pop().unwrap().unwrap(),
            SseEvent {
                event: Some("delta".into()),
                data: "{\"text\":\"你好\"}".into(),
            }
        );
        assert!(!parser.is_done());
        assert!(parser.finish().is_empty());
    }

    #[test]
    fn eof_rejects_incomplete_utf8() {
        let mut parser = SseParser::new();
        parser.push(b"data: \xe4\xbd");
        assert!(parser.finish().pop().unwrap().is_err());
        assert!(!parser.is_done());
    }
}
