use super::*;
use crate::state::EngineHandle;
use gpui::{Focusable, TestAppContext, WindowHandle};
use rcode_rpc::{ClientFrame, RpcClient};
use tokio::sync::mpsc;

struct Server {
    requests: mpsc::Receiver<String>,
    replies: mpsc::Sender<String>,
    engine: EngineHandle,
}

impl Server {
    fn request(&mut self, cx: &mut TestAppContext) -> ClientFrame {
        cx.run_until_parked();
        loop {
            let frame: ClientFrame =
                serde_json::from_str(&self.requests.try_recv().expect("RPC request")).unwrap();
            if !frame.cancel {
                return frame;
            }
        }
    }

    fn reply(&mut self, runtime: &tokio::runtime::Runtime, request: &ClientFrame, value: Value) {
        self.deliver(runtime, json!({"id":request.id,"ok":value}));
    }

    fn fail(&mut self, runtime: &tokio::runtime::Runtime, request: &ClientFrame, reason: &str) {
        self.deliver(runtime, json!({"id":request.id,"err":reason}));
    }

    fn deliver(&mut self, runtime: &tokio::runtime::Runtime, value: Value) {
        let engine = self.engine.clone();
        runtime.block_on(async {
            self.replies.send(value.to_string()).await.unwrap();
            let barrier = engine.client().call("DevicesTestBarrier", Value::Null);
            tokio::pin!(barrier);
            loop {
                let frame = tokio::select! {
                    _ = &mut barrier => panic!("barrier replied before it was requested"),
                    frame = self.requests.recv() => frame.unwrap(),
                };
                let frame: ClientFrame = serde_json::from_str(&frame).unwrap();
                if frame.cancel {
                    continue;
                }
                assert_eq!(frame.method.as_deref(), Some("DevicesTestBarrier"));
                self.replies
                    .send(json!({"id":frame.id,"ok":null}).to_string())
                    .await
                    .unwrap();
                barrier.await.unwrap();
                return;
            }
        });
    }

    fn assert_idle(&mut self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        while let Ok(frame) = self.requests.try_recv() {
            let frame: ClientFrame = serde_json::from_str(&frame).unwrap();
            assert!(frame.cancel, "unexpected RPC: {:?}", frame.method);
        }
    }
}

fn server() -> Server {
    let (out, requests) = mpsc::channel(64);
    let (replies, inbound) = mpsc::channel(64);
    Server {
        requests,
        replies,
        engine: EngineHandle::from_test_client(RpcClient::new(out, inbound)),
    }
}

fn finish_context_load(
    server: &mut Server,
    runtime: &tokio::runtime::Runtime,
    cx: &mut TestAppContext,
) {
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("GetRemoteAccess"));
    server.reply(
        runtime,
        &request,
        json!({"enabled":false,"port":29666,"hasApiKey":false,"error":null}),
    );
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("ListRemoteHosts"));
    server.reply(
        runtime,
        &request,
        json!([{
            "id":"new-host","name":"New host","url":"ws://127.0.0.1:29666",
            "enabled":true,"connected":true,"error":null
        }]),
    );
    cx.run_until_parked();
}

fn replace_engine(
    window: WindowHandle<DevicesPage>,
    engine: EngineHandle,
    cx: &mut TestAppContext,
) {
    window
        .update(cx, |page, _, cx| {
            page.state.update(cx, |state, cx| {
                state.set_test_engine(engine);
                cx.notify();
            });
        })
        .unwrap();
    cx.run_until_parked();
}

fn replace_engine_and_act_before_observers(
    window: WindowHandle<DevicesPage>,
    engine: EngineHandle,
    cx: &mut TestAppContext,
    action: impl FnOnce(&mut DevicesPage, &mut Context<DevicesPage>),
) {
    window
        .update(cx, |page, _, cx| {
            page.state.update(cx, |state, cx| {
                state.set_test_engine(engine);
                cx.notify();
            });
            // This is the same UI callback, before queued notifications can
            // reset the page. Exercise real RPC tasks rather than a manually
            // detached task that production does not retain across replacement.
            assert_ne!(page.loaded_context, page.context(cx));
            action(page, cx);
        })
        .unwrap();
}

