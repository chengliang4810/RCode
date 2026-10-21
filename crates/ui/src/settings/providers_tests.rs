use super::*;
use crate::state::EngineHandle;
use gpui::{TestAppContext, WindowHandle};
use rcode_harness::provider_store::ProviderStore;
use rcode_rpc::{ClientFrame, RpcClient};
use tokio::sync::mpsc;

fn test_runtime() -> tokio::runtime::Runtime {
    // GPUI's deterministic scheduler requires wakeups on the test thread.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn setup_model_order(
    runtime: &tokio::runtime::Runtime,
    cx: &mut TestAppContext,
) -> (Server, WindowHandle<ProvidersPage>) {
    let (mut server, window) = setup(runtime, cx, &["first", "second"]);
    let mut first = entry("first");
    for name in ["a", "b"] {
        let mut model = first.models[0].clone();
        model.id = format!("model-{name}");
        model.model = format!("vendor-{name}");
        first.models.push(model);
    }
    server
        .store
        .save(
            ProviderDraft {
                default_model: None,
                providers: vec![first, entry("second")],
                web_search: None,
            },
            &server.store.view().revision,
        )
        .unwrap();
    window.update(cx, |page, _, cx| page.load(cx)).unwrap();
    let load = server.request(cx, methods::GET_PROVIDER_SETTINGS);
    server.reply(
        runtime,
        &load,
        serde_json::to_value(server.store.view()).unwrap(),
    );
    cx.run_until_parked();
    cx.update(|cx| crate::motion::set_reduced_motion(cx, true));
    draw(cx, window);
    (server, window)
}

fn drag_model(
    visual: &mut gpui::VisualTestContext,
    from: gpui::Point<gpui::Pixels>,
    to: gpui::Point<gpui::Pixels>,
) {
    visual.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
    visual.simulate_mouse_move(to, Some(gpui::MouseButton::Left), Default::default());
    visual.simulate_mouse_move(to, Some(gpui::MouseButton::Left), Default::default());
    visual.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
        window.draw(cx).clear();
        assert!(cx.has_active_drag());
    });
}

#[gpui::test]
fn model_drag_previews_and_autosaves_in_both_directions_without_changing_other_providers(
    cx: &mut TestAppContext,
) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup_model_order(&runtime, cx);
    let original = server.store.view().providers[0].models.clone();
    for (from_id, to_id, expected) in [
        (
            "providers-model-2-row-drag",
            "providers-model-0-row-drag",
            ["model-b", "first-model", "model-a"],
        ),
        (
            "providers-model-0-row-drag",
            "providers-model-2-row-drag",
            ["first-model", "model-a", "model-b"],
        ),
    ] {
        let before = server.store.view();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(gpui::size(gpui::px(1200.0), gpui::px(900.0)));
        draw(&mut visual, window);
        let from = visual.debug_bounds(from_id).unwrap().center();
        let to = visual.debug_bounds(to_id).unwrap().center();
        drag_model(&mut visual, from, to);
        window
            .read_with(&visual, |page, _| {
                assert!(page.model_drag.is_some());
                assert_eq!(page.edit_generation, page.saved_generation);
                assert_eq!(
                    page.draft.as_ref().unwrap().providers[0]
                        .models
                        .iter()
                        .map(|model| &model.id)
                        .collect::<Vec<_>>(),
                    before.providers[0]
                        .models
                        .iter()
                        .map(|model| &model.id)
                        .collect::<Vec<_>>()
                );
            })
            .unwrap();
        visual.simulate_mouse_up(to, gpui::MouseButton::Left, Default::default());
        drop(visual);
        server.save(&runtime, cx);
        let view = server.store.view();
        assert_eq!(
            view.providers[0]
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(view.providers[1].models, before.providers[1].models);
        for model in &view.providers[0].models {
            assert_eq!(
                model,
                original.iter().find(|old| old.id == model.id).unwrap()
            );
        }
        window
            .read_with(cx, |page, _| {
                assert!(page.model_drag.is_none());
                assert!(!page.has_pending_save());
            })
            .unwrap();
        server.assert_no_save(cx);
    }
}

#[gpui::test]
fn scrolled_provider_model_list_reorders_using_the_model_list_coordinates(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup_model_order(&runtime, cx);
    let mut provider = entry("first");
    let template = provider.models[0].clone();
    for index in 0..9 {
        let mut model = template.clone();
        model.id = format!("long-{index}");
        model.model = format!("vendor-long-{index}");
        provider.models.push(model);
    }
    server
        .store
        .save(
            ProviderDraft {
                default_model: None,
                providers: vec![provider, entry("second")],
                web_search: None,
            },
            &server.store.view().revision,
        )
        .unwrap();
    window.update(cx, |page, _, cx| page.load(cx)).unwrap();
    let load = server.request(cx, methods::GET_PROVIDER_SETTINGS);
    server.reply(
        &runtime,
        &load,
        serde_json::to_value(server.store.view()).unwrap(),
    );
    cx.run_until_parked();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(1100.0), px(600.0)));
    draw(&mut visual, window);
    let first = visual
        .debug_bounds("providers-model-0-row-drag")
        .unwrap()
        .center();
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: first,
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-400.0))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    draw(&mut visual, window);
    let from = visual
        .debug_bounds("providers-model-5-row-drag")
        .unwrap()
        .center();
    let to = visual
        .debug_bounds("providers-model-7-row-drag")
        .unwrap()
        .center();
    assert!(from.y > px(0.0) && to.y < px(600.0));
    drag_model(&mut visual, from, to);
    visual.simulate_mouse_up(to, gpui::MouseButton::Left, Default::default());
    drop(visual);
    server.save(&runtime, cx);
    assert_eq!(server.store.view().providers[0].models[7].id, "long-4");
}

