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
    #[cfg(test)]
    pub fn test_channel(outbox: PathBuf) -> (Self, mpsc::Receiver<Value>) {
        let (tx, rx) = mpsc::channel();
        (
            Self {
                tx,
                stopped: Arc::new(AtomicBool::new(false)),
                pending: Arc::new(Mutex::new(BTreeMap::new())),
                outbox,
            },
            rx,
        )
    }
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
                if config.host == "@local" {
                    if let Err(message) = install("@local") {
                        let _ = events.send_blocking(json!({"type":"transport","message":message}));
                        std::thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                }
                let _ = events.send_blocking(json!({"type":"transport","status":"connecting"}));
                let script = format!(
                    "exec node \"$HOME/.local/share/herdr-chat/client.mjs\" {} {} {} {}",
                    quote(&config.id),
                    quote(&config.folder),
                    quote(&config.model),
                    quote(config.resume.as_deref().unwrap_or_default())
                );
                let local = config.host == "@local";
                let local_root = local.then(local_service_root);
                let script = if let Some(root) = &local_root {
                    format!(
                        "export PATH=\"$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH\"; exec node {} {} {} {} {}",
                        quote(&root.join("client.mjs").to_string_lossy()),
                        quote(&config.id),
                        quote(&config.folder),
                        quote(&config.model),
                        quote(config.resume.as_deref().unwrap_or_default())
                    )
                } else {
                    script
                };
                let mut command = crate::spawn::command(if local { "bash" } else { "ssh" });
                if local {
                    command
                        .args(["-lc", &script])
                        .env("HERDR_CHAT_HOME", local_root.unwrap())
                        .env("HERDR_CHAT_LOCAL", "1")
                        .env("HERDR_APP_HOME", crate::mods::app_dir())
                        .env("HERDR_APP_BIN", std::env::current_exe().unwrap_or_default());
                    if let Some(host) = crate::settings::load_person()
                        .ok()
                        .and_then(|p| p.local_claude_auth_host)
                    {
                        command.env("HERDR_LOCAL_CLAUDE_AUTH_HOST", host);
                    }
                    if let Some(proxy) = crate::settings::load_person()
                        .ok()
                        .and_then(|p| p.local_chat_proxy)
                    {
                        let existing = std::env::var("NO_PROXY")
                            .or_else(|_| std::env::var("no_proxy"))
                            .unwrap_or_default();
                        let direct = format!(
                            "{existing},.corp.alva.xyz,.corp.stg.alva.xyz,forge.corp.stg.alva.xyz,bytebase-et.corp.alva.xyz,localhost,127.0.0.1"
                        );
                        command.env("NO_PROXY", &direct).env("no_proxy", &direct);
                        for key in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
                            command.env(key, &proxy);
                        }
                    }
                    if config.id == super::SUPER_ID {
                        command.env("HERDR_SUPER_AGENT", "1");
                    }
                } else {
                    command.args([
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
                    ]);
                }
                let child = command
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn();
                let Ok(mut child) = child else {
                    let _ = events
                        .send_blocking(json!({"type":"transport","message":if local {"Local chat service could not start"}else{"SSH could not start"}}));
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
                let write = |input: &mut std::process::ChildStdin,
                             v: &Value|
                 -> std::io::Result<()> {
                    if v["action"] == "send"
                        && v["images"].as_array().is_some_and(|a| !a.is_empty())
                    {
                        let _ = events
                            .send_blocking(json!({"type":"upload","message":"Uploading images…"}));
                        if let Err(message) = upload_images(&config, v) {
                            let _ =
                                events.send_blocking(json!({"type":"upload","message":message}));
                            return Err(std::io::Error::other("image upload failed"));
                        }
                    }
                    serde_json::to_writer(&mut *input, v)?;
                    input.write_all(b"\n")?;
                    input.flush()
                };
                let mut connected =
                    write(&mut input, &json!({"action":"subscribe","after":cursor})).is_ok();
                if connected {
                    let replay: Vec<_> = pending.lock().unwrap().values().cloned().collect();
                    for v in &replay {
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

fn upload_images(config: &ChatConfig, command: &Value) -> Result<(), String> {
    use base64::Engine as _;
    let ids = command["images"]
        .as_array()
        .ok_or("Invalid image attachments")?;
    if ids.len() > super::attachments::MAX_IMAGES {
        return Err("Too many images".into());
    }
    let mut entries = Vec::new();
    for id in ids {
        let id = id
            .as_str()
            .filter(|s| super::attachments::valid_id(s))
            .ok_or("Invalid image attachment")?;
        let bytes = std::fs::read(super::attachments::path(&local_dir(&config.id), id))
            .map_err(|_| "Image draft is missing; queued message is kept")?;
        if bytes.len() > super::attachments::MAX_BYTES {
            return Err("Image exceeds upload limit".into());
        }
        entries
            .push(json!({"id":id,"data":base64::engine::general_purpose::STANDARD.encode(bytes)}));
    }
    let remote = format!(
        "python3 \"$HOME/.local/share/herdr-chat/images.py\" {}",
        quote(&config.id)
    );
    let mut command = crate::spawn::command(if config.host == "@local" {
        "python3"
    } else {
        "ssh"
    });
    if config.host == "@local" {
        command
            .arg(local_service_root().join("images.py"))
            .arg(&config.id)
            .env("HERDR_CHAT_HOME", local_service_root());
    } else {
        command.args([
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
            &remote,
        ]);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Could not start image upload")?;
    let result = (|| -> Result<(), String> {
        let mut input = child.stdin.take().ok_or("Image upload unavailable")?;
        serde_json::to_writer(&mut input, &json!({"images":entries}))
            .map_err(|_| "Image upload failed")?;
        drop(input);
        for _ in 0..600 {
            if let Some(status) = child.try_wait().map_err(|_| "Image upload failed")? {
                return if status.success() {
                    Ok(())
                } else {
                    Err("Image upload failed; message is kept for retry".into())
                };
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err("Image upload timed out; message is kept for retry".into())
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

pub fn install(host: &str) -> Result<(), String> {
    use base64::Engine as _;
    if !valid_host(host) {
        return Err("Choose a valid SSH host".into());
    }
    let files = [
        ("goal_store.py", include_str!("../../bridge/goal_store.py")),
        ("goal_watch.py", include_str!("../../bridge/goal_watch.py")),
        (
            "memory_store.py",
            include_str!("../../bridge/memory_store.py"),
        ),
        (
            "memory_context.mjs",
            include_str!("../../bridge/memory_context.mjs"),
        ),
        ("checkins.py", include_str!("../../bridge/checkins.py")),
        ("checkins.mjs", include_str!("../../bridge/checkins.mjs")),
        ("codex_rpc.py", include_str!("../../bridge/codex_rpc.py")),
        (
            "codex_workers.py",
            include_str!("../../bridge/codex_workers.py"),
        ),
        (
            "codex_workers.mjs",
            include_str!("../../bridge/codex_workers.mjs"),
        ),
        ("redaction.py", include_str!("../../bridge/redaction.py")),
        (
            "schedule_store.py",
            include_str!("../../bridge/schedule_store.py"),
        ),
        ("scheduler.mjs", include_str!("../../bridge/scheduler.mjs")),
        (
            "session_reads.py",
            include_str!("../../bridge/session_reads.py"),
        ),
        ("local_auth.py", include_str!("../../bridge/local_auth.py")),
        ("codex.mjs", include_str!("../../bridge/codex.mjs")),
        (
            "supervisor.mjs",
            include_str!("../../bridge/supervisor.mjs"),
        ),
        ("supervisor.py", include_str!("../../bridge/supervisor.py")),
        (
            "session_release.py",
            include_str!("../../bridge/session_release.py"),
        ),
        (
            "runtime_release.py",
            include_str!("../../bridge/runtime_release.py"),
        ),
        (
            "supervisor-prompt.md",
            include_str!("../../bridge/supervisor-prompt.md"),
        ),
        ("package.json", include_str!("../../bridge/package.json")),
        ("journal.mjs", include_str!("../../bridge/journal.mjs")),
        ("display.mjs", include_str!("../../bridge/display.mjs")),
        ("statusline.py", include_str!("../../bridge/statusline.py")),
        (
            "statusline.mjs",
            include_str!("../../bridge/statusline.mjs"),
        ),
        ("upgrade.py", include_str!("../../bridge/upgrade.py")),
        ("workspace.mjs", include_str!("../../bridge/workspace.mjs")),
        ("import.mjs", include_str!("../../bridge/import.mjs")),
        ("daemon.mjs", include_str!("../../bridge/daemon.mjs")),
        ("steering.mjs", include_str!("../../bridge/steering.mjs")),
        (
            "memory_maintenance.mjs",
            include_str!("../../bridge/memory_maintenance.mjs"),
        ),
        (
            "upgrade_idle.py",
            include_str!("../../bridge/upgrade_idle.py"),
        ),
        ("client.mjs", include_str!("../../bridge/client.mjs")),
        ("runner.py", include_str!("../../bridge/runner.py")),
        ("hook.py", include_str!("../../bridge/hook.py")),
        ("images.py", include_str!("../../bridge/images.py")),
        ("images.mjs", include_str!("../../bridge/images.mjs")),
        (
            "image-client.mjs",
            include_str!("../../bridge/image-client.mjs"),
        ),
    ];
    if host == "@local" {
        static LOCAL_INSTALL: Mutex<()> = Mutex::new(());
        let _guard = LOCAL_INSTALL
            .lock()
            .map_err(|_| "Local install unavailable".to_owned())?;
        let root = local_service_root();
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let meta = std::fs::symlink_metadata(&root).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() || meta.uid() != unsafe { libc::geteuid() } {
                return Err("Local bridge path has an unexpected owner".into());
            }
        }
        let durable = crate::mods::app_dir().join("data/local-chat");
        std::fs::create_dir_all(&durable).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
            let link = root.join("conversations");
            if !link.exists() {
                match symlink(&durable, &link) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
            if std::fs::read_link(&link).map_err(|e| e.to_string())? != durable {
                return Err("Unexpected local conversation directory".into());
            }
        }
        for (name, body) in files {
            let tmp = root.join(format!("{name}.{}.new", super::fresh_id()));
            std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
            std::fs::rename(tmp, root.join(name)).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let mut script = String::from(
        "set -eu\nmkdir -p \"$HOME/.local/share/herdr-chat\"\nchmod 700 \"$HOME/.local/share/herdr-chat\"\ncd \"$HOME/.local/share/herdr-chat\"\n",
    );
    for (name, body) in files {
        script.push_str(&format!(
            "printf '%s' '{}' | base64 -d > {}.new\nmv -f {}.new {}\n",
            base64::engine::general_purpose::STANDARD.encode(body),
            quote(name),
            quote(name),
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

/// The caller has disabled sends while updating; the remote helper rechecks idleness.
pub fn upgrade(host: &str, id: &str) -> Result<(), String> {
    install(host)?;
    if id.len() < 16 || id.len() > 64 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid conversation identity".into());
    }
    let script = format!(
        "python3 \"$HOME/.local/share/herdr-chat/upgrade.py\" {}",
        quote(id)
    );
    let status = crate::spawn::command("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=8",
            "--",
            host,
            &format!("bash -lc {}", quote(&script)),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| "SSH update did not finish".to_owned())?;
    match status.code() {
        Some(0) => Ok(()),
        Some(3) => Err("Finish the current turn or approval before updating this chat.".into()),
        _ => Err("The service could not be restarted. Its conversation history is kept.".into()),
    }
}

/// A short private path keeps Unix sockets below macOS's 104-byte limit.
pub fn local_service_root() -> PathBuf {
    use sha2::{Digest, Sha256};
    let home = crate::mods::app_dir();
    let hash = format!("{:x}", Sha256::digest(home.to_string_lossy().as_bytes()));
    std::path::PathBuf::from("/tmp").join(format!("hchat-{}", &hash[..16]))
}

/// Direct local UI edits share the manager's transactional memory store, without a model turn.
pub fn edit_memory(home: &Path, args: &Value) -> Result<Vec<Value>, String> {
    let runtime = home.join("data/super-agent/editor-runtime");
    std::fs::create_dir_all(&runtime).map_err(|_| "无法打开记忆存储".to_owned())?;
    for (name, source) in [
        (
            "memory_store.py",
            include_str!("../../bridge/memory_store.py"),
        ),
        ("goal_store.py", include_str!("../../bridge/goal_store.py")),
        ("goal_watch.py", include_str!("../../bridge/goal_watch.py")),
        ("redaction.py", include_str!("../../bridge/redaction.py")),
    ] {
        std::fs::write(runtime.join(name), source).map_err(|_| "无法准备记忆存储".to_owned())?;
    }
    let mut child = std::process::Command::new("python3")
        .arg(runtime.join("memory_store.py"))
        .arg("--manual")
        .arg(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "无法启动记忆存储，草稿已保留".to_owned())?;
    if let Some(mut input) = child.stdin.take() {
        serde_json::to_writer(&mut input, args)
            .map_err(|_| "无法写入记忆，草稿已保留".to_owned())?;
    }
    let output = child
        .wait_with_output()
        .map_err(|_| "记忆存储未确认，草稿已保留".to_owned())?;
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "记忆存储未确认，草稿已保留".to_owned())?;
    if !output.status.success() {
        return Err(value["error"]
            .as_str()
            .unwrap_or("保存失败，草稿已保留")
            .to_owned());
    }
    value["memories"]
        .as_array()
        .cloned()
        .ok_or_else(|| "记忆存储未确认，草稿已保留".into())
}

/// Delete a goal from human UI without stopping its worker or deleting evidence.
pub fn delete_goal(home: &Path, id: &str, revision: u64) -> Result<Vec<Value>, String> {
    let runtime = home.join("data/super-agent/goal-editor-runtime");
    std::fs::create_dir_all(&runtime).map_err(|e| e.to_string())?;
    for (name, source) in [
        ("goal_store.py", include_str!("../../bridge/goal_store.py")),
        ("redaction.py", include_str!("../../bridge/redaction.py")),
    ] {
        std::fs::write(runtime.join(name), source).map_err(|e| e.to_string())?;
    }
    let mut child = std::process::Command::new("python3")
        .arg(runtime.join("goal_store.py"))
        .arg("--delete-manual")
        .arg(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    serde_json::to_writer(
        child.stdin.take().unwrap(),
        &json!({"id":id,"expected_revision":revision}),
    )
    .map_err(|e| e.to_string())?;
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(value["error"].as_str().unwrap_or("删除失败").to_owned());
    }
    value["goals"]
        .as_array()
        .cloned()
        .ok_or_else(|| "目标列表未确认".to_owned())
}

#[cfg(test)]
mod memory_editor_tests {
    use super::*;
    #[test]
    fn local_memory_editor_saves_updates_deletes_offline_and_rejects_stale_edits() {
        let home =
            std::env::temp_dir().join(format!("herdr-manual-memory-{}", super::super::fresh_id()));
        let a = json!({"action":"save","id":"personal.fixture","expected_revision":0,"title":"先给结论","content":"需要核实证据"});
        let rows = edit_memory(&home, &a).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["authority"], false);
        assert!(edit_memory(&home, &a).is_err());
        let mut a = a;
        a["expected_revision"] = json!(1);
        a["content"] = json!("简短报告，附上证据");
        assert_eq!(edit_memory(&home, &a).unwrap()[0]["revision"], 2);
        assert!(
            edit_memory(
                &home,
                &json!({"action":"delete","id":"personal.fixture","expected_revision":1})
            )
            .is_err()
        );
        assert!(
            edit_memory(
                &home,
                &json!({"action":"delete","id":"personal.fixture","expected_revision":2})
            )
            .unwrap()
            .is_empty()
        );
        std::fs::remove_dir_all(home).unwrap();
    }
}
