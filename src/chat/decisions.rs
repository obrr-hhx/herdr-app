//! Durable human decision queue. Only the app writes this file.
use serde_json::{Value, json};

pub fn path() -> std::path::PathBuf {
    crate::mods::app_dir().join("data/super-agent/decisions.json")
}
pub fn read() -> Result<Vec<Value>, String> {
    match std::fs::read(path()) {
        Ok(bytes) => {
            let rows: Vec<Value> = serde_json::from_slice(&bytes)
                .map_err(|_| "Decision list is damaged; original file preserved".to_owned())?;
            if rows.iter().any(|r| {
                !r.is_object()
                    || !r["id"].is_string()
                    || !r["title"].is_string()
                    || !r["address"].is_string()
                    || !r["context"].is_string()
                    || ![Some("pending"), Some("answered"), Some("resolved")]
                        .contains(&r["status"].as_str())
            }) {
                return Err("Decision list is damaged; original file preserved".into());
            }
            Ok(rows)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.to_string()),
    }
}
fn save(rows: &[Value]) -> Result<(), String> {
    let path = path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    crate::persist::write_atomic(
        &path,
        &serde_json::to_string(rows).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
/// Repeated observations update context, never erase the human's answer or completion.
pub fn record(a: &Value) -> Result<Value, String> {
    for key in ["id", "title", "context", "address"] {
        let s = a[key].as_str().ok_or_else(|| format!("Missing {key}"))?;
        if s.trim().is_empty() || s.len() > 12000 {
            return Err(format!("Invalid {key}"));
        }
    }
    if a["id"].as_str().unwrap().len() > 160
        || a["title"].as_str().unwrap().len() > 500
        || a["address"].as_str().unwrap().len() > 256
        || a["evidence"].as_str().is_some_and(|s| s.len() > 4000)
    {
        return Err("Decision identifier, title, address or evidence too long".into());
    }
    let options = a.get("options").cloned().unwrap_or(json!([]));
    if !options.as_array().is_some_and(|v| {
        v.len() <= 6
            && v.iter().all(|s| {
                s.as_str()
                    .is_some_and(|s| !s.trim().is_empty() && s.len() <= 500)
            })
    }) {
        return Err("Use up to six short text options".into());
    }
    let mut rows = read()?;
    let index = rows.iter().position(|r| r["id"] == a["id"]);
    let old = index.map(|i| rows[i].clone()).unwrap_or(json!({}));
    if index.is_some() && old["address"] != a["address"] {
        return Err("Decision ID belongs to another session".into());
    }
    let now = crate::model::now_ms();
    let row = json!({"id":a["id"],"title":a["title"],"context":a["context"],"address":a["address"],
        "evidence":a["evidence"].as_str().unwrap_or(""),"options":options,
        "status":old["status"].as_str().unwrap_or("pending"),"answer":old["answer"],
        "request_id":old["request_id"],"created_at":old["created_at"].as_u64().unwrap_or(now),"updated_at":now});
    if index.is_some()
        && ["title", "context", "options", "evidence"]
            .iter()
            .all(|k| old[*k] == row[*k])
    {
        return Ok(old);
    }
    if let Some(i) = index {
        rows[i] = row.clone();
    } else {
        rows.push(row.clone());
    }
    save(&rows)?;
    Ok(row)
}
pub fn update(id: &str, status: &str, answer: Option<(&str, &str)>) -> Result<Value, String> {
    if !["pending", "answered", "resolved"].contains(&status) {
        return Err("Invalid decision state".into());
    }
    let mut rows = read()?;
    let row = rows
        .iter_mut()
        .find(|r| r["id"] == id)
        .ok_or("Decision not found")?;
    row["status"] = json!(status);
    row["updated_at"] = json!(crate::model::now_ms());
    if let Some((text, request)) = answer {
        row["answer"] = json!(text);
        row["request_id"] = json!(request);
    }
    let result = row.clone();
    save(&rows)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_observation_retains_answer_completion_and_rejects_corruption() {
        let root =
            std::env::temp_dir().join(format!("herdr-decisions-{}", crate::chat::fresh_id()));
        let old = crate::mods::test_app_home(Some(root.clone()));
        let a = json!({"id":"question-1","title":"Choose migration","context":"Two options","address":"t10","options":["A","B"]});
        assert_eq!(record(&a).unwrap()["status"], "pending");
        update("question-1", "answered", Some(("B", "reply-1"))).unwrap();
        assert_eq!(record(&a).unwrap()["answer"], "B");
        update("question-1", "resolved", None).unwrap();
        assert_eq!(record(&a).unwrap()["status"], "resolved");
        assert_eq!(read().unwrap().len(), 1);
        let mut other = a.clone();
        other["address"] = json!("t11");
        assert!(record(&other).is_err());
        std::fs::write(path(), b"broken").unwrap();
        assert!(record(&a).is_err());
        assert_eq!(std::fs::read(path()).unwrap(), b"broken");
        crate::mods::test_app_home(old);
        let _ = std::fs::remove_dir_all(root);
    }
}
