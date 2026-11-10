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
                    [
                        "status",
                        "title",
                        "context",
                        "options",
                        "reason",
                        "decision_key",
                    ]
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
            .filter(|r| {
                !self.decision_memory
                    && !self.decision_goals
                    && super::super::decisions::needs_attention(r) != self.decision_completed
            })
            .cloned()
            .collect();
        if rows.is_empty() && !self.decision_goals && !self.decision_memory {
            content = content.child(div().py_6().text_sm().text_color(color("dim")).child(
                if self.decision_completed {
                    "还没有已归档事项"
                } else {
                    "暂时没有待你决策的事项。\nSuper Agent 会把问题和待验收结果放在这里。"
                },
            ));
        }
        if self.decision_memory {
            content = content.child(self.memory_panel(window, cx));
        }
        if self.decision_goals && !self.decision_memory {
            if self.goal_rows.is_empty() {
                content = content.child(
                    div()
                        .text_sm()
                        .text_color(color("dim"))
                        .child("管家会将工作整理成目标；普通结果不需要逐条验收。"),
                );
            }
            for goal in
                self.goal_rows
                    .clone()
                    .into_iter()
                    .take(if self.expanded.contains("other-goals") {
                        usize::MAX
                    } else {
                        3
                    })
            {
                let id = goal["id"].as_str().unwrap_or("").to_owned();
                let key = format!("goal:{id}");
                let open = self.expanded.contains(&key);
                let phase = if goal["phase"] == "observed_complete" {
                    "目标结果已核实"
                } else if goal["needs_user"] == true {
                    "需要你决定"
                } else if goal["enabled"] == false {
                    if goal["mandate"]["scope"].is_string() {
                        "已暂停接续"
                    } else {
                        "按需跟进"
                    }
                } else if goal["phase"] == "blocked" {
                    "正在处理阻塞"
                } else {
                    "持续推进中"
                };
                let mut card = div()
                    .id(SharedString::from(format!("goal-{id}")))
                    .debug_selector(|| "goal-card".into())
                    .flex_shrink_0()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(color("border"))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_xs().text_color(color("dim")).child(phase))
                    .child(
                        div()
                            .text_sm()
                            .child(goal["title"].as_str().unwrap_or("").to_owned()),
                    )
                    .child(
                        TextView::markdown(
                            SharedString::from(format!("goal-summary-{id}")),
                            goal["summary"].as_str().unwrap_or("").to_owned(),
                        )
                        .text_sm(),
                    )
                    .child(div().text_sm().text_color(color("dim")).child(format!(
                        "接下来：{}",
                        goal["next_action"].as_str().unwrap_or("核对原会话最新证据")
                    )))
                    .child(
                        div()
                            .id(SharedString::from(format!("goal-details-{id}")))
                            .debug_selector(|| "goal-details".into())
                            .cursor_pointer()
                            .text_xs()
                            .text_color(color("dim"))
                            .child(if open {
                                "▾ 收起授权与目标"
                            } else {
                                "▸ 授权与目标"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.expanded.remove(&key) {
                                    this.expanded.insert(key.clone());
                                }
                                cx.notify();
                            })),
                    );
                if open {
                    card = card.child(
                        TextView::markdown(
                            SharedString::from(format!("goal-scope-{id}")),
                            format!(
                                "目标：{}\n\n授权：{}\n\n限制：{}",
                                goal["outcome"].as_str().unwrap_or(""),
                                goal["mandate"]["scope"]
                                    .as_str()
                                    .unwrap_or("尚未记录执行授权"),
                                goal["mandate"]["exclusions"]
                                    .as_str()
                                    .unwrap_or("不扩大授权，不批准工具")
                            ),
                        )
                        .text_sm(),
                    );
                }
                let deleting = self.expanded.contains(&format!("delete-goal:{id}"));
                if deleting {
                    card = card.child(
                        TextView::markdown(
                            SharedString::from(format!("goal-delete-hint-{id}")),
                            "停止目标跟进；保留会话、历史和已派发任务。".to_owned(),
                        )
                        .text_sm(),
                    );
                }
                let delete_id = id.clone();
                let revision = goal["revision"].as_u64().unwrap_or(0);
                card = card.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            div()
                                .id(SharedString::from(format!("delete-goal-{id}")))
                                .debug_selector(|| "goal-delete".into())
                                .cursor_pointer()
                                .text_xs()
                                .text_color(color("dim"))
                                .child(if deleting {
                                    "确认删除"
                                } else {
                                    "删除目标"
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let key = format!("delete-goal:{delete_id}");
                                    if !this.expanded.remove(&key) {
                                        this.expanded.insert(key);
                                        cx.notify();
                                        return;
                                    }
                                    let id = delete_id.clone();
                                    let home = crate::mods::app_dir();
                                    cx.spawn(async move |view, cx| {
                                        let result = cx
                                            .background_executor()
                                            .spawn(async move {
                                                transport::delete_goal(&home, &id, revision)
                                            })
                                            .await;
                                        let _ = view.update(cx, |this, cx| {
                                            match result {
                                                Ok(rows) => {
                                                    this.goal_rows = rows;
                                                    this.decision_error.clear();
                                                }
                                                Err(e) => {
                                                    this.decision_error = e;
                                                }
                                            }
                                            cx.notify();
                                        });
                                    })
                                    .detach();
                                    cx.notify();
                                })),
                        )
                        .when(deleting, |row| {
                            row.child(
                                div()
                                    .id(SharedString::from(format!("cancel-delete-goal-{id}")))
                                    .debug_selector(|| "goal-delete-cancel".into())
                                    .cursor_pointer()
                                    .text_xs()
                                    .child("取消")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.expanded.remove(&format!("delete-goal:{id}"));
                                        cx.notify();
                                    })),
                            )
                        }),
                );
                content = content.child(card);
            }
        }
        if self.decision_goals && self.goal_rows.len() > 3 {
            content = content.child(
                div()
                    .id("goal-more")
                    .debug_selector(|| "goal-more".into())
                    .cursor_pointer()
                    .text_sm()
                    .text_color(color("dim"))
                    .py_2()
                    .child(if self.expanded.contains("other-goals") {
                        "收起其余目标".to_owned()
                    } else {
                        format!("其余目标（{}）", self.goal_rows.len() - 3)
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        if !this.expanded.remove("other-goals") {
                            this.expanded.insert("other-goals".into());
                        }
                        cx.notify();
                    })),
            );
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
                        "superseded" => "已合并 · 原记录保留",
                        "withdrawn" => "管家已撤回 · 非任务完成",
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
            for (field, label) in [("reason", "为何需要你"), ("curation_reason", "整理说明")]
            {
                if let Some(reason) = row[field].as_str().filter(|s| !s.is_empty()) {
                    card = card.child(
                        div()
                            .text_xs()
                            .text_color(color("dim"))
                            .child(format!("{label}：{reason}")),
                    );
                }
            }
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
            card = card.when(status != "superseded", |card| {
                card.child(
                    div()
                        .id(SharedString::from(format!("decision-complete-{id}")))
                        .debug_selector(|| "decision-complete".into())
                        .cursor_pointer()
                        .text_xs()
                        .text_color(color("dim"))
                        .py_1()
                        .child(if !super::super::decisions::needs_attention(&row) {
                            "↶ 重新打开"
                        } else {
                            "✓ 标为已处理"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.decision_state(
                                &key,
                                if status == "resolved" || status == "withdrawn" {
                                    "pending"
                                } else {
                                    "resolved"
                                },
                                cx,
                            )
                        })),
                )
            });
            content = content.child(card);
        }
        div()
            .id("decision-sidebar")
            .debug_selector(|| "decision-sidebar".into())
            .h_full()
            .w(px(if narrow {
                (width - 20.).clamp(220., if self.decision_memory { 420. } else { 340. })
            } else if self.decision_memory {
                400.
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
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(self.decision_memory, |el| {
                                el.child(
                                    div()
                                        .id("memory-back")
                                        .debug_selector(|| "memory-back".into())
                                        .cursor_pointer()
                                        .text_color(color("dim"))
                                        .child("←")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.decision_memory = false;
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(if self.decision_memory {
                                "协作记忆"
                            } else {
                                "目标与决策"
                            }),
                    )
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
            .when(!self.decision_memory, |el| {
                el.child(
                    div()
                        .px_3()
                        .pb_3()
                        .flex()
                        .gap_4()
                        .text_sm()
                        .child(
                            div()
                                .id("goal-tab")
                                .debug_selector(|| "goal-tab".into())
                                .cursor_pointer()
                                .text_color(color(
                                    if self.decision_goals && !self.decision_memory {
                                        "text"
                                    } else {
                                        "dim"
                                    },
                                ))
                                .child("目标")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.decision_memory = false;
                                    this.decision_goals = true;
                                    this.decision_scroll.set_offset(point(px(0.), px(0.)));
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .id("decision-pending-tab")
                                .debug_selector(|| "decision-pending-tab".into())
                                .cursor_pointer()
                                .text_color(color(
                                    if self.decision_completed
                                        || self.decision_goals
                                        || self.decision_memory
                                    {
                                        "dim"
                                    } else {
                                        "text"
                                    },
                                ))
                                .child("待处理")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.decision_memory = false;
                                    this.decision_goals = false;
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
                                .text_color(color(
                                    if self.decision_completed
                                        && !self.decision_goals
                                        && !self.decision_memory
                                    {
                                        "text"
                                    } else {
                                        "dim"
                                    },
                                ))
                                .child("已归档")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.decision_memory = false;
                                    this.decision_goals = false;
                                    this.decision_completed = true;
                                    this.decision_scroll.set_offset(point(px(0.), px(0.)));
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when(!self.decision_memory, |el| {
                el.child(
                    div().px_3().pb_2().child(
                        div()
                            .id("memory-tab")
                            .debug_selector(|| "memory-tab".into())
                            .cursor_pointer()
                            .text_sm()
                            .text_color(color(if self.decision_memory { "text" } else { "dim" }))
                            .child(format!("协作记忆 · {}", self.memory_rows.len()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.decision_memory = true;
                                this.decision_scroll.set_offset(point(px(0.), px(0.)));
                                cx.notify();
                            })),
                    ),
                )
            })
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
                    .child(if self.decision_memory {
                        "手动修改直接保存；不改变聊天草稿或操作授权。"
                    } else {
                        "答复进入管家队列；标记完成只整理清单。工具审批仍需单独确认。"
                    }),
            )
            .into_any_element()
    }
}

impl ChatView {
    pub fn open_memory_view(
        &mut self,
        mode: &str,
        id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.config.id != super::super::SUPER_ID {
            return Err("请先打开 Super Agent".into());
        }
        if !["summary", "records", "new", "edit"].contains(&mode) {
            return Err("记忆视图应为 summary、records、new 或 edit".into());
        }
        if self.memory_editor.is_some() || self.memory_saving {
            return Err("请先保存或取消当前记忆编辑；草稿已保留".into());
        }
        if mode == "edit" && !id.is_some_and(|id| self.memory_rows.iter().any(|r| r["id"] == id)) {
            return Err("未找到这条记忆".into());
        }
        self.decision_open = Some(true);
        self.decision_memory = true;
        self.memory_records = mode == "records";
        self.decision_scroll.set_offset(point(px(0.), px(0.)));
        if mode == "new" {
            self.start_memory_edit(None, window, cx);
        }
        if mode == "edit" {
            self.start_memory_edit(id, window, cx);
        }
        cx.notify();
        Ok(())
    }
    fn start_memory_edit(&mut self, id: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        if self.memory_saving {
            return;
        }
        let row = id
            .and_then(|id| self.memory_rows.iter().find(|r| r["id"] == id))
            .cloned()
            .unwrap_or(json!({}));
        self.memory_editor = Some(MemoryEditor {
            id: id
                .map(str::to_owned)
                .unwrap_or_else(|| format!("personal.{}", fresh_id())),
            revision: row["revision"].as_u64().unwrap_or(0),
            title: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("标题，例如：汇报方式")
                    .default_value(row["title"].as_str().unwrap_or(""))
            }),
            content: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(3, 8)
                    .submit_on_enter(false)
                    .placeholder("希望管家记住的协作方式…")
                    .default_value(row["content"].as_str().unwrap_or(""))
            }),
        });
        let style = gpui_base::input::InputEditorStyle {
            foreground: color("text"),
            muted_foreground: color("dim"),
            background: color("bg"),
            border: color("border"),
            selection: color("accent").opacity(0.3),
            caret: color("text"),
            ..Default::default()
        };
        if let Some(edit) = &self.memory_editor {
            edit.title
                .update(cx, |s, _| s.set_editor_style(style.clone()));
            edit.content.update(cx, |s, _| s.set_editor_style(style));
            edit.title.read(cx).focus_handle(cx).focus(window, cx);
        }
        self.memory_records = true;
        self.decision_scroll.set_offset(point(px(0.), px(0.)));
        self.memory_delete = None;
        self.decision_error.clear();
        cx.notify();
    }
    pub(super) fn apply_memory_edit(
        &mut self,
        deleting: Option<(String, u64)>,
        cx: &mut Context<Self>,
    ) {
        if self.memory_saving {
            return;
        }
        let args = if let Some((id, revision)) = deleting {
            json!({"action":"delete","id":id,"expected_revision":revision})
        } else {
            let Some(edit) = &self.memory_editor else {
                return;
            };
            let title = edit.title.read(cx).value().trim().to_owned();
            let content = edit.content.read(cx).value().trim().to_owned();
            if title.is_empty() || content.is_empty() {
                self.decision_error = "请填写标题和内容".into();
                cx.notify();
                return;
            }
            json!({"action":"save","id":edit.id,"expected_revision":edit.revision,"title":title,"content":content})
        };
        self.memory_saving = true;
        self.decision_error.clear();
        let home = crate::mods::app_dir();
        cx.spawn(async move |view, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { transport::edit_memory(&home, &args) })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.memory_saving = false;
                match result {
                    Ok(rows) => {
                        this.memory_rows = rows;
                        this.memory_snapshot = Self::load_memory_snapshot();
                        this.memory_editor = None;
                        this.memory_delete = None;
                        this.decision_error.clear();
                    }
                    Err(e) => {
                        this.memory_rows = Self::load_memories();
                        this.memory_snapshot = Self::load_memory_snapshot();
                        this.decision_error = e;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn memory_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // Children in this scroll column must keep their natural height. Inputs fill
        // their viewport, so give each an explicit bounded viewport as well.
        let mut panel = div()
            .id("memory-workspace")
            .debug_selector(|| "memory-workspace".into())
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_4()
            .text_sm();
        if let Some(edit) = &self.memory_editor {
            let heading = if edit.revision == 0 {
                "新增记忆"
            } else {
                "编辑记忆"
            };
            let mut form = div()
                .id("memory-editor")
                .debug_selector(|| "memory-editor".into())
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .gap_3()
                .child(div().flex_shrink_0().text_base().child(heading))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(color("dim"))
                        .child("保存后会更新当前摘要；聊天草稿不受影响。"),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().text_xs().text_color(color("dim")).child("标题"))
                        .child(
                            div()
                                .id("memory-title-field")
                                .debug_selector(|| "memory-title-field".into())
                                .h(px(40.))
                                .flex_shrink_0()
                                .px_2()
                                .py_2()
                                .rounded_md()
                                .border_1()
                                .border_color(color("border"))
                                .overflow_hidden()
                                .child(Input::new(&edit.title)),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().text_xs().text_color(color("dim")).child("记忆内容"))
                        .child(
                            div()
                                .id("memory-content-field")
                                .debug_selector(|| "memory-content-field".into())
                                .w_full()
                                .min_w_0()
                                .flex_shrink_0()
                                .p_2()
                                .rounded_md()
                                .border_1()
                                .border_color(color("border"))
                                .flex()
                                .flex_col()
                                .min_h(px(110.))
                                .child(Textarea::new(&edit.content)),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(color("dim"))
                        .child("标题最多 120 字，内容最多 1600 字。"),
                );
            let mut actions = div()
                .id("memory-editor-actions")
                .debug_selector(|| "memory-editor-actions".into())
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_3()
                .pt_2();
            if self.memory_saving {
                actions = actions.child("正在保存…");
            } else {
                actions = actions
                    .child(
                        div()
                            .id("memory-save")
                            .debug_selector(|| "memory-save".into())
                            .cursor_pointer()
                            .px_4()
                            .py_2()
                            .rounded_md()
                            .bg(color("text"))
                            .text_color(color("bg"))
                            .child("保存记忆")
                            .on_click(
                                cx.listener(|this, _, _, cx| this.apply_memory_edit(None, cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("memory-cancel")
                            .cursor_pointer()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .text_color(color("dim"))
                            .child("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.memory_editor = None;
                                this.decision_error.clear();
                                cx.notify();
                            })),
                    );
            }
            form = form.child(actions);
            return panel.child(form).into_any_element();
        }
        panel = panel.child(
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .child(
                            div()
                                .id("memory-overview-tab")
                                .debug_selector(|| "memory-overview-tab".into())
                                .cursor_pointer()
                                .py_2()
                                .text_color(color(if self.memory_records { "dim" } else { "text" }))
                                .child("当前摘要")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.memory_records = false;
                                    this.decision_scroll.set_offset(point(px(0.), px(0.)));
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .id("memory-records-tab")
                                .debug_selector(|| "memory-records-tab".into())
                                .cursor_pointer()
                                .py_2()
                                .text_color(color(if self.memory_records { "text" } else { "dim" }))
                                .child(format!("详细记忆 {}", self.memory_rows.len()))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.memory_records = true;
                                    this.decision_scroll.set_offset(point(px(0.), px(0.)));
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .id("memory-add")
                        .debug_selector(|| "memory-add".into())
                        .cursor_pointer()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(color("input"))
                        .child("＋ 新增")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.start_memory_edit(None, window, cx)
                        })),
                ),
        );
        if !self.memory_records {
            let compiled = self.memory_snapshot["compiled"] == true;
            let summary = self.memory_snapshot["summary"].as_str().unwrap_or("");
            let body = memory_summary_body(summary, &self.memory_rows);
            let mut card = div()
                .id("memory-summary")
                .debug_selector(|| "memory-summary".into())
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .p_3()
                .rounded_lg()
                .border_1()
                .border_color(color("border"))
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().child("Super Agent 当前使用的摘要"))
                        .child(
                            div()
                                .text_xs()
                                .text_color(color(if compiled { "accent" } else { "dim" }))
                                .child(if compiled { "已整理" } else { "已生效" }),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(color("dim"))
                        .child(if compiled {
                            "从详细记忆整理而来；偏好始终加载，来源按需读取。"
                        } else {
                            "已保存的内容已生效；后台空闲时会进一步归并。"
                        }),
                );
            if body.is_empty() {
                card = card.child(
                    div()
                        .text_color(color("dim"))
                        .child("还没有摘要。新增记忆后会在这里显示。"),
                );
            } else {
                card = card.child(
                    div()
                        .id("memory-summary-body")
                        .debug_selector(|| "memory-summary-body".into())
                        .w_full()
                        .min_w_0()
                        .flex_shrink_0()
                        .child(TextView::markdown("memory-stable-summary", body).text_sm()),
                );
            }
            if let Some(at) = self.memory_snapshot["last_reviewed_at"]
                .as_i64()
                .filter(|t| *t > 0)
            {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(color("dim"))
                        .child(format!("最近整理：{}", memory_age(at))),
                );
            }
            panel=panel.child(card).child(div().flex_shrink_0().text_xs().text_color(color("dim")).child("要调整它的理解，请编辑对应的详细记忆。摘要会随修改更新；当前指令始终优先。"));
            return panel.into_any_element();
        }
        if self.memory_rows.is_empty() {
            panel = panel.child(
                div()
                    .py_4()
                    .text_color(color("dim"))
                    .child("还没有记忆。点击“新增”，或让管家从你的反馈中学习。"),
            );
        }
        for row in self.memory_rows.clone() {
            let id = row["id"].as_str().unwrap_or("").to_owned();
            let key = format!("memory:{id}");
            let open = self.expanded.contains(&key);
            let toggle = key.clone();
            let mut card = div()
                .id(SharedString::from(key))
                .debug_selector(|| "memory-card".into())
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .p_3()
                .rounded_lg()
                .border_1()
                .border_color(color("border"))
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(color("dim"))
                        .child(if row["edited_by_user"] == true {
                            "手动记录"
                        } else if row["confidence"] == "inferred" {
                            "从反馈中归纳"
                        } else {
                            "来自你的明确要求"
                        }),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("memory-title-{id}")))
                        .debug_selector(|| "memory-card-title".into())
                        .flex_shrink_0()
                        .child(row["title"].as_str().unwrap_or("").to_owned()),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("memory-body-{id}")))
                        .debug_selector(|| "memory-card-body".into())
                        .w_full()
                        .min_w_0()
                        .flex_shrink_0()
                        .text_color(color("dim"))
                        .child(
                            TextView::markdown(
                                SharedString::from(format!("memory-content-{id}")),
                                row["content"].as_str().unwrap_or("").to_owned(),
                            )
                            .text_sm(),
                        ),
                );
            let target = id.clone();
            let delete = id.clone();
            let revision = row["revision"].as_u64().unwrap_or(0);
            if !self.memory_saving {
                card = card.child(
                    div()
                        .id(SharedString::from(format!("memory-actions-{id}")))
                        .debug_selector(|| "memory-card-actions".into())
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .id(SharedString::from(format!("memory-detail-{id}")))
                                .debug_selector(|| "memory-details".into())
                                .cursor_pointer()
                                .text_xs()
                                .text_color(color("dim"))
                                .child(if open {
                                    "▾ 收起来源"
                                } else {
                                    "▸ 查看来源"
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if !this.expanded.remove(&toggle) {
                                        this.expanded.insert(toggle.clone());
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    div()
                                        .id(SharedString::from(format!("memory-edit-{id}")))
                                        .debug_selector(|| "memory-edit".into())
                                        .cursor_pointer()
                                        .px_2()
                                        .py_1()
                                        .rounded_md()
                                        .bg(color("input"))
                                        .text_xs()
                                        .child("编辑")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.start_memory_edit(Some(&target), window, cx)
                                        })),
                                )
                                .child(
                                    div()
                                        .id(SharedString::from(format!("memory-delete-{id}")))
                                        .debug_selector(|| "memory-delete".into())
                                        .cursor_pointer()
                                        .px_2()
                                        .py_1()
                                        .rounded_md()
                                        .text_xs()
                                        .text_color(color("dim"))
                                        .child("删除")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.memory_delete = Some((delete.clone(), revision));
                                            cx.notify();
                                        })),
                                ),
                        ),
                );
            }
            if self
                .memory_delete
                .as_ref()
                .is_some_and(|(target, _)| target == &id)
                && !self.memory_saving
            {
                let target = id.clone();
                let delete_revision = self.memory_delete.as_ref().unwrap().1;
                card = card.child(
                    div()
                        .flex_shrink_0()
                        .p_2()
                        .rounded_md()
                        .bg(color("input"))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .child("删除这条记忆？它会从当前摘要移除，原聊天记录保留。"),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_3()
                                .child(
                                    div()
                                        .id("memory-delete-confirm")
                                        .debug_selector(|| "memory-delete-confirm".into())
                                        .cursor_pointer()
                                        .text_xs()
                                        .child("确认删除")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.apply_memory_edit(
                                                Some((target.clone(), delete_revision)),
                                                cx,
                                            )
                                        })),
                                )
                                .child(
                                    div()
                                        .id("memory-delete-cancel")
                                        .cursor_pointer()
                                        .text_xs()
                                        .text_color(color("dim"))
                                        .child("取消")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.memory_delete = None;
                                            cx.notify();
                                        })),
                                ),
                        ),
                );
            }
            if open {
                if let Some(sources) = row["sources"].as_array() {
                    for (i, source) in sources.iter().enumerate() {
                        card = card.child(
                            div()
                                .id(SharedString::from(format!("memory-source-{id}-{i}")))
                                .debug_selector(|| "memory-source".into())
                                .w_full()
                                .min_w_0()
                                .flex_shrink_0()
                                .pl_3()
                                .border_l_1()
                                .border_color(color("border"))
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(div().text_xs().text_color(color("dim")).child(
                                    if source["origin"] == "user_editor" {
                                        "你在编辑器中保存".to_owned()
                                    } else {
                                        format!(
                                            "对话中的反馈 · {}",
                                            source["time"]
                                                .as_i64()
                                                .map(memory_age)
                                                .unwrap_or_default()
                                        )
                                    },
                                ))
                                .child(
                                    div()
                                        .text_sm()
                                        .child(source["quote"].as_str().unwrap_or("").to_owned()),
                                ),
                        );
                    }
                }
            }
            panel = panel.child(card);
        }
        panel.into_any_element()
    }
}

