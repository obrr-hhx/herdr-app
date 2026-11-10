//! Conversation facts are structured events, never terminal cells.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Debug, Default)]
pub struct Message {
    pub id: String,
    pub role: String,
    pub text: String,
    pub images: Vec<String>,
    pub at: u64,
}
#[derive(Default)]
pub struct Conversation {
    pub provider: String,
    pub active_turn: String,
    pub dispatches: BTreeMap<String, Value>,
    pub models: Vec<Value>,
    pub cursor: u64,
    pub completion: u64,
    pub messages: Vec<Message>,
    positions: HashMap<String, usize>,
    dirty_rows: BTreeSet<usize>,
    pub permissions: BTreeMap<String, Value>,
    pub reminders: BTreeMap<String, Value>,
    pub status: String,
    execution_status: String,
    importing_history: bool,
    pub capabilities: Vec<String>,
    pub files: Vec<String>,
    pub commands: Vec<Value>,
    pub workspace_loaded: bool,
    pub files_query: String,
    pub files_truncated: bool,
    pub workspace_error: String,
    pub model: Option<String>,
    pub effort: String,
    pub settings_applied: bool,
    pub status_line: Value,
    pub last_progress: u64,
    pub phase: String,
    pub active_tool: String,
    pub permission_mode: String,
}
impl Conversation {
    pub fn activity_label(&self, now: u64) -> String {
        if !self.permissions.is_empty() {
            return "Waiting for you".into();
        }
        if !self.reminders.is_empty() {
            return format!("{} 条提醒待确认", self.reminders.len());
        }
        if self.status == "working" && self.last_progress > 0 {
            let seconds = now.saturating_sub(self.last_progress) / 1000;
            if seconds >= 90 {
                return format!("No updates for {}m · Stop to recover", seconds / 60);
            }
            return match self.phase.as_str() {
                "tool" if !self.active_tool.is_empty() => format!("Running {}", self.active_tool),
                "thinking" => "Waiting for Claude".into(),
                "responding" => "Responding".into(),
                "submitting" => "Sending…".into(),
                _ => "Working".into(),
            };
        }
        match self.status.as_str() {
            "idle" | "starting" => "Ready",
            "working" => "Working",
            "interrupting" => "Stopping…",
            "interrupted" => "Stopped · send to resume",
            "error" => "Needs attention",
            _ => "Connecting…",
        }
        .into()
    }
    pub fn agent_word(&self) -> &'static str {
        if !self.permissions.is_empty() || !self.reminders.is_empty() {
            return "blocked";
        }
        // Connection health never means that a person must answer a question,
        // and losing transport is not proof that a running turn has finished.
        let execution = if self.execution_status.is_empty() {
            &self.status
        } else {
            &self.execution_status
        };
        match execution.as_str() {
            "working" | "interrupting" => "working",
            _ => "idle",
        }
    }
    pub fn take_dirty_rows(&mut self) -> BTreeSet<usize> {
        std::mem::take(&mut self.dirty_rows)
    }
    pub fn put(&mut self, id: String, role: &str, text: String, append: bool) {
        if let Some(&i) = self.positions.get(&id) {
            self.dirty_rows.insert(i);
            self.messages[i].role = role.into();
            if append {
                self.messages[i].text.push_str(&text);
            } else {
                self.messages[i].text = text;
            }
        } else {
            self.dirty_rows.insert(self.messages.len());
            self.positions.insert(id.clone(), self.messages.len());
            self.messages.push(Message {
                id,
                role: role.into(),
                text,
                images: Vec::new(),
                at: 0,
            });
        }
    }
    /// Reject gaps and duplicates. The bridge replays from the durable local cursor.
    pub fn apply(&mut self, e: &Value) -> bool {
        if let Some(seq) = e["seq"].as_u64() {
            if seq <= self.cursor || seq != self.cursor + 1 {
                return false;
            }
            self.cursor = seq;
        }
        if matches!(e["type"].as_str(), Some("activity" | "connected" | "state")) {
            if let Some(time) = e["last_progress"].as_u64() {
                self.last_progress = time;
                self.phase = e["phase"].as_str().unwrap_or_default().into();
                self.active_tool = e["tool"].as_str().unwrap_or_default().into();
                self.permission_mode = e["permission_mode"].as_str().unwrap_or_default().into();
            }
        }
        if e.get("active_turn").is_some() {
            self.active_turn = e["active_turn"].as_str().unwrap_or_default().into();
        }
        let id = e["id"].as_str().unwrap_or_default().to_owned();
        match e["type"].as_str().unwrap_or_default() {
            "session" if e["resumed"] == true => self.importing_history = true,
            "import_complete" => self.importing_history = false,
            "capabilities" => {
                self.capabilities = e["capabilities"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
            }
            "dispatch_receipt" | "prompt_confirmed" => {
                let receipt = if e["type"] == "prompt_confirmed" {
                    serde_json::json!({"id":id,"mode":e["mode"],"stage":"confirmed"})
                } else {
                    e.clone()
                };
                self.dispatches.insert(id.clone(), receipt);
                if let Some(i) = self.positions.get(&id) {
                    self.dirty_rows.insert(*i);
                }
            }
            "user" | "review_requested" => {
                if e["mode"] == "steer" {
                    self.dispatches.entry(id.clone()).or_insert_with(
                        || serde_json::json!({"id":id,"mode":"steer","stage":"submitting"}),
                    );
                }
                self.put(
                    id.clone(),
                    "You",
                    e["text"].as_str().unwrap_or_default().into(),
                    false,
                );
                if let Some(&i) = self.positions.get(&id) {
                    self.messages[i].images = e["images"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str())
                        .filter(|s| super::attachments::valid_id(s))
                        .map(str::to_owned)
                        .take(4)
                        .collect();
                }
            }
            "delta" if e["kind"] == "text_delta" => self.put(
                id,
                "Claude",
                e["text"].as_str().unwrap_or_default().into(),
                true,
            ),
            "assistant" => {
                let text = e["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|b| b["type"] == "text")
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if !text.is_empty() {
                    self.put(id, "Claude", text, false);
                }
            }
            "tool" => {
                self.put(
                    id.clone(),
                    e["name"].as_str().unwrap_or("Tool"),
                    serde_json::to_string_pretty(&e["input"]).unwrap_or_default(),
                    false,
                );
                if let Some(i) = self.positions.get(&id) {
                    self.messages[*i].at = if self.importing_history {
                        0
                    } else {
                        e["time"].as_u64().unwrap_or(0)
                    };
                }
            }
            "tool_result" => {
                self.put(
                    format!("result:{id}"),
                    if e["is_error"] == true {
                        "Tool error"
                    } else {
                        "Tool result"
                    },
                    if let Some(text) = e["content"].as_str() {
                        text.into()
                    } else {
                        serde_json::to_string_pretty(&e["content"]).unwrap_or_default()
                    },
                    false,
                );
                if let Some(i) = self.positions.get(&format!("result:{id}")) {
                    self.messages[*i].at = if self.importing_history {
                        0
                    } else {
                        e["time"].as_u64().unwrap_or(0)
                    };
                }
            }
            "permission" => {
                self.permissions.insert(id, e.clone());
            }
            "permission_resolved" => {
                self.permissions.remove(&id);
            }
            "statusline" => {
                self.status_line = e.clone();
            }
            "workspace" => {
                self.files = e["files"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|f| f.as_str().map(str::to_owned))
                    .collect();
                self.commands = e["commands"].as_array().cloned().unwrap_or_default();
                self.workspace_loaded = true;
                self.files_query = e["query"].as_str().unwrap_or_default().into();
                self.files_truncated = e["truncated"] == true;
                self.workspace_error = e["error"].as_str().unwrap_or_default().into();
            }
            "models" => {
                if e["provider"] == self.provider {
                    self.models = e["models"].as_array().cloned().unwrap_or_default();
                }
            }
            "settings" => {
                if let Some(provider) = e["provider"].as_str() {
                    if self.provider != provider {
                        self.status_line = Value::Null;
                    }
                    self.provider = provider.to_owned();
                }
                self.model = e["model"].as_str().map(str::to_owned);
                self.effort = e["effort"].as_str().unwrap_or_default().into();
                self.settings_applied = e["applied"] == true;
            }
            "state" | "connected" | "transport" => {
                if e["type"] == "state"
                    && e["status"] == "idle"
                    && matches!(self.execution_status.as_str(), "working" | "interrupting")
                {
                    self.completion = e["seq"].as_u64().unwrap_or(self.completion);
                }
                if e["type"] != "transport" {
                    self.execution_status = e["status"].as_str().unwrap_or_default().into();
                }
                if e["type"] == "connected" {
                    if let Some(pending) = e["pending"].as_array() {
                        self.permissions
                            .retain(|id, _| pending.iter().any(|p| p.as_str() == Some(id)));
                    }
                    self.capabilities = e["capabilities"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect();
                }
                self.status = e["message"]
                    .as_str()
                    .or_else(|| e["status"].as_str())
                    .unwrap_or_default()
                    .into();
            }
            "worker_decision" => {
                let address = e["address"].as_str().unwrap_or("Codex");
                let answers = e["answers"].as_object().filter(|a| !a.is_empty()).map(|a| {
                    a.iter()
                        .map(|(q, v)| format!("{q}\n{}", v.as_str().unwrap_or_default()))
                        .collect::<Vec<_>>()
                        .join("\n\n")
                });
                let decision = answers.unwrap_or_else(|| {
                    if e["decision"] == "allow" {
                        "仅允许这一次操作".into()
                    } else {
                        "拒绝这一次操作".into()
                    }
                });
                self.put(
                    format!("worker-decision:{}", e["id"].as_str().unwrap_or_default()),
                    "You",
                    format!("答复 {address}\n\n{decision}"),
                    false,
                );
            }
            "reminder" => {
                if let Some(id) = e["id"].as_str() {
                    self.reminders.insert(id.to_owned(), e.clone());
                    let title = e["title"].as_str().unwrap_or("Reminder");
                    let text = e["text"].as_str().unwrap_or_default();
                    let at = e["at"].as_str().unwrap_or_default();
                    let zone = e["timezone"].as_str().unwrap_or_default();
                    let zone_label = if zone == "Asia/Shanghai" {
                        "北京时间"
                    } else {
                        zone
                    };
                    let display_at = at.get(..19).unwrap_or(at).replace('T', " ");
                    let late = if e["overdue_ms"].as_u64().unwrap_or(0) > 60000 {
                        " · 补发"
                    } else {
                        ""
                    };
                    self.put(
                        format!("reminder:{id}"),
                        "Reminder",
                        format!("⏰ **{title}**{late}\n\n{text}\n\n{display_at} · {zone_label}"),
                        false,
                    );
                }
            }
            "reminder_ack" => {
                if let Some(id) = e["id"].as_str() {
                    self.reminders.remove(id);
                }
            }
            "notice" | "command_error" => self.put(
                format!("notice:{}", self.cursor),
                "Notice",
                e["message"].as_str().unwrap_or_default().into(),
                false,
            ),
            _ => {}
        }
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn external_question_survives_reconnect_until_its_runtime_resolves_it() {
        let mut c = Conversation::default();
        c.apply(&json!({"type":"permission","id":"codexq_qa","source":"codex_worker","name":"AskUserQuestion","input":{"questions":[{"question":"A or B?"}]}}));
        c.apply(&json!({"type":"connected","status":"idle","pending":["codexq_qa"]}));
        assert!(c.permissions.contains_key("codexq_qa"));
        c.apply(
            &json!({"type":"worker_decision","id":"ui-1","address":"t2","answers":{"A or B?":"B"}}),
        );
        assert!(
            c.messages
                .iter()
                .any(|m| m.text.contains("答复 t2") && m.text.contains("A or B?\nB"))
        );
        c.apply(&json!({"type":"permission_resolved","id":"codexq_qa"}));
        assert!(c.permissions.is_empty());
    }
    #[test]
    fn changing_manager_provider_clears_the_previous_providers_usage() {
        let mut c = Conversation::default();
        c.apply(&json!({"type":"settings","provider":"claude","model":"opus"}));
        c.apply(&json!({"type":"statusline","model":"Opus","cost_usd":12}));
        c.apply(&json!({"type":"settings","provider":"codex","model":"default"}));
        assert!(c.status_line.is_null());
        assert_eq!(c.provider, "codex");
    }

    #[test]
    fn completion_stamp_survives_reconnect_and_only_advances_on_finished_work() {
        let mut c = Conversation::default();
        c.apply(&json!({"seq":1,"type":"state","status":"working"}));
        c.apply(&json!({"seq":2,"type":"state","status":"idle"}));
        assert_eq!(c.completion, 2);
        c.apply(&json!({"type":"transport","status":"connecting"}));
        c.apply(&json!({"type":"connected","status":"idle","pending":[]}));
        c.apply(&json!({"seq":3,"type":"state","status":"idle"}));
        assert_eq!(c.completion, 2);
        c.apply(&json!({"seq":4,"type":"state","status":"working"}));
        c.apply(&json!({"seq":5,"type":"state","status":"idle"}));
        assert_eq!(c.completion, 5);
    }
    #[test]
    fn only_changed_message_rows_need_layout() {
        let mut c = Conversation::default();
        c.apply(&json!({"type":"user","id":"a","text":"hello"}));
        c.apply(&json!({"type":"delta","id":"b","kind":"text_delta","text":"first"}));
        assert_eq!(c.take_dirty_rows(), BTreeSet::from([0, 1]));
        c.apply(&json!({"type":"activity","last_progress":1000,"phase":"tool"}));
        c.apply(&json!({"type":"state","status":"working"}));
        c.apply(&json!({"type":"settings","model":"opus"}));
        assert!(c.take_dirty_rows().is_empty());
        c.apply(&json!({"type":"delta","id":"b","kind":"text_delta","text":" more"}));
        assert_eq!(c.take_dirty_rows(), BTreeSet::from([1]));
    }
    #[test]
    fn activity_does_not_turn_a_heartbeat_into_progress() {
        let mut c = Conversation::default();
        c.apply(&json!({"type":"connected","status":"working","last_progress":1000,"phase":"tool","tool":"Bash","permission_mode":"bypassPermissions"}));
        assert_eq!(c.activity_label(2000), "Running Bash");
        c.apply(&json!({"type":"activity","last_progress":1000,"phase":"tool","tool":"Bash","permission_mode":"bypassPermissions"}));
        assert_eq!(
            c.activity_label(121000),
            "No updates for 2m · Stop to recover"
        );
        assert!(c.messages.is_empty());
        assert_eq!(c.permission_mode, "bypassPermissions");
        c.permissions.insert("question".into(), json!({}));
        assert_eq!(c.activity_label(121000), "Waiting for you");
        c.permissions.clear();
        c.status = "idle".into();
        assert_eq!(c.activity_label(121000), "Ready");
    }
    #[test]
    fn composer_metadata_does_not_create_chat_messages() {
        let mut c = Conversation::default();
        c.apply(
            &json!({"type":"connected","status":"idle","capabilities":["workspace","settings"]}),
        );
        c.apply(&json!({"type":"statusline","context_pct":42,"five_hour":{"used_pct":30}}));
        assert_eq!(c.status_line["context_pct"], 42);
        assert!(c.take_dirty_rows().is_empty());
        c.apply(&json!({"type":"workspace","files":["src/main.rs"],"commands":[{"name":"review"}],"query":"src","truncated":false}));
        c.apply(&json!({"seq":1,"type":"settings","model":"opus","effort":"high","applied":true}));
        assert!(c.messages.is_empty());
        assert_eq!(c.model.as_deref(), Some("opus"));
        assert_eq!(c.effort, "high");
        assert!(c.settings_applied);
        assert_eq!(c.files, ["src/main.rs"]);
        assert_eq!(c.files_query, "src");
        assert!(c.capabilities.iter().any(|x| x == "workspace"));
    }
    #[test]
    fn agent_attention_tracks_permission_and_connection_state() {
        let mut c = Conversation::default();
        c.status = "working".into();
        assert_eq!(c.agent_word(), "working");
        c.apply(&json!({"type":"permission","id":"a"}));
        assert_eq!(c.agent_word(), "blocked");
        c.apply(&json!({"type":"permission_resolved","id":"a"}));
        assert_eq!(c.agent_word(), "working");
        c.status = "idle".into();
        assert_eq!(c.agent_word(), "idle");
        c.status = "SSH disconnected — reconnecting".into();
        assert_eq!(c.agent_word(), "idle");
    }

    #[test]
    fn transport_failure_is_neither_a_question_nor_a_completed_turn() {
        let mut c = Conversation::default();
        c.apply(&json!({"type":"state","status":"working"}));
        c.apply(&json!({"type":"transport","message":"SSH disconnected — reconnecting"}));
        assert_eq!(c.agent_word(), "working");
        c.apply(&json!({"type":"connected","status":"idle","pending":[]}));
        assert_eq!(c.agent_word(), "idle");
        c.apply(&json!({"type":"permission","id":"expired-question"}));
        assert_eq!(c.agent_word(), "blocked");
        c.apply(&json!({"type":"connected","status":"idle","pending":[]}));
        assert_eq!(c.agent_word(), "idle");
        assert!(c.permissions.is_empty());
        c.apply(&json!({"type":"permission","id":"live-question"}));
        c.apply(&json!({"type":"connected","status":"idle","pending":["live-question"]}));
        assert_eq!(c.agent_word(), "blocked");
        c.apply(&json!({"type":"permission_resolved","id":"live-question"}));
        for status in ["error", "interrupted"] {
            c.apply(&json!({"type":"state","status":status}));
            assert_eq!(c.agent_word(), "idle");
        }
    }
    #[test]
    fn final_message_replaces_partial_text_and_replay_cannot_duplicate_it() {
        let mut c = Conversation::default();
        for e in [
            json!({"seq":1,"type":"delta","id":"m","kind":"text_delta","text":"你"}),
            json!({"seq":2,"type":"delta","id":"m","kind":"text_delta","text":"好"}),
            json!({"seq":3,"type":"assistant","id":"m","content":[{"type":"text","text":"你好。"}]}),
        ] {
            assert!(c.apply(&e));
            assert!(!c.apply(&e));
        }
        assert_eq!(c.messages.len(), 1);
        assert_eq!(c.messages[0].text, "你好。");
        assert!(!c.apply(&json!({"seq":5,"type":"user","id":"wrong","text":"gap"})));
        assert_eq!(c.cursor, 3);
    }
    #[test]
    fn approvals_are_replayed_until_resolved() {
        let mut c = Conversation::default();
        c.apply(&json!({"seq":1,"type":"permission","id":"p","name":"Bash"}));
        assert!(c.permissions.contains_key("p"));
        c.apply(&json!({"seq":2,"type":"permission_resolved","id":"p"}));
        assert!(c.permissions.is_empty());
    }
    #[test]
    fn codex_catalog_survives_provider_switch_and_event_replay() {
        let mut c = Conversation::default();
        for e in [
            json!({"seq":1,"type":"settings","provider":"codex","model":"test-model"}),
            json!({"seq":2,"type":"models","provider":"codex","models":[{"model":"test-model"}]}),
            json!({"seq":3,"type":"settings","provider":"claude","model":"opus"}),
            json!({"seq":4,"type":"settings","provider":"codex","model":"test-model"}),
        ] {
            assert!(c.apply(&e));
        }
        assert_eq!(c.models.len(), 1);
        assert_eq!(c.models[0]["model"], "test-model");
        c.apply(&json!({"seq":5,"type":"models","provider":"claude","models":[]}));
        assert_eq!(c.models.len(), 1);
    }
    #[test]
    fn reminders_remain_needs_you_through_idle_and_replay_until_acknowledged() {
        let mut c = Conversation::default();
        let e = json!({"seq":1,"type":"reminder","id":"r@1","title":"审批提醒","text":"等待单独 apply","at":"2026-11-06T09:00:00+08:00","timezone":"Asia/Shanghai"});
        assert!(c.apply(&e));
        assert_eq!(c.agent_word(), "blocked");
        assert!(!c.apply(&e));
        assert_eq!(c.messages.len(), 1);
        c.apply(&json!({"seq":2,"type":"state","status":"idle"}));
        assert_eq!(c.agent_word(), "blocked");
        assert_eq!(c.reminders.len(), 1);
        assert!(c.messages[0].text.contains("2026-11-06 09:00:00"));
        assert!(c.messages[0].text.contains("北京时间"));
        c.apply(&json!({"seq":3,"type":"reminder_ack","id":"r@1"}));
        assert_eq!(c.agent_word(), "idle");
        assert!(c.reminders.is_empty());
        assert_eq!(c.messages.len(), 1);
    }
    #[test]
    fn steering_receipts_require_exact_confirmation_and_survive_reconnect() {
        let mut c = Conversation::default();
        c.apply(&json!({"type":"connected","status":"working","active_turn":"original-turn","capabilities":["steer"]}));
        c.apply(
            &json!({"seq":1,"type":"user","id":"steer-request","text":"change","mode":"steer"}),
        );
        c.apply(&json!({"seq":2,"type":"dispatch_receipt","id":"steer-request","mode":"steer","stage":"submitted"}));
        assert_eq!(c.dispatches["steer-request"]["stage"], "submitted");
        c.apply(&json!({"seq":3,"type":"prompt_confirmed","id":"steer-request","mode":"steer"}));
        c.apply(&json!({"type":"connected","status":"working","active_turn":"original-turn","capabilities":["steer"]}));
        assert_eq!(c.dispatches["steer-request"]["stage"], "confirmed");
        assert_eq!(c.active_turn, "original-turn");
    }
}
