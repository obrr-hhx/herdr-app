//! Read status from an existing Herdr attach client without touching its agent.
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::process::Stdio;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Source {
    pub binary: String,
    pub session: String,
    pub pane: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub word: String,
    pub app: String,
    pub title: String,
}
impl Source {
    pub fn from_record(record: &Value) -> Option<Self> {
        let argv = record
            .pointer("/foreground/argv")
            .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
            .or_else(|| record.get("argv"))?;
        let a: Vec<&str> = argv
            .as_array()?
            .iter()
            .map(Value::as_str)
            .collect::<Option<_>>()?;
        // hc wraps new terminals in a login shell. Never search arbitrary shell
        // text or SSH command strings for something that looks like an agent.
        let a = if a.get(2) == Some(&"\"$@\"; exec \"$0\" -li") {
            a.get(4..)?
        } else {
            &a[..]
        };
        if std::path::Path::new(a.first()?).file_name()?.to_str()? != "herdr" {
            return None;
        }
        let (session, i) = if a.get(1) == Some(&"--session") {
            ((*a.get(2)?).to_owned(), 3)
        } else {
            ("default".into(), 1)
        };
        if a.get(i..i + 2)? != ["agent", "attach"] {
            return None;
        }
        Some(Self {
            binary: a[0].into(),
            session,
            pane: (*a.get(i + 2)?).into(),
        })
    }
}
pub fn targets(live: &HashMap<String, Value>) -> BTreeMap<Source, String> {
    live.iter()
        .filter_map(|(address, record)| Some((Source::from_record(record)?, address.clone())))
        .collect()
}
fn decode(value: &Value, pane: &str) -> Option<Status> {
    let row = value
        .pointer("/result/agents")?
        .as_array()?
        .iter()
        .find(|a| a["pane_id"] == pane)?;
    let word = match row["agent_status"].as_str()? {
        "working" => "working",
        "idle" => "idle",
        "done" => "done",
        "blocked" => "blocked",
        "error" => "unavailable",
        _ => return None,
    };
    Some(Status {
        word: word.into(),
        app: row["agent"].as_str()?.into(),
        title: row["terminal_title_stripped"]
            .as_str()
            .unwrap_or_default()
            .into(),
    })
}
fn query(binary: &str, session: &str) -> Option<Value> {
    let mut child = crate::spawn::command(binary)
        .args(["--session", session, "agent", "list"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                break;
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut bytes = Vec::new();
    child
        .stdout
        .take()?
        .take(1024 * 1024)
        .read_to_end(&mut bytes)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}
pub fn poll(targets: BTreeMap<Source, String>) -> HashMap<String, Status> {
    let mut groups: BTreeMap<(String, String), Vec<(String, String)>> = BTreeMap::new();
    for (s, address) in targets {
        groups
            .entry((s.binary, s.session))
            .or_default()
            .push((s.pane, address));
    }
    let mut result = HashMap::new();
    for ((binary, session), panes) in groups {
        let value = query(&binary, &session)
            .filter(|v| v.pointer("/result/agents").is_some_and(Value::is_array));
        for (pane, address) in panes {
            let status = value.as_ref().and_then(|v| decode(v, &pane));
            if let Some(s) = status {
                result.insert(address, s);
            } else if value.is_none() {
                result.insert(
                    address,
                    Status {
                        word: "unavailable".into(),
                        app: "agent".into(),
                        title: "Agent status unavailable".into(),
                    },
                );
            }
        }
    }
    result
}
/// A failed status query is missing evidence, not a new request for user input.
/// Keep a prior confirmed status only while that source is still listed.
pub fn merge_statuses(
    incoming: HashMap<String, Status>,
    previous: &HashMap<String, Status>,
) -> HashMap<String, Status> {
    incoming
        .into_iter()
        .filter_map(|(address, status)| {
            if status.word == "unavailable" {
                previous.get(&address).cloned().map(|old| (address, old))
            } else {
                Some((address, status))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn attach_identity_is_exact_and_shells_or_ssh_are_not_agents() {
        let s=Source::from_record(&json!({"argv":["/bin/zsh","-lic","\"$@\"; exec \"$0\" -li","/bin/zsh","/home/u/herdr","--session","work","agent","attach","w2:p1","--takeover"]})).unwrap();
        assert_eq!((s.session.as_str(), s.pane.as_str()), ("work", "w2:p1"));
        assert!(
            Source::from_record(&json!({"argv":["ssh","vm","herdr","agent","attach","w2:p1"]}))
                .is_none()
        );
        assert!(
            Source::from_record(&json!({"argv":["herdr","terminal","attach","term1"]})).is_none()
        );
        assert!(
            Source::from_record(
                &json!({"argv":["herdr","agent","attach","p1"],"foreground":{"argv":["zsh"]}})
            )
            .is_none()
        );
    }
    #[test]
    fn failed_queries_do_not_invent_attention_or_completion() {
        let row = |word: &str| Status {
            word: word.into(),
            app: "codex".into(),
            title: "task".into(),
        };
        let old = HashMap::from([("a".into(), row("working"))]);
        let failed = HashMap::from([("a".into(), row("unavailable"))]);
        assert_eq!(merge_statuses(failed.clone(), &old)["a"].word, "working");
        assert!(merge_statuses(failed, &HashMap::new()).is_empty());
        assert!(merge_statuses(HashMap::new(), &old).is_empty());
        let approval = HashMap::from([("a".into(), row("blocked"))]);
        assert_eq!(merge_statuses(approval, &old)["a"].word, "blocked");
        let error =
            json!({"result":{"agents":[{"pane_id":"a","agent":"codex","agent_status":"error"}]}});
        assert_eq!(decode(&error, "a").unwrap().word, "unavailable");
    }
    #[test]
    fn status_follows_source_pane_instead_of_list_position() {
        let v = json!({"result":{"agents":[{"pane_id":"p2","agent":"codex","agent_status":"working","terminal_title_stripped":"Build"},{"pane_id":"p1","agent":"codex","agent_status":"blocked"}]}});
        assert_eq!(decode(&v, "p1").unwrap().word, "blocked");
        assert_eq!(decode(&v, "p2").unwrap().word, "working");
        assert!(decode(&v, "missing").is_none());
    }
}