#[gpui::test]
fn model_drag_cancel_does_not_save_and_switching_providers_cancels_the_preview(
    cx: &mut TestAppContext,
) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup_model_order(&runtime, cx);
    let original = server.store.view();
    for cancel in ["escape", "outside", "provider"] {
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        draw(&mut visual, window);
        let from = visual
            .debug_bounds("providers-model-0-row-drag")
            .unwrap()
            .center();
        let to = visual
            .debug_bounds("providers-model-2-row-drag")
            .unwrap()
            .center();
        drag_model(&mut visual, from, to);
        match cancel {
            "escape" => visual.simulate_keystrokes("escape"),
            "outside" => visual.simulate_mouse_up(
                gpui::point(px(4.0), px(4.0)),
                gpui::MouseButton::Left,
                Default::default(),
            ),
            _ => {
                window
                    .update(&mut visual, |page, window, cx| page.select(1, window, cx))
                    .unwrap();
            }
        }
        visual.simulate_mouse_up(to, gpui::MouseButton::Left, Default::default());
        drop(visual);
        window
            .read_with(cx, |page, _| {
                assert!(page.model_drag.is_none());
                assert!(!page.has_pending_save());
            })
            .unwrap();
        server.assert_no_save(cx);
        assert_eq!(
            server.store.view().providers[0].models,
            original.providers[0].models
        );
    }
}

struct Server {
    _dir: tempfile::TempDir,
    store: ProviderStore,
    requests: mpsc::Receiver<String>,
    replies: mpsc::Sender<String>,
    engine: EngineHandle,
}

fn entry(id: &str) -> ProviderDraftEntry {
    ProviderDraftEntry {
        id: id.into(),
        label: Some(id.into()),
        protocol: Protocol::OpenAiChat,
        base_url: None,
        credential: CredentialUpdate::Keep,
        models: vec![ModelConfig {
            id: format!("{id}-model"),
            model: "vendor-model".into(),
            label: Some("Curated model label".into()),
            context_window: None,
            max_tokens: None,
            input_format: InputFormat::TextAndImages,
            reasoning_levels: vec![],
        }],
    }
}

impl Server {
    fn new(ids: &[&str]) -> (Self, EngineHandle) {
        let dir = tempfile::tempdir().unwrap();
        let store = ProviderStore::open(dir.path()).unwrap();
        store
            .save(
                ProviderDraft {
                    default_model: None,
                    providers: ids.iter().map(|id| entry(id)).collect(),
                    web_search: None,
                },
                &store.view().revision,
            )
            .unwrap();
        let (out, requests) = mpsc::channel(64);
        let (replies, inbound) = mpsc::channel(64);
        let engine = EngineHandle::from_test_client(RpcClient::new(out, inbound));
        (
            Self {
                _dir: dir,
                store,
                requests,
                replies,
                engine: engine.clone(),
            },
            engine,
        )
    }

    fn request(&mut self, cx: &mut TestAppContext, method: &str) -> ClientFrame {
        cx.run_until_parked();
        loop {
            let frame: ClientFrame =
                serde_json::from_str(&self.requests.try_recv().expect("RPC request")).unwrap();
            if frame.cancel {
                continue;
            }
            assert_eq!(frame.method.as_deref(), Some(method));
            return frame;
        }
    }

    fn reply(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        request: &ClientFrame,
        value: serde_json::Value,
    ) {
        self.deliver(runtime, serde_json::json!({"id":request.id,"ok":value}));
    }

    fn reject(&mut self, runtime: &tokio::runtime::Runtime, request: &ClientFrame, reason: &str) {
        self.deliver(runtime, serde_json::json!({"id":request.id,"err":reason}));
    }

    fn deliver(&mut self, runtime: &tokio::runtime::Runtime, frame: serde_json::Value) {
        const BARRIER: &str = "ProviderTestBarrier";
        let engine = self.engine.clone();
        runtime.block_on(async {
            self.replies.send(frame.to_string()).await.unwrap();
            // A second round trip ensures the Tokio reader has processed the
            // reply before the deterministic GPUI executor is driven again.
            let barrier = engine.client().call(BARRIER, serde_json::Value::Null);
            tokio::pin!(barrier);
            loop {
                let request = tokio::select! {
                    result = &mut barrier => panic!("barrier completed before its reply: {result:?}"),
                    request = self.requests.recv() => request.unwrap(),
                };
                let request: ClientFrame = serde_json::from_str(&request).unwrap();
                if request.cancel { continue; }
                assert_eq!(request.method.as_deref(), Some(BARRIER));
                self.replies.send(serde_json::json!({"id":request.id,"ok":null}).to_string()).await.unwrap();
                barrier.await.unwrap();
                break;
            }
        });
    }

    fn save(&mut self, runtime: &tokio::runtime::Runtime, cx: &mut TestAppContext) -> ClientFrame {
        let request = self.request(cx, methods::SAVE_PROVIDER_SETTINGS);
        let draft = serde_json::from_value(request.params["draft"].clone()).unwrap();
        let revision = request.params["expectedRevision"].as_str().unwrap();
        let view = self.store.save(draft, revision).unwrap();
        self.reply(runtime, &request, serde_json::to_value(view).unwrap());
        cx.run_until_parked();
        request
    }

