//! Compact tab presentation for program-reported state.

use rcode_presentation::TextKey;

use super::program_status::{BlockedKind, ProgramState, ProgramStatuses};
use crate::{icons, theme::Theme};

pub(super) struct StatusPresentation {
    pub icon: &'static str,
    pub color: gpui::Hsla,
    pub tooltip: String,
}

pub(super) fn presentation(
    statuses: &ProgramStatuses,
    tab_title: &str,
    theme: &Theme,
) -> Option<StatusPresentation> {
    let (id, primary) = statuses.primary()?;
    let (icon, color) = match primary.state {
        ProgramState::Idle => (icons::TERMINAL, theme.text_muted),
        ProgramState::Working => (icons::CLOCK_CIRCLE, theme.busy),
        ProgramState::Blocked => (
            if matches!(
                primary.kind,
                Some(BlockedKind::Permission | BlockedKind::Auth)
            ) {
                icons::KEY_MINIMALISTIC
            } else {
                icons::INFO_CIRCLE
            },
            theme.warning,
        ),
        ProgramState::Done => (icons::CHECK, theme.success),
        ProgramState::Error => (icons::DANGER_TRIANGLE, theme.danger),
    };
    let localizer = crate::l10n::localizer();
    let mut tooltip = tab_title.to_string();
    // Primary first, then the remaining records. A busy parent must not hide
    // a child waiting for the user. The tab title identifies the source PTY.
    for (record_id, record) in std::iter::once((id, primary))
        .chain(statuses.records().filter(|(record_id, _)| *record_id != id))
        .take(8)
    {
        let key = match (record.state, record.kind) {
            (ProgramState::Idle, _) => TextKey::TerminalStatusIdle,
            (ProgramState::Working, _) => TextKey::TerminalStatusWorking,
            (ProgramState::Done, _) => TextKey::TerminalStatusDone,
            (ProgramState::Error, _) => TextKey::TerminalStatusError,
            (ProgramState::Blocked, Some(BlockedKind::Permission)) => {
                TextKey::TerminalStatusPermission
            }
            (ProgramState::Blocked, Some(BlockedKind::Question)) => TextKey::TerminalStatusQuestion,
            (ProgramState::Blocked, Some(BlockedKind::Auth)) => TextKey::TerminalStatusAuth,
            (ProgramState::Blocked, None) => TextKey::TerminalStatusBlocked,
        };
        tooltip.push('\n');
        if let Some(label) = record
            .title
            .as_deref()
            .filter(|label| !label.is_empty())
            .or_else(|| {
                if record_id.is_empty() {
                    statuses.app(record_id)
                } else {
                    Some(record_id)
                }
            })
        {
            tooltip.push_str(label);
            tooltip.push_str(": ");
        }
        tooltip.push_str(&localizer.text(key));
        if let Some(progress) = record.progress {
            tooltip.push_str(&format!(" {progress}%"));
        }
        if let Some(message) = record
            .message
            .as_deref()
            .filter(|message| !message.is_empty())
        {
            tooltip.push_str(" · ");
            tooltip.extend(message.chars().take(160));
            if message.chars().count() > 160 {
                tooltip.push('…');
            }
        }
    }
    let remaining = statuses.records().count().saturating_sub(8);
    if remaining > 0 {
        tooltip.push('\n');
        tooltip.push_str(&localizer.format_with(
            TextKey::TerminalStatusMore,
            &[("count", &remaining.to_string())],
        ));
    }
    Some(StatusPresentation {
        icon,
        color,
        tooltip,
    })
}
