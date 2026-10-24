//! OSC 7501 (Program Status Protocol, revision 0.4).
//!
//! The bounded stream filter consumes this extension's payload before
//! alacritty's ANSI processor, which otherwise ignores it. Forwarding its empty
//! OSC envelope preserves ANSI cancellation semantics without buffering payloads
//! twice. Everything else is forwarded intact.
//! https://www.superlogical.com/rex/docs/build/program-status

use std::collections::BTreeMap;

use base64::Engine as _;

const MAX_SEQUENCE: usize = 4096;
const MAX_RECORDS: usize = 256;
const FEATURES: &[u8] =
    b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramState {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockedKind {
    Permission,
    Question,
    Auth,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramStatus {
    pub state: ProgramState,
    pub kind: Option<BlockedKind>,
    pub progress: Option<u8>,
    pub app: Option<String>,
    pub title: Option<String>,
    pub message: Option<String>,
}

#[derive(Default)]
pub struct ProgramStatuses {
    // Updating a record moves it to the back. With at most 256 records this also
    // gives deterministic LRU eviction without counters that eventually wrap.
    records: BTreeMap<String, ProgramStatus>,
    order: Vec<String>,
}

impl ProgramStatuses {
    pub fn records(&self) -> impl Iterator<Item = (&str, &ProgramStatus)> {
        self.order
            .iter()
            .filter_map(|id| self.records.get(id).map(|record| (id.as_str(), record)))
    }

    pub fn app<'a>(&'a self, id: &str) -> Option<&'a str> {
        let mut path = id;
        loop {
            if let Some(app) = self
                .records
                .get(path)
                .and_then(|record| record.app.as_deref())
            {
                return Some(app);
            }
            if path.is_empty() {
                return None;
            }
            path = path.rsplit_once('/').map_or("", |(parent, _)| parent);
        }
    }

    /// Show an actionable child even while its parent is working or idle.
    /// For equal priorities the most recently updated record wins.
    pub fn primary(&self) -> Option<(&str, &ProgramStatus)> {
        self.records().max_by_key(|(_, record)| match record.state {
            ProgramState::Blocked => 5,
            ProgramState::Error => 4,
            ProgramState::Done => 3,
            ProgramState::Working => 2,
            ProgramState::Idle => 1,
        })
    }

    fn retain(&mut self, keep: impl Fn(&str, &ProgramStatus) -> bool) {
        self.records.retain(|id, record| keep(id, record));
        self.order.retain(|id| self.records.contains_key(id));
    }

    /// OSC 133 A or the attached process exiting ends transient records.
    pub fn end_program(&mut self) {
        self.retain(|_, record| matches!(record.state, ProgramState::Done | ProgramState::Error));
    }

    /// A real keypress acknowledges completed results, never an ANSI query reply.
    pub fn acknowledge(&mut self) {
        self.retain(|_, record| !matches!(record.state, ProgramState::Done | ProgramState::Error));
    }

    fn clear(&mut self, id: &str) {
        self.retain(|path, _| {
            !(id.is_empty()
                || path == id
                || path
                    .strip_prefix(id)
                    .is_some_and(|suffix| suffix.starts_with('/')))
        });
    }

    fn report(&mut self, body: &[u8]) {
        let Some((id, record)) = parse_report(body) else {
            return;
        };
        let Some(record) = record else {
            self.clear(&id);
            return;
        };
        self.order.retain(|key| key != &id);
        if !self.records.contains_key(&id) && self.records.len() == MAX_RECORDS {
            let oldest = self.order.remove(0);
            self.records.remove(&oldest);
        }
        self.order.push(id.clone());
        self.records.insert(id, record);
    }
}

fn identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.+-".contains(&byte))
}

fn text(value: &str, encoded_max: usize, decoded_max: usize) -> Option<String> {
    if value.len() > encoded_max {
        return None;
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(value))
        .ok()?;
    if bytes.len() > decoded_max {
        return None;
    };
    let decoded = String::from_utf8(bytes).ok()?;
    if decoded.chars().any(char::is_control) {
        return None;
    };
    // These strings are displayed outside the terminal grid. Disarm bidi
    // overrides, isolates and invisible formatting instead of interpreting them.
    Some(
        decoded
            .chars()
            .filter(|ch| {
                !matches!(*ch,
                    '\u{00ad}' | '\u{061c}' | '\u{180e}' | '\u{200b}'..='\u{200f}'
                    | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}'
                    | '\u{fff9}'..='\u{fffb}' | '\u{e0001}' | '\u{e0020}'..='\u{e007f}'
                )
            })
            .collect(),
    )
}