#[gpui::test]
fn engine_switch_and_leave_in_one_callback_never_send_the_old_access_draft(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut old, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| {
            page.port
                .update(cx, |input, cx| input.set_text("28777", cx));
        })
        .unwrap();
    let mut new = server();
    replace_engine_and_act_before_observers(window, new.engine.clone(), cx, |page, cx| {
        page.set_active(false, cx);
    });
    // The first actual request on the replacement connection must be its own
    // load, never SetRemoteAccess carrying the previous page's port.
    finish_context_load(&mut new, &runtime, cx);
    new.assert_idle(cx);
    old.assert_idle(cx);
    window
        .update(cx, |page, _, cx| {
            assert_eq!(page.port.read(cx).text(), "29666");
            assert!(!page.saving);
            assert!(page.pending_actions.is_empty());
        })
        .unwrap();
}

#[gpui::test]
fn engine_switch_and_host_edit_in_one_callback_never_request_the_old_host_key(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut old, window) = setup(&runtime, cx);
    let mut new = server();
    replace_engine_and_act_before_observers(window, new.engine.clone(), cx, |page, cx| {
        page.edit_host(Some(host("old-host")), cx);
    });
    finish_context_load(&mut new, &runtime, cx);
    new.assert_idle(cx);
    old.assert_idle(cx);
    window
        .update(cx, |page, _, _| assert!(page.editor.is_none()))
        .unwrap();
}

#[gpui::test]
fn engine_switch_and_host_write_in_one_callback_never_send_the_old_host_payload(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut old, window) = setup(&runtime, cx);
    let mut new = server();
    replace_engine_and_act_before_observers(window, new.engine.clone(), cx, |page, cx| {
        page.write_host(
            "SaveRemoteHost",
            json!({"id":"old-host","name":"Old draft"}),
            true,
            cx,
        );
    });
    finish_context_load(&mut new, &runtime, cx);
    new.assert_idle(cx);
    old.assert_idle(cx);
    window
        .update(cx, |page, _, _| {
            assert!(!page.saving);
            assert!(page.pending_actions.is_empty());
        })
        .unwrap();
}

fn setup(
    runtime: &tokio::runtime::Runtime,
    cx: &mut TestAppContext,
) -> (Server, WindowHandle<DevicesPage>) {
    let (mut server, window) = setup_loading(cx);
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("GetRemoteAccess"));
    server.reply(
        runtime,
        &request,
        json!({"enabled":false,"port":27655,"hasApiKey":false,"error":null}),
    );
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("ListRemoteHosts"));
    server.reply(runtime, &request, json!([]));
    cx.run_until_parked();
    (server, window)
}

fn setup_loading(cx: &mut TestAppContext) -> (Server, WindowHandle<DevicesPage>) {
    cx.update(|cx| {
        gpui_base::init(cx);
        cx.set_global(Theme::dark());
        crate::app_menus::init(cx);
        crate::composer::init(cx, crate::settings::ComposerSendBehavior::default());
    });
    let server = server();
    let engine = server.engine.clone();
    let window = cx.add_window(|_, cx| {
        let state = cx.new(|_| {
            let mut state = AppState::new();
            state.set_test_engine(engine);
            state
        });
        DevicesPage::new(state, cx)
    });
    // Stop the periodic refresh so the tests exercise only user operations.
    window.update(cx, |page, _, _| page._poll = None).unwrap();
    (server, window)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn host(id: &str) -> RemoteHost {
    RemoteHost {
        id: id.into(),
        name: "Fixture".into(),
        url: "ws://127.0.0.1:27655".into(),
        enabled: true,
        connected: true,
        error: None,
    }
}

fn draw(cx: &mut TestAppContext, window: WindowHandle<DevicesPage>) {
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
}

fn finish_access_save(
    server: &mut Server,
    runtime: &tokio::runtime::Runtime,
    cx: &mut TestAppContext,
    enabled: bool,
    port: u16,
    key: &str,
) {
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("SetRemoteAccess"));
    assert_eq!(request.params["enabled"], enabled);
    assert_eq!(request.params["port"], port);
    server.reply(
        runtime,
        &request,
        json!({"enabled":enabled,"port":port,"hasApiKey":!key.is_empty(),"error":null}),
    );
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("GetRemoteAccessApiKey"));
    server.reply(runtime, &request, json!({"apiKey":key}));
    cx.run_until_parked();
}

