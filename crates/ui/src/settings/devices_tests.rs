use super::*;
use crate::state::EngineHandle;
use gpui::{TestAppContext, WindowHandle};
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
        let engine = self.engine.clone();
        runtime.block_on(async {
            self.replies
                .send(json!({"id":request.id,"ok":value}).to_string())
                .await
                .unwrap();
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

fn setup(
    runtime: &tokio::runtime::Runtime,
    cx: &mut TestAppContext,
) -> (Server, WindowHandle<DevicesPage>) {
    cx.update(|cx| {
        gpui_base::init(cx);
        cx.set_global(Theme::dark());
        crate::app_menus::init(cx);
        crate::composer::init(cx, crate::settings::ComposerSendBehavior::default());
    });
    let (out, requests) = mpsc::channel(64);
    let (replies, inbound) = mpsc::channel(64);
    let engine = EngineHandle::from_test_client(RpcClient::new(out, inbound));
    let mut server = Server {
        requests,
        replies,
        engine: engine.clone(),
    };
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
