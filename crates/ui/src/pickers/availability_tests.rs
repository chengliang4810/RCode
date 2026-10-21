use super::*;

impl Pickers {
    /// Deliver a native catalog through the same path as a provider-settings refresh.
    pub(crate) fn test_native_catalog(&mut self, models: Vec<Model>, cx: &mut Context<Self>) {
        self.harnesses = Loadable::Ready(vec![HarnessDescriptor {
            id: HarnessId::Native,
            name: "RCode".into(),
            installed: true,
            enabled: Some(true),
            supports_steering: true,
            steering_mode: rcode_proto::SteeringMode::StepBoundary,
            reasoning_levels: vec![],
        }]);
        self.harnesses_stale = false;
        self.land_model_reload(HarnessId::Native, Loadable::Ready(models), cx);
    }
}

fn model(id: &str) -> Model {
    Model {
        id: id.into(),
        label: format!("Vendor · {id}"),
        description: None,
        reasoning_levels: vec![ReasoningLevel::Low, ReasoningLevel::High],
        options: vec![],
    }
}

#[gpui::test]
fn native_empty_catalog_clears_fresh_and_remembered_draft_labels(cx: &mut gpui::TestAppContext) {
    let state = cx.new(|_| AppState::new());
    let picker = cx.new(|cx| Pickers::new(state, cx));
    picker.update(cx, |picker, cx| {
        assert_eq!(picker.model_name(cx), ModelName::Loading);
        for remembered in [false, true] {
            if remembered {
                picker.defaults.remember_model(
                    HarnessId::Native,
                    "deleted".into(),
                    "Vendor · deleted".into(),
                );
                picker.defaults.remember_reasoning(
                    HarnessId::Native,
                    Some("deleted"),
                    ReasoningLevel::Max,
                );
            }
            picker.test_native_catalog(vec![], cx);
            assert!(picker.needs_model_setup(cx));
            assert!(picker.model_unavailable(cx));
            assert_eq!(picker.model_name(cx), ModelName::None { no_agents: false });
            assert!(picker.selected_model_label(cx).is_none());
            assert!(picker.model_chip_suffix(cx).is_none());
            assert!(picker.compact_effort(cx).is_none());
            assert!(picker.resolved(cx).model.is_none());
            assert!(picker.resolved(cx).reasoning.is_none());
            assert!(picker.resolved(cx).model_options.is_empty());
        }
        // Adding the first model recovers immediately, without clearing history or restarting.
        picker.test_native_catalog(vec![model("added")], cx);
        assert!(!picker.model_unavailable(cx));
        assert_eq!(
            picker.selected_model_label(cx).as_deref(),
            Some("Vendor · added")
        );
        assert_eq!(picker.resolved(cx).model.as_deref(), Some("added"));
    });
}

#[gpui::test]
fn native_removed_draft_pick_falls_back_to_a_live_model_and_its_effort(
    cx: &mut gpui::TestAppContext,
) {
    let state = cx.new(|_| AppState::new());
    let picker = cx.new(|cx| Pickers::new(state, cx));
    picker.update(cx, |picker, cx| {
        picker.test_native_catalog(vec![model("deleted"), model("remaining")], cx);
        picker.defaults.remember_reasoning(
            HarnessId::Native,
            Some("remaining"),
            ReasoningLevel::Low,
        );
        picker.pick_model("deleted".into(), cx);
        picker.config.reasoning = Some(ReasoningLevel::High);
        picker.test_native_catalog(vec![model("remaining")], cx);
        assert_eq!(
            picker.selected_model_label(cx).as_deref(),
            Some("Vendor · remaining")
        );
        assert_eq!(picker.resolved(cx).model.as_deref(), Some("remaining"));
        assert_eq!(picker.resolved(cx).reasoning, Some(ReasoningLevel::Low));
        assert!(!picker.model_unavailable(cx));
        // The remembered selection must also fall back after a restart.
        picker.config = DraftConfig::default();
        assert_eq!(picker.resolved(cx).model.as_deref(), Some("remaining"));
        // Deleting the last provider also invalidates an explicit draft pick.
        picker.pick_model("remaining".into(), cx);
        picker.test_native_catalog(vec![], cx);
        assert!(picker.needs_model_setup(cx));
        assert!(picker.selected_model_label(cx).is_none());
        assert!(picker.resolved(cx).model.is_none());
    });
}