#[gpui::test]
fn loading_finishes_the_save_requested_by_blur_or_leaving(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    window.update(cx, |page, _, cx| page.load(cx)).unwrap();
    let request = server.request(cx);
    server.reply(
        &runtime,
        &request,
        json!({"enabled":true,"port":27655,"hasApiKey":true,"error":null}),
    );
    let request = server.request(cx);
    server.reply(&runtime, &request, json!([]));
    let key_load = server.request(cx);
    assert_eq!(key_load.method.as_deref(), Some("GetRemoteAccessApiKey"));
    window
        .update(cx, |page, _, cx| {
            page.port
                .update(cx, |input, cx| input.set_text("28888", cx));
            page.key
                .update(cx, |input, cx| input.set_text("LatestDraft123", cx));
            page.save_access_draft(cx);
            page.set_active(false, cx);
        })
        .unwrap();
    server.assert_idle(cx);
    server.reply(&runtime, &key_load, json!({"apiKey":"OldStored123"}));
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("SetRemoteAccess"));
    assert_eq!(
        request.params,
        json!({"enabled":true,"port":28888,"apiKey":"LatestDraft123"})
    );
    server.reply(
        &runtime,
        &request,
        json!({"enabled":true,"port":28888,"hasApiKey":true,"error":null}),
    );
    let request = server.request(cx);
    server.reply(&runtime, &request, json!({"apiKey":"LatestDraft123"}));
    server.assert_idle(cx);
    window
        .update(cx, |page, _, cx| {
            assert_eq!(page.port.read(cx).text(), "28888");
            assert_eq!(page.key.read(cx).text(), "LatestDraft123");
            assert!(page.pending_actions.is_empty());
        })
        .unwrap();
}

#[gpui::test]
fn adding_and_editing_hosts_waits_for_the_automatic_local_save(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| {
            page.port
                .update(cx, |input, cx| input.set_text("28888", cx));
            page.save_access_draft(cx);
            page.edit_host(None, cx);
        })
        .unwrap();
    finish_access_save(&mut server, &runtime, cx, false, 28888, "");
    window
        .update(cx, |page, _, cx| {
            assert!(page.editor.as_ref().unwrap().id.is_none());
            page.port
                .update(cx, |input, cx| input.set_text("28889", cx));
            page.save_access_draft(cx);
            page.edit_host(Some(host("existing")), cx);
        })
        .unwrap();
    finish_access_save(&mut server, &runtime, cx, false, 28889, "");
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("GetRemoteHostApiKey"));
    assert_eq!(request.params, json!({"id":"existing"}));
    server.reply(&runtime, &request, json!({"apiKey":"StoredHost123"}));
    server.assert_idle(cx);
    window
        .update(cx, |page, _, cx| {
            assert_eq!(
                page.editor.as_ref().unwrap().id.as_deref(),
                Some("existing")
            );
            assert_eq!(
                page.editor.as_ref().unwrap().key.read(cx).text(),
                "StoredHost123"
            );
        })
        .unwrap();
}

#[gpui::test]
fn remote_writes_keep_their_order_after_an_automatic_local_save(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| {
            page.port
                .update(cx, |input, cx| input.set_text("28888", cx));
            page.save_access_draft(cx);
            page.write_host(
                "SaveRemoteHost",
                json!({"id":"first","enabled":true}),
                false,
                cx,
            );
            page.write_host("RemoveRemoteHost", json!({"id":"second"}), false, cx);
        })
        .unwrap();
    finish_access_save(&mut server, &runtime, cx, false, 28888, "");
    for (method, id) in [("SaveRemoteHost", "first"), ("RemoveRemoteHost", "second")] {
        let request = server.request(cx);
        assert_eq!(request.method.as_deref(), Some(method));
        assert_eq!(request.params["id"], id);
        server.reply(&runtime, &request, json!({}));
        let request = server.request(cx);
        assert_eq!(request.method.as_deref(), Some("ListRemoteHosts"));
        server.reply(&runtime, &request, json!([]));
    }
    server.assert_idle(cx);
}

#[gpui::test]
fn queued_old_editor_writes_do_not_close_the_new_editor(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| {
            page.port
                .update(cx, |input, cx| input.set_text("28888", cx));
            page.save_access_draft(cx);
            page.edit_host(None, cx);
            page.write_host("SaveRemoteHost", json!({"id":"old"}), true, cx);
        })
        .unwrap();
    finish_access_save(&mut server, &runtime, cx, false, 28888, "");
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("SaveRemoteHost"));
    window
        .update(cx, |page, _, cx| {
            page.editor
                .as_ref()
                .unwrap()
                .name
                .update(cx, |input, cx| input.set_text("New draft", cx));
        })
        .unwrap();
    server.reply(&runtime, &request, json!({}));
    let request = server.request(cx);
    server.reply(&runtime, &request, json!([]));
    server.assert_idle(cx);
    window
        .update(cx, |page, _, cx| {
            assert_eq!(
                page.editor.as_ref().unwrap().name.read(cx).text(),
                "New draft"
            );
        })
        .unwrap();
}

