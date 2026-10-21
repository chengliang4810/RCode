//! Translate display names while retaining the provider's protocol values.
use rcode_presentation::{Localizer, TextKey};
use rcode_proto::ReasoningLevel;

pub(super) fn reasoning(level: ReasoningLevel, localizer: &Localizer) -> String {
    localizer.text(match level {
        ReasoningLevel::Minimal => TextKey::PickersReasoningMinimal,
        ReasoningLevel::Low => TextKey::PickersReasoningLow,
        ReasoningLevel::Medium => TextKey::PickersReasoningMedium,
        ReasoningLevel::High => TextKey::PickersReasoningHigh,
        ReasoningLevel::XHigh => TextKey::PickersReasoningXhigh,
        ReasoningLevel::Max => TextKey::PickersReasoningMax,
        ReasoningLevel::Ultra => TextKey::PickersReasoningUltra,
        ReasoningLevel::Ultracode => TextKey::PickersReasoningUltracode,
        ReasoningLevel::Ultrathink => TextKey::PickersReasoningUltrathink,
    })
}
fn effort_option(id: &str) -> bool {
    ["effort", "reasoning", "reasoning_effort"].contains(&id)
}
fn context_option(id: &str) -> bool {
    ["contextwindow", "context_window", "context-window"]
        .contains(&id.to_ascii_lowercase().as_str())
}
pub(super) fn option_label(id: &str, fallback: &str, localizer: &Localizer) -> String {
    if context_option(id) {
        localizer.text(TextKey::PickersContextWindow)
    } else if effort_option(id) {
        localizer.text(TextKey::PickersReasoningEffort)
    } else {
        fallback.to_owned()
    }
}
pub(super) fn choice_label(
    option: &str,
    id: &str,
    fallback: &str,
    localizer: &Localizer,
) -> String {
    if effort_option(option) {
        let level = match id.to_ascii_lowercase().as_str() {
            "minimal" => Some(ReasoningLevel::Minimal),
            "low" => Some(ReasoningLevel::Low),
            "medium" => Some(ReasoningLevel::Medium),
            "high" => Some(ReasoningLevel::High),
            "xhigh" => Some(ReasoningLevel::XHigh),
            "max" => Some(ReasoningLevel::Max),
            "ultra" => Some(ReasoningLevel::Ultra),
            "ultracode" => Some(ReasoningLevel::Ultracode),
            "ultrathink" => Some(ReasoningLevel::Ultrathink),
            _ => None,
        };
        if let Some(level) = level {
            return reasoning(level, localizer);
        }
    }
    if context_option(option) {
        match id {
            "standard" => return localizer.text(TextKey::PickersContextStandard),
            "default" => return localizer.text(TextKey::PickersDefault),
            _ => {}
        }
    }
    fallback.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcode_presentation::LanguagePreference;
    #[test]
    fn chinese_and_english_labels_keep_context_and_reasoning_values_unchanged() {
        for (language, effort, context, high) in [
            (LanguagePreference::ZhCn, "推理强度", "上下文窗口", "高"),
            (
                LanguagePreference::EnUs,
                "Reasoning effort",
                "Context window",
                "High",
            ),
        ] {
            let localizer = Localizer::with_system_locale(language, None);
            assert_eq!(reasoning(ReasoningLevel::High, &localizer), high);
            let model = rcode_proto::Model {
                id: "m".into(),
                label: "model".into(),
                description: None,
                reasoning_levels: vec![],
                options: vec![rcode_proto::ModelOption {
                    id: "contextWindow".into(),
                    label: "Context Window".into(),
                    default_choice: "1m".into(),
                    choices: vec![rcode_proto::ModelOptionChoice {
                        id: "1m".into(),
                        label: "1M".into(),
                    }],
                }],
            };
            let groups = crate::pickers::build_setting_groups_for(
                vec![ReasoningLevel::High],
                Some(ReasoningLevel::High),
                Some(&model),
                &serde_json::Map::new(),
                &localizer,
            );
            assert_eq!(groups[0].label, effort);
            assert_eq!(groups[0].choices[0].label, high);
            assert_eq!(groups[1].label, context);
            assert_eq!(groups[1].choices[0].value, "1m");
            assert!(groups[1].choices[0].selected);
            assert_eq!(model.options[0].id, "contextWindow");
            assert_eq!(model.options[0].label, "Context Window");
            assert_eq!(choice_label("effort", "high", "High", &localizer), high);
            assert_eq!(
                option_label("vendor-option", "Vendor label", &localizer),
                "Vendor label"
            );
            for level in crate::pickers::ALL_REASONING_LEVELS {
                assert!(!reasoning(level, &localizer).contains("pickers-"));
            }
        }
    }
}
