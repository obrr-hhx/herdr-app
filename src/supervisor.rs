//! Durable session discovery, independent of selection and unread markers.
use serde_json::{Value, json};
use std::sync::Mutex;
static LAST: Mutex<(String, u64)> = Mutex::new((String::new(), 0));
pub fn snapshot_path() -> std::path::PathBuf {
    crate::mods::app_dir().join("data/super-agent/sessions.json")
}
pub fn publish(sessions: &[Value], machines: &[Value]) {
    let value = json!({"sessions":sessions,"machines":machines});
    let text = value.to_string();
    let now = crate::model::now_ms();
    let mut last = LAST.lock().unwrap();
    if text == last.0 && now.saturating_sub(last.1) < 10000 {
        return;
    }
    let path = snapshot_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let value =
        json!({"sessions":sessions,"machines":machines,"at":now,"core":crate::core_bin::path()});
    if crate::persist::write_atomic(&path, &value.to_string()).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        *last = (text, now);
    }
}
/// Closing is an archive of the app entry, not destruction of worker data/runtime.
pub fn archive_path() -> std::path::PathBuf {
    crate::mods::app_dir().join("data/super-agent/closed-sessions.json")
}
pub fn archives() -> Result<Vec<Value>, String> {
    match std::fs::read(archive_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| "Closed-session registry needs recovery; no action taken".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.to_string()),
    }
}
pub fn session_identity(row: &Value) -> Option<Value> {
    if let Some(id) = row["conversation"].as_str().filter(|s| !s.is_empty()) {
        Some(json!({"conversation":id,"host":row["host"]}))
    } else if row["read_identity"]["pid"].as_u64().is_some()
        && row["read_identity"]["created_ms"].as_u64().is_some()
    {
        Some(row["read_identity"].clone())
    } else {
        None
    }
}
pub fn archived(row: &Value, closed: &[Value]) -> bool {
    session_identity(row).is_some_and(|identity| {
        closed
            .iter()
            .any(|c| c["address"] == row["address"] && c["identity"] == identity)
    })
}
pub fn archive(
    row: &Value,
    request: &str,
    expected: &Value,
    content: Value,
) -> Result<Value, String> {
    if row["super_agent"] == true {
        return Err("Cannot close Super Agent".into());
    }
    let identity = session_identity(row).ok_or("Session identity unavailable; nothing closed")?;
    if &identity != expected {
        return Err("Session identity changed; refresh list_sessions before closing".into());
    }
    let mut closed = archives()?;
    if let Some(old) = closed.iter().find(|c| c["request_id"] == request) {
        if old["address"] != row["address"] || old["identity"] != identity {
            return Err("request_id reused for a different session".into());
        }
        return Ok(old.clone());
    }
    let record = json!({"address":row["address"],"identity":identity,"request_id":request,"session":row,"content":content,"closed_at":crate::model::now_ms(),"history_preserved":true,"runtime_stopped":false});
    closed.push(record.clone());
    let path = archive_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    crate::persist::write_atomic(
        &path,
        serde_json::to_vec_pretty(&closed).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(record)
}
/// Keep only small runtime facts on the UI thread; history stays in the bridge.
pub fn codex_facts() -> Value {
    type Cache = (Option<std::time::SystemTime>, Value);
    static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
    let path = crate::mods::app_dir().join("data/super-agent/codex-workers.json");
    let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let mut cache = CACHE.lock().unwrap();
    if let Some((previous, value)) = cache.as_ref() {
        if stamp.is_some() && *previous == stamp {
            return value.clone();
        }
    }
    let mut value: Value = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(rows) = value["sessions"].as_object_mut() {
        for row in rows.values_mut() {
            let attention = row["pending_questions"]
                .as_object()
                .map(|q| q.keys().cloned().collect::<Vec<_>>().join(","))
                .unwrap_or_default();
            if let Some(fields) = row.as_object_mut() {
                fields.retain(|k, _| {
                    matches!(
                        k.as_str(),
                        "read_identity"
                            | "at"
                            | "connected"
                            | "binding_live"
                            | "execution_state"
                            | "completion"
                            | "thread_id"
                    )
                });
                fields.insert("attention".into(), json!(attention));
            }
        }
    }
    *cache = Some((stamp, value.clone()));
    value
}
pub fn codex_fact<'a>(facts: &'a Value, address: &str, record: &Value) -> Option<&'a Value> {
    let row = &facts["sessions"][address];
    (row["connected"] == true
        && row["binding_live"] == true
        && crate::model::now_ms().saturating_sub(row["at"].as_u64().unwrap_or(0)) < 15000
        && row["read_identity"] == json!({"pid":record["pid"],"created_ms":record["created_ms"]}))
    .then_some(row)
}
pub fn latest_user_event() -> Value {
    let path = crate::mods::app_dir()
        .join("data/forge-chat")
        .join(crate::chat::SUPER_ID)
        .join("events.jsonl");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Value::Null;
    };
    text.split('\n')
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|e| {
            e["type"] == "user" && !e["id"].as_str().unwrap_or_default().starts_with("observe_")
        })
        .unwrap_or(Value::Null)
}
pub fn latest_intent() -> Value {
    let e = latest_user_event();
    json!({"id":e["id"],"time":e["time"]})
}
pub fn read() -> Value {
    let mut value: Value = std::fs::read(snapshot_path())
        .ok()
        .and_then(|s| serde_json::from_slice(&s).ok())
        .unwrap_or_else(|| json!({"sessions":[],"machines":[]}));
    value["latest_user_intent"] = latest_intent();
    value["closed_sessions"] = json!(archives().unwrap_or_default());
    value
}