#[gpui::test]
fn engine_replacement_discards_queued_remote_intents(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut old, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| {
            page.port
                .update(cx, |input, cx| input.set_text("28888", cx));
            page.save_access_draft(cx);
            page.edit_host(None, cx);
            page.write_host("RemoveRemoteHost", json!({"id":"old-host"}), false, cx);
        })
        .unwrap();
    let old_write = old.request(cx);
    let mut new = server();
    replace_engine(window, new.engine.clone(), cx);
    finish_context_load(&mut new, &runtime, cx);
    old.reply(
        &runtime,
        &old_write,
        json!({"enabled":false,"port":28888,"hasApiKey":false,"error":null}),
    );
    new.assert_idle(cx);
    window
        .update(cx, |page, _, _| {
            assert!(page.pending_actions.is_empty());
            assert!(page.editor.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn canceling_a_host_editor_waits_for_the_local_save(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| {
            page.edit_host(None, cx);
            page.port
                .update(cx, |input, cx| input.set_text("28888", cx));
            page.save_access_draft(cx);
            page.cancel_host(page.editor_generation, cx);
            page.save_access_draft(cx);
            page.save_access_draft(cx);
            assert_eq!(
                page.pending_actions
                    .iter()
                    .filter(|queued| matches!(&queued.action, PendingAction::AccessDraft))
                    .count(),
                1
            );
        })
        .unwrap();
    finish_access_save(&mut server, &runtime, cx, false, 28888, "");
    server.assert_idle(cx);
    window
        .update(cx, |page, _, _| assert!(page.editor.is_none()))
        .unwrap();
}

#[gpui::test]
fn failed_writes_continue_the_fifo_without_closing_a_new_editor(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| {
            page.edit_host(None, cx);
            page.port
                .update(cx, |input, cx| input.set_text("28888", cx));
            page.save_access_draft(cx);
            page.edit_host(None, cx);
            page.write_host(
                "RemoveRemoteHost",
                json!({"id":"failed-old-host"}),
                true,
                cx,
            );
            page.write_host(
                "SaveRemoteHost",
                json!({"id":"next-host","enabled":true}),
                false,
                cx,
            );
        })
        .unwrap();
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("SetRemoteAccess"));
    server.fail(&runtime, &request, "port unavailable");
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("RemoveRemoteHost"));
    window
        .update(cx, |page, _, cx| {
            page.editor
                .as_ref()
                .unwrap()
                .name
                .update(cx, |input, cx| input.set_text("Keep new draft", cx));
        })
        .unwrap();
    server.fail(&runtime, &request, "remote removal failed");
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("SaveRemoteHost"));
    assert_eq!(request.params, json!({"id":"next-host","enabled":true}));
    server.reply(&runtime, &request, json!({}));
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("ListRemoteHosts"));
    server.reply(&runtime, &request, json!([]));
    server.assert_idle(cx);
    window
        .update(cx, |page, _, cx| {
            assert_eq!(
                page.editor.as_ref().unwrap().name.read(cx).text(),
                "Keep new draft"
            );
            assert_eq!(page.access.as_ref().unwrap().port, 27655);
            assert!(
                page.error
                    .as_deref()
                    .is_some_and(|error| error.contains("remote removal failed"))
            );
        })
        .unwrap();
}

