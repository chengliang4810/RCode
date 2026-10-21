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
            let request: ClientFrame =
                serde_json::from_str(&self.requests.try_recv().expect("request")).unwrap();
            if !request.cancel {
                return request;
            }
        }
    }

    fn deliver(&mut self, runtime: &tokio::runtime::Runtime, reply: serde_json::Value) {
        let engine = self.engine.clone();
        runtime.block_on(async {
            self.replies.send(reply.to_string()).await.unwrap();
            let barrier = engine
                .client()
                .call("PersonalizationTestBarrier", serde_json::Value::Null);
            tokio::pin!(barrier);
            loop {
                let request = tokio::select! {
                    _ = &mut barrier => panic!("barrier replied too soon"),
                    request = self.requests.recv() => request.unwrap(),
                };
                let request: ClientFrame = serde_json::from_str(&request).unwrap();
                if request.cancel {
                    continue;
                }
                assert_eq!(
                    request.method.as_deref(),
                    Some("PersonalizationTestBarrier")
                );
                self.replies
                    .send(serde_json::json!({"id":request.id,"ok":null}).to_string())
                    .await
                    .unwrap();
                barrier.await.unwrap();
                break;
            }
        });
    }

    fn ack(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        request: &ClientFrame,
        content: &str,
        revision: &str,
    ) {
        self.deliver(
            runtime,
            serde_json::json!({"id":request.id,"ok":{"content":content,"revision":revision}}),
        );
    }

    fn assert_idle(&mut self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        while let Ok(frame) = self.requests.try_recv() {
            let frame: ClientFrame = serde_json::from_str(&frame).unwrap();
            assert!(frame.cancel, "unexpected request: {:?}", frame.method);
        }
    }
}

fn setup(
    runtime: &tokio::runtime::Runtime,
    cx: &mut TestAppContext,
) -> (Server, WindowHandle<PersonalizationPage>) {
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
        PersonalizationPage::new(state, cx)
    });
    window
        .update(cx, |_, window, _| window.activate_window())
        .unwrap();
    // Both cards read independently on opening. Drain both before the reply barrier.
    let first = server.request(cx);
    let second = server.request(cx);
    for request in [first, second] {
        let value = match request.method.as_deref() {
            Some(methods::GET_AGENT_SETTINGS) => serde_json::json!({"memory":true}),
            Some(methods::GET_CUSTOM_INSTRUCTIONS) => {
                serde_json::json!({"content":"原有指令\n","revision":"r0"})
            }
            method => panic!("unexpected method: {method:?}"),
        };
        server.deliver(runtime, serde_json::json!({"id":request.id,"ok":value}));
    }
    cx.run_until_parked();
    draw(cx, window);
    (server, window)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn draw(cx: &mut TestAppContext, window: WindowHandle<PersonalizationPage>) {
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
}

fn edit(cx: &mut TestAppContext, window: WindowHandle<PersonalizationPage>, content: &str) {
    window
        .update(cx, |page, window, cx| {
            window.focus(&page.input.focus_handle(cx), cx);
            page.input
                .update(cx, |input, cx| input.set_text(content, cx));
        })
        .unwrap();
    draw(cx, window);
}

fn blur(cx: &mut TestAppContext, window: WindowHandle<PersonalizationPage>) {
    window.update(cx, |_, window, _| window.blur()).unwrap();
    draw(cx, window);
    cx.run_until_parked();
}

#[gpui::test]
fn personalization_saves_only_on_blur_and_enter_inserts_a_newline(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    server.assert_idle(cx);
    edit(cx, window, "新的指令");
    cx.simulate_keystrokes(window.into(), "enter");
    window
        .update(cx, |page, _, cx| {
            assert_eq!(page.input.read(cx).text(), "新的指令\n")
        })
        .unwrap();
    server.assert_idle(cx);
    blur(cx, window);
    let save = server.request(cx);
    assert_eq!(
        save.method.as_deref(),
        Some(methods::SAVE_CUSTOM_INSTRUCTIONS)
    );
    assert_eq!(save.params["content"], "新的指令\n");
    assert_eq!(save.params["expectedRevision"], "r0");
    server.ack(&runtime, &save, "新的指令\n", "r1");
    cx.run_until_parked();
    window
        .update(cx, |page, _, cx| assert!(!page.has_pending_save(cx)))
        .unwrap();
    edit(cx, window, "新的指令\n");
    blur(cx, window);
    server.assert_idle(cx);
}

#[gpui::test]
fn personalization_serializes_blurs_and_preserves_newer_edits(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    edit(cx, window, "first");
    blur(cx, window);
    let first = server.request(cx);
    edit(cx, window, "second");
    blur(cx, window);
    server.assert_idle(cx);
    server.ack(&runtime, &first, "first", "r1");
    let second = server.request(cx);
    assert_eq!(second.params["content"], "second");
    assert_eq!(second.params["expectedRevision"], "r1");
    edit(cx, window, "still typing");
    server.ack(&runtime, &second, "second", "r2");
    cx.run_until_parked();
    server.assert_idle(cx);
    window
        .update(cx, |page, _, cx| {
            assert_eq!(page.input.read(cx).text(), "still typing");
            assert!(page.has_pending_save(cx));
            page.flush_pending(cx);
        })
        .unwrap();
    let final_save = server.request(cx);
    assert_eq!(final_save.params["content"], "still typing");
    assert_eq!(final_save.params["expectedRevision"], "r2");
    server.ack(&runtime, &final_save, "still typing", "r3");
}

#[gpui::test]
fn personalization_failed_save_retains_draft_and_retries_on_next_blur(cx: &mut TestAppContext) {
    let runtime = runtime();
    let _guard = runtime.enter();
    let (mut server, window) = setup(&runtime, cx);
    edit(cx, window, "keep this draft");
    blur(cx, window);
    let save = server.request(cx);
    server.deliver(
        &runtime,
        serde_json::json!({"id":save.id,"err":"disk unavailable"}),
    );
    cx.run_until_parked();
    window
        .update(cx, |page, _, cx| {
            assert_eq!(page.input.read(cx).text(), "keep this draft");
            assert_eq!(page.stored.as_ref().unwrap().content, "原有指令\n");
            assert!(page.error.is_some());
            assert!(page.has_pending_save(cx));
        })
        .unwrap();
    edit(cx, window, "keep this draft");
    blur(cx, window);
    let retry = server.request(cx);
    assert_eq!(retry.params["expectedRevision"], "r0");
    server.ack(&runtime, &retry, "keep this draft", "r1");
}