// None = reject atomically; Some((id, None)) = clear the addressed subtree.
fn parse_report(body: &[u8]) -> Option<(String, Option<ProgramStatus>)> {
    let mut fields = BTreeMap::new();
    let mut title = None;
    let mut message = None;
    for pair in body.split(|byte| *byte == b':') {
        let Some(equal) = pair.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let key = pair[..equal].trim_ascii();
        let value = pair[equal + 1..].trim_ascii();
        if key.len() > 16 {
            return None;
        };
        // An explicitly invalid id must never fall back to the root, including
        // ids that are malformed at the generic pair level (e.g. whitespace).
        if key == b"id"
            && !std::str::from_utf8(value).ok().is_some_and(|id| {
                id.len() <= 128
                    && id.split('/').count() <= 8
                    && id.split('/').all(|segment| identifier(segment, 32))
            })
        {
            return None;
        }
        if matches!(key, b"msg") && value.len() > 2732
            || matches!(key, b"title") && value.len() > 256
            || matches!(key, b"app") && value.len() > 32
        {
            return None;
        }
        if key.is_empty()
            || !key.iter().all(u8::is_ascii_lowercase)
            || !value
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.,+/=-".contains(byte))
        {
            continue;
        }
        let key = std::str::from_utf8(key).ok()?;
        let value = std::str::from_utf8(value).ok()?;
        // Limits apply to every pair, including duplicate keys, before mutation.
        match key {
            "msg" => message = Some(text(value, 2732, 2048)?),
            "title" => title = Some(text(value, 256, 192)?),
            "app" if value.len() > 32 => return None,
            "id" if value.len() > 128
                || value.split('/').count() > 8
                || value.split('/').any(|segment| segment.len() > 32) =>
            {
                return None;
            }
            _ => {}
        }
        fields.insert(key, value);
    }
    let id = fields.get("id").copied().unwrap_or("");
    if fields.contains_key("id") && !id.split('/').all(|segment| identifier(segment, 32)) {
        return None;
    }
    let state = match *fields.get("state")? {
        "idle" => ProgramState::Idle,
        "working" => ProgramState::Working,
        "done" => ProgramState::Done,
        "blocked" => ProgramState::Blocked,
        "error" => ProgramState::Error,
        "clear" => return Some((id.into(), None)),
        _ => return None,
    };
    let kind = if state == ProgramState::Blocked {
        match fields.get("kind").copied() {
            Some("permission") => Some(BlockedKind::Permission),
            Some("question") => Some(BlockedKind::Question),
            Some("auth") => Some(BlockedKind::Auth),
            _ => None,
        }
    } else {
        None
    };
    let progress = if matches!(state, ProgramState::Working | ProgramState::Blocked) {
        fields
            .get("progress")
            .filter(|value| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|value| value.parse::<u8>().ok())
            .filter(|value| *value <= 100)
    } else {
        None
    };
    let app = fields
        .get("app")
        .filter(|value| identifier(value, 32))
        .map(|value| (*value).into());
    Some((
        id.into(),
        Some(ProgramStatus {
            state,
            kind,
            progress,
            app,
            title,
            message,
        }),
    ))
}

#[derive(Default)]
enum Scan {
    #[default]
    Ground,
    Escape,
    // ESC and an intervening C0 have already been forwarded.
    EscapePassthrough,
    EscapeIntermediate,
    Csi,
    String,
    Osc,
    ReportEscape,
}

#[derive(Default, PartialEq)]
enum OscMode {
    #[default]
    Probe,
    Report,
    Passthrough,
}

