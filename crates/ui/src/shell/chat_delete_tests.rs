use super::*;
use gpui::{AppContext, TestAppContext};

#[gpui::test]
fn delete_chat_waits_for_ack_and_survives_other_sidebar_mutations(cx: &mut TestAppContext) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();
    let (out, mut requests) = tokio::sync::mpsc::channel(64);
    let (replies, inbound) = tokio::sync::mpsc::channel(64);
    let engine =
        crate::state::EngineHandle::from_test_client(rcode_rpc::RpcClient::new(out, inbound));
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        settings::init(UiSettings::default(), dir.path(), cx);
        crate::history::init(
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            cx,
        );
        gpui_base::init(cx);
        cx.set_global(Theme::default());
        crate::app_menus::init(cx);
    });
    let window = cx.add_window(|_, cx| {
        let state = cx.new(|_| {
            let mut state = AppState::new();
            state.chats = ["a", "b"]
                .into_iter()
                .map(|id| {
                    serde_json::from_value(serde_json::json!({
                        "id": id, "deviceId": "local", "archived": false,
                        "createdAt": Utc::now(),
                    }))
                    .unwrap()
                })
                .collect();
            state.selected_chat = Some("a".into());
            state.set_test_engine(engine);
            state
        });
        Shell::new(
            state,
            EngineBootConfig {
                roots: DataRoots::same(dir.path()),
                ipc_port: 0,
                edge_url: "http://127.0.0.1:1".into(),
                edge_token: None,
                org_id: None,
                default_harness: rcode_proto::HarnessId::Mock,
            },
            cx,
        )
    });
    window
        .update(cx, |shell, _, cx| {
            shell.delete_chat("a".into(), cx);
            shell.delete_chat("a".into(), cx); // duplicate confirmation
            shell.delete_chat("b".into(), cx);
            assert_eq!(shell.state.read(cx).chats.len(), 2);
            assert_eq!(shell.state.read(cx).selected_chat.as_deref(), Some("a"));
            shell.mutate(
                serde_json::json!({"op":"renameChat","chatId":"a","title":"new"}),
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();
    let mut deletes = Vec::new();
    while let Ok(raw) = requests.try_recv() {
        let request: serde_json::Value = serde_json::from_str(&raw).unwrap();
        if request["params"]["op"] == "deleteChat" {
            deletes.push(request);
        }
    }
    assert_eq!(deletes.len(), 2);
    for request in deletes {
        let reply = if request["params"]["chatId"] == "a" {
            serde_json::json!({"id":request["id"], "ok":{"ok":true}})
        } else {
            serde_json::json!({"id":request["id"], "err":"rejected"})
        };
        runtime.block_on(async {
            replies.send(reply.to_string()).await.unwrap();
            while replies.capacity() < replies.max_capacity() {
                tokio::task::yield_now().await;
            }
        });
    }
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert!(shell.delete_chat_tasks.is_empty());
            assert_ne!(shell.state.read(cx).selected_chat.as_deref(), Some("a"));
            assert_eq!(
                shell
                    .state
                    .read(cx)
                    .chats
                    .iter()
                    .map(|chat| chat.id.as_str())
                    .collect::<Vec<_>>(),
                ["b"]
            );
            assert!(shell.sidebar_notice.is_some() || !crate::toast::messages(cx).is_empty());
        })
        .unwrap();
}