#[gpui::test]
fn failed_initial_load_retains_intents_until_retry_and_does_not_retarget_them(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup_loading(cx);
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("GetRemoteAccess"));
    window
        .update(cx, |page, _, cx| {
            page.port
                .update(cx, |input, cx| input.set_text("28888", cx));
            page.save_access_draft(cx);
            page.edit_host(None, cx);
            page.write_host("SaveRemoteHost", json!({"id":"first"}), false, cx);
        })
        .unwrap();
    server.fail(&runtime, &request, "initial load failed");
    server.assert_idle(cx);
    window
        .update(cx, |page, _, cx| {
            page.port
                .update(cx, |input, cx| input.set_text("28889", cx));
            page.write_host("RemoveRemoteHost", json!({"id":"second"}), false, cx);
            page.save_access(true, false, cx);
        })
        .unwrap();
    // Failure does not authorize writes from unknown local configuration, nor
    // may new clicks jump ahead of the retained FIFO.
    server.assert_idle(cx);
    window.update(cx, |page, _, cx| page.load(cx)).unwrap();
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("GetRemoteAccess"));
    server.reply(
        &runtime,
        &request,
        json!({"enabled":false,"port":27655,"hasApiKey":false,"error":null}),
    );
    let request = server.request(cx);
    server.reply(&runtime, &request, json!([]));
    finish_access_save(&mut server, &runtime, cx, false, 28889, "");
    for (method, id) in [("SaveRemoteHost", "first"), ("RemoveRemoteHost", "second")] {
        let request = server.request(cx);
        assert_eq!(request.method.as_deref(), Some(method));
        assert_eq!(request.params["id"], id);
        server.reply(&runtime, &request, json!({}));
        let request = server.request(cx);
        assert_eq!(request.method.as_deref(), Some("ListRemoteHosts"));
        server.reply(&runtime, &request, json!([]));
    }
    finish_access_save(&mut server, &runtime, cx, true, 28889, "GeneratedKey123");
    server.assert_idle(cx);
    window
        .update(cx, |page, _, cx| {
            assert!(page.editor.as_ref().unwrap().id.is_none());
            assert_eq!(page.port.read(cx).text(), "28889");
            page.load(cx);
            page.write_host(
                "RemoveRemoteHost",
                json!({"id":"old-engine-only"}),
                false,
                cx,
            );
        })
        .unwrap();
    let request = server.request(cx);
    server.fail(&runtime, &request, "another load failed");
    server.assert_idle(cx);
    let mut replacement = super::tests::server();
    replace_engine(window, replacement.engine.clone(), cx);
    finish_context_load(&mut replacement, &runtime, cx);
    replacement.assert_idle(cx);
    server.assert_idle(cx);
    window
        .update(cx, |page, _, _| assert!(page.editor.is_none()))
        .unwrap();
}

#[gpui::test]
fn local_access_fields_save_on_blur_including_the_disabled_port(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    window
        .update(cx, |_, window, _| window.activate_window())
        .unwrap();
    draw(cx, window);
    for enabled in [false, true] {
        window
            .update(cx, |page, window, cx| {
                page.access.as_mut().unwrap().enabled = enabled;
                let input = if enabled { &page.key } else { &page.port };
                window.focus(&input.focus_handle(cx), cx);
            })
            .unwrap();
        draw(cx, window);
        window
            .update(cx, |page, _, cx| {
                page.port.update(cx, |input, cx| {
                    input.set_text(if enabled { "28889" } else { "28888" }, cx)
                });
                if enabled {
                    page.key
                        .update(cx, |input, cx| input.set_text("UpdatedKey123", cx));
                }
            })
            .unwrap();
        server.assert_idle(cx);
        window.update(cx, |_, window, _| window.blur()).unwrap();
        draw(cx, window);
        let request = server.request(cx);
        assert_eq!(request.method.as_deref(), Some("SetRemoteAccess"));
        let port = if enabled { 28889 } else { 28888 };
        let expected = if enabled {
            json!({"enabled":true,"port":port,"apiKey":"UpdatedKey123"})
        } else {
            json!({"enabled":false,"port":port})
        };
        assert_eq!(request.params, expected);
        server.reply(
            &runtime,
            &request,
            json!({"enabled":enabled,"port":port,"hasApiKey":enabled,"error":null}),
        );
        let request = server.request(cx);
        assert_eq!(request.method.as_deref(), Some("GetRemoteAccessApiKey"));
        server.reply(
            &runtime,
            &request,
            json!({"apiKey":if enabled { "UpdatedKey123" } else { "" }}),
        );
        cx.run_until_parked();
        // Refocusing an unchanged field must not rewrite the configuration.
        window
            .update(cx, |page, window, cx| {
                window.focus(&page.port.focus_handle(cx), cx)
            })
            .unwrap();
        draw(cx, window);
        window.update(cx, |_, window, _| window.blur()).unwrap();
        draw(cx, window);
        server.assert_idle(cx);
    }
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    assert!(visual.debug_bounds("devices-access-save").is_none());
    assert!(visual.debug_bounds("devices-add-host").is_some());
}

#[gpui::test]
fn leaving_devices_saves_pending_local_changes(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| {
            page.port
                .update(cx, |input, cx| input.set_text("28888", cx));
            page.set_active(false, cx);
        })
        .unwrap();
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("SetRemoteAccess"));
    assert_eq!(request.params, json!({"enabled":false,"port":28888}));
    server.reply(
        &runtime,
        &request,
        json!({"enabled":false,"port":28888,"hasApiKey":false,"error":null}),
    );
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("GetRemoteAccessApiKey"));
    server.reply(&runtime, &request, json!({"apiKey":""}));
    server.assert_idle(cx);
}

