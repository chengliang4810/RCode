//! Stable terminal-open failures carried by the existing string RPC error slot.
//! The prefix distinguishes a structured application failure from native stderr.
use serde::{Deserialize, Serialize};

const PREFIX: &str = "rcode.terminal.open:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum TerminalOpenFailure {
    Limit { maximum: usize },
    WorkingDirectoryUnavailable,
    Startup { reason: String },
    Launch { shell: String, reason: String },
}

impl TerminalOpenFailure {
    pub fn encode(&self) -> String {
        format!(
            "{PREFIX}{}",
            serde_json::to_string(self).expect("terminal failure serializes")
        )
    }

    pub fn decode(message: &str) -> Option<Self> {
        serde_json::from_str(message.strip_prefix(PREFIX)?).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_failure_codes_round_trip_with_native_diagnostics() {
        for failure in [
            TerminalOpenFailure::Limit { maximum: 32 },
            TerminalOpenFailure::WorkingDirectoryUnavailable,
            TerminalOpenFailure::Startup {
                reason: "权限不足\nEACCES".into(),
            },
            TerminalOpenFailure::Launch {
                shell: "pwsh".into(),
                reason: "ENOENT".into(),
            },
        ] {
            assert_eq!(
                TerminalOpenFailure::decode(&failure.encode()),
                Some(failure)
            );
        }
        assert!(TerminalOpenFailure::decode("connection closed").is_none());
        assert!(TerminalOpenFailure::decode("rcode.terminal.open:{\"code\":\"future\"}").is_none());
    }
}
