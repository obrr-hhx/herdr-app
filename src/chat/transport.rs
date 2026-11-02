//! SSH transports structured events. Its lifetime never owns the remote interactive Claude process.
use super::ChatConfig;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

pub fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('-')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-@:".contains(&b))
}
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
pub fn local_dir(id: &str) -> PathBuf {
    crate::mods::app_dir().join("data/forge-chat").join(id)
}
pub fn load_history(path: &Path) -> std::io::Result<Vec<Value>> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let end = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let mut events = Vec::new();
    for line in bytes[..end]
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
    {
        let e: Value = serde_json::from_slice(line)?;
        if e["seq"].as_u64() != Some(events.len() as u64 + 1) {
            return Err(std::io::Error::other("History sequence gap"));
        }
        events.push(e);
    }
    if end < bytes.len() {
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)?
            .set_len(end as u64)?;
    }
    Ok(events)
}
fn persist(path: &Path, value: &impl serde::Serialize) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp)?;
    serde_json::to_writer(&mut f, value)?;
    f.sync_all()?;
    std::fs::rename(tmp, path)
}
pub struct Connection {
    tx: mpsc::Sender<Value>,
    stopped: Arc<AtomicBool>,
    pending: Arc<Mutex<BTreeMap<String, Value>>>,
    outbox: PathBuf,
}
impl Connection {
    pub fn send(&self, v: Value) -> bool {
        if self.stopped.load(Ordering::Acquire) {
            return false;
        }
        let Some(id) = v["request_id"].as_str() else {
            return false;
        };
        let Ok(mut pending) = self.pending.lock() else {
            return false;
        };
        pending.insert(id.to_owned(), v.clone());
        if persist(&self.outbox, &*pending).is_err() {
            pending.remove(id);
            return false;
        }
        // Persistence is acceptance locally, including while SSH reconnects.
        let _ = self.tx.send(v);
        true
    }
    pub fn start(config: ChatConfig, cursor: u64) -> (Self, async_channel::Receiver<Value>) {
        let (tx, rx) = mpsc::channel::<Value>();
        let (events, out) = async_channel::bounded(1024);
        let dir = local_dir(&config.id);
        let _ = std::fs::create_dir_all(&dir);
        let outbox = dir.join("outbox.json");
        let pending: Arc<Mutex<BTreeMap<String, Value>>> = Arc::new(Mutex::new(
            std::fs::read(&outbox)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default(),
        ));
        let worker_pending = pending.clone();
        let worker_outbox = outbox.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        std::thread::spawn(move || {
            let dir = local_dir(&config.id);
            let _ = std::fs::create_dir_all(&dir);
            let pending = worker_pending;
            let outbox = worker_outbox;
            let mut cursor = cursor;
            while !stop.load(Ordering::Acquire) {
                let _ = events.send_blocking(json!({"type":"transport","status":"connecting"}));
                let script = format!(
                    "exec node \"$HOME/.local/share/herdr-chat/client.mjs\" {} {} {}",
                    quote(&config.id),
                    quote(&config.folder),
                    quote(&config.model)
                );
                let mut command = crate::spawn::command("ssh");
                let child = command
                    .args([
                        "-o",
                        "BatchMode=yes",
                        "-o",
                        "ConnectTimeout=8",
                        "-o",
                        "ServerAliveInterval=10",
                        "-o",
                        "ServerAliveCountMax=3",
                        "--",
                        &config.host,
                        &format!("bash -lc {}", quote(&script)),
                    ])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn();
                let Ok(mut child) = child else {
                    let _ = events
                        .send_blocking(json!({"type":"transport","message":"SSH could not start"}));
                    break;
                };
                let (Some(mut input), Some(output)) = (child.stdin.take(), child.stdout.take())
                else {
                    let _ = child.kill();
                    break;
                };
                let (incoming, received) = mpsc::sync_channel::<Value>(1024);
                let read_stop = stop.clone();
                let reader = std::thread::spawn(move || {
                    let mut reader = BufReader::new(output);
                    loop {
                        let mut bytes = Vec::new();
                        match reader
                            .by_ref()
                            .take(1024 * 1024 + 1)
                            .read_until(b'\n', &mut bytes)
                        {
                            Ok(0) | Err(_) => break,
                            Ok(_) if bytes.len() > 1024 * 1024 => break,
                            Ok(_) => {
                                if read_stop.load(Ordering::Acquire) {
                                    break;
                                }
                                let Ok(v) = serde_json::from_slice(&bytes) else {
                                    break;
                                };
                                if incoming.send(v).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
                let write =
                    |input: &mut std::process::ChildStdin, v: &Value| -> std::io::Result<()> {
                        serde_json::to_writer(&mut *input, v)?;
                        input.write_all(b"\n")?;
                        input.flush()
                    };
                let mut connected =
                    write(&mut input, &json!({"action":"subscribe","after":cursor})).is_ok();
                if connected {
                    for v in pending.lock().unwrap().values() {
                        if write(&mut input, v).is_err() {
                            connected = false;
                            break;
                        }
                    }
                }
                while connected && !stop.load(Ordering::Acquire) {
                    while let Ok(v) = rx.try_recv() {
                        if write(&mut input, &v).is_err() {
                            connected = false;
                            break;
                        }
                    }
                    match received.recv_timeout(Duration::from_millis(25)) {
                        Ok(v) => {
                            if v["type"] == "ack" {
                                if let Some(id) = v["request_id"].as_str() {
                                    let mut pending = pending.lock().unwrap();
                                    pending.remove(id);
                                    let _ = persist(&outbox, &*pending);
                                }
                                continue;
                            }
                            if let Some(seq) = v["seq"].as_u64() {
                                if seq <= cursor {
                                    continue;
                                }
                                if seq != cursor + 1 {
                                    connected = false;
                                    continue;
                                }
                                let line = format!("{v}\n");
                                let saved = std::fs::OpenOptions::new()
                                    .create(true)
                                    .append(true)
                                    .open(dir.join("events.jsonl"))
                                    .and_then(|mut f| f.write_all(line.as_bytes()));
                                if saved.is_err() {
                                    let _=events.send_blocking(json!({"type":"transport","message":"Cannot save received history"}));
                                    connected = false;
                                    continue;
                                }
                                cursor = seq;
                            }
                            if events.send_blocking(v).is_err() {
                                connected = false;
                                stop.store(true, Ordering::Release);
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => connected = false,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if child.try_wait().ok().flatten().is_some() {
                                connected = false;
                            }
                        }
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                drop(received);
                let _ = reader.join();
                if !stop.load(Ordering::Acquire) {
                    let _ = events.send_blocking(
                        json!({"type":"transport","message":"SSH disconnected — reconnecting"}),
                    );
                    for _ in 0..20 {
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            }
            stop.store(true, Ordering::Release);
        });
        (
            Self {
                tx,
                stopped,
                pending,
                outbox,
            },
            out,
        )
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}

pub fn install(host: &str) -> Result<(), String> {
    use base64::Engine as _;
    if !valid_host(host) {
        return Err("Choose a valid SSH host".into());
    }
    let files = [
        ("package.json", include_str!("../../bridge/package.json")),
        ("journal.mjs", include_str!("../../bridge/journal.mjs")),
        ("display.mjs", include_str!("../../bridge/display.mjs")),
        ("daemon.mjs", include_str!("../../bridge/daemon.mjs")),
        ("client.mjs", include_str!("../../bridge/client.mjs")),
        ("runner.py", include_str!("../../bridge/runner.py")),
        ("hook.py", include_str!("../../bridge/hook.py")),
    ];
    let mut script = String::from(
        "set -eu\nmkdir -p \"$HOME/.local/share/herdr-chat\"\nchmod 700 \"$HOME/.local/share/herdr-chat\"\ncd \"$HOME/.local/share/herdr-chat\"\n",
    );
    for (name, body) in files {
        script.push_str(&format!(
            "printf '%s' '{}' | base64 -d > {}\n",
            base64::engine::general_purpose::STANDARD.encode(body),
            quote(name)
        ));
    }
    script.push_str(
        "command -v node >/dev/null\ncommand -v python3 >/dev/null\ncommand -v claude >/dev/null\n",
    );
    let mut child = crate::spawn::command("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=8",
            "--",
            host,
            "bash -l -s",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "SSH could not start")?;
    if let Some(mut input) = child.stdin.take() {
        input
            .write_all(script.as_bytes())
            .map_err(|_| "Bridge installation disconnected")?;
    }
    if child
        .wait()
        .map_err(|_| "Bridge installation stopped")?
        .success()
    {
        Ok(())
    } else {
        Err("Could not install bridge. Check SSH and Node.js, Python 3 and Claude Code on this machine.".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_repairs_only_a_torn_tail_and_outgoing_is_saved_before_dispatch() {
        let dir =
            std::env::temp_dir().join(format!("herdr-chat-durable-{}", super::super::fresh_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("events.jsonl");
        std::fs::write(&file, b"{\"seq\":1,\"type\":\"state\"}\n{partial").unwrap();
        assert_eq!(load_history(&file).unwrap().len(), 1);
        assert!(std::fs::read(&file).unwrap().ends_with(b"\n"));
        std::fs::write(&file, b"invalid\n").unwrap();
        assert!(load_history(&file).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"invalid\n");
        let (tx, _rx) = mpsc::channel();
        let c = Connection {
            tx,
            stopped: Arc::new(AtomicBool::new(false)),
            pending: Arc::new(Mutex::new(BTreeMap::new())),
            outbox: dir.join("outbox.json"),
        };
        assert!(c.send(json!({"action":"send","request_id":"durable-1","text":"draft"})));
        let saved: Value = serde_json::from_slice(&std::fs::read(&c.outbox).unwrap()).unwrap();
        assert_eq!(saved["durable-1"]["text"], "draft");
        std::fs::remove_file(&c.outbox).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            !c.send(json!({"action":"send","request_id":"durable-2","text":"keep this draft"}))
        );
    }
    #[test]
    fn host_and_shell_arguments_cannot_inject_commands() {
        for s in ["-oProxyCommand=evil", "host;rm", "host\nother", ""] {
            assert!(!valid_host(s));
        }
        assert!(valid_host("forge@vm-1"));
        assert_eq!(quote("a'b"), "'a'\\''b'");
    }
}