    fn assert_no_save(&mut self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        while let Ok(frame) = self.requests.try_recv() {
            let frame: ClientFrame = serde_json::from_str(&frame).unwrap();
            assert!(
                frame.cancel,
                "Unexpected RPC before blur or during an in-flight save"
            );
        }
    }
}

fn setup(
    runtime: &tokio::runtime::Runtime,
    cx: &mut TestAppContext,
    ids: &[&str],
) -> (Server, WindowHandle<ProvidersPage>) {
    cx.update(|cx| {
        gpui_base::init(cx);
        cx.set_global(Theme::dark());
        crate::app_menus::init(cx);
    });
    let (mut server, engine) = Server::new(ids);
    let window = cx.add_window(|_, cx| {
        let state = cx.new(|_| {
            let mut state = AppState::new();
            state.set_test_engine(engine);
            state
        });
        ProvidersPage::new(state, cx)
    });
    window
        .update(cx, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    let load = server.request(cx, methods::GET_PROVIDER_SETTINGS);
    server.reply(
        runtime,
        &load,
        serde_json::to_value(server.store.view()).unwrap(),
    );
    cx.run_until_parked();
    draw(cx, window);
    (server, window)
}

fn draw(cx: &mut TestAppContext, window: WindowHandle<ProvidersPage>) {
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
}

fn tick(cx: &mut TestAppContext) {
    cx.run_until_parked();
}

fn blur(cx: &mut TestAppContext, window: WindowHandle<ProvidersPage>) {
    window.update(cx, |_, window, _| window.blur()).unwrap();
    draw(cx, window);
    cx.run_until_parked();
}

fn focus_label(cx: &mut TestAppContext, window: WindowHandle<ProvidersPage>) {
    window
        .update(cx, |page, window, cx| {
            window.focus(&page.form.as_ref().unwrap().label.focus_handle(cx), cx);
        })
        .unwrap();
    draw(cx, window);
}

fn reload_with_stored_key(
    server: &mut Server,
    runtime: &tokio::runtime::Runtime,
    cx: &mut TestAppContext,
    window: WindowHandle<ProvidersPage>,
) -> ClientFrame {
    let mut provider = entry("first");
    provider.credential = CredentialUpdate::ReplaceLiteral {
        value: "synthetic-stored-key".into(),
    };
    server
        .store
        .save(
            ProviderDraft {
                default_model: None,
                providers: vec![provider, entry("second")],
                web_search: None,
            },
            &server.store.view().revision,
        )
        .unwrap();
    window.update(cx, |page, _, cx| page.load(cx)).unwrap();
    let load = server.request(cx, methods::GET_PROVIDER_SETTINGS);
    server.reply(
        runtime,
        &load,
        serde_json::to_value(server.store.view()).unwrap(),
    );
    cx.run_until_parked();
    draw(cx, window);
    server.request(cx, methods::GET_PROVIDER_API_KEY)
}

fn rendered_secret(cx: &mut TestAppContext, window: WindowHandle<ProvidersPage>) -> String {
    window
        .update(cx, |page, window, cx| {
            page.form.as_ref().unwrap().secret.update(cx, |input, cx| {
                let end = input.text().encode_utf16().count();
                gpui::EntityInputHandler::text_for_range(input, 0..end, &mut None, window, cx)
                    .unwrap()
            })
        })
        .unwrap()
}

#[gpui::test]
fn saved_api_keys_fill_the_masked_input_and_visibility_does_not_save(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &["first", "second"]);
    let key = reload_with_stored_key(&mut server, &runtime, cx, window);
    server.reply(
        &runtime,
        &key,
        serde_json::json!({"apiKey":"synthetic-stored-key"}),
    );
    cx.run_until_parked();
    assert_eq!(
        rendered_secret(cx, window),
        "*".repeat("synthetic-stored-key".len())
    );
    draw(cx, window);
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let reveal = visual.debug_bounds("providers-reveal").unwrap();
    visual.simulate_click(reveal.center(), gpui::Modifiers::default());
    assert_eq!(rendered_secret(cx, window), "synthetic-stored-key");
    window
        .update(cx, |page, _, cx| {
            page.toggle_reveal_key(cx);
            page.commit_form(cx);
        })
        .unwrap();
    assert_eq!(
        rendered_secret(cx, window),
        "*".repeat("synthetic-stored-key".len())
    );
    server.assert_no_save(cx);
    window
        .update(cx, |page, window, cx| {
            page.select(1, window, cx);
            assert!(!page.form.as_ref().unwrap().secret_visible);
            page.select(0, window, cx);
            assert!(!page.form.as_ref().unwrap().secret_visible);
        })
        .unwrap();
    let key = server.request(cx, methods::GET_PROVIDER_API_KEY);
    server.reply(
        &runtime,
        &key,
        serde_json::json!({"apiKey":"synthetic-stored-key"}),
    );
    cx.run_until_parked();
    assert_eq!(
        rendered_secret(cx, window),
        "*".repeat("synthetic-stored-key".len())
    );
    server.assert_no_save(cx);
}

