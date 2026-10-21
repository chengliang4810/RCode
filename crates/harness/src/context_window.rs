//! Context budgets offered below a model's configured maximum.

use rcode_proto::{ModelOption, ModelOptionChoice};
use serde_json::{Map, Value};

pub const OPTION_ID: &str = "contextWindow";

const TIERS: [(&str, &str, u32); 3] = [
    ("1m", "1M", 1_000_000),
    ("500k", "500K", 500_000),
    ("200k", "200K", 200_000),
];

pub fn model_options(maximum: Option<u32>) -> Vec<ModelOption> {
    let Some(maximum) = maximum.filter(|maximum| *maximum >= 500_000) else {
        return Vec::new();
    };
    let choices: Vec<_> = TIERS
        .iter()
        .filter(|(_, _, tokens)| *tokens <= maximum)
        .map(|(id, label, _)| ModelOptionChoice {
            id: (*id).into(),
            label: (*label).into(),
        })
        .collect();
    vec![ModelOption {
        id: OPTION_ID.into(),
        label: "Context Window".into(),
        default_choice: choices[0].id.clone(),
        choices,
    }]
}

/// Invalid or stale selections fall back to the largest offered tier.
pub fn selected_window(option: &ModelOption, selections: &Map<String, Value>) -> Option<u32> {
    let choice = selections
        .get(&option.id)
        .and_then(Value::as_str)
        .filter(|id| option.choices.iter().any(|choice| choice.id == *id))
        .unwrap_or(&option.default_choice);
    TIERS
        .iter()
        .find(|(id, _, _)| *id == choice)
        .map(|(_, _, tokens)| *tokens)
}

pub fn effective_window(maximum: Option<u32>, selections: &Map<String, Value>) -> Option<u32> {
    match model_options(maximum).first() {
        Some(option) => selected_window(option, selections),
        None => maximum,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_tiers_follow_the_configured_maximum() {
        for maximum in [
            None,
            Some(32_000),
            Some(64_000),
            Some(100_000),
            Some(200_000),
            Some(499_999),
        ] {
            assert!(model_options(maximum).is_empty(), "{maximum:?}");
            assert_eq!(effective_window(maximum, &Map::new()), maximum);
        }
        for (maximum, expected, default) in [
            (500_000, vec!["500k", "200k"], 500_000),
            (750_000, vec!["500k", "200k"], 500_000),
            (999_999, vec!["500k", "200k"], 500_000),
            (1_000_000, vec!["1m", "500k", "200k"], 1_000_000),
            (2_000_000, vec!["1m", "500k", "200k"], 1_000_000),
        ] {
            let options = model_options(Some(maximum));
            assert_eq!(
                options[0]
                    .choices
                    .iter()
                    .map(|choice| choice.id.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(effective_window(Some(maximum), &Map::new()), Some(default));
        }
    }

    #[test]
    fn context_selection_cannot_exceed_or_replace_the_available_tiers() {
        for (value, expected) in [
            (serde_json::json!("200k"), 200_000),
            (serde_json::json!("500k"), 500_000),
            (serde_json::json!("1m"), 500_000),
            (serde_json::json!("64k"), 500_000),
            (serde_json::json!(1_000_000), 500_000),
            (Value::Null, 500_000),
        ] {
            let selections = Map::from_iter([(OPTION_ID.into(), value)]);
            assert_eq!(effective_window(Some(750_000), &selections), Some(expected));
            assert_eq!(effective_window(Some(64_000), &selections), Some(64_000));
            assert_eq!(effective_window(None, &selections), None);
        }
    }
}