#[gpui::test]
fn access_toggle_waits_for_the_automatic_field_save(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| {
            page.port
                .update(cx, |input, cx| input.set_text("28888", cx));
            page.save_access_draft(cx);
            page.save_access(true, false, cx);
        })
        .unwrap();
    for enabled in [false, true] {
        let request = server.request(cx);
        assert_eq!(request.method.as_deref(), Some("SetRemoteAccess"));
        assert_eq!(request.params, json!({"enabled":enabled,"port":28888}));
        server.reply(
            &runtime,
            &request,
            json!({"enabled":enabled,"port":28888,"hasApiKey":enabled,"error":null}),
        );
        let request = server.request(cx);
        assert_eq!(request.method.as_deref(), Some("GetRemoteAccessApiKey"));
        server.reply(
            &runtime,
            &request,
            json!({"apiKey":if enabled { "Fixture12345" } else { "" }}),
        );
    }
    server.assert_idle(cx);
    window
        .update(cx, |page, _, _| {
            assert!(page.access.as_ref().unwrap().enabled)
        })
        .unwrap();
}

#[gpui::test]
fn automatic_access_save_rejects_invalid_values_without_changing_saved_settings(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| {
            page.port.update(cx, |input, cx| input.set_text("0", cx));
            page.save_access_draft(cx);
            assert!(page.error.is_some());
            assert_eq!(page.access.as_ref().unwrap().port, 27655);
            page.access.as_mut().unwrap().enabled = true;
            page.port
                .update(cx, |input, cx| input.set_text("27655", cx));
            page.key
                .update(cx, |input, cx| input.set_text("invalid", cx));
            page.save_access_draft(cx);
            assert!(page.error.is_some());
            assert!(page.saved_key.is_empty());
        })
        .unwrap();
    server.assert_idle(cx);
}

#[gpui::test]
fn enabling_external_access_uses_the_generated_key_without_sending_an_empty_replacement(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| page.save_access(true, false, cx))
        .unwrap();
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("SetRemoteAccess"));
    assert_eq!(request.params, json!({"enabled":true,"port":27655}));
    server.reply(
        &runtime,
        &request,
        json!({"enabled":true,"port":27655,"hasApiKey":true,"error":null}),
    );
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("GetRemoteAccessApiKey"));
    server.reply(&runtime, &request, json!({"apiKey":"Fixture12345"}));
    cx.run_until_parked();
    window
        .update(cx, |page, _, cx| {
            assert_eq!(page.key.read(cx).text(), "Fixture12345");
            assert!(!page.key_visible);
            assert!(page.access.as_ref().unwrap().enabled);
        })
        .unwrap();
    server.assert_idle(cx);
}

#[gpui::test]
fn delayed_host_key_never_replaces_a_newer_draft_or_another_host(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| page.edit_host(Some(host("first")), cx))
        .unwrap();
    let request = server.request(cx);
    assert_eq!(request.params, json!({"id":"first"}));
    window
        .update(cx, |page, _, cx| {
            page.editor
                .as_ref()
                .unwrap()
                .key
                .update(cx, |input, cx| input.set_text("NewDraft1234", cx));
        })
        .unwrap();
    server.reply(&runtime, &request, json!({"apiKey":"SavedKey1234"}));
    cx.run_until_parked();
    window
        .update(cx, |page, _, cx| {
            assert_eq!(
                page.editor.as_ref().unwrap().key.read(cx).text(),
                "NewDraft1234"
            );
            page.edit_host(Some(host("second")), cx);
        })
        .unwrap();
    let request = server.request(cx);
    window
        .update(cx, |page, _, cx| {
            let key = page.editor.as_ref().unwrap().key.clone();
            key.update(cx, |input, cx| input.set_text("Draft1234567", cx));
            key.update(cx, |input, cx| input.set_text("", cx));
        })
        .unwrap();
    server.reply(&runtime, &request, json!({"apiKey":"SecondKey123"}));
    cx.run_until_parked();
    window
        .update(cx, |page, _, cx| {
            assert!(page.editor.as_ref().unwrap().key.read(cx).text().is_empty());
            page.edit_host(Some(host("third")), cx);
        })
        .unwrap();
    let request = server.request(cx);
    window
        .update(cx, |page, _, cx| page.edit_host(None, cx))
        .unwrap();
    server.reply(&runtime, &request, json!({"apiKey":"SecondKey123"}));
    cx.run_until_parked();
    window
        .update(cx, |page, _, cx| {
            let editor = page.editor.as_ref().unwrap();
            assert!(editor.id.is_none());
            assert!(editor.key.read(cx).text().is_empty());
        })
        .unwrap();
}