#[gpui::test]
fn a_delayed_saved_key_does_not_replace_new_input(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &["first", "second"]);
    let key = reload_with_stored_key(&mut server, &runtime, cx, window);
    window
        .update(cx, |page, _, cx| {
            page.form
                .as_ref()
                .unwrap()
                .secret
                .update(cx, |input, cx| input.set_text("synthetic-replacement", cx));
        })
        .unwrap();
    server.reply(
        &runtime,
        &key,
        serde_json::json!({"apiKey":"synthetic-stored-key"}),
    );
    cx.run_until_parked();
    assert_eq!(
        rendered_secret(cx, window),
        "*".repeat("synthetic-replacement".len())
    );
    window
        .update(cx, |page, _, cx| page.commit_form(cx))
        .unwrap();
    server.save(&runtime, cx);
    assert_eq!(
        server.store.snapshot().config.providers[0]
            .api_key
            .as_deref(),
        Some("synthetic-replacement")
    );
}

#[gpui::test]
fn provider_fields_save_on_blur_without_rebuilding_the_form(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &["first", "second"]);
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    assert!(visual.debug_bounds("providers-add").is_some());
    assert!(visual.debug_bounds("providers-refresh").is_none());
    assert!(visual.debug_bounds("providers-save").is_none());
    let label = window
        .update(cx, |page, _, cx| {
            assert_eq!(page.selected, Some(0));
            let form = page.form.as_ref().unwrap();
            let label = form.label.clone();
            label.update(cx, |input, cx| input.set_text("Renamed", cx));
            form.base_url
                .update(cx, |input, cx| input.set_text("https://example.com/v1", cx));
            form.secret
                .update(cx, |input, cx| input.set_text("synthetic-test-key", cx));
            label
        })
        .unwrap();
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(5));
    server.assert_no_save(cx);
    blur(cx, window);
    server.save(&runtime, cx);
    window
        .update(cx, |page, _, cx| {
            assert_eq!(page.selected, Some(0));
            assert_eq!(page.form.as_ref().unwrap().label, label);
            assert_eq!(label.read(cx).text(), "Renamed");
            assert_eq!(
                page.form.as_ref().unwrap().secret.read(cx).text(),
                "synthetic-test-key"
            );
            assert_eq!(page.saved_generation, page.edit_generation);
        })
        .unwrap();
    draw(cx, window);
    assert_eq!(
        server.store.view().providers[0].label.as_deref(),
        Some("Renamed")
    );
    assert_eq!(
        server.store.view().providers[0].credential,
        CredentialView::Stored
    );
    assert_eq!(server.store.view().providers[1].id, "second");
    assert_eq!(
        server.store.view().providers[0].models[0].label.as_deref(),
        Some("Curated model label")
    );
    tick(cx);
    server.assert_no_save(cx);
}

#[gpui::test]
fn provider_autosave_serializes_edits_and_preserves_a_newer_key(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &["first"]);
    window
        .update(cx, |page, _, cx| {
            let form = page.form.as_ref().unwrap();
            form.label
                .update(cx, |input, cx| input.set_text("Before", cx));
            form.secret
                .update(cx, |input, cx| input.set_text("synthetic-old", cx));
        })
        .unwrap();
    blur(cx, window);
    let first = server.request(cx, methods::SAVE_PROVIDER_SETTINGS);
    focus_label(cx, window);
    window
        .update(cx, |page, _, cx| {
            let form = page.form.as_ref().unwrap();
            form.label
                .update(cx, |input, cx| input.set_text("After", cx));
            form.secret
                .update(cx, |input, cx| input.set_text("synthetic-new", cx));
        })
        .unwrap();
    blur(cx, window);
    server.assert_no_save(cx);
    let old = server
        .store
        .save(
            serde_json::from_value(first.params["draft"].clone()).unwrap(),
            first.params["expectedRevision"].as_str().unwrap(),
        )
        .unwrap();
    server.reply(&runtime, &first, serde_json::to_value(old.clone()).unwrap());
    cx.run_until_parked();
    window
        .update(cx, |page, _, cx| {
            assert_eq!(page.form.as_ref().unwrap().label.read(cx).text(), "After");
            assert!(
                !page
                    .form
                    .as_ref()
                    .unwrap()
                    .secret
                    .read(cx)
                    .text()
                    .is_empty()
            );
        })
        .unwrap();
    tick(cx);
    let second = server.save(&runtime, cx);
    assert_eq!(
        second.params["expectedRevision"].as_str(),
        Some(old.revision.as_str())
    );
    assert_eq!(
        server.store.view().providers[0].label.as_deref(),
        Some("After")
    );
    assert_eq!(
        server.store.snapshot().config.providers[0]
            .api_key
            .as_deref(),
        Some("synthetic-new")
    );
    window
        .update(cx, |page, _, _| {
            assert!(!page.saving);
            assert_eq!(page.saved_generation, page.edit_generation);
        })
        .unwrap();
}

#[gpui::test]
fn deleting_a_provider_selects_the_first_remaining_provider_and_autosaves(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &["first", "second", "third"]);
    window
        .update(cx, |page, window, cx| page.select(1, window, cx))
        .unwrap();
    for (deleted, remaining) in [
        ("second", Some("first")),
        ("first", Some("third")),
        ("third", None),
    ] {
        window
            .update(cx, |page, window, cx| {
                page.delete_provider(deleted, window, cx);
                assert_eq!(page.selected, remaining.map(|_| 0));
                assert_eq!(
                    page.form.as_ref().map(|form| form.label.read(cx).text()),
                    remaining
                );
            })
            .unwrap();
        tick(cx);
        server.save(&runtime, cx);
        window
            .update(cx, |page, _, cx| {
                assert_eq!(page.selected, remaining.map(|_| 0));
                assert_eq!(
                    page.form.as_ref().map(|form| form.label.read(cx).text()),
                    remaining
                );
            })
            .unwrap();
    }
    assert!(server.store.view().providers.is_empty());
}