#[gpui::test]
fn native_existing_chat_falls_back_to_a_live_model_and_its_settings(cx: &mut gpui::TestAppContext) {
    let state = cx.new(|_| {
        let mut state = AppState::new();
        state.chats.push(serde_json::from_value(serde_json::json!({
            "id":"saved", "deviceId":"local", "archived":false, "createdAt":"2026-10-20T00:00:00Z",
            "config":{"harness":"native", "model":"deleted", "reasoning":"max", "modelOptions":{"tier":"fast"}}
        })).unwrap());
        state.selected_chat = Some("saved".into());
        state
    });
    let picker = cx.new(|cx| Pickers::new(state.clone(), cx));
    picker.update(cx, |picker, cx| {
        picker.test_native_catalog(vec![model("deleted")], cx);
        let mut replacement = model("remaining");
        replacement.options.push(rcode_proto::ModelOption {
            id: "tier".into(),
            label: "Tier".into(),
            choices: vec![
                rcode_proto::ModelOptionChoice {
                    id: "standard".into(),
                    label: "Standard".into(),
                },
                rcode_proto::ModelOptionChoice {
                    id: "fast".into(),
                    label: "Fast".into(),
                },
            ],
            default_choice: "standard".into(),
        });
        picker.defaults.remember_reasoning(
            HarnessId::Native,
            Some("remaining"),
            ReasoningLevel::Low,
        );
        picker.test_native_catalog(vec![replacement.clone()], cx);
        assert!(!picker.model_unavailable(cx));
        assert_eq!(
            picker.selected_model_label(cx).as_deref(),
            Some("Vendor · remaining")
        );
        assert_eq!(picker.resolved(cx).model.as_deref(), Some("remaining"));
        assert_eq!(picker.resolved(cx).reasoning, Some(ReasoningLevel::Low));
        assert!(
            picker.resolved(cx).model_options.is_empty(),
            "The old model's fast mode must not leak into its replacement"
        );
        assert!(picker.compact_effort(cx).is_some());
        assert_eq!(
            picker.model_chip_suffix(cx).unwrap().0.as_ref(),
            reasoning_label(ReasoningLevel::Low)
        );
        assert!(
            !picker
                .visible_model_rows(cx)
                .iter()
                .any(|row| row.selected_only)
        );

        // Reopening a saved chat with an already loaded catalog also resolves the replacement.
        state.update(cx, |state, _| state.selected_chat = None);
        state.update(cx, |state, _| state.selected_chat = Some("saved".into()));
        assert_eq!(picker.resolved(cx).model.as_deref(), Some("remaining"));

        // With no configured models left, existing chats share the Add model state.
        picker.test_native_catalog(vec![], cx);
        assert!(picker.needs_model_setup(cx));
        assert!(picker.model_unavailable(cx));
        assert!(picker.selected_model_label(cx).is_none());
        assert!(picker.model_chip_suffix(cx).is_none());
        assert!(picker.compact_effort(cx).is_none());
        assert!(picker.resolved(cx).model.is_none());
        assert!(picker.resolved(cx).reasoning.is_none());
        assert!(picker.resolved(cx).model_options.is_empty());

        // A different model being added recovers the chat without manual selection.
        picker.test_native_catalog(vec![replacement], cx);
        assert!(!picker.model_unavailable(cx));
        assert_eq!(picker.resolved(cx).model.as_deref(), Some("remaining"));
    });
}

#[gpui::test]
fn native_loading_failed_and_other_device_catalogs_are_not_empty_states(
    cx: &mut gpui::TestAppContext,
) {
    let state = cx.new(|_| AppState::new());
    let picker = cx.new(|cx| Pickers::new(state, cx));
    picker.update(cx, |picker, cx| {
        picker
            .defaults
            .remember_model(HarnessId::Native, "saved".into(), "Saved model".into());
        for catalog in [
            Loadable::Idle,
            Loadable::Loading,
            Loadable::Error("offline".into()),
        ] {
            picker.models.insert(HarnessId::Native, catalog);
            assert!(!picker.model_unavailable(cx));
            assert!(!picker.needs_model_setup(cx));
            assert_eq!(picker.resolved(cx).model.as_deref(), Some("saved"));
        }
        picker.test_native_catalog(vec![], cx);
        picker.stale_models.insert(HarnessId::Native);
        assert!(!picker.model_unavailable(cx));
        assert_eq!(picker.resolved(cx).model.as_deref(), Some("saved"));
        picker.stale_models.clear();
        picker.harnesses_stale = true;
        assert!(!picker.needs_model_setup(cx));
        // Title settings retain their existing automatic-model semantics.
        picker.harnesses_stale = false;
        picker.title = Some(TitleSettings {
            harness: Some(HarnessId::Native),
            model: None,
        });
        assert!(!picker.model_unavailable(cx));
        assert!(!picker.needs_model_setup(cx));
    });
}