#[gpui::test]
fn refreshing_access_never_replaces_edits_made_while_the_key_load_is_pending(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    window.update(cx, |page, _, cx| page.load(cx)).unwrap();
    let request = server.request(cx);
    server.reply(
        &runtime,
        &request,
        json!({"enabled":true,"port":27655,"hasApiKey":true,"error":null}),
    );
    let request = server.request(cx);
    server.reply(&runtime, &request, json!([]));
    let request = server.request(cx);
    assert_eq!(request.method.as_deref(), Some("GetRemoteAccessApiKey"));
    window
        .update(cx, |page, _, cx| {
            page.key
                .update(cx, |input, cx| input.set_text("NewDraft1234", cx));
            page.port
                .update(cx, |input, cx| input.set_text("28888", cx));
        })
        .unwrap();
    server.reply(&runtime, &request, json!({"apiKey":"SavedKey1234"}));
    cx.run_until_parked();
    window
        .update(cx, |page, _, cx| {
            assert_eq!(page.key.read(cx).text(), "NewDraft1234");
            assert_eq!(page.port.read(cx).text(), "28888");
        })
        .unwrap();
}

#[gpui::test]
fn invalid_key_or_port_keeps_the_existing_access_configuration(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| {
            page.key
                .update(cx, |input, cx| input.set_text("onlyletters", cx));
            page.save_access(true, true, cx);
            assert!(page.error.is_some());
            assert!(!page.access.as_ref().unwrap().enabled);
            page.port.update(cx, |input, cx| input.set_text("0", cx));
            page.save_access(true, false, cx);
            assert!(!page.access.as_ref().unwrap().enabled);
        })
        .unwrap();
    server.assert_idle(cx);
}

#[gpui::test]
fn closing_external_access_ignores_unsaved_invalid_port_and_key(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    for draft in ["", "0", "unfinished"] {
        window
            .update(cx, |page, _, cx| {
                page.access = Some(RemoteAccess {
                    enabled: true,
                    port: 28888,
                    has_api_key: true,
                    error: None,
                });
                page.port.update(cx, |input, cx| input.set_text(draft, cx));
                page.key
                    .update(cx, |input, cx| input.set_text("unfinished", cx));
                page.save_access(false, false, cx);
            })
            .unwrap();
        let request = server.request(cx);
        assert_eq!(request.method.as_deref(), Some("SetRemoteAccess"));
        assert_eq!(request.params, json!({"enabled":false,"port":28888}));
        server.reply(
            &runtime,
            &request,
            json!({"enabled":false,"port":28888,"hasApiKey":true,"error":null}),
        );
        let request = server.request(cx);
        assert_eq!(request.method.as_deref(), Some("GetRemoteAccessApiKey"));
        server.reply(&runtime, &request, json!({"apiKey":"Fixture12345"}));
        cx.run_until_parked();
        window
            .update(cx, |page, _, _| {
                assert!(!page.access.as_ref().unwrap().enabled);
                assert!(page.error.is_none());
            })
            .unwrap();
    }
    server.assert_idle(cx);
}

#[gpui::test]
fn losing_the_engine_clears_sensitive_state_and_cancels_pending_operations(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| {
            page.key_visible = true;
            page.key.update(cx, |input, cx| {
                input.set_text("OldSecret123", cx);
                input.set_masked(false, cx);
            });
            page.hosts = vec![host("old-host")];
            page.edit_host(Some(host("old-host")), cx);
        })
        .unwrap();
    let old_key = server.request(cx);
    assert_eq!(old_key.method.as_deref(), Some("GetRemoteHostApiKey"));
    window
        .update(cx, |page, _, cx| page.save_access(true, false, cx))
        .unwrap();
    let old_write = server.request(cx);
    window
        .update(cx, |page, _, cx| {
            page.state
                .update(cx, |state, cx| state.prepare_runtime_replacement(cx));
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |page, _, cx| {
            assert!(page.loaded_context.is_none());
            assert!(page.access.is_none());
            assert!(page.hosts.is_empty());
            assert!(page.editor.is_none());
            assert!(page.key.read(cx).text().is_empty());
            assert!(!page.key_visible);
            assert_eq!(page.port.read(cx).text(), "27655");
            assert!(!page.loading && !page.saving);
            assert!(!page.key.read(cx).read_only && !page.port.read(cx).read_only);
            assert!(page._load.is_none() && page._write.is_none() && page._key_load.is_none());
        })
        .unwrap();
    server.reply(&runtime, &old_key, json!({"apiKey":"OldSecret123"}));
    server.reply(
        &runtime,
        &old_write,
        json!({"enabled":true,"port":27655,"hasApiKey":true,"error":null}),
    );
    server.assert_idle(cx);
}

