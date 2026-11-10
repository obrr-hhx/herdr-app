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
                    || ![
                        Some("pending"),
                        Some("answered"),
                        Some("resolved"),
                        Some("superseded"),
                        Some("withdrawn"),
                    ]
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
/// Only pending/answered cards belong in the human inbox.
pub fn needs_attention(row: &Value) -> bool {
    matches!(row["status"].as_str(), Some("pending" | "answered"))
}
fn strings(value: &Value, max: usize) -> Result<Vec<String>, String> {
    if value.is_null() {
        return Ok(vec![]);
    }
    let a = value.as_array().ok_or("Expected a list of identifiers")?;
    if a.len() > max
        || a.iter().any(|s| {
            s.as_str()
                .is_none_or(|s| s.trim().is_empty() || s.len() > 200)
        })
    {
        return Err("Invalid decision identifiers".into());
    }
    Ok(a.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
}
fn sources(row: &Value) -> Vec<String> {
    let mut ids = strings(&row["source_ids"], 24).unwrap_or_default();
    // Old cards stored references as prose. Exact message IDs are candidates,
    // never proof that two different questions in one message are identical.
    for token in row["evidence"]
        .as_str()
        .unwrap_or("")
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
    {
        if token.starts_with("msg_") && token.len() >= 12 {
            ids.push(token.to_owned());
        }
    }
    ids.sort();
    ids.dedup();
    ids
}
fn canonical(rows: &[Value], mut i: usize) -> Result<usize, String> {
    for _ in 0..rows.len() {
        if rows[i]["status"] != "superseded" {
            return Ok(i);
        }
        let target = &rows[i]["merged_into"];
        i = rows
            .iter()
            .position(|r| &r["id"] == target)
            .ok_or("Merged decision target missing")?;
    }
    Err("Decision alias cycle".into())
}
/// Stable semantic keys deduplicate renames; shared evidence requires agent judgement.
pub fn record(a: &Value) -> Result<Value, String> {
    for (key, max) in [
        ("id", 160),
        ("title", 500),
        ("context", 12000),
        ("address", 256),
    ] {
        if a[key]
            .as_str()
            .is_none_or(|s| s.trim().is_empty() || s.len() > max)
        {
            return Err(format!("Invalid {key}"));
        }
    }
    for (key, max) in [("evidence", 4000), ("reason", 2000), ("decision_key", 160)] {
        if let Some(v) = a.get(key) {
            if v.as_str()
                .is_none_or(|s| (key != "evidence" && s.trim().is_empty()) || s.len() > max)
            {
                return Err(format!("Invalid {key}"));
            }
        }
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
    let source_ids = strings(&a["source_ids"], 12)?;
    let distinct = strings(&a["distinct_from"], 12)?;
    let mut rows = read()?;
    let by_id = rows.iter().position(|r| {
        r["id"] == a["id"]
            || r["aliases"]
                .as_array()
                .is_some_and(|v| v.contains(&a["id"]))
    });
    if by_id.is_some_and(|i| rows[i]["address"] != a["address"]) {
        return Err("Decision ID belongs to another session".into());
    }
    let by_key = a["decision_key"].as_str().and_then(|key| {
        rows.iter().position(|r| {
            r["address"] == a["address"] && r["decision_key"] == key && r["status"] != "superseded"
        })
    });
    let index = by_id.or(by_key).map(|i| canonical(&rows, i)).transpose()?;
    let old = index.map(|i| rows[i].clone()).unwrap_or(json!({}));
    if by_id.is_some_and(|i| rows[i]["status"] == "superseded") {
        return Ok(old);
    }
    if index.is_some()
        && old["decision_key"].is_string()
        && a["decision_key"].is_string()
        && old["decision_key"] != a["decision_key"]
    {
        return Err("Existing ID names another decision; use its original decision_key".into());
    }
    if index.is_none() {
        let refs = sources(a);
        let conflicts: Vec<_> = rows
            .iter()
            .filter(|r| {
                r["address"] == a["address"]
                    && r["status"] != "superseded"
                    && sources(r).iter().any(|id| refs.contains(id))
                    && !distinct
                        .iter()
                        .any(|id| r["id"].as_str() == Some(id.as_str()))
            })
            .map(|r| r["id"].as_str().unwrap_or_default())
            .collect();
        if !conflicts.is_empty() {
            return Err(format!(
                "Existing cards share source evidence: {}. Read them; reuse/merge the same decision. For genuinely separate questions use distinct_from and explain the distinction in reason.",
                conflicts.join(", ")
            ));
        }
        if !distinct.is_empty() && a["reason"].as_str().is_none_or(|s| s.trim().is_empty()) {
            return Err("Explain why these are separate human decisions".into());
        }
    }
    // An observation never revises a question the person already answered/handled.
    if index.is_some() && old["status"] != "pending" {
        return Ok(old);
    }
    let now = crate::model::now_ms();
    let mut row = if index.is_some() {
        old.clone()
    } else {
        json!({"id":a["id"],"status":"pending","answer":null,"request_id":null,"created_at":now})
    };
    for key in ["title", "context", "address"] {
        row[key] = a[key].clone();
    }
    row["options"] = options;
    row["evidence"] = a["evidence"]
        .as_str()
        .map(|s| json!(s))
        .unwrap_or(json!(""));
    for key in ["decision_key", "reason"] {
        if a.get(key).is_some() {
            row[key] = a[key].clone();
        }
    }
    let mut refs = sources(&old);
    refs.extend(source_ids);
    refs.sort();
    refs.dedup();
    if refs.len() > 24 {
        return Err("Too many source identifiers; retain bounded relevant evidence".into());
    }
    row["source_ids"] = json!(refs);
    if row["id"] != a["id"] {
        let mut aliases = strings(&old["aliases"], 64)?;
        let id = a["id"].as_str().unwrap().to_owned();
        if !aliases.contains(&id) {
            aliases.push(id);
        }
        if aliases.len() > 64 {
            return Err("Reuse the returned canonical decision ID".into());
        }
        row["aliases"] = json!(aliases);
    }
    if row == old {
        return Ok(old);
    }
    row["updated_at"] = json!(now);
    if let Some(i) = index {
        rows[i] = row.clone();
    } else {
        rows.push(row.clone());
    }
    save(&rows)?;
    Ok(row)
}
/// Agent curation only changes inbox presentation, never user acceptance or worker state.
pub fn curate(a: &Value) -> Result<Value, String> {
    let id = a["id"].as_str().ok_or("Missing keeper ID")?;
    let reason = a["reason"]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 2000)
        .ok_or("Explain why this card should be merged/withdrawn")?;
    let mut rows = read()?;
    let i = rows
        .iter()
        .position(|r| r["id"].as_str() == Some(id))
        .ok_or("Decision not found")?;
    // Retry returns the same archival receipt without touching human state.
    if a["action"] == "withdraw" && rows[i]["status"] == "withdrawn" {
        return Ok(rows[i].clone());
    }
    let ids = strings(&a["duplicate_ids"], 12)?;
    if a["action"] == "merge"
        && !ids.is_empty()
        && ids.iter().all(|id| {
            rows.iter().any(|r| {
                r["id"] == *id && r["status"] == "superseded" && r["merged_into"] == rows[i]["id"]
            })
        })
    {
        return Ok(rows[i].clone());
    }
    if rows[i]["status"] != "pending" || rows[i]["answer"].is_string() {
        return Err("Cannot curate a card the human already answered/handled; preserve it".into());
    }
    let now = crate::model::now_ms();
    match a["action"].as_str() {
        Some("withdraw") => {
            if !ids.is_empty() {
                return Err("Withdrawal does not merge other cards".into());
            }
            rows[i]["status"] = json!("withdrawn");
        }
        Some("merge") => {
            if ids.is_empty() || ids.iter().any(|v| v == id) {
                return Err("Choose other duplicate IDs to merge into the keeper".into());
            }
            let mut indices = vec![];
            for id in ids {
                let j = rows
                    .iter()
                    .position(|r| r["id"].as_str() == Some(id.as_str()))
                    .ok_or("Duplicate decision not found")?;
                if rows[j]["address"] != rows[i]["address"]
                    || rows[j]["status"] != "pending"
                    || rows[j]["answer"].is_string()
                {
                    return Err(
                        "Only unanswered pending duplicates from the same session can be merged"
                            .into(),
                    );
                }
                indices.push(j);
            }
            let mut refs = sources(&rows[i]);
            for j in indices {
                refs.extend(sources(&rows[j]));
                rows[j]["status"] = json!("superseded");
                rows[j]["merged_into"] = json!(id);
                rows[j]["curation_reason"] = json!(reason);
                rows[j]["updated_at"] = json!(now);
            }
            refs.sort();
            refs.dedup();
            if refs.len() > 24 {
                return Err("Too many merged source identifiers".into());
            }
            rows[i]["source_ids"] = json!(refs);
        }
        _ => return Err("Use merge or withdraw; curation cannot accept work".into()),
    }
    rows[i]["curation_reason"] = json!(reason);
    rows[i]["updated_at"] = json!(now);
    let result = rows[i].clone();
    save(&rows)?;
    Ok(result)
}

pub fn update(id: &str, status: &str, answer: Option<(&str, &str)>) -> Result<Value, String> {
    if !["pending", "answered", "resolved"].contains(&status) {
        return Err("Invalid decision state".into());
    }
    let mut rows = read()?;
    let row = rows
        .iter_mut()
        .find(|r| r["id"].as_str() == Some(id))
        .ok_or("Decision not found")?;
    if row["status"] == "superseded" {
        return Err("This card was merged; open its canonical decision".into());
    }
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
    fn with_home(test: impl FnOnce()) {
        let root = std::env::temp_dir().join(format!("herdr-curation-{}", crate::chat::fresh_id()));
        let previous = crate::mods::test_app_home(Some(root.clone()));
        test();
        crate::mods::test_app_home(previous);
        let _ = std::fs::remove_dir_all(root);
    }
    fn card(id: &str, key: &str) -> Value {
        json!({"id":id,"decision_key":key,"title":"Choose next step","context":"A concrete human question","address":"t9","reason":"Two mutually exclusive authorized scopes require the owner's choice","source_ids":["msg_fixture_question_0001"],"options":["Keep current scope","Pause"]})
    }
    #[test]
    fn stable_topic_reuses_one_card_across_titles_and_preserves_human_answer() {
        with_home(|| {
            let a = card("first-id", "release-conflict");
            record(&a).unwrap();
            let mut b = card("different-id", "release-conflict");
            b["title"] = json!("Renamed same decision");
            assert_eq!(record(&b).unwrap()["id"], "first-id");
            assert_eq!(read().unwrap().len(), 1);
            update(
                "first-id",
                "answered",
                Some(("Keep current scope", "human-reply")),
            )
            .unwrap();
            b["context"] = json!("Changed after the human answered");
            let result = record(&b).unwrap();
            assert_eq!(result["answer"], "Keep current scope");
            assert_ne!(result["context"], b["context"]);
        });
    }
    #[test]
    fn common_source_requires_agent_judgement_and_allows_separate_questions() {
        with_home(|| {
            let mut a = card("legacy-id", "first-choice");
            a.as_object_mut().unwrap().remove("source_ids");
            a["evidence"] = json!("t9 · msg_fixture_question_0001");
            record(&a).unwrap();
            let mut b = card("new-id", "other-choice");
            assert!(record(&b).unwrap_err().contains("legacy-id"));
            assert_eq!(read().unwrap().len(), 1);
            b["distinct_from"] = json!(["legacy-id"]);
            b["reason"] =
                json!("The same message asks two independent choices: scope and scheduling");
            record(&b).unwrap();
            assert_eq!(read().unwrap().len(), 2);
        });
    }
    #[test]
    fn agent_merge_and_withdraw_archive_originals_without_accepting_work() {
        with_home(|| {
            let a = record(&card("clear-question", "scope-choice")).unwrap();
            let mut duplicate = a.clone();
            duplicate["id"] = json!("old-auto-task");
            duplicate["context"] = json!("Old automatic task summary");
            duplicate["options"] = json!([]);
            save(&[a.clone(), duplicate.clone()]).unwrap();
            let merge = json!({"action":"merge","id":"clear-question","duplicate_ids":["old-auto-task"],"reason":"The original question and task summary ask for the same choice"});
            assert_eq!(curate(&merge).unwrap()["status"], "pending");
            curate(&merge).unwrap();
            let rows = read().unwrap();
            assert_eq!(rows.iter().filter(|r| needs_attention(r)).count(), 1);
            assert_eq!(rows[1]["context"], duplicate["context"]);
            assert_eq!(rows[1]["status"], "superseded");
            duplicate["title"] = json!("Replay of old task card");
            assert_eq!(record(&duplicate).unwrap()["id"], "clear-question");
            assert_eq!(read().unwrap()[0]["options"], a["options"]);
            let withdraw = json!({"action":"withdraw","id":"clear-question","reason":"Current evidence shows the human has already authorized this scope"});
            assert_eq!(curate(&withdraw).unwrap()["status"], "withdrawn");
            assert!(!read().unwrap().iter().any(needs_attention));
            assert_eq!(
                record(&card("another-id", "scope-choice")).unwrap()["status"],
                "withdrawn"
            );
            update("clear-question", "pending", None).unwrap();
            update("clear-question", "answered", Some(("Pause", "reply"))).unwrap();
            assert!(curate(&withdraw).is_err());
            assert!(
                curate(&json!({"action":"withdraw","id":"old-auto-task","reason":"stale"}))
                    .is_err()
            );
            assert_eq!(read().unwrap()[0]["answer"], "Pause");
        });
    }
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