#[gpui::test]
fn new_providers_and_model_dialog_edits_autosave_without_a_save_action(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &[]);
    window
        .update(cx, |page, window, cx| page.add_provider(window, cx))
        .unwrap();
    tick(cx);
    server.save(&runtime, cx);
    assert_eq!(server.store.view().providers.len(), 1);
    assert!(server.store.view().providers[0].models.is_empty());
    window
        .update(cx, |page, window, cx| {
            page.open_model_dialog(0, None, window, cx);
            let dialog = page.model_dialog.as_ref().unwrap();
            assert!(dialog.id.starts_with("m_"));
            assert_eq!(dialog.id.len(), 10);
            dialog
                .model
                .update(cx, |input, cx| input.set_text("vendor-model", cx));
            page.commit_model(cx);
        })
        .unwrap();
    tick(cx);
    server.save(&runtime, cx);
    window
        .update(cx, |page, window, cx| {
            page.open_model_dialog(0, Some(0), window, cx);
            let dialog = page.model_dialog.as_ref().unwrap();
            assert_eq!(
                dialog.id,
                page.draft.as_ref().unwrap().providers[0].models[0].id
            );
            dialog
                .context_window
                .update(cx, |input, cx| input.set_text("4096", cx));
            dialog
                .max_tokens
                .update(cx, |input, cx| input.set_text("1024", cx));
            page.set_dialog_input_format(InputFormat::Text, cx);
            page.add_dialog_level(ReasoningLevel::High, cx);
        })
        .unwrap();
    tick(cx);
    server.save(&runtime, cx);
    let model = &server.store.view().providers[0].models[0];
    assert_eq!(model.context_window, Some(4096));
    assert_eq!(model.max_tokens, Some(1024));
    assert_eq!(model.input_format, InputFormat::Text);
    assert_eq!(model.reasoning_levels, vec![ReasoningLevel::High]);
    window
        .update(cx, |page, _, cx| {
            assert!(
                page.model_dialog.is_some(),
                "Autosave must leave the editor open"
            );
            page.close_model_dialog(cx);
            page.delete_model(0, 0, cx);
        })
        .unwrap();
    tick(cx);
    server.save(&runtime, cx);
    assert!(server.store.view().providers[0].models.is_empty());
}

fn seed_model_catalog(server: &Server, window: WindowHandle<ProvidersPage>, cx: &mut TestAppContext) {
    let root = server._dir.path().to_path_buf();
    std::fs::create_dir_all(root.join("cache")).unwrap();
    std::fs::write(rcode_harness::model_catalog::cache_path(&root), serde_json::json!({
        "vendor": {"models": {"known-model": {
            "id":"known-model", "name":"Known Model", "canonical_model_id":"vendor/known-model",
            "limit":{"context":1000000,"output":64000},
            "modalities":{"input":["text","image"]}, "reasoning":true,
            "reasoning_options":[{"type":"effort","values":["low","high","max"]}]
        }}}
    }).to_string()).unwrap();
    window.update(cx, |page, _, cx| page.state.update(cx, |state, _| state.data_dir = Some(root))).unwrap();
}

#[gpui::test]
fn model_catalog_fills_on_typing_and_immediate_add_and_preserves_manual_overrides(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &["first"]);
    seed_model_catalog(&server, window, cx);
    window.update(cx, |page, window, cx| {
        page.open_model_dialog(0, None, window, cx);
        let dialog = page.model_dialog.as_ref().unwrap();
        assert_eq!(dialog.context_window.read(cx).text(), "200000");
        assert_eq!(dialog.max_tokens.read(cx).text(), "128000");
        dialog.model.update(cx, |input, cx| input.set_text("known-model", cx));
    }).unwrap();
    cx.run_until_parked();
    window.update(cx, |page, _, cx| {
        let dialog = page.model_dialog.as_ref().unwrap();
        assert_eq!(dialog.context_window.read(cx).text(), "1000000");
        assert_eq!(dialog.max_tokens.read(cx).text(), "64000");
        assert_eq!(dialog.input_format, InputFormat::TextAndImages);
        assert_eq!(dialog.reasoning_levels, [ReasoningLevel::Low,ReasoningLevel::High,ReasoningLevel::Max]);
        dialog.max_tokens.update(cx, |input, cx| input.set_text("8192", cx));
        page.commit_model(cx);
    }).unwrap();
    server.save(&runtime, cx);
    assert_eq!(server.store.view().providers[0].models[1].max_tokens,Some(8192));
    window.update(cx, |page, window, cx| {
        page.open_model_dialog(0, Some(1), window, cx);
        assert_eq!(page.model_dialog.as_ref().unwrap().max_tokens.read(cx).text(), "8192");
        page.close_model_dialog(cx);
        page.open_model_dialog(0, None, window, cx);
        page.model_dialog.as_ref().unwrap().model.update(cx, |input, cx| input.set_text("known-model", cx));
        // Commit before the model Edited event is dispatched.
        page.commit_model(cx);
    }).unwrap();
    server.save(&runtime, cx);
    assert_eq!(server.store.view().providers[0].models[2].context_window,Some(1000000));
    window.update(cx, |page, window, cx| {
        page.open_model_dialog(0, Some(2), window, cx);
        page.model_dialog.as_ref().unwrap().model.update(cx, |input, cx| input.set_text("unknown-model", cx));
        page.commit_model(cx);
    }).unwrap();
    server.save(&runtime, cx);
    let unknown = &server.store.view().providers[0].models[2];
    assert_eq!(unknown.context_window,Some(200000));
    assert_eq!(unknown.max_tokens,Some(128000));
    assert_eq!(unknown.input_format,InputFormat::Text);
    assert!(unknown.reasoning_levels.is_empty());
}

