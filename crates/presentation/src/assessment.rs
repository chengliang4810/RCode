//! Localized PR assessment labels. Protocol values remain language independent.
use crate::{Localizer, TextKey};
use rcode_proto::change_request_assessment::{
    AttentionReason, Blocker, CiState, MissingInformation, SuggestedAction,
};

pub fn ci_state(localizer: &Localizer, state: CiState) -> String {
    localizer.text(match state {
        CiState::Unknown => TextKey::PrAssessmentCiUnknown,
        CiState::NoChecks => TextKey::PrAssessmentCiNoChecks,
        CiState::Pending => TextKey::PrAssessmentCiPending,
        CiState::Passed => TextKey::PrAssessmentCiPassed,
        CiState::Skipped => TextKey::PrAssessmentCiSkipped,
        CiState::Failed => TextKey::PrAssessmentCiFailed,
    })
}

pub fn blocker(localizer: &Localizer, value: Blocker) -> String {
    localizer.text(match value {
        Blocker::FailingCi => TextKey::PrAssessmentCiFailed,
        Blocker::ChangesRequested => TextKey::PrDetailStatusChangesRequested,
        Blocker::MergeConflicts => TextKey::PrAssessmentConflicts,
    })
}

pub fn missing(localizer: &Localizer, value: MissingInformation) -> String {
    localizer.text(match value {
        MissingInformation::HeadCommit => TextKey::PrAssessmentMissingHead,
        MissingInformation::Ci => TextKey::PrAssessmentCiUnknown,
        MissingInformation::Mergeability => TextKey::PrAssessmentMissingMergeability,
        MissingInformation::ReviewDecision => TextKey::PrAssessmentMissingReview,
        MissingInformation::ViewerRelationship => TextKey::PrAssessmentMissingViewer,
        MissingInformation::CheckDetails => TextKey::PrAssessmentMissingChecks,
    })
}

pub fn action(localizer: &Localizer, value: SuggestedAction) -> String {
    localizer.text(match value {
        SuggestedAction::FixChecks => TextKey::PrAssessmentActionChecks,
        SuggestedAction::AddressFeedback => TextKey::PrAssessmentActionFeedback,
        SuggestedAction::ResolveConflicts => TextKey::PrAssessmentActionConflicts,
        SuggestedAction::Review => TextKey::PrAssessmentActionReview,
        SuggestedAction::RefreshMetadata => TextKey::PrAssessmentActionRefresh,
    })
}

pub fn attention(localizer: &Localizer, value: AttentionReason) -> String {
    localizer.text(match value {
        AttentionReason::ReviewRequested => TextKey::PrAssessmentAttentionReview,
        AttentionReason::FeedbackOnAuthored => TextKey::PrAssessmentAttentionFeedback,
        AttentionReason::FailingCi => TextKey::PrAssessmentAttentionCi,
    })
}

/// Known check statuses are display labels; unknown vendor values remain useful diagnostics.
pub fn check_status(localizer: &Localizer, raw: &str) -> String {
    let key = match raw.to_ascii_uppercase().replace(' ', "_").as_str() {
        "SUCCESS" => TextKey::PrDetailStatusPassed,
        "FAILURE" => TextKey::PrCheckFailure,
        "ERROR" => TextKey::PrCheckError,
        "TIMED_OUT" => TextKey::PrCheckTimeout,
        "ACTION_REQUIRED" => TextKey::PrCheckActionRequired,
        "IN_PROGRESS" => TextKey::PrCheckInProgress,
        "PENDING" => TextKey::PrCheckPending,
        "QUEUED" => TextKey::PrCheckQueued,
        "WAITING" => TextKey::PrCheckWaiting,
        "SKIPPED" => TextKey::PrCheckSkipped,
        "CANCELLED" => TextKey::PrCheckCancelled,
        "NEUTRAL" => TextKey::PrCheckNeutral,
        "COMPLETED" => TextKey::PrCheckCompleted,
        "STARTUP_FAILURE" => TextKey::PrCheckStartupFailure,
        "" => TextKey::PrDetailStatusNotReported,
        _ => {
            let text = raw.replace('_', " ").to_lowercase();
            let mut chars = text.chars();
            return chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default();
        }
    };
    localizer.text(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LanguagePreference;

    #[test]
    fn assessments_and_check_states_follow_the_selected_language() {
        let zh = Localizer::new(LanguagePreference::ZhCn);
        let en = Localizer::new(LanguagePreference::EnUs);
        assert_eq!(blocker(&zh, Blocker::MergeConflicts), "合并冲突");
        assert_eq!(blocker(&en, Blocker::MergeConflicts), "Merge conflicts");
        assert_eq!(action(&zh, SuggestedAction::FixChecks), "修复失败的检查");
        assert_eq!(check_status(&zh, "in progress"), "进行中");
        assert_eq!(check_status(&en, "IN_PROGRESS"), "In progress");
        assert_eq!(check_status(&zh, "VENDOR_STATE"), "Vendor state");
        assert_eq!(ci_state(&zh, CiState::Pending), "CI 等待中");
    }
}