pub enum StatusOutput<'a> {
    Bytes(&'a [u8]),
    Reply(&'static [u8]),
}

#[derive(Default)]
pub struct StatusDecoder {
    scan: Scan,
    mode: OscMode,
    // For other OSCs retain only their prefix (to recognize shell prompts).
    // For OSC 7501 retain at most MAX_SEQUENCE bytes, even without a terminator.
    body: Vec<u8>,
    length: usize,
    statuses: ProgramStatuses,
}

#[cfg(test)]
#[path = "program_status_tests.rs"]
mod tests;

impl StatusDecoder {
    pub fn statuses(&self) -> &ProgramStatuses {
        &self.statuses
    }
    pub fn statuses_mut(&mut self) -> &mut ProgramStatuses {
        &mut self.statuses
    }

    pub fn process_exited(&mut self) {
        self.statuses.end_program();
        self.scan = Scan::Ground;
        self.body.clear();
    }

    fn finish(&mut self, terminator_len: usize, output: &mut impl FnMut(StatusOutput<'_>)) {
        if self.mode == OscMode::Report {
            if self.length + terminator_len <= MAX_SEQUENCE {
                if self.body.trim_ascii() == b"?" {
                    output(StatusOutput::Reply(FEATURES));
                } else {
                    self.statuses.report(&self.body);
                }
            }
        } else if self.body == b"133;A" || self.body.starts_with(b"133;A;") {
            self.statuses.end_program();
        }
        self.body.clear();
    }

    pub fn advance(&mut self, bytes: &[u8], mut output: impl FnMut(StatusOutput<'_>)) {
        let mut plain = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            match self.scan {
                Scan::Osc => {
                    if matches!(byte, 0x18 | 0x1a) {
                        if self.mode == OscMode::Probe {
                            plain.extend_from_slice(b"\x1b]");
                            plain.extend_from_slice(&self.body);
                        }
                        plain.push(byte);
                        self.body.clear();
                        self.scan = Scan::Ground;
                    } else if byte == 0x07 || byte == 0x1b {
                        if self.mode == OscMode::Report && byte == 0x1b {
                            self.scan = Scan::ReportEscape;
                            continue;
                        }
                        if self.mode == OscMode::Probe {
                            plain.extend_from_slice(b"\x1b]");
                            plain.extend_from_slice(&self.body);
                        }
                        if byte == 0x07 {
                            plain.push(byte);
                        }
                        // Drain ordinary output before a reply to keep query
                        // responses ordered with DA/DSR in the same PTY frame.
                        output(StatusOutput::Bytes(&plain));
                        plain.clear();
                        self.finish(1, &mut output);
                        if byte == 0x1b {
                            self.scan = Scan::Escape;
                        } else {
                            self.scan = Scan::Ground;
                        }
                    } else {
                        self.length = self.length.saturating_add(1).min(MAX_SEQUENCE + 1);
                        match self.mode {
                            OscMode::Probe => {
                                self.body.push(byte);
                                if self.body == b"7501;" {
                                    self.mode = OscMode::Report;
                                    self.body.clear();
                                    plain.extend_from_slice(b"\x1b]7501;");
                                } else if !b"7501;".starts_with(&self.body) {
                                    self.mode = OscMode::Passthrough;
                                    plain.extend_from_slice(b"\x1b]");
                                    plain.extend_from_slice(&self.body);
                                }
                            }
                            OscMode::Report => {
                                if self.length <= MAX_SEQUENCE {
                                    self.body.push(byte)
                                }
                            }
                            OscMode::Passthrough => {
                                if self.body.len() < 6 {
                                    self.body.push(byte)
                                }
                                plain.push(byte);
                            }
                        }
                    }
                }
                Scan::ReportEscape if byte == b'\\' => {
                    plain.extend_from_slice(b"\x1b\\");
                    output(StatusOutput::Bytes(&plain));
                    plain.clear();
                    self.finish(2, &mut output);
                    self.scan = Scan::Ground;
                }
                Scan::Escape | Scan::ReportEscape => {
                    // An unexpected ESC cancels a report; reinterpret the new
                    // escape normally, without publishing a partial record.
                    self.body.clear();
                    if byte == 0x1b {
                        plain.push(0x1b);
                        self.scan = Scan::Escape;
                    } else if byte < 0x20 && !matches!(byte, 0x18 | 0x1a) {
                        plain.extend_from_slice(&[0x1b, byte]);
                        self.scan = Scan::EscapePassthrough;
                    } else if byte == b']' {
                        self.mode = OscMode::Probe;
                        self.length = 2; // ESC ]
                        self.scan = Scan::Osc;
                    } else {
                        plain.extend_from_slice(&[0x1b, byte]);
                        self.scan = match byte {
                            b'[' => Scan::Csi,
                            b'P' | b'X' | b'^' | b'_' => Scan::String,
                            0x20..=0x2f => Scan::EscapeIntermediate,
                            _ => Scan::Ground,
                        };
                        if byte == b'c' {
                            self.statuses.clear("")
                        }
                    }
                }
                Scan::EscapePassthrough if byte != 0x1b => {
                    plain.push(byte);
                    self.scan = match byte {
                        b']' => {
                            self.mode = OscMode::Passthrough;
                            self.body.clear();
                            self.length = 2;
                            Scan::Osc
                        }
                        b'[' => Scan::Csi,
                        b'P' | b'X' | b'^' | b'_' => Scan::String,
                        0x20..=0x2f => Scan::EscapeIntermediate,
                        0x00..=0x17 | 0x19 | 0x1c..=0x1f => Scan::EscapePassthrough,
                        _ => Scan::Ground,
                    };
                    if byte == b'c' {
                        self.statuses.clear("");
                    }
                }
                _ if byte == 0x1b => self.scan = Scan::Escape,
                _ => {
                    plain.push(byte);
                    self.scan = match self.scan {
                        _ if matches!(byte, 0x18 | 0x1a) => Scan::Ground,
                        Scan::Csi if (0x40..=0x7e).contains(&byte) => Scan::Ground,
                        Scan::EscapeIntermediate if (0x30..=0x7e).contains(&byte) => Scan::Ground,
                        _ => std::mem::take(&mut self.scan),
                    };
                }
            }
        }
        output(StatusOutput::Bytes(&plain));
    }
}
