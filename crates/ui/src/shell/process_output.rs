use super::*;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OutputTarget {
    pub chat: String,
    pub device: Option<String>,
    pub id: String,
    pub title: String,
}
#[derive(Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    status: String,
    output: String,
    output_path: String,
    #[serde(default)]
    rotated: bool,
}
impl Snapshot {
    fn running(&self) -> bool {
        matches!(self.status.as_str(), "running" | "stopping")
    }
}

pub(super) struct OutputView {
    pub target: OutputTarget,
    engine: crate::state::EngineHandle,
    latest: Option<Snapshot>,
    displayed: String,
    following: bool,
    visible: bool,
    error: Option<String>,
    scroll: gpui::ScrollHandle,
    poll: Option<Task<()>>,
    polling: bool,
    epoch: u64,
    stopping: bool,
}
pub(super) struct OpenLog {
    pub chat: String,
    pub path: String,
}
impl gpui::EventEmitter<OpenLog> for OutputView {}

impl OutputView {
    fn new(target: OutputTarget, engine: crate::state::EngineHandle) -> Self {
        Self {
            target,
            engine,
            latest: None,
            displayed: String::new(),
            following: true,
            visible: false,
            error: None,
            scroll: gpui::ScrollHandle::new(),
            poll: None,
            polling: false,
            epoch: 0,
            stopping: false,
        }
    }
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.epoch += 1;
        if visible {
            self.refresh(cx);
        }
    }
    fn apply(&mut self, value: Snapshot) {
        if self.following {
            self.displayed = value.output.clone();
        }
        self.latest = Some(value);
        self.error = None;
    }
    fn follow(&mut self, cx: &mut Context<Self>) {
        self.following = true;
        if let Some(latest) = &self.latest {
            self.displayed = latest.output.clone();
        }
        if self.error.is_some() {
            self.refresh(cx);
        }
        cx.notify();
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.epoch += 1;
        if !self.visible || self.polling {
            return;
        }
        self.polling = true;
        self.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                let Ok(request) = this.update(cx, |this, _| {
                    if !this.visible { this.polling = false; return None; }
                    Some((this.engine.clone(), this.epoch, serde_json::json!({"chatId":this.target.chat,"id":this.target.id,"targetDeviceId":this.target.device})))
                }) else { return; };
                let Some((engine, epoch, params)) = request else { return; };
                let result = engine.client().call(methods::READ_COMMAND_OUTPUT, params).await
                    .map_err(|e| e.to_string()).and_then(|value| serde_json::from_value::<Snapshot>(value).map_err(|e| e.to_string()));
                let Ok((again, stale)) = this.update(cx, |this, cx| {
                    let stale = epoch != this.epoch;
                    let running = if this.visible && !stale {
                        match result {
                            Ok(value) => { let running = value.running(); this.apply(value); running },
                            Err(error) => { this.error = Some(error); false }
                        }
                    } else { false };
                    let again = this.visible && (stale || running);
                    if !again { this.polling = false; }
                    cx.notify(); (again, stale)
                }) else { return; };
                if !again { return; }
                if !stale { cx.background_executor().timer(Duration::from_secs(1)).await; }
            }
        }));
    }
    fn stop(&mut self, cx: &mut Context<Self>) {
        if self.stopping || !self.latest.as_ref().is_some_and(Snapshot::running) {
            return;
        }
        self.stopping = true;
        let engine = self.engine.clone();
        let params = serde_json::json!({"chatId":self.target.chat,"id":self.target.id,"targetDeviceId":self.target.device});
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::STOP_COMMAND_PROCESS, params)
                .await;
            this.update(cx, |this, cx| {
                this.stopping = false;
                if let Err(error) = result {
                    this.error = Some(error.to_string());
                }
                this.refresh(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
    fn action(
        &self,
        id: &'static str,
        key: TextKey,
        cx: &Context<Self>,
        action: fn(&mut Self, &mut Context<Self>),
    ) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let label = crate::l10n::localizer().text(key);
        let view = cx.entity().downgrade();
        crate::settings::widgets::text_action(
            &theme,
            crate::settings::widgets::ActionTone::Quiet,
            label.clone(),
        )
        .id(id)
        .debug_selector(move || id.into())
        .role(gpui::Role::Button)
        .tab_index(0)
        .aria_label(label)
        .on_click(cx.listener(move |this, _, _, cx| action(this, cx)))
        .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                action(this, cx);
            }
        }))
        .on_a11y_action(gpui::AccessibleAction::Click, move |_, _, cx| {
            view.update(cx, action).ok();
        })
    }
}
impl Render for OutputView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let l = crate::l10n::localizer();
        if self.following {
            self.scroll
                .set_offset(gpui::point(px(0.0), px(-1_000_000.0)));
        }
        let running = self.latest.as_ref().is_some_and(Snapshot::running);
        let status = match self.latest.as_ref().map(|v| v.status.as_str()) {
            Some("running") => TextKey::ShellOutputRunning,
            Some("stopping") => TextKey::ShellProcessesStopping,
            Some("failed") => TextKey::ShellOutputFailed,
            Some("stopped") => TextKey::ShellOutputStopped,
            Some(_) => TextKey::ShellOutputExited,
            None => TextKey::ActionLoading,
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .text_color(theme.text)
            .text_size(crate::typography::ui_rems(12.0))
            .child(
                crate::surface_chrome::toolbar(&theme)
                    .h_auto()
                    .min_h(px(crate::surface_chrome::HEADER_HEIGHT))
                    .py(px(4.0))
                    .gap(px(6.0))
                    .flex_wrap()
                    .child(div().flex_1().child(l.text(status)))
                    .child(
                        self.action("output-copy", TextKey::ShellOutputCopy, cx, |this, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(this.displayed.clone()))
                        }),
                    )
                    .when(self.latest.is_some(), |el| {
                        el.child(self.action(
                            "output-file",
                            TextKey::ShellOutputFile,
                            cx,
                            |this, cx| {
                                if let Some(value) = &this.latest {
                                    cx.emit(OpenLog {
                                        chat: this.target.chat.clone(),
                                        path: value.output_path.clone(),
                                    });
                                }
                            },
                        ))
                    })
                    .child(self.action(
                        "output-follow",
                        if self.following {
                            TextKey::ShellOutputPause
                        } else {
                            TextKey::ShellOutputFollow
                        },
                        cx,
                        |this, cx| {
                            if this.following {
                                this.following = false;
                                cx.notify();
                            } else {
                                this.follow(cx);
                            }
                        },
                    ))
                    .when(running, |el| {
                        el.child(self.action(
                            "output-stop",
                            if self.stopping {
                                TextKey::ShellProcessesStopping
                            } else {
                                TextKey::ShellSubagentStop
                            },
                            cx,
                            Self::stop,
                        ))
                    }),
            )
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .px(px(12.0))
                        .py(px(6.0))
                        .flex()
                        .gap(px(6.0))
                        .child(div().flex_1().child(
                            l.format_with(TextKey::ShellOutputError, &[("error", error.as_str())]),
                        ))
                        .child(self.action(
                            "output-retry",
                            TextKey::ShellOutputRetry,
                            cx,
                            Self::refresh,
                        )),
                )
            })
            .when(self.latest.as_ref().is_some_and(|v| v.rotated), |el| {
                el.child(
                    div()
                        .px(px(12.0))
                        .text_color(theme.text_muted)
                        .child(l.text(TextKey::ShellOutputRotated)),
                )
            })
            .child(
                div()
                    .id("command-output-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_scroll_wheel(cx.listener(
                        |this, event: &gpui::ScrollWheelEvent, window, cx| {
                            if event.delta.pixel_delta(px(20.0)).y > px(0.0) {
                                this.following = false;
                                cx.notify();
                            }
                            let view = cx.entity().downgrade();
                            window.on_next_frame(move |_, cx| {
                                view.update(cx, |this, cx| {
                                    if !this.following
                                        && this.scroll.max_offset().y > px(0.0)
                                        && this.scroll.offset().y + this.scroll.max_offset().y
                                            <= px(8.0)
                                    {
                                        this.follow(cx);
                                    }
                                })
                                .ok();
                            });
                        },
                    ))
                    .child(
                        div()
                            .p(px(12.0))
                            .line_height(px(18.0))
                            .font_family(theme.font_mono.clone())
                            .child(if self.displayed.is_empty() {
                                l.text(TextKey::ShellOutputEmpty)
                            } else {
                                self.displayed.clone()
                            }),
                    ),
            )
    }
}