#[gpui::test]
fn model_catalog_import_uses_the_cache_and_falls_back_for_unknown_models(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &["first"]);
    seed_model_catalog(&server, window, cx);
    window.update(cx, |page, window, cx| page.open_model_fetch(0, window, cx)).unwrap();
    let request = server.request(cx, methods::FETCH_PROVIDER_MODELS);
    server.reply(&runtime,&request,serde_json::json!([
        {"model":"known-model", "displayName":null, "ownedBy":"vendor"},
        {"model":"unknown-model", "displayName":null, "ownedBy":null}
    ]));
    cx.run_until_parked();
    window.update(cx, |page, window, cx| page.import_fetched_models(window, cx)).unwrap();
    server.save(&runtime, cx);
    let view = server.store.view();
    let known = &view.providers[0].models[1];
    assert_eq!(known.context_window,Some(1000000));
    assert_eq!(known.max_tokens,Some(64000));
    assert!(known.input_format.accepts_images());
    assert_eq!(known.reasoning_levels,[ReasoningLevel::Low,ReasoningLevel::High,ReasoningLevel::Max]);
    assert_eq!(known.label.as_deref(),Some("Known Model"));
    let unknown = &view.providers[0].models[2];
    assert_eq!(unknown.context_window,Some(200000));
    assert_eq!(unknown.max_tokens,Some(128000));
    assert_eq!(unknown.input_format,InputFormat::Text);
    assert!(unknown.reasoning_levels.is_empty());
    assert_eq!(view.providers[0].models[0].context_window,None);
}

#[gpui::test]
fn generated_model_ids_are_unique_and_stable_across_edits(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &["first", "second"]);
    let mut generated = std::collections::HashSet::new();
    for provider in [0, 0, 1] {
        let id = window
            .update(cx, |page, window, cx| {
                page.open_model_dialog(provider, None, window, cx);
                let dialog = page.model_dialog.as_ref().unwrap();
                assert!(dialog.model.focus_handle(cx).is_focused(window));
                let id = dialog.id.clone();
                assert!(id.starts_with("m_"));
                assert_eq!(id.len(), 10);
                assert!(id[2..].bytes().all(|byte| byte.is_ascii_hexdigit()));
                // A validation error does not recreate the internal identity.
                page.commit_model(cx);
                assert_eq!(page.model_dialog.as_ref().unwrap().id, id);
                assert!(page.model_dialog.as_ref().unwrap().error.is_some());
                id
            })
            .unwrap();
        server.assert_no_save(cx);
        assert!(generated.insert(id.clone()));
        window
            .update(cx, |page, _, cx| {
                page.model_dialog
                    .as_ref()
                    .unwrap()
                    .model
                    .update(cx, |input, cx| {
                        input.set_text("same-vendor-model", cx);
                    });
                page.commit_model(cx);
            })
            .unwrap();
        server.save(&runtime, cx);
        let view = server.store.view();
        let model = view.providers[provider].models.last().unwrap();
        assert_eq!(model.id, id);
        assert_eq!(model.model, "same-vendor-model");
    }
    // Both legacy and generated IDs survive changes to the vendor model name.
    for slot in [0, 1] {
        let original = server.store.view().providers[0].models[slot].id.clone();
        window
            .update(cx, |page, window, cx| {
                page.open_model_dialog(0, Some(slot), window, cx);
                assert_eq!(page.model_dialog.as_ref().unwrap().id, original);
                page.model_dialog
                    .as_ref()
                    .unwrap()
                    .model
                    .update(cx, |input, cx| {
                        input.set_text("renamed-vendor-model", cx);
                    });
                page.commit_model(cx);
            })
            .unwrap();
        server.save(&runtime, cx);
        assert_eq!(server.store.view().providers[0].models[slot].id, original);
    }
}

#[gpui::test]
fn leaving_the_provider_page_flushes_edits_without_waiting_for_blur(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &["first"]);
    window
        .update(cx, |page, _, cx| {
            page.form
                .as_ref()
                .unwrap()
                .label
                .update(cx, |input, cx| input.set_text("Persist after close", cx));
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |page, window, cx| {
            page.flush_pending(cx);
            window.remove_window();
        })
        .unwrap();
    tick(cx);
    server.save(&runtime, cx);
    assert_eq!(
        server.store.view().providers[0].label.as_deref(),
        Some("Persist after close")
    );
}

