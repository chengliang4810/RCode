use super::*;

fn configured() -> crate::providers::ProvidersConfig {
    serde_json::from_value(serde_json::json!({"providers":[{
        "id":"provider","protocol":"open_ai_chat","apiKey":"test-key","models":[
            {"id":"first","model":"vendor-first","contextWindow":32000},
            {"id":"second","model":"vendor-second","contextWindow":64000}
        ]
    }]}))
    .unwrap()
}

#[tokio::test]
async fn only_real_models_are_listed_and_retired_conversation_selections_use_the_default() {
    let live = configured();
    let harness = NativeHarness::new(live.clone());
    let models = harness.models().await.unwrap();
    assert_eq!(
        models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    for legacy in ["route:haiku", "route:sonnet", "route:opus", "route:fable"] {
        assert_eq!(
            harness.configured_model(legacy, &live).unwrap().1.id,
            "first"
        );
        assert_eq!(harness.context_window(legacy), Some(32000));
    }
    assert_eq!(
        harness.configured_model("second", &live).unwrap().1.id,
        "second"
    );
    assert!(
        harness
            .configured_model("missing-real-model", &live)
            .is_none()
    );
}

#[test]
fn a_real_model_with_a_retired_identifier_keeps_its_exact_identity() {
    let mut live = configured();
    live.providers[0].models[1].id = "route:haiku".into();
    let harness = NativeHarness::new(live.clone());
    assert_eq!(
        harness
            .configured_model("route:haiku", &live)
            .unwrap()
            .1
            .model,
        "vendor-second"
    );
}
