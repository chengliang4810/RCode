use super::*;
use crate::state::EngineHandle;
use gpui::{TestAppContext, WindowHandle};
use rcode_rpc::{ClientFrame, RpcClient};
use serde_json::{Value, json};
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

    fn deliver(&mut self, runtime: &tokio::runtime::Runtime, id: u64, value: Value) {
        let engine = self.engine.clone();
        runtime.block_on(async {
            self.replies
                .send(json!({"id":id,"ok":value}).to_string())
                .await
                .unwrap();
            let barrier = engine.client().call("MarketplaceTestBarrier", Value::Null);
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
                assert_eq!(request.method.as_deref(), Some("MarketplaceTestBarrier"));
                self.replies
                    .send(json!({"id":request.id,"ok":null}).to_string())
                    .await
                    .unwrap();
                barrier.await.unwrap();
                break;
            }
        });
    }
}

fn listing(names: &[&str]) -> Value {
    json!({"cached":false,"plugins":names.iter().map(|name| json!({
        "name":name,"description":format!("Description for {name}"),"author":"Author","installed":false,"unsupported":false
    })).collect::<Vec<_>>()})
}

#[gpui::test]
fn fresh_cache_skips_network_and_manual_refresh_preserves_browsing(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _enter = runtime.enter();
    let (window, mut server) = setup(cx);
    let request = server.request(cx);
    assert_eq!(request.params["refresh"], false);
    let mut cached = listing(&["review"]);
    cached["cached"] = json!(true);
    cached["stale"] = json!(false);
    server.deliver(&runtime, request.id, cached);
    cx.run_until_parked();
    assert!(
        server.requests.try_recv().is_err(),
        "fresh cache must not trigger another request"
    );
    window
        .update(cx, |page, _, cx| {
            assert!(page.loaded && !page.loading && !page.cached);
            page.search
                .update(cx, |input, cx| input.set_text("review", cx));
            page.reload(true, cx);
        })
        .unwrap();
    let request = server.request(cx);
    assert_eq!(request.params["refresh"], true);
    server.deliver(&runtime, request.id, listing(&["review", "format"]));
    cx.run_until_parked();
    window
        .read_with(cx, |page, cx| {
            assert_eq!(page.items.len(), 2);
            assert_eq!(page.search.read(cx).text(), "review");
            assert!(!page.loading);
        })
        .unwrap();
}

#[gpui::test]
fn expired_cache_is_visible_during_background_refresh_and_survives_failure(
    cx: &mut TestAppContext,
) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _enter = runtime.enter();
    let (window, mut server) = setup(cx);
    let request = server.request(cx);
    let names: Vec<_> = (0..45).map(|index| format!("review-{index:02}")).collect();
    let mut cached = listing(&names.iter().map(String::as_str).collect::<Vec<_>>());
    cached["cached"] = json!(true);
    cached["stale"] = json!(true);
    server.deliver(&runtime, request.id, cached.clone());
    let background = server.request(cx);
    assert_eq!(
        background.params,
        json!({"source":"official","refresh":true})
    );
    window
        .update(cx, |page, _, cx| {
            assert!(page.loaded && page.loading);
            assert_eq!(page.items.len(), 45);
            assert!(!page.cached);
            page.search
                .update(cx, |input, cx| input.set_text("review", cx));
        })
        .unwrap();
    cx.run_until_parked();
    window.update(cx, |page, _, cx| page.change_page(1, cx)).unwrap();
    server.deliver(&runtime, background.id, cached);
    cx.run_until_parked();
    window
        .read_with(cx, |page, cx| {
            assert!(page.loaded && !page.loading && page.cached);
            assert_eq!(page.items.len(), 45);
            assert_eq!(page.page, 1);
            assert_eq!(page.search.read(cx).text(), "review");
            assert!(page.error.is_none());
        })
        .unwrap();
    assert!(
        server.requests.try_recv().is_err(),
        "failed background refresh must not loop"
    );
}

#[gpui::test]
fn background_refresh_does_not_block_install_or_overwrite_installed_state(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _enter = runtime.enter();
    let (window, mut server) = setup(cx);
    let request = server.request(cx);
    let mut cached = listing(&["review"]);
    cached["cached"] = json!(true);
    cached["stale"] = json!(true);
    server.deliver(&runtime, request.id, cached);
    let background = server.request(cx);
    window
        .update(cx, |page, _, cx| page.install("review".into(), cx))
        .unwrap();
    let install = server.request(cx);
    assert_eq!(
        install.method.as_deref(),
        Some(methods::INSTALL_MARKETPLACE_PLUGIN)
    );
    server.deliver(&runtime, install.id, json!([]));
    server.deliver(&runtime, background.id, listing(&["obsolete"]));
    cx.run_until_parked();
    window
        .read_with(cx, |page, _| {
            assert_eq!(page.items[0].name, "review");
            assert!(page.items[0].installed);
            assert!(!page.loading && !page.has_pending_install());
        })
        .unwrap();
}

fn setup(cx: &mut TestAppContext) -> (WindowHandle<MarketplacePage>, Server) {
    cx.update(|cx| {
        gpui_base::init(cx);
        cx.set_global(Theme::dark());
        crate::app_menus::init(cx);
        crate::composer::init(cx, crate::settings::ComposerSendBehavior::default());
    });
    let (out, requests) = mpsc::channel(64);
    let (replies, inbound) = mpsc::channel(64);
    let engine = EngineHandle::from_test_client(RpcClient::new(out, inbound));
    let server = Server {
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
        MarketplacePage::new(state, cx)
    });
    (window, server)
}