fn memory_age(at: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let minutes = (now.saturating_sub(at) / 60000).max(0);
    if minutes < 1 {
        "刚刚".into()
    } else if minutes < 60 {
        format!("{minutes} 分钟前")
    } else if minutes < 1440 {
        format!("{} 小时前", minutes / 60)
    } else {
        format!("{} 天前", minutes / 1440)
    }
}

fn memory_summary_body(summary: &str, rows: &[Value]) -> String {
    let content = summary
        .find("## ")
        .map(|i| &summary[i..])
        .unwrap_or(summary);
    let ids: std::collections::HashSet<_> = rows.iter().filter_map(|r| r["id"].as_str()).collect();
    let mut clean = String::new();
    let mut rest = content;
    while let Some(start) = rest.find('[') {
        clean.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        if let Some(end) = tail.find(']') {
            let reference = &tail[..end];
            if !reference.split(',').all(|id| ids.contains(id.trim())) {
                clean.push_str(&rest[start..start + end + 2]);
            }
            rest = &tail[end + 1..];
        } else {
            clean.push_str(&rest[start..]);
            rest = "";
            break;
        }
    }
    clean.push_str(rest);
    let sections = clean
        .split("## ")
        .filter(|section| !section.trim().is_empty())
        .filter(|section| {
            let body = section
                .split_once('\n')
                .map(|(_, body)| body.trim())
                .unwrap_or("");
            !body.is_empty() && body != "(No recorded notes)" && body != "暂无记录"
        })
        .map(|section| format!("## {}", section.trim()))
        .collect::<Vec<_>>()
        .join("\n\n");
    sections
        .replace(
            "## Collaboration preferences and memory routes",
            "## 协作偏好与记忆索引",
        )
        .replace(
            "Summary shortened; list_memories/read_memory contain the complete records.",
            "摘要较长，完整内容请查看详细记忆。",
        )
        .replace("## User profile", "## 用户画像")
        .replace("## Collaboration preferences", "## 协作偏好")
        .replace("## Detailed memory index", "## 相关记忆索引")
        .replace("(No recorded notes)", "暂无记录")
}

#[cfg(test)]
mod memory_display_tests {
    use super::memory_summary_body;
    use serde_json::json;
    #[test]
    fn summary_display_hides_routing_ids_and_empty_sections_but_preserves_meaning() {
        let rows = vec![
            json!({"id":"workflow.first"}),
            json!({"id":"workflow.second"}),
        ];
        let text = "Internal context snapshot v1\n\n## User profile\n(No recorded notes)\n\n## Collaboration preferences\n- 核实证据后同步 [workflow.first, workflow.second]\n- 保留 [实际术语]\n\n## Detailed memory index\n(No recorded notes)";
        let display = memory_summary_body(text, &rows);
        assert!(display.contains("协作偏好"));
        assert!(display.contains("核实证据后同步"));
        assert!(display.contains("[实际术语]"));
        assert!(!display.contains("workflow.first"));
        assert!(!display.contains("Internal context"));
        assert!(!display.contains("用户画像"));
        assert!(!display.contains("暂无记录"));
    }
}