#[gpui::test]
fn autosave_failures_keep_edits_for_retry_and_conflicts_do_not_overwrite(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &["first"]);
    window
        .update(cx, |page, _, cx| {
            page.form
                .as_ref()
                .unwrap()
                .label
                .update(cx, |input, cx| input.set_text("Keep this edit", cx));
        })
        .unwrap();
    blur(cx, window);
    let failed = server.request(cx, methods::SAVE_PROVIDER_SETTINGS);
    server.reject(&runtime, &failed, "synthetic write failure");
    cx.run_until_parked();
    window
        .update(cx, |page, _, cx| {
            assert!(page.error.is_some());
            assert_eq!(
                page.form.as_ref().unwrap().label.read(cx).text(),
                "Keep this edit"
            );
            page.save(cx);
        })
        .unwrap();
    server.save(&runtime, cx);
    focus_label(cx, window);
    window
        .update(cx, |page, _, cx| {
            page.form
                .as_ref()
                .unwrap()
                .label
                .update(cx, |input, cx| input.set_text("Conflicting edit", cx));
        })
        .unwrap();
    blur(cx, window);
    let conflict = server.request(cx, methods::SAVE_PROVIDER_SETTINGS);
    server.reject(
        &runtime,
        &conflict,
        "revision-conflict: synthetic external change",
    );
    cx.run_until_parked();
    window
        .update(cx, |page, _, cx| {
            assert!(page.conflict.is_some());
            assert_eq!(
                page.form.as_ref().unwrap().label.read(cx).text(),
                "Conflicting edit"
            );
        })
        .unwrap();
    tick(cx);
    server.assert_no_save(cx);
    assert_eq!(
        server.store.view().providers[0].label.as_deref(),
        Some("Keep this edit")
    );
}

#[gpui::test]
fn switching_providers_flushes_the_previous_form_without_blur(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &["first", "second"]);
    window
        .update(cx, |page, window, cx| {
            page.form.as_ref().unwrap().label.update(cx, |input, cx| {
                input.set_text("Renamed before switching", cx);
            });
            page.select(1, window, cx);
        })
        .unwrap();
    server.save(&runtime, cx);
    assert_eq!(
        server.store.view().providers[0].label.as_deref(),
        Some("Renamed before switching")
    );
    window
        .update(cx, |page, _, cx| {
            assert_eq!(page.selected, Some(1));
            assert_eq!(page.form.as_ref().unwrap().label.read(cx).text(), "second");
        })
        .unwrap();
}

#[gpui::test]
fn a_save_reply_does_not_commit_text_still_being_typed(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &["first"]);
    window
        .update(cx, |page, _, cx| {
            page.form
                .as_ref()
                .unwrap()
                .secret
                .update(cx, |input, cx| input.set_text("synthetic-first", cx));
        })
        .unwrap();
    blur(cx, window);
    let first = server.request(cx, methods::SAVE_PROVIDER_SETTINGS);
    focus_label(cx, window);
    window
        .update(cx, |page, _, cx| {
            page.form
                .as_ref()
                .unwrap()
                .secret
                .update(cx, |input, cx| input.set_text("synthetic-still-typing", cx));
        })
        .unwrap();
    let view = server
        .store
        .save(
            serde_json::from_value(first.params["draft"].clone()).unwrap(),
            first.params["expectedRevision"].as_str().unwrap(),
        )
        .unwrap();
    server.reply(&runtime, &first, serde_json::to_value(view).unwrap());
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(5));
    server.assert_no_save(cx);
    blur(cx, window);
    server.save(&runtime, cx);
    assert_eq!(
        server.store.snapshot().config.providers[0]
            .api_key
            .as_deref(),
        Some("synthetic-still-typing")
    );
}

#[gpui::test]
fn model_fields_validate_and_save_on_blur_and_flush_on_leaving(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &["first"]);
    window
        .update(cx, |page, window, cx| {
            page.open_model_dialog(0, Some(0), window, cx)
        })
        .unwrap();
    draw(cx, window);
    for invalid in ["abc", "0", "4294967296"] {
        window
            .update(cx, |page, window, cx| {
                let input = &page.model_dialog.as_ref().unwrap().context_window;
                window.focus(&input.focus_handle(cx), cx);
            })
            .unwrap();
        draw(cx, window);
        window
            .update(cx, |page, _, cx| {
                page.model_dialog
                    .as_ref()
                    .unwrap()
                    .context_window
                    .update(cx, |input, cx| input.set_text(invalid, cx));
            })
            .unwrap();
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(5));
        server.assert_no_save(cx);
        blur(cx, window);
        server.assert_no_save(cx);
        window
            .update(cx, |page, _, _| {
                assert!(page.model_dialog.as_ref().unwrap().error.is_some())
            })
            .unwrap();
    }
    window
        .update(cx, |page, window, cx| {
            let dialog = page.model_dialog.as_ref().unwrap();
            dialog
                .context_window
                .update(cx, |input, cx| input.set_text("4096", cx));
            dialog
                .max_tokens
                .update(cx, |input, cx| input.set_text("8192", cx));
            window.focus(&dialog.max_tokens.focus_handle(cx), cx);
        })
        .unwrap();
    draw(cx, window);
    blur(cx, window);
    server.assert_no_save(cx);
    window
        .update(cx, |page, window, cx| {
            let input = &page.model_dialog.as_ref().unwrap().max_tokens;
            window.focus(&input.focus_handle(cx), cx);
            input.update(cx, |input, cx| input.set_text("1024", cx));
        })
        .unwrap();
    draw(cx, window);
    server.assert_no_save(cx);
    blur(cx, window);
    server.save(&runtime, cx);
    let model = &server.store.view().providers[0].models[0];
    assert_eq!(model.context_window, Some(4096));
    assert_eq!(model.max_tokens, Some(1024));
    window
        .update(cx, |page, _, cx| {
            assert!(page.model_dialog.as_ref().unwrap().error.is_none());
            page.model_dialog
                .as_ref()
                .unwrap()
                .model
                .update(cx, |input, cx| input.set_text("updated-before-leaving", cx));
            page.flush_pending(cx);
        })
        .unwrap();
    server.save(&runtime, cx);
    assert_eq!(
        server.store.view().providers[0].models[0].model,
        "updated-before-leaving"
    );
}