impl Shell {
    pub(super) fn open_process_output(&mut self, target: OutputTarget, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let existing = self
            .process_outputs
            .iter()
            .find(|(_, view)| view.read(cx).target == target)
            .map(|(id, _)| *id);
        let id = existing.unwrap_or_else(|| {
            let view = cx.new(|_| OutputView::new(target, engine));
            let id = view.entity_id().as_u64();
            cx.subscribe(&view, |this: &mut Self, _, event: &OpenLog, cx| {
                this.pending_process_log = Some((event.chat.clone(), event.path.clone()));
                cx.notify();
            })
            .detach();
            self.process_outputs.insert(id, view);
            id
        });
        self.set_surfaces_open(true, cx);
        let key = self.panel_key(cx);
        let surface = RightSurface::ProcessOutput(id);
        push_unique_right_surface(self.right_tabs.entry(key).or_default(), surface);
        self.set_right_active(surface, cx);
    }
    pub(super) fn sync_process_output_visibility(&mut self, cx: &mut Context<Self>) {
        let active = self.resolved_right_active(cx);
        let visible = self.right_pane_open(cx)
            && self.horizontal_fit().right
            && matches!(self.route, Route::Chat);
        let engine = self.state.read(cx).engine().cloned();
        for (id, view) in &self.process_outputs {
            view.update(cx, |view, cx| {
                if let Some(engine) = &engine
                    && !view.engine.same_connection(engine)
                {
                    view.engine = engine.clone();
                    view.poll = None;
                    view.polling = false;
                    view.visible = false;
                }
                view.set_visible(visible && active == RightSurface::ProcessOutput(*id), cx);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext;

    fn target() -> OutputTarget {
        OutputTarget {
            chat: "chat".into(),
            device: Some("host".into()),
            id: "process".into(),
            title: "npm run dev".into(),
        }
    }
    fn snapshot(output: &str, status: &str) -> Snapshot {
        Snapshot {
            status: status.into(),
            output: output.into(),
            output_path: "server.log".into(),
            rotated: false,
        }
    }

    struct OutputHost(Entity<OutputView>);
    impl Render for OutputHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(360.0)).h(px(400.0)).child(self.0.clone())
        }
    }
    #[gpui::test]
    fn rendered_actions_pause_resume_and_open_the_log_without_model_calls(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let (out, mut requests) = tokio::sync::mpsc::channel(64);
        let (_replies, inbound) = tokio::sync::mpsc::channel(64);
        let engine =
            crate::state::EngineHandle::from_test_client(rcode_rpc::RpcClient::new(out, inbound));
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            settings::init(UiSettings::default(), dir.path(), cx);
        });
        let (host, cx) = cx.add_window_view(|_, cx| {
            OutputHost(cx.new(|_| {
                let mut view = OutputView::new(target(), engine);
                view.apply(snapshot("initial output", "running"));
                view
            }))
        });
        let view = host.read_with(cx, |host, _| host.0.clone());
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        let pause = cx
            .debug_bounds("output-follow")
            .expect("follow control is visible");
        cx.simulate_click(pause.center(), gpui::Modifiers::default());
        assert!(!view.read_with(cx, |view, _| view.following));
        view.update(cx, |view, cx| {
            view.apply(snapshot("final output", "completed"));
            cx.notify();
        });
        assert_eq!(
            view.read_with(cx, |view, _| view.displayed.clone()),
            "initial output"
        );
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        let resume = cx.debug_bounds("output-follow").unwrap();
        cx.simulate_click(resume.center(), gpui::Modifiers::default());
        assert_eq!(
            view.read_with(cx, |view, _| view.displayed.clone()),
            "final output"
        );
        let opened = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = opened.clone();
        let _subscription = cx.update(|_, cx| {
            cx.subscribe(&view, move |_, event: &OpenLog, _| {
                *captured.borrow_mut() = Some((event.chat.clone(), event.path.clone()));
            })
        });
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        let open_file = cx.debug_bounds("output-file").unwrap();
        cx.simulate_click(open_file.center(), gpui::Modifiers::default());
        assert_eq!(*opened.borrow(), Some(("chat".into(), "server.log".into())));
        cx.run_until_parked();
        assert!(requests.try_recv().is_err());
    }

    #[gpui::test]
    fn paused_output_preserves_reading_position_and_final_output(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let (out, mut requests) = tokio::sync::mpsc::channel(64);
        let (_replies, inbound) = tokio::sync::mpsc::channel(64);
        let engine =
            crate::state::EngineHandle::from_test_client(rcode_rpc::RpcClient::new(out, inbound));
        let view = cx.new(|_| OutputView::new(target(), engine));
        view.update(cx, |view, cx| {
            view.apply(snapshot("first", "running"));
            view.following = false;
            view.apply(snapshot("final", "completed"));
            assert_eq!(view.displayed, "first");
            assert!(!view.latest.as_ref().unwrap().running());
            view.follow(cx);
            assert_eq!(view.displayed, "final");
            view.stop(cx); // Already finished: neither stop nor model request.
        });
        cx.run_until_parked();
        assert!(requests.try_recv().is_err());
    }

    #[gpui::test]
    fn hidden_and_closed_tabs_do_not_poll_or_stop_and_ignore_late_results(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let (out, mut requests) = tokio::sync::mpsc::channel(64);
        let (replies, inbound) = tokio::sync::mpsc::channel(64);
        let engine =
            crate::state::EngineHandle::from_test_client(rcode_rpc::RpcClient::new(out, inbound));
        let view = cx.new(|_| OutputView::new(target(), engine));
        cx.run_until_parked();
        assert!(requests.try_recv().is_err());
        view.update(cx, |view, cx| view.set_visible(true, cx));
        cx.run_until_parked();
        let request: serde_json::Value =
            serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        assert_eq!(request["method"], methods::READ_COMMAND_OUTPUT);
        assert_eq!(request["params"]["targetDeviceId"], "host");
        assert!(requests.try_recv().is_err(), "one request at a time");
        view.update(cx, |view, cx| view.set_visible(false, cx));
        runtime.block_on(async {
            replies.send(serde_json::json!({"id":request["id"],"ok":{"status":"running","output":"stale","outputPath":"file"}}).to_string()).await.unwrap();
            while replies.capacity() < replies.max_capacity() { tokio::task::yield_now().await; }
        });
        cx.run_until_parked();
        view.update(cx, |view, cx| {
            assert!(view.latest.is_none());
            assert!(!view.polling);
            view.set_visible(true, cx);
        });
        cx.run_until_parked();
        let request: serde_json::Value =
            serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        assert_eq!(request["method"], methods::READ_COMMAND_OUTPUT);
        view.update(cx, |view, cx| {
            view.set_visible(false, cx);
            view.set_visible(true, cx);
        });
        cx.run_until_parked();
        assert!(requests.try_recv().is_err(), "reopening waits for the existing slow query");
        runtime.block_on(async {
            replies.send(serde_json::json!({"id":request["id"],"ok":{"status":"running","output":"stale-after-reopen","outputPath":"file"}}).to_string()).await.unwrap();
            while replies.capacity() < replies.max_capacity() { tokio::task::yield_now().await; }
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| view.latest.is_none()));
        let next: serde_json::Value = serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        assert_eq!(next["method"], methods::READ_COMMAND_OUTPUT);
        drop(view);
        cx.run_until_parked();
        assert!(
            requests.try_recv().is_err(),
            "closing must not stop the process"
        );
    }
}
