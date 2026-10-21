use super::widgets;
use rcode_harness::{provider_store::ProviderSettingsView, providers::ModelSelection};
use rcode_presentation::TextKey;

pub(super) fn model_items(
    view: Option<&ProviderSettingsView>,
    current: &str,
) -> Vec<widgets::CascadeItem> {
    let localizer = crate::l10n::localizer();
    let selected = ModelSelection::parse(current);
    let mut items = vec![widgets::CascadeItem::choice(
        "inherit",
        localizer.text(TextKey::SettingsSubagentsInherit),
    )];
    for provider in view.into_iter().flat_map(|view| &view.providers) {
        let children = provider
            .models
            .iter()
            .map(|model| {
                widgets::CascadeItem::choice(
                    ModelSelection {
                        provider_id: provider.id.clone(),
                        model_id: model.id.clone(),
                        reasoning: selected
                            .as_ref()
                            .filter(|selection| {
                                selection.provider_id == provider.id
                                    && selection.model_id == model.id
                            })
                            .map(|selection| selection.reasoning)
                            .unwrap_or_else(|| model.reasoning_levels.last().copied()),
                    }
                    .selection(),
                    model.label.as_deref().unwrap_or(&model.model).to_owned(),
                )
            })
            .collect::<Vec<_>>();
        if !children.is_empty() {
            items.push(widgets::CascadeItem::group(
                provider.label.as_deref().unwrap_or(&provider.id).to_owned(),
                children,
            ));
        }
    }
    if let Some(target) = ModelSelection::parse(current)
        && !items
            .iter()
            .flat_map(|item| &item.children)
            .any(|item| item.value.as_deref() == Some(current))
    {
        items.push(widgets::CascadeItem::choice(
            current,
            format!(
                "{} / {} · {}",
                target.provider_id,
                target.model_id,
                localizer.text(TextKey::SettingsSubagentsModelDeleted)
            ),
        ));
    }
    items
}

pub(super) fn reasoning_levels(
    view: Option<&ProviderSettingsView>,
    current: &str,
) -> Vec<rcode_proto::ReasoningLevel> {
    let Some(selection) = ModelSelection::parse(current) else {
        return vec![];
    };
    view.into_iter()
        .flat_map(|view| &view.providers)
        .find(|provider| provider.id == selection.provider_id)
        .and_then(|provider| {
            provider
                .models
                .iter()
                .find(|model| model.id == selection.model_id)
        })
        .map(|model| model.reasoning_levels.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_models_choose_the_highest_effort_and_existing_choices_stay_intact() {
        use rcode_proto::ReasoningLevel::{High, Low};
        let view: ProviderSettingsView = serde_json::from_value(serde_json::json!({"revision":"test","providers":[
            {"id":"p","protocol":"openai-chat","credential":{"kind":"none"},"models":[{"id":"m","model":"Model","reasoningLevels":["low","high"]}]}
        ]})).unwrap();
        let offered = model_items(Some(&view), "inherit")[1].children[0]
            .value
            .clone()
            .unwrap();
        let mut choice = ModelSelection::parse(&offered).unwrap();
        assert_eq!(choice.reasoning, Some(High));
        choice.reasoning = Some(Low);
        let existing = choice.selection();
        assert_eq!(
            model_items(Some(&view), &existing)[1].children[0]
                .value
                .as_deref(),
            Some(existing.as_str())
        );
        choice.reasoning = None;
        let legacy = choice.selection();
        assert_eq!(
            model_items(Some(&view), &legacy)[1].children[0]
                .value
                .as_deref(),
            Some(legacy.as_str())
        );
        assert_eq!(reasoning_levels(Some(&view), &legacy), [Low, High]);
        assert!(reasoning_levels(Some(&view), "inherit").is_empty());
        assert!(reasoning_levels(None, &existing).is_empty());
    }
    #[test]
    fn only_inheritance_and_provider_models_are_offered_in_configured_order() {
        let view: ProviderSettingsView = serde_json::from_value(serde_json::json!({"revision":"test","providers":[
            {"id":"first","protocol":"openai-chat","credential":{"kind":"none"},"models":[{"id":"b","model":"B"},{"id":"a","model":"A"}]},
            {"id":"second","protocol":"openai-chat","credential":{"kind":"none"},"models":[{"id":"c","model":"A"}]}
        ]})).unwrap();
        let items = model_items(Some(&view), "inherit");
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].value.as_deref(), Some("inherit"));
        assert_eq!(
            items[1]
                .children
                .iter()
                .map(|item| ModelSelection::parse(item.value.as_deref().unwrap())
                    .unwrap()
                    .model_id)
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert_eq!(
            ModelSelection::parse(items[2].children[0].value.as_deref().unwrap())
                .unwrap()
                .provider_id,
            "second"
        );
        let deleted = ModelSelection {
            provider_id: "deleted".into(),
            model_id: "removed".into(),
            reasoning: None,
        }
        .selection();
        assert_eq!(
            model_items(Some(&view), &deleted)
                .last()
                .unwrap()
                .value
                .as_deref(),
            Some(deleted.as_str())
        );
        assert_eq!(model_items(None, "haiku").len(), 1);
    }
}
