use super::*;

impl ChatView {
    pub(super) fn decision_reply(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(row) = self.decision_rows.iter().find(|r| r["id"] == id).cloned() else {
            return;
        };
        if row["status"] != "pending" {
            return;
        }
        match super::super::decisions::read() {
            Ok(rows) => {
                let latest = rows.iter().find(|r| r["id"] == id);
                if latest.is_none_or(|r| {
                    ["status", "title", "context", "options"]
                        .iter()
                        .any(|k| r[*k] != row[*k])
                }) {
                    self.decision_rows = rows;
                    self.decision_choices.remove(id);
                    self.decision_error = "事项已更新，请重新阅读后确认；补充草稿已保留".into();
                    cx.notify();
                    return;
                }
            }
            Err(e) => {
                self.decision_error = e;
                cx.notify();
                return;
            }
        }
        let prefix = format!("[决策答复 {id}]\n");
        if let Some(message) = self
            .conversation
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "user" && m.text.starts_with(&prefix))
        {
            if row["request_id"].as_str() != Some(message.id.as_str()) {
                self.decision_error = "此事项的回答已入队，请勿重复提交；可在聊天记录核对。".into();
                cx.notify();
                return;
            }
        }
        let custom = self
            .decision_inputs
            .get(id)
            .map(|s| s.read(cx).value().trim().to_owned())
            .unwrap_or_default();
        let selected = self.decision_choices.get(id).cloned().unwrap_or_default();
        let text = match (selected.is_empty(), custom.is_empty()) {
            (false, false) => format!("{selected}\n补充：{custom}"),
            (false, true) => selected,
            _ => custom,
        };
        if text.is_empty() {
            self.decision_error = "请先选择一项或填写回答".into();
            cx.notify();
            return;
        }
        // Same durable outbox as the main composer; never edit its draft or approve a worker tool.
        let request = fresh_id();
        let message = format!(
            "[决策答复 {}]\n来源：{}\n问题：{}\n我的决定：{}\n请在原会话推进，并核实结果；此回复不代替工具的一次性审批。",
            id,
            row["address"].as_str().unwrap_or(""),
            row["title"].as_str().unwrap_or(""),
            text
        );
        let command = json!({"action":"send","request_id":request,"text":message,"images":[]});
        if !self.connection.as_ref().is_some_and(|c| c.send(command)) {
            self.decision_error = "未能加入发送队列，回答已保留，请连接后重试".into();
            cx.notify();
            return;
        }
        self.conversation
            .apply(&json!({"type":"user","id":request,"text":message}));
        self.sync_rows();
        match super::super::decisions::update(id, "answered", Some((&text, &request))) {
            Ok(_) => {
                self.decision_error.clear();
                self.decision_rows =
                    super::super::decisions::read().unwrap_or_else(|_| self.decision_rows.clone());
            }
            Err(e) => {
                self.decision_error = format!("答复已入队，但清单保存失败：{e}。请勿重复提交。")
            }
        }
        cx.notify();
    }
    pub(super) fn decision_state(&mut self, id: &str, status: &str, cx: &mut Context<Self>) {
        match super::super::decisions::update(id, status, None) {
            Ok(_) => {
                self.decision_rows = super::super::decisions::read().unwrap_or_default();
                self.decision_error.clear();
            }
            Err(e) => self.decision_error = e,
        }
        cx.notify();
    }
    pub(super) fn decision_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let width = self.pane_width.get();
        let narrow = width > 0. && width < 940.;
        let mut content = div()
            .id("decision-list")
            .debug_selector(|| "decision-list".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.decision_scroll)
            .p_3()
            .flex()
            .flex_col()
            .gap_3();
        let rows: Vec<_> = self
            .decision_rows
            .iter()
            .filter(|r| (r["status"] == "resolved") == self.decision_completed)
            .cloned()
            .collect();
        if rows.is_empty() {
            content = content.child(div().py_6().text_sm().text_color(color("dim")).child(
                if self.decision_completed {
                    "还没有已完成事项"
                } else {
                    "暂时没有待你决策的事项。\nSuper Agent 会把问题和待验收结果放在这里。"
                },
            ));
        }
        for row in rows {
            let id = row["id"].as_str().unwrap_or_default().to_owned();
            let status = row["status"].as_str().unwrap_or("pending").to_owned();
            let mut card = div()
                .id(SharedString::from(format!("decision-{id}")))
                .debug_selector(|| "decision-card".into())
                .flex_shrink_0()
                .p_3()
                .rounded_lg()
                .border_1()
                .border_color(color("border"))
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_xs().text_color(color("dim")).child(format!(
                    "{}  ·  {}",
                    row["address"].as_str().unwrap_or(""),
                    match status.as_str() {
                        "answered" => "已答复 · 待推进",
                        "resolved" => "已完成",
                        _ => "待你决策",
                    }
                )))
                .child(
                    div()
                        .text_sm()
                        .child(row["title"].as_str().unwrap_or("").to_owned()),
                )
                .child(
                    TextView::markdown(
                        SharedString::from(format!("decision-context-{id}")),
                        row["context"].as_str().unwrap_or("").to_owned(),
                    )
                    .selectable(true),
                );
            let source = row["address"].as_str().unwrap_or("").to_owned();
            card = card.child(
                div()
                    .id(SharedString::from(format!("decision-source-{id}")))
                    .cursor_pointer()
                    .text_xs()
                    .text_color(color("dim"))
                    .child("打开原会话 ↗")
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(ChatEvent::OpenDecisionSource {
                            address: source.clone(),
                        })
                    })),
            );
            if let Some(evidence) = row["evidence"].as_str().filter(|s| !s.is_empty()) {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(color("dim"))
                        .child(evidence.to_owned()),
                );
            }
            if status == "pending" {
                for (i, option) in row["options"].as_array().into_iter().flatten().enumerate() {
                    let text = option.as_str().unwrap_or("").to_owned();
                    let selected = self.decision_choices.get(&id) == Some(&text);
                    let key = id.clone();
                    card = card.child(
                        div()
                            .id(SharedString::from(format!("decision-option-{id}-{i}")))
                            .debug_selector(|| "decision-option".into())
                            .cursor_pointer()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .border_1()
                            .border_color(color(if selected { "text" } else { "border" }))
                            .bg(color("input").opacity(if selected { 1. } else { 0.35 }))
                            .text_sm()
                            .child(format!("{} {}", if selected { "●" } else { "○" }, text))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.decision_choices.insert(key.clone(), text.clone());
                                cx.notify();
                            })),
                    );
                }
                let field = self
                    .decision_inputs
                    .entry(id.clone())
                    .or_insert_with(|| {
                        cx.new(|cx| {
                            InputState::new(window, cx).placeholder("补充说明或填写其他决定…")
                        })
                    })
                    .clone();
                card = card.child(
                    div()
                        .p_2()
                        .rounded_md()
                        .bg(color("input"))
                        .child(Input::new(&field)),
                );
                let key = id.clone();
                card = card.child(
                    div()
                        .id(SharedString::from(format!("decision-send-{id}")))
                        .debug_selector(|| "decision-submit".into())
                        .cursor_pointer()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(color("text"))
                        .text_color(color("bg"))
                        .text_sm()
                        .child("提交给 Super Agent →")
                        .on_click(cx.listener(move |this, _, _, cx| this.decision_reply(&key, cx))),
                );
            } else if let Some(answer) = row["answer"].as_str() {
                card = card.child(
                    div()
                        .p_2()
                        .rounded_md()
                        .bg(color("input"))
                        .text_sm()
                        .child(format!("你的决定：{answer}")),
                );
            }
            let key = id.clone();
            card = card.child(
                div()
                    .id(SharedString::from(format!("decision-complete-{id}")))
                    .debug_selector(|| "decision-complete".into())
                    .cursor_pointer()
                    .text_xs()
                    .text_color(color("dim"))
                    .py_1()
                    .child(if status == "resolved" {
                        "↶ 重新打开"
                    } else {
                        "✓ 标为已处理"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.decision_state(
                            &key,
                            if status == "resolved" {
                                "pending"
                            } else {
                                "resolved"
                            },
                            cx,
                        )
                    })),
            );
            content = content.child(card);
        }
        div()
            .id("decision-sidebar")
            .debug_selector(|| "decision-sidebar".into())
            .h_full()
            .w(px(if narrow {
                (width - 20.).clamp(220., 340.)
            } else {
                320.
            }))
            .flex_shrink_0()
            .border_l_1()
            .border_color(color("border"))
            .bg(color("bg"))
            .flex()
            .flex_col()
            .when(narrow, |el| el.absolute().right_0().top_0().occlude())
            .child(
                div()
                    .p_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child("待你决策")
                    .child(
                        div()
                            .id("decision-close")
                            .cursor_pointer()
                            .child("×")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.decision_open = Some(false);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .px_3()
                    .pb_3()
                    .flex()
                    .gap_4()
                    .text_sm()
                    .child(
                        div()
                            .id("decision-pending-tab")
                            .cursor_pointer()
                            .text_color(color(if self.decision_completed {
                                "dim"
                            } else {
                                "text"
                            }))
                            .child("待处理")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.decision_completed = false;
                                this.decision_scroll.set_offset(point(px(0.), px(0.)));
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("decision-done-tab")
                            .debug_selector(|| "decision-done-tab".into())
                            .cursor_pointer()
                            .text_color(color(if self.decision_completed {
                                "text"
                            } else {
                                "dim"
                            }))
                            .child("已完成")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.decision_completed = true;
                                this.decision_scroll.set_offset(point(px(0.), px(0.)));
                                cx.notify();
                            })),
                    ),
            )
            .when(!self.decision_error.is_empty(), |el| {
                el.child(div().px_3().text_sm().child(self.decision_error.clone()))
            })
            .child(content)
            .child(
                div()
                    .p_3()
                    .border_t_1()
                    .border_color(color("border"))
                    .text_xs()
                    .text_color(color("dim"))
                    .child("答复进入管家队列；标记完成只整理清单。工具审批仍需单独确认。"),
            )
            .into_any_element()
    }
}
