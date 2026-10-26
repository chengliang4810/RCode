use super::*;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ProcessRow {
    id: String,
    command: String,
    pid: u32,
    #[serde(default)]
    recovered: bool,
}

pub(super) struct CommandProcesses {
    state: Entity<AppState>,
    target: Option<(String, Option<String>)>,
    engine: Option<crate::state::EngineHandle>,
    rows: Vec<ProcessRow>,
    stopping: HashSet<String>,
    stops: HashMap<String, Task<()>>,
    poll: Option<Task<()>>,
    _selection: Subscription,
}
impl gpui::EventEmitter<super::process_output::OutputTarget> for CommandProcesses {}
impl CommandProcesses {
    pub(super) fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let observation = cx.observe(&state, |this: &mut Self, _, cx| this.select(cx));
        let mut this = Self {
            state,
            target: None,
            engine: None,
            rows: vec![],
            stopping: HashSet::new(),
            stops: HashMap::new(),
            poll: None,
            _selection: observation,
        };
        this.select(cx);
        this
    }
    fn select(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let target = state
            .selected_chat
            .as_ref()
            .map(|chat| (chat.clone(), state.terminal_target_device(chat)));
        let engine = state.engine().cloned();
        let same_engine = self
            .engine
            .as_ref()
            .zip(engine.as_ref())
            .is_some_and(|(old, new)| old.same_connection(new));
        if self.target == target && same_engine && self.poll.is_some() {
            return;
        }
        self.target = target.clone();
        self.engine = engine.clone();
        self.rows.clear();
        self.poll = None;
        let Some((chat, device)) = target else {
            cx.notify();
            return;
        };
        let Some(engine) = engine else {
            cx.notify();
            return;
        };
        let params = serde_json::json!({"chatId":chat,"targetDeviceId":device});
        self.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                let result = engine
                    .client()
                    .call(methods::LIST_COMMAND_PROCESSES, params.clone())
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|value| {
                        serde_json::from_value::<Vec<ProcessRow>>(value)
                            .map_err(|error| error.to_string())
                    });
                let failed = result.is_err();
                if this
                    .update(cx, |this, cx| {
                        if let Ok(rows) = result {
                            this.stopping
                                .retain(|id| rows.iter().any(|row| &row.id == id));
                            if this.rows != rows {
                                this.rows = rows;
                                cx.notify();
                            }
                        }
                    })
                    .is_err()
                {
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_secs(if failed { 5 } else { 1 }))
                    .await;
            }
        }));
        cx.notify();
    }
    fn open(&mut self, id: String, title: String, cx: &mut Context<Self>) {
        if let Some((chat, device)) = &self.target {
            cx.emit(super::process_output::OutputTarget {
                chat: chat.clone(),
                device: device.clone(),
                id,
                title,
            });
        }
    }
    fn stop(&mut self, id: String, cx: &mut Context<Self>) {
        if self.stopping.contains(&id) {
            return;
        }
        let Some((chat, device)) = self.target.clone() else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.stopping.insert(id.clone());
        let key = id.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(
                    methods::STOP_COMMAND_PROCESS,
                    serde_json::json!({"chatId":chat,"targetDeviceId":device,"id":id}),
                )
                .await;
            this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.stopping.remove(&id);
                    crate::toast::error(cx, error.to_string());
                }
                // Poll the OS to confirm termination before removing the row.
                this.stops.remove(&id);
                cx.notify();
            })
            .ok();
        });
        self.stops.insert(key, task);
        cx.notify();
    }
}
impl Render for CommandProcesses {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.rows.is_empty() {
            return div().into_any_element();
        }
        let theme = Theme::of(cx).for_popup();
        let l = crate::l10n::localizer();
        let rows = self
            .rows
            .iter()
            .map(|row| {
                let id = row.id.clone();
                let open_id = id.clone();
                let open_title = row.command.clone();
                let key_open_id = id.clone();
                let key_open_title = row.command.clone();
                let a11y_open_id = id.clone();
                let a11y_open_title = row.command.clone();
                let open_view = cx.entity().downgrade();
                let key_id = id.clone();
                let a11y_id = id.clone();
                let view = cx.entity().downgrade();
                let stopping = self.stopping.contains(&id);
                let label = l.text(if stopping {
                    TextKey::ShellProcessesStopping
                } else {
                    TextKey::ShellSubagentStop
                });
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .py(px(3.0))
                    .child(
                        div()
                            .id(SharedString::from(format!("process-open-{id}")))
                            .cursor_pointer()
                            .role(gpui::Role::Button)
                            .tab_index(0)
                            .aria_label(l.format_with(
                                TextKey::ShellOutputOpen,
                                &[("command", row.command.as_str())],
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open(open_id.clone(), open_title.clone(), cx)
                            }))
                            .on_key_down(cx.listener(
                                move |this, event: &gpui::KeyDownEvent, _, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        this.open(key_open_id.clone(), key_open_title.clone(), cx);
                                    }
                                },
                            ))
                            .on_a11y_action(gpui::AccessibleAction::Click, move |_, _, cx| {
                                open_view
                                    .update(cx, |this, cx| {
                                        this.open(a11y_open_id.clone(), a11y_open_title.clone(), cx)
                                    })
                                    .ok();
                            })
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .id(SharedString::from(format!("process-label-{id}")))
                                    .text_ellipsis()
                                    .child(row.command.clone())
                                    .tooltip(crate::settings::widgets::text_tooltip(
                                        row.command.clone(),
                                    )),
                            )
                            .child(div().text_color(theme.text_muted).child(format!(
                                "PID {}{}",
                                row.pid,
                                if row.recovered {
                                    format!(" · {}", l.text(TextKey::ShellProcessesRecovered))
                                } else {
                                    String::new()
                                }
                            ))),
                    )
                    .child(
                        crate::settings::widgets::text_action(
                            &theme,
                            crate::settings::widgets::ActionTone::Quiet,
                            label.clone(),
                        )
                        .id(SharedString::from(format!("process-stop-{id}")))
                        .role(gpui::Role::Button)
                        .tab_index(0)
                        .aria_label(l.format_with(
                            TextKey::ShellProcessesStopAria,
                            &[("command", row.command.as_str())],
                        ))
                        .when(stopping, |el| el.opacity(0.45))
                        .on_click(cx.listener(move |this, _, _, cx| this.stop(id.clone(), cx)))
                        .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                cx.stop_propagation();
                                this.stop(key_id.clone(), cx);
                            }
                        }))
                        .on_a11y_action(
                            gpui::AccessibleAction::Click,
                            move |_, _, cx| {
                                view.update(cx, |this, cx| this.stop(a11y_id.clone(), cx))
                                    .ok();
                            },
                        ),
                    )
            })
            .collect::<Vec<_>>();
        div()
            .flex_none()
            .mt(px(4.0))
            .border_t_1()
            .border_color(theme.border)
            .px(px(9.0))
            .py(px(6.0))
            .text_color(theme.text)
            .text_size(crate::typography::ui_rems(12.0))
            .child(
                div()
                    .pb(px(4.0))
                    .text_color(theme.text_muted)
                    .child(l.text(TextKey::ShellProcessesTitle)),
            )
            .child(
                div()
                    .id("command-processes")
                    .max_h(px(200.0))
                    .overflow_y_scroll()
                    .children(rows),
            )
            .into_any_element()
    }
}
