//! Test what the built binary actually ships, rather than the source directory.
#[cfg(unix)]
#[test]
fn installed_management_bridge_contains_its_runtime_dependencies() {
    use sha2::{Digest, Sha256};
    use std::process::Command;
    let home = std::env::temp_dir().join(format!(
        "herdr-bridge-assets-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let hash = format!("{:x}", Sha256::digest(home.to_string_lossy().as_bytes()));
    let root = std::path::PathBuf::from("/tmp").join(format!("hchat-{}", &hash[..16]));
    let bin = env!("CARGO_BIN_EXE_herdr-app");
    let result = Command::new(bin)
        .args(["chat-install", "@local"])
        .env("HERDR_APP_HOME", &home)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for name in [
        "supervisor.py",
        "steering.mjs",
        "upgrade_idle.py",
        "goal_store.py",
        "memory_store.py",
        "memory_context.mjs",
        "memory_maintenance.mjs",
        "session_release.py",
        "runtime_release.py",
        "scheduler.mjs",
        "schedule_store.py",
        "session_reads.py",
        "local_auth.py",
        "codex_rpc.py",
        "codex_workers.py",
        "codex_workers.mjs",
        "redaction.py",
        "checkins.py",
        "checkins.mjs",
    ] {
        assert!(
            root.join(name).is_file(),
            "installed runtime dependency missing: {name}"
        );
    }
    if Command::new("python3").arg("--version").output().is_ok() {
        let tools = Command::new("python3")
            .arg(root.join("supervisor.py"))
            .arg("--tools")
            .env("HERDR_APP_HOME", &home)
            .env("HERDR_APP_BIN", bin)
            .output()
            .unwrap();
        assert!(
            tools.status.success(),
            "{}",
            String::from_utf8_lossy(&tools.stderr)
        );
        let tools: serde_json::Value = serde_json::from_slice(&tools.stdout).unwrap();
        assert!(
            tools
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["name"] == "schedule_reminder")
        );
    }
    if Command::new("node").arg("--version").output().is_ok() {
        let import = format!(
            "Promise.all([import({}),import({})])",
            serde_json::to_string(&root.join("scheduler.mjs").to_string_lossy()).unwrap(),
            serde_json::to_string(&root.join("codex_workers.mjs").to_string_lossy()).unwrap()
        );
        let result = Command::new("node")
            .args(["--input-type=module", "-e", &import])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(home).unwrap();
}
