use super::{
    ChatConfig, fresh_id,
    state::Conversation,
    transport::{self, Connection},
};
use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_base::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_base::text::TextView;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
fn color(name: &str) -> Hsla {
    crate::settings::color_or_literal(match name {
        "bg" => "surface",
        "text" => "foreground",
        other => other,
    })
}

pub struct ChatView {
    config: ChatConfig,
    host: Entity<InputState>,
    folder: Entity<InputState>,
    editor: Entity<TextareaState>,
    conversation: Conversation,
    list: ListState,
    connection: Option<Connection>,
    expanded: HashSet<String>,
    answers: HashMap<String, HashMap<String, String>>,
    history_ok: bool,
    free_answers: HashMap<(String, String), Entity<InputState>>,
    installing: bool,
    notice: String,
    _input: Subscription,
}
impl ChatView {
    pub fn new(mut config: ChatConfig, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dir = transport::local_dir(&config.id);
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(bytes) = std::fs::read(dir.join("connection.json")) {
            if let Ok(saved) = serde_json::from_slice::<ChatConfig>(&bytes) {
                if saved.id == config.id {
                    config = saved;
                }
            }
        }
        let style = gpui_base::input::InputEditorStyle {
            foreground: color("text"),
            muted_foreground: color("dim"),
            selection: color("accent").opacity(0.3),
            caret: color("text"),
            ..Default::default()
        };
        let host = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("SSH host, e.g. forge-vm")
                .default_value(config.host.clone())
        });
        let folder = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Absolute folder on remote machine")
                .default_value(config.folder.clone())
        });
        let draft = std::fs::read_to_string(dir.join("draft.txt")).unwrap_or_default();
        let editor = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 6)
                .submit_on_enter(true)
                .placeholder("Message Claude…")
                .default_value(draft)
        });
        host.update(cx, |s, _| s.set_editor_style(style.clone()));
        folder.update(cx, |s, _| s.set_editor_style(style.clone()));
        editor.update(cx, |s, _| s.set_editor_style(style));
        let input = cx.subscribe_in(&editor, window, |this, _, e: &InputEvent, window, cx| {
            if matches!(e, InputEvent::Change) {
                let text = this.editor.read(cx).value();
                let _ = std::fs::write(
                    transport::local_dir(&this.config.id).join("draft.txt"),
                    text.as_bytes(),
                );
            }
            if matches!(
                e,
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false
                }
            ) {
                this.send(window, cx);
            }
        });
        let mut conversation = Conversation::default();
        let history = transport::load_history(&dir.join("events.jsonl"));
        let cache_error = history.is_err();
        if let Ok(history) = history {
            for e in history {
                conversation.apply(&e);
            }
        }
        if let Ok(bytes) = std::fs::read(dir.join("outbox.json")) {
            if let Ok(pending) =
                serde_json::from_slice::<std::collections::BTreeMap<String, Value>>(&bytes)
            {
                for (id, command) in pending {
                    if command["action"] == "send" {
                        if let Some(text) = command["text"].as_str() {
                            conversation.put(id, "You", text.into(), false);
                        }
                    }
                }
            }
        }
        let list = ListState::new(
            conversation.messages.len(),
            ListAlignment::Bottom,
            px(1000.),
        );
        let mut this = Self {
            config,
            host,
            folder,
            editor,
            conversation,
            list,
            connection: None,
            expanded: HashSet::new(),
            answers: HashMap::new(),
            history_ok: !cache_error,
            free_answers: HashMap::new(),
            installing: false,
            notice: String::new(),
            _input: input,
        };
        if cache_error {
            this.notice="Saved history is damaged. Keep this chat for recovery and open a new chat; nothing was overwritten.".into();
        }
        if this.valid() && !cache_error {
            this.connect(cx);
        }
        this
    }
    fn valid(&self) -> bool {
        transport::valid_host(&self.config.host)
            && self.config.folder.starts_with('/')
            && self.config.id.bytes().all(|b| b.is_ascii_hexdigit())
    }
    fn save_config(&self) {
        let dir = transport::local_dir(&self.config.id);
        if let Ok(text) = serde_json::to_vec_pretty(&self.config) {
            let _ = std::fs::write(dir.join("connection.json"), text);
        }
    }
    fn connect(&mut self, cx: &mut Context<Self>) {
        if !self.history_ok {
            self.notice = "Saved history needs recovery; open a new chat to continue.".into();
            cx.notify();
            return;
        }
        if !self.valid() {
            self.notice = "Choose an SSH host and an absolute remote folder.".into();
            cx.notify();
            return;
        }
        self.save_config();
        self.connection = None;
        let (connection, events) = Connection::start(self.config.clone(), self.conversation.cursor);
        self.connection = Some(connection);
        cx.spawn(async move |view, cx| {
            while let Ok(first) = events.recv().await {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let mut batch = vec![first];
                while batch.len() < 256 {
                    match events.try_recv() {
                        Ok(e) => batch.push(e),
                        Err(_) => break,
                    }
                }
                if view
                    .update(cx, |this, cx| {
                        let old = this.conversation.messages.len();
                        let following = this.list.is_scrolled_to_end().unwrap_or(true);
                        let mut changed = false;
                        for e in batch {
                            if e["type"] == "prompt_confirmed" || e["type"] == "permission_resolved"
                            {
                                this.notice.clear();
                            }
                            changed |= this.conversation.apply(&e);
                        }
                        if changed {
                            let new = this.conversation.messages.len();
                            if new > old {
                                this.list.splice(old..old, new - old);
                            }
                            if new > 0 {
                                this.list
                                    .remeasure_items(0..this.conversation.messages.len());
                            }
                            if following {
                                this.list.scroll_to_end();
                            }
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }
    fn configure(&mut self, cx: &mut Context<Self>) {
        let host = self.host.read(cx).value().trim().to_owned();
        let folder = self.folder.read(cx).value().trim().to_owned();
        if self.conversation.cursor > 0
            && (host != self.config.host || folder != self.config.folder)
        {
            self.notice="Open a new chat to use a different machine or folder; this history stays with its original target.".into();
            cx.notify();
            return;
        }
        self.config.host = host;
        self.config.folder = folder;
        self.connect(cx);
    }
    fn command(&mut self, mut value: Value, cx: &mut Context<Self>) -> bool {
        value["request_id"] = json!(fresh_id());
        let id = value["request_id"].as_str().unwrap_or_default().to_owned();
        let text = value["text"].as_str().map(str::to_owned);
        let sent = self.connection.as_ref().is_some_and(|c| c.send(value));
        if sent {
            if let Some(text) = text {
                let old = self.conversation.messages.len();
                self.conversation.put(id, "You", text, false);
                self.list.splice(old..old, 1);
            }
        }
        if !sent {
            self.notice = "Connect to the remote machine first. Your draft is kept.".into();
            cx.notify();
        }
        sent
    }
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        if self.command(json!({"action":"send","text":text}), cx) {
            self.editor
                .update(cx, |input, cx| input.set_value("", window, cx));
            let _ = std::fs::write(transport::local_dir(&self.config.id).join("draft.txt"), "");
            self.list.scroll_to_end();
            self.notice = "Sending…".into();
            cx.notify();
        }
    }
    pub fn set_draft(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        cx.notify();
    }
    pub fn status(&self, cx: &App) -> Value {
        json!({"host":self.config.host,"folder":self.config.folder,"conversation":self.config.id,"cursor":self.conversation.cursor,"messages":self.conversation.messages.len(),"pending_permissions":self.conversation.permissions.len(),"status":self.conversation.status,"draft_bytes":self.editor.read(cx).value().len(),"scroll_item":self.list.logical_scroll_top().item_ix,"scroll_offset":f32::from(self.list.logical_scroll_top().offset_in_item),"following":self.list.is_scrolled_to_end()})
    }
    fn install_bridge(&mut self, cx: &mut Context<Self>) {
        let host = self.host.read(cx).value().trim().to_owned();
        if !transport::valid_host(&host) {
            self.notice = "Choose a valid SSH host first.".into();
            cx.notify();
            return;
        }
        if self.installing {
            return;
        }
        self.installing = true;
        self.notice = "Installing the execution service…".into();
        cx.spawn(async move |view, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { transport::install(&host) })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.installing = false;
                this.notice = match result {
                    Ok(()) => "Execution service installed. Connect to start chatting.".into(),
                    Err(e) => e,
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn permission(&mut self, id: &str, decision: &str, cx: &mut Context<Self>) {
        let mut answers = self.answers.get(id).cloned().unwrap_or_default();
        for ((pid, q), field) in &self.free_answers {
            if pid == id {
                let v = field.read(cx).value();
                if !v.trim().is_empty() {
                    answers.insert(q.clone(), v.to_string());
                }
            }
        }
        if decision == "allow" {
            if let Some(p) = self.conversation.permissions.get(id) {
                if p["name"] == "AskUserQuestion"
                    && p["input"]["questions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|q| {
                            answers
                                .get(q["question"].as_str().unwrap_or_default())
                                .is_none_or(|a| a.trim().is_empty())
                        })
                {
                    self.notice = "Choose or type an answer to every question first.".into();
                    cx.notify();
                    return;
                }
            }
        }
        if self.command(
            json!({"action":"permission","id":id,"decision":decision,"answers":answers}),
            cx,
        ) {
            self.notice = "Decision sent; waiting for the remote acknowledgement.".into();
        }
    }
    fn message(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(m) = self.conversation.messages.get(index) else {
            return div().into_any_element();
        };
        let id = m.id.clone();
        let tool = !matches!(m.role.as_str(), "You" | "Claude" | "Notice");
        let expanded = self.expanded.contains(&id);
        let text = if tool && !expanded {
            m.text.chars().take(220).collect::<String>()
        } else {
            m.text.clone()
        };
        let copy = m.text.clone();
        let row_id = id.clone();
        let mut header = div()
            .flex()
            .justify_between()
            .items_center()
            .mb_2()
            .text_sm()
            .text_color(color("dim"))
            .child(m.role.clone());
        header = header.child(
            div()
                .flex()
                .gap_3()
                .when(tool, |el| {
                    el.child(
                        div()
                            .id(("expand", index))
                            .cursor_pointer()
                            .child(if expanded { "Collapse" } else { "Expand" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.expanded.remove(&id) {
                                    this.expanded.insert(id.clone());
                                }
                                this.list
                                    .remeasure_items(0..this.conversation.messages.len());
                                cx.notify();
                            })),
                    )
                })
                .child(
                    div()
                        .id(("copy", index))
                        .cursor_pointer()
                        .child("Copy")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                        }),
                ),
        );
        div()
            .id(("message", index))
            .w_full()
            .px_6()
            .py_4()
            .border_b_1()
            .border_color(color("border"))
            .child(header)
            .child(
                TextView::markdown(SharedString::from(format!("chat-text-{row_id}")), text)
                    .selectable(true),
            )
            .into_any_element()
    }
}
impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let approvals = self.conversation.permissions.clone();
        let mut controls = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(color("border"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(format!("Claude · {} · Interactive", self.config.model))
                    .child(if self.conversation.status == "starting" {
                        "Connected · ready to start".into()
                    } else {
                        self.conversation.status.clone()
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .p_2()
                            .rounded_md()
                            .bg(color("input"))
                            .child(Input::new(&self.host)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .p_2()
                            .rounded_md()
                            .bg(color("input"))
                            .child(Input::new(&self.folder)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .text_sm()
                    .child(
                        div()
                            .id("chat-connect")
                            .cursor_pointer()
                            .child("Connect")
                            .on_click(cx.listener(|this, _, _, cx| this.configure(cx))),
                    )
                    .child(
                        div()
                            .id("chat-install")
                            .cursor_pointer()
                            .child(if self.installing {
                                "Installing…"
                            } else {
                                "Install service"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.install_bridge(cx))),
                    )
                    .child(
                        div()
                            .id("chat-interrupt")
                            .cursor_pointer()
                            .text_color(color("blocked"))
                            .child("Stop")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.command(json!({"action":"interrupt"}), cx);
                            })),
                    )
                    .child(
                        div()
                            .id("chat-bottom")
                            .cursor_pointer()
                            .child("Latest ↓")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.list.scroll_to_end();
                                cx.notify();
                            })),
                    ),
            );
        if !self.notice.is_empty() {
            controls = controls.child(
                div()
                    .text_xs()
                    .text_color(color("dim"))
                    .child(self.notice.clone()),
            );
        }
        let mut approvals_view = div()
            .id("chat-approvals")
            .max_h(px(
                (f32::from(window.viewport_size().height) - 430.).clamp(56., 240.)
            ))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2();
        let mut footer = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_t_1()
            .border_color(color("border"));
        for (id, permission) in approvals {
            let deny = id.clone();
            let allow = id.clone();
            let input = &permission["input"];
            let mut card = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(color("blocked"))
                .child(format!(
                    "Approval required · {}",
                    permission["name"].as_str().unwrap_or("Tool")
                ));
            if permission["name"] == "AskUserQuestion" {
                for q in input["questions"].as_array().into_iter().flatten() {
                    let question = q["question"].as_str().unwrap_or_default().to_owned();
                    card = card.child(question.clone());
                    let multi = q["multiSelect"] == true;
                    let field = self
                        .free_answers
                        .entry((id.clone(), question.clone()))
                        .or_insert_with(|| {
                            cx.new(|cx| {
                                InputState::new(window, cx).placeholder("Or type your answer…")
                            })
                        })
                        .clone();
                    card = card.child(div().p_2().bg(color("input")).child(Input::new(&field)));
                    for option in q["options"].as_array().into_iter().flatten() {
                        let label = option["label"].as_str().unwrap_or_default().to_owned();
                        let pid = id.clone();
                        let q = question.clone();
                        let value = label.clone();
                        let selected = self
                            .answers
                            .get(&id)
                            .and_then(|a| a.get(&question))
                            .is_some_and(|v| v.split(", ").any(|x| x == label));
                        card = card.child(
                            div()
                                .id(SharedString::from(format!(
                                    "answer-{id}-{question}-{label}"
                                )))
                                .cursor_pointer()
                                .text_color(color(if selected { "accent" } else { "text" }))
                                .child(format!("{} {label}", if selected { "●" } else { "○" }))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let answer = this
                                        .answers
                                        .entry(pid.clone())
                                        .or_default()
                                        .entry(q.clone())
                                        .or_default();
                                    if multi {
                                        let mut values: Vec<String> = answer
                                            .split(", ")
                                            .filter(|s| !s.is_empty())
                                            .map(str::to_owned)
                                            .collect();
                                        if values.contains(&value) {
                                            values.retain(|v| v != &value);
                                        } else {
                                            values.push(value.clone());
                                        }
                                        *answer = values.join(", ");
                                    } else {
                                        *answer = value.clone();
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                }
            } else {
                let text = serde_json::to_string_pretty(input).unwrap_or_default();
                card = card.child(TextView::markdown(
                    SharedString::from(format!("permission-{id}")),
                    format!(
                        "```json\n{}\n```",
                        text.chars().take(3000).collect::<String>()
                    ),
                ));
            }
            approvals_view = approvals_view.child(
                card.child(
                    div()
                        .flex()
                        .gap_4()
                        .child(
                            div()
                                .id(SharedString::from(format!("deny-{id}")))
                                .cursor_pointer()
                                .child("Deny")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.permission(&deny, "deny", cx)
                                })),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("allow-{id}")))
                                .cursor_pointer()
                                .text_color(color("accent"))
                                .child("Allow / Submit answers")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.permission(&allow, "allow", cx)
                                })),
                        ),
                ),
            );
        }
        footer = footer
            .when(!self.conversation.permissions.is_empty(), |el| {
                el.child(approvals_view)
            })
            .child(
                div()
                    .p_3()
                    .rounded_md()
                    .bg(color("input"))
                    .min_h(px(70.))
                    .max_h(px(180.))
                    .child(Textarea::new(&self.editor)),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_sm()
                    .child("Enter sends · Shift+Enter newline · history stays local")
                    .child(
                        div()
                            .id("chat-send")
                            .cursor_pointer()
                            .text_color(color("accent"))
                            .child("Send ↑")
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            );
        div()
            .size_full()
            .flex()
            .flex_col()
            .capture_action(
                cx.listener(|this, _: &gpui_base::input::MovePageUp, _, cx| {
                    this.list.scroll_by(px(-480.));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .capture_action(
                cx.listener(|this, _: &gpui_base::input::MovePageDown, _, cx| {
                    this.list.scroll_by(px(480.));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .capture_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                match e.keystroke.key.as_str() {
                    "pageup" => this.list.scroll_by(px(-480.)),
                    "pagedown" => this.list.scroll_by(px(480.)),
                    "home" if e.keystroke.modifiers.control => {
                        this.list.scroll_to(ListOffset::default())
                    }
                    "end" if e.keystroke.modifiers.control => this.list.scroll_to_end(),
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .bg(color("bg"))
            .text_color(color("text"))
            .overflow_hidden()
            .child(controls)
            .child(
                div().flex_1().min_h_0().overflow_hidden().child(
                    list(self.list.clone(), move |i, _, cx| {
                        view.update(cx, |this, cx| this.message(i, cx))
                    })
                    .size_full(),
                ),
            )
            .child(footer)
    }
}