#[gpui::test]
fn source_switching_and_installation_use_only_marketplace_identity(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _enter = runtime.enter();
    let (window, mut server) = setup(cx);
    let request = server.request(cx);
    assert_eq!(
        request.method.as_deref(),
        Some(methods::LIST_PLUGIN_MARKETPLACE)
    );
    assert_eq!(request.params["source"], "official");
    server.deliver(&runtime, request.id, listing(&["review", "format"]));
    cx.run_until_parked();
    window
        .read_with(cx, |page, _| assert_eq!(page.items.len(), 2))
        .unwrap();

    window
        .update(cx, |page, _, cx| page.select_source(1, cx))
        .unwrap();
    let ponytail = server.request(cx);
    assert_eq!(ponytail.params["source"], "ponytail");
    window
        .update(cx, |page, _, cx| page.select_source(2, cx))
        .unwrap();
    let superpowers = server.request(cx);
    assert_eq!(superpowers.params["source"], "superpowers");
    server.deliver(&runtime, ponytail.id, listing(&["outdated-reply"]));
    server.deliver(&runtime, superpowers.id, listing(&["superpowers"]));
    cx.run_until_parked();
    window
        .read_with(cx, |page, _| {
            assert_eq!(page.source, 2);
            assert_eq!(page.items[0].name, "superpowers");
        })
        .unwrap();

    window
        .update(cx, |page, _, cx| page.install("superpowers".into(), cx))
        .unwrap();
    let install = server.request(cx);
    assert_eq!(
        install.method.as_deref(),
        Some(methods::INSTALL_MARKETPLACE_PLUGIN)
    );
    assert_eq!(
        install.params,
        json!({"source":"superpowers","name":"superpowers"})
    );
    window
        .update(cx, |page, _, cx| page.select_source(0, cx))
        .unwrap();
    window
        .read_with(cx, |page, _| assert_eq!(page.source, 2))
        .unwrap();
    server.deliver(&runtime, install.id, json!([]));
    cx.run_until_parked();
    window
        .read_with(cx, |page, _| {
            assert!(page.items[0].installed);
            assert!(!page.has_pending_install());
        })
        .unwrap();
}

#[gpui::test]
fn search_filters_the_rendered_marketplace_rows(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _enter = runtime.enter();
    let (window, mut server) = setup(cx);
    let request = server.request(cx);
    server.deliver(&runtime, request.id, listing(&["review", "format"]));
    cx.run_until_parked();
    window
        .update(cx, |page, _, cx| {
            page.search
                .update(cx, |input, cx| input.set_text("review", cx));
            cx.notify();
        })
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, cx| {
        window.draw(cx).clear();
        window.draw(cx).clear();
    });
    assert!(visual.debug_bounds("marketplace-install-0").is_some());
    assert!(visual.debug_bounds("marketplace-install-1").is_none());
}

#[gpui::test]
fn pagination_limits_rows_and_search_returns_to_the_first_page(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _enter = runtime.enter();
    let (window, mut server) = setup(cx);
    let request = server.request(cx);
    let names: Vec<_> = (0..45).map(|index| format!("row-{index:02}")).collect();
    server.deliver(
        &runtime,
        request.id,
        listing(&names.iter().map(String::as_str).collect::<Vec<_>>()),
    );
    cx.run_until_parked();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, cx| {
        window.draw(cx).clear();
        window.draw(cx).clear();
    });
    assert!(visual.debug_bounds("marketplace-install-0").is_some());
    assert!(visual.debug_bounds("marketplace-install-20").is_none());
    let next = visual.debug_bounds("marketplace-next").unwrap().center();
    visual.simulate_click(next, Default::default());
    window
        .read_with(&visual, |page, _| assert_eq!(page.page, 1))
        .unwrap();
    visual.update(|window, cx| {
        window.draw(cx).clear();
        window.draw(cx).clear();
    });
    assert!(visual.debug_bounds("marketplace-install-0").is_none());
    assert!(visual.debug_bounds("marketplace-install-20").is_some());
    let next = visual.debug_bounds("marketplace-next").unwrap().center();
    visual.simulate_click(next, Default::default());
    window
        .read_with(&visual, |page, _| assert_eq!(page.page, 2))
        .unwrap();
    visual.update(|window, cx| {
        window.draw(cx).clear();
        window.draw(cx).clear();
    });
    assert!(visual.debug_bounds("marketplace-install-40").is_some());
    assert!(visual.debug_bounds("marketplace-install-39").is_none());
    let next = visual.debug_bounds("marketplace-next").unwrap().center();
    visual.simulate_click(next, Default::default());
    window
        .read_with(&visual, |page, _| assert_eq!(page.page, 2))
        .unwrap();
    window
        .update(&mut visual, |page, _, cx| {
            page.search.update(cx, |input, cx| {
                input.set_text("row-00", cx);
                cx.emit(ComposerInputEvent::Edited);
            });
        })
        .unwrap();
    window
        .read_with(&visual, |page, _| assert_eq!(page.page, 0))
        .unwrap();
    visual.update(|window, cx| {
        window.draw(cx).clear();
        window.draw(cx).clear();
    });
    assert!(visual.debug_bounds("marketplace-install-0").is_some());
    assert!(visual.debug_bounds("marketplace-next").is_none());
}
