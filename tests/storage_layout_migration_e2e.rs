//! `storage.migrate` (dry run) and `storage.status` through the real
//! `openhuman-core` binary, against a layout-v1 profile built in a tempdir.
//!
//! SAFETY: `HOME` and `USERPROFILE` point at the tempdir, so `~/.openhuman` is
//! `<tmp>/.openhuman`; nothing here can reach the developer's real profile.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn call(home: &Path, method: &str, params: &str) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .args(["call", "--method", method, "--params", params])
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("OPENHUMAN_KEYRING_MASTER_KEY", "22".repeat(32))
        .env_remove("OPENHUMAN_KEYRING_MASTER_KEY_FILE")
        .env_remove("OPENHUMAN_WORKSPACE")
        .env_remove("OPENHUMAN_STORAGE_URL")
        .output()
        .expect("run openhuman-core");
    assert!(
        output.status.success(),
        "{method} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let start = stdout.find('{').expect("a JSON object on stdout");
    let value: Value = serde_json::from_str(&stdout[start..]).expect("JSON result");
    // A log envelope wraps the payload as {result: {...}, logs: [...]}.
    match value.get("result") {
        Some(inner) if inner.is_object() => inner.clone(),
        _ => value,
    }
}

/// `<home>/.openhuman/users/local` with a v1 workspace; returns the profile dir.
fn v1_profile(home: &Path) -> std::path::PathBuf {
    let profile = home.join(".openhuman").join("users").join("local");
    write(&profile.join("config.toml"), "");
    let ws = profile.join("workspace");
    write(&ws.join("skills/s1/SKILL.md"), "# skill\n");
    write(&ws.join("STYLE.md"), "brief\n");
    write(&ws.join("memory/jobs.json"), "{}");
    write(&ws.join("mystery/x.bin"), "???");
    profile
}

fn digest(root: &Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let len = entry.metadata().unwrap().len();
                let rel = path.strip_prefix(root).unwrap().display().to_string();
                // The binary's own boot writes its config and the v1 database;
                // what must not change is everything else.
                if rel.starts_with("config.toml") || rel.starts_with("workspace/storage") {
                    continue;
                }
                out.push((rel, len));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn migrate_dry_run_plans_a_v1_profile_and_writes_nothing() {
    let home = tempfile::tempdir().unwrap();
    let profile = v1_profile(home.path());
    let before = digest(&profile);

    let plan = call(
        home.path(),
        "openhuman.storage_migrate",
        r#"{"dry_run":true}"#,
    );
    assert_eq!(plan["result"], "dry_run", "{plan}");
    let paths = |key: &str| -> Vec<String> {
        plan[key]
            .as_array()
            .unwrap_or_else(|| panic!("{key} in {plan}"))
            .iter()
            .map(|e| e["path"].as_str().unwrap().to_owned())
            .collect()
    };
    assert!(paths("move_entries").contains(&"skills".to_owned()));
    assert!(paths("delete_entries").contains(&"memory/jobs.json".to_owned()));
    assert_eq!(paths("unknown_entries"), vec!["mystery".to_owned()]);
    assert!(plan["archive"]["bytes"].as_u64().unwrap() > 0);
    assert_eq!(digest(&profile), before, "a dry run writes nothing");
    assert!(!home.path().join(".openhuman/backups").exists());
    for created in [
        ".layout",
        ".layout.lock",
        ".layout.state",
        "storage.db",
        "files",
    ] {
        assert!(
            !profile.join(created).exists(),
            "{created} must not exist after a dry run"
        );
    }
}

#[test]
fn status_reports_layout_v1_then_nothing_changes() {
    let home = tempfile::tempdir().unwrap();
    let profile = v1_profile(home.path());
    let before = digest(&profile);
    let status = call(home.path(), "openhuman.storage_status", "{}");
    assert_eq!(status["layout_version"], 1, "{status}");
    assert_eq!(status["migration_in_progress"], false);
    assert_eq!(status["explicit_url"], false);
    assert_eq!(digest(&profile), before, "status is read-only");
}