#[gpui::test]
fn old_access_and_key_completions_cannot_modify_a_new_engine_or_unlock_its_write(
    cx: &mut TestAppContext,
) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut old, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| page.edit_host(Some(host("old-host")), cx))
        .unwrap();
    let old_key = old.request(cx);
    window
        .update(cx, |page, _, cx| page.save_access(true, false, cx))
        .unwrap();
    let old_write = old.request(cx);
    // Retain already-running callbacks outside the page to exercise the
    // generation guards even when cancellation cannot remove the completion.
    let _old_tasks = window
        .update(cx, |page, _, _| (page._write.take(), page._key_load.take()))
        .unwrap();
    let mut new = server();
    replace_engine(window, new.engine.clone(), cx);
    finish_context_load(&mut new, &runtime, cx);
    window
        .update(cx, |page, _, cx| {
            page.edit_host(None, cx);
            page.editor
                .as_ref()
                .unwrap()
                .key
                .update(cx, |input, cx| input.set_text("NewHostKey12", cx));
            page.key
                .update(cx, |input, cx| input.set_text("NewDraft1234", cx));
            page.save_access(true, false, cx);
        })
        .unwrap();
    let new_write = new.request(cx);
    old.reply(&runtime, &old_key, json!({"apiKey":"OldSecret123"}));
    old.reply(
        &runtime,
        &old_write,
        json!({"enabled":true,"port":27777,"hasApiKey":true,"error":null}),
    );
    let old_key_fetch = old.request(cx);
    assert_eq!(
        old_key_fetch.method.as_deref(),
        Some("GetRemoteAccessApiKey")
    );
    old.reply(&runtime, &old_key_fetch, json!({"apiKey":"OldSecret123"}));
    cx.run_until_parked();
    window
        .update(cx, |page, _, cx| {
            assert!(page.saving);
            assert!(page.key.read(cx).read_only);
            assert!(!page.access.as_ref().unwrap().enabled);
            assert_eq!(page.access.as_ref().unwrap().port, 29666);
            assert_eq!(page.key.read(cx).text(), "NewDraft1234");
            assert_eq!(
                page.editor.as_ref().unwrap().key.read(cx).text(),
                "NewHostKey12"
            );
            assert_eq!(page.hosts[0].id, "new-host");
            assert!(page.error.is_none());
        })
        .unwrap();
    new.reply(
        &runtime,
        &new_write,
        json!({"enabled":true,"port":29666,"hasApiKey":true,"error":null}),
    );
    let request = new.request(cx);
    assert_eq!(request.method.as_deref(), Some("GetRemoteAccessApiKey"));
    new.reply(&runtime, &request, json!({"apiKey":"NewActive123"}));
    cx.run_until_parked();
    window
        .update(cx, |page, _, cx| {
            assert!(!page.saving);
            assert!(page.access.as_ref().unwrap().enabled);
            assert_eq!(page.key.read(cx).text(), "NewActive123");
        })
        .unwrap();
}

#[gpui::test]
fn old_host_write_cannot_replace_the_new_registry_or_close_its_editor(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut old, window) = setup(&runtime, cx);
    window
        .update(cx, |page, _, cx| {
            page.write_host("SaveRemoteHost", json!({"id":"old-host"}), true, cx);
        })
        .unwrap();
    let old_write = old.request(cx);
    let _old_task = window.update(cx, |page, _, _| page._write.take()).unwrap();
    let mut new = server();
    replace_engine(window, new.engine.clone(), cx);
    finish_context_load(&mut new, &runtime, cx);
    window
        .update(cx, |page, _, cx| {
            page.edit_host(None, cx);
            page.editor
                .as_ref()
                .unwrap()
                .name
                .update(cx, |input, cx| input.set_text("New draft", cx));
        })
        .unwrap();
    old.reply(&runtime, &old_write, json!({"id":"old-host"}));
    let old_list = old.request(cx);
    assert_eq!(old_list.method.as_deref(), Some("ListRemoteHosts"));
    old.reply(&runtime, &old_list, json!([]));
    cx.run_until_parked();
    window
        .update(cx, |page, _, cx| {
            assert_eq!(page.hosts[0].id, "new-host");
            assert_eq!(
                page.editor.as_ref().unwrap().name.read(cx).text(),
                "New draft"
            );
            assert!(page.error.is_none());
        })
        .unwrap();
}