#[gpui::test]
fn model_fetch_defaults_to_all_selected_and_imports_only_checked_new_models(
    cx: &mut TestAppContext,
) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &["first", "second"]);
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let fetch = visual.debug_bounds("providers-model-fetch").unwrap();
    let add = visual.debug_bounds("providers-model-add").unwrap();
    assert!(fetch.center().x < add.center().x);
    visual.simulate_click(fetch.center(), gpui::Modifiers::default());
    let request = server.request(cx, methods::FETCH_PROVIDER_MODELS);
    assert_eq!(request.params["providerId"], "first");
    server.reply(
        &runtime,
        &request,
        serde_json::json!([
            {"model":"vendor-model", "displayName":null, "ownedBy":"vendor"},
            {"model":"new-a", "displayName":"Model A", "ownedBy":"vendor"},
            {"model":"new-b", "displayName":null, "ownedBy":null}
        ]),
    );
    cx.run_until_parked();
    window
        .update(cx, |page, _, _| {
            assert_eq!(page.fetch_dialog.as_ref().unwrap().selected.len(), 3);
        })
        .unwrap();
    draw(cx, window);
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let all = visual.debug_bounds("providers-fetch-all").unwrap();
    visual.simulate_click(all.center(), gpui::Modifiers::default());
    window
        .update(cx, |page, window, cx| {
            assert!(page.fetch_dialog.as_ref().unwrap().selected.is_empty());
            page.import_fetched_models(window, cx);
            assert!(page.fetch_dialog.is_some(), "empty selection cannot import");
            page.toggle_all_fetched_models(cx);
        })
        .unwrap();
    draw(cx, window);
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let row = visual.debug_bounds("providers-fetch-check-2").unwrap();
    visual.simulate_click(row.center(), gpui::Modifiers::default());
    window
        .update(cx, |page, _, _| {
            assert!(
                !page
                    .fetch_dialog
                    .as_ref()
                    .unwrap()
                    .selected
                    .contains("new-b")
            );
        })
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    let import = visual.debug_bounds("providers-fetch-import").unwrap();
    visual.simulate_click(import.center(), gpui::Modifiers::default());
    server.save(&runtime, cx);
    let view = server.store.view();
    let models = &view.providers[0].models;
    assert_eq!(models.len(), 2, "already configured models are skipped");
    assert_eq!(models[0].id, "first-model");
    assert_eq!(models[1].model, "new-a");
    assert!(models[1].id.starts_with("m_"));
    assert_eq!(models[1].label.as_deref(), Some("Model A"));
    assert_eq!(view.providers[1].models.len(), 1);
    window
        .update(cx, |page, _, _| assert!(page.fetch_dialog.is_none()))
        .unwrap();
}

#[gpui::test]
fn model_fetch_waits_for_config_saves_and_recovers_from_request_errors(cx: &mut TestAppContext) {
    let runtime = test_runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx, &["first"]);
    window
        .update(cx, |page, window, cx| {
            page.form
                .as_ref()
                .unwrap()
                .secret
                .update(cx, |input, cx| input.set_text("synthetic-latest", cx));
            page.open_model_fetch(0, window, cx);
        })
        .unwrap();
    let failed_save = server.request(cx, methods::SAVE_PROVIDER_SETTINGS);
    server.reject(&runtime, &failed_save, "synthetic save failure");
    cx.run_until_parked();
    server.assert_no_save(cx);
    window
        .update(cx, |page, _, cx| {
            assert!(matches!(
                page.fetch_dialog.as_ref().unwrap().models,
                Loadable::Idle
            ));
            assert!(page.error.is_some());
            page.retry_model_fetch(cx);
        })
        .unwrap();
    server.save(&runtime, cx);
    let request = server.request(cx, methods::FETCH_PROVIDER_MODELS);
    assert_eq!(
        request.params["expectedRevision"],
        server.store.view().revision
    );
    assert_eq!(
        server.store.snapshot().config.providers[0]
            .api_key
            .as_deref(),
        Some("synthetic-latest")
    );
    server.reject(&runtime, &request, "HTTP 401");
    cx.run_until_parked();
    window.update(cx, |page, _, cx| {
        assert!(matches!(&page.fetch_dialog.as_ref().unwrap().models, Loadable::Error(error) if error.contains("401")));
        page.retry_model_fetch(cx);
    }).unwrap();
    let retry = server.request(cx, methods::FETCH_PROVIDER_MODELS);
    server.reply(&runtime, &retry, serde_json::json!([]));
    cx.run_until_parked();
    window.update(cx, |page, window, cx| {
        assert!(matches!(&page.fetch_dialog.as_ref().unwrap().models, Loadable::Ready(models) if models.is_empty()));
        page.close_model_fetch(window, cx);
        page.form.as_ref().unwrap().label.update(cx, |input, cx| input.set_text("Changed before cancelling", cx));
        page.open_model_fetch(0, window, cx);
        page.close_model_fetch(window, cx);
    }).unwrap();
    server.save(&runtime, cx);
    server.assert_no_save(cx);
    assert_eq!(
        server.store.view().providers[0].label.as_deref(),
        Some("Changed before cancelling")
    );
}
