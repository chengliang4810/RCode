//! Display language shared with the native application at process startup.

use std::sync::OnceLock;

use rcode_presentation::{LanguagePreference, Localizer, TextKey};

pub(crate) fn localizer() -> &'static Localizer {
    static LOCALIZER: OnceLock<Localizer> = OnceLock::new();
    LOCALIZER.get_or_init(|| {
        // iOS supplies Bundle's preferred localization before starting the core.
        // This also respects its per-app language preference without an ABI change.
        let language = std::env::var("RCODE_DISPLAY_LANGUAGE").ok();
        match language {
            Some(language) => {
                Localizer::with_system_locale(LanguagePreference::System, Some(&language))
            }
            None => Localizer::new(LanguagePreference::System),
        }
    })
}

pub(crate) fn reasoning_label(localizer: &Localizer, level: &str) -> String {
    let key = match level {
        "minimal" => TextKey::PickersReasoningMinimal,
        "low" => TextKey::PickersReasoningLow,
        "medium" => TextKey::PickersReasoningMedium,
        "high" => TextKey::PickersReasoningHigh,
        "xhigh" => TextKey::PickersReasoningXhigh,
        "max" => TextKey::PickersReasoningMax,
        "ultra" => TextKey::PickersReasoningUltra,
        "ultracode" => TextKey::PickersReasoningUltracode,
        "ultrathink" => TextKey::PickersReasoningUltrathink,
        _ => return rcode_client::catalog::reasoning_label(level),
    };
    localizer.text(key)
}

pub(crate) fn relative_time(localizer: &Localizer, at_ms: i64, now_ms: i64) -> String {
    let seconds = now_ms.saturating_sub(at_ms).max(0) / 1_000;
    let (key, count) = if seconds < 60 {
        return localizer.text(TextKey::CompactTimeJustNow);
    } else if seconds < 3_600 {
        (TextKey::CompactTimeMinutesAgo, seconds / 60)
    } else if seconds < 86_400 {
        (TextKey::CompactTimeHoursAgo, seconds / 3_600)
    } else {
        // Preserve mobile row thresholds (days remain days after one week).
        (TextKey::CompactTimeDaysAgo, seconds / 86_400)
    };
    localizer.format_count(key, "count", count)
}

pub(crate) fn relative_time_now(at_ms: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    relative_time(localizer(), at_ms, now)
}

/// Decode application failures at the native display boundary. Relay errors
/// carry an RPC method prefix; unrecognized and legacy diagnostics stay intact.
pub(crate) fn host_failure(localizer: &Localizer, message: &str) -> String {
    use rcode_proto::terminal_errors::TerminalOpenFailure;
    use rcode_proto::workspace_errors::WorkspaceFailure;

    let mut candidate = message;
    for _ in 0..3 {
        if let Some(failure) = WorkspaceFailure::decode(candidate) {
            return rcode_presentation::workspace::workspace_failure(localizer, &failure);
        }
        if TerminalOpenFailure::decode(candidate).is_some() {
            return rcode_presentation::workspace::terminal_open_failure(localizer, candidate);
        }
        let Some((scope, detail)) = candidate.split_once(": ") else {
            break;
        };
        let method = !scope.is_empty()
            && scope
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        if method || matches!(scope, "host error" | "invalid argument" | "not found") {
            candidate = detail;
        } else {
            break;
        }
    }
    message.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_language_tags_resolve_and_mobile_labels_follow_them() {
        let zh = Localizer::with_system_locale(LanguagePreference::System, Some("zh-Hans"));
        let en = Localizer::with_system_locale(LanguagePreference::System, Some("en"));
        assert_eq!(reasoning_label(&zh, "high"), "高");
        assert_eq!(reasoning_label(&en, "high"), "High");
        assert_eq!(reasoning_label(&zh, "vendor-effort"), "Vendor-effort");
        assert_eq!(relative_time(&zh, 0, 2 * 86_400_000), "2天");
        assert_eq!(relative_time(&en, 0, 2 * 86_400_000), "2d");
        assert_eq!(relative_time(&zh, 100, 0), "刚刚");
    }

    #[test]
    fn known_host_error_codes_localize_without_exposing_wire_payloads() {
        use rcode_proto::terminal_errors::TerminalOpenFailure;
        use rcode_proto::workspace_errors::WorkspaceFailure;

        let zh = Localizer::new(LanguagePreference::ZhCn);
        let en = Localizer::new(LanguagePreference::EnUs);
        for (method, failure) in [
            ("ReadWorkspaceFile", WorkspaceFailure::FileMissing.encode()),
            (
                "SearchWorkspaceFiles",
                WorkspaceFailure::QueryTooLong { maximum: 42 }.encode(),
            ),
            (
                "OpenTerminal",
                TerminalOpenFailure::Limit { maximum: 32 }.encode(),
            ),
            (
                "OpenTerminal",
                TerminalOpenFailure::Launch {
                    shell: "pwsh".into(),
                    reason: "ENOENT".into(),
                }
                .encode(),
            ),
        ] {
            let english = host_failure(&en, &failure);
            let chinese = host_failure(&zh, &failure);
            assert_ne!(english, chinese);
            assert!(!english.contains("rcode.") && !chinese.contains("rcode."));
            for envelope in [
                format!("{method}: {failure}"),
                format!("host error: {method}: {failure}"),
            ] {
                assert_eq!(host_failure(&en, &envelope), english);
                assert_eq!(host_failure(&zh, &envelope), chinese);
            }
        }
        let launch = TerminalOpenFailure::Launch {
            shell: "pwsh".into(),
            reason: "ENOENT".into(),
        }
        .encode();
        let message = host_failure(&zh, &launch);
        assert!(message.contains("pwsh") && message.contains("ENOENT"));
        for legacy in [
            "old host error",
            "ReadWorkspaceFile: old host error",
            "rcode.workspace:{\"code\":\"future\"}",
            "rcode.terminal.open:{\"code\":\"future\"}",
            "ReadWorkspaceFile: rcode.workspace:{\"code\":\"future\"}",
        ] {
            assert_eq!(host_failure(&zh, legacy), legacy);
        }
    }
}