pub fn question_reply(question: &Value, intent: &Value) -> Result<String, String> {
    if question["name"] != "AskUserQuestion" {
        return Err("Tool approvals must be handled explicitly in the worker pane".into());
    }
    if intent["time"].as_u64().unwrap_or(0) < question["time"].as_u64().unwrap_or(u64::MAX) {
        return Err("A newer user reply is required".into());
    }
    let answer = intent["text"].as_str().unwrap_or_default();
    if answer.trim().is_empty() {
        return Err("Missing user reply".into());
    }
    Ok(answer.to_owned())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archives_are_idempotent_bound_to_identity_and_preserve_history() {
        let root = std::env::temp_dir().join(format!("herdr-close-{}", crate::chat::fresh_id()));
        std::fs::create_dir_all(&root).unwrap();
        let previous = crate::mods::test_app_home(Some(root.clone()));
        let row = json!({"address":"t4","pane":"p4","read_identity":{"pid":4,"created_ms":10}});
        let identity = session_identity(&row).unwrap();
        assert!(
            archive(
                &row,
                "request-close",
                &json!({"pid":5,"created_ms":10}),
                Value::Null
            )
            .is_err()
        );
        let first = archive(&row, "request-close", &identity, json!({"saved":"pane"})).unwrap();
        assert_eq!(
            archive(&row, "request-close", &identity, Value::Null).unwrap(),
            first
        );
        assert_eq!(archives().unwrap().len(), 1);
        assert!(archived(&row, &archives().unwrap()));
        assert!(!archived(
            &json!({"address":"t4","read_identity":{"pid":5,"created_ms":11}}),
            &archives().unwrap()
        ));
        assert!(
            archive(
                &json!({"address":"manager","super_agent":true}),
                "close-manager",
                &identity,
                Value::Null
            )
            .is_err()
        );
        assert_eq!(first["runtime_stopped"], false);
        assert_eq!(first["history_preserved"], true);
        std::fs::write(archive_path(), "broken").unwrap();
        assert!(archives().is_err());
        crate::mods::test_app_home(previous);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn runtime_facts_reject_stale_disconnected_and_reused_sources() {
        let record = json!({"pid":1,"created_ms":10});
        let row = json!({"at":crate::model::now_ms(),"connected":true,"binding_live":true,"read_identity":record});
        let mut facts = json!({"sessions":{"t2":row}});
        assert!(codex_fact(&facts, "t2", &record).is_some());
        facts["sessions"]["t2"]["connected"] = json!(false);
        assert!(codex_fact(&facts, "t2", &record).is_none());
        facts["sessions"]["t2"]["connected"] = json!(true);
        assert!(codex_fact(&facts, "t2", &json!({"pid":2,"created_ms":10})).is_none());
        facts["sessions"]["t2"]["at"] = json!(1);
        assert!(codex_fact(&facts, "t2", &record).is_none());
    }
    #[test]
    fn a_question_uses_a_newer_verbatim_human_reply_and_cannot_approve_tools() {
        let q = json!({"name":"AskUserQuestion","time":20});
        let reply = json!({"text":"选 B。","time":21});
        assert_eq!(question_reply(&q, &reply).unwrap(), "选 B。");
        assert!(question_reply(&q, &json!({"text":"earlier unrelated answer","time":19})).is_err());
        assert!(question_reply(&json!({"name":"Bash","time":20}), &reply).is_err());
        assert!(question_reply(&q, &json!({"text":"","time":21})).is_err());
    }
}
