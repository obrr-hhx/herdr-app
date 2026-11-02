//! Conversation facts are structured events, never terminal cells.
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, Default)]
pub struct Message {
    pub id: String,
    pub role: String,
    pub text: String,
}
#[derive(Default)]
pub struct Conversation {
    pub cursor: u64,
    pub messages: Vec<Message>,
    positions: HashMap<String, usize>,
    pub permissions: BTreeMap<String, Value>,
    pub status: String,
}
impl Conversation {
    pub fn put(&mut self, id: String, role: &str, text: String, append: bool) {
        if let Some(&i) = self.positions.get(&id) {
            self.messages[i].role = role.into();
            if append {
                self.messages[i].text.push_str(&text);
            } else {
                self.messages[i].text = text;
            }
        } else {
            self.positions.insert(id.clone(), self.messages.len());
            self.messages.push(Message {
                id,
                role: role.into(),
                text,
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
        let id = e["id"].as_str().unwrap_or_default().to_owned();
        match e["type"].as_str().unwrap_or_default() {
            "user" => self.put(
                id,
                "You",
                e["text"].as_str().unwrap_or_default().into(),
                false,
            ),
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
            "tool" => self.put(
                id,
                e["name"].as_str().unwrap_or("Tool"),
                serde_json::to_string_pretty(&e["input"]).unwrap_or_default(),
                false,
            ),
            "tool_result" => self.put(
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
            ),
            "permission" => {
                self.permissions.insert(id, e.clone());
            }
            "permission_resolved" => {
                self.permissions.remove(&id);
            }
            "state" | "connected" | "transport" => {
                self.status = e["message"]
                    .as_str()
                    .or_else(|| e["status"].as_str())
                    .unwrap_or_default()
                    .into();
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
}
