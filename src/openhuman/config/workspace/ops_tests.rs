use super::*;
use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;
use tempfile::tempdir;

/// RAII guard for `OPENHUMAN_WORKSPACE`. Sets the env var on
/// construction and clears it on drop so a panicking test doesn't
/// leak the override into sibling tests. Must be constructed while
/// holding `ENV_LOCK` — mutating process env vars concurrently is
/// unsafe and the lock serialises every test in this module.
struct WorkspaceEnvGuard;

impl WorkspaceEnvGuard {
    fn set(path: &std::path::Path) -> Self {
        // SAFETY: Caller holds `ENV_LOCK`, so no other thread in
        // this process is reading or mutating this env var.
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", path);
        }
        Self
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        // SAFETY: Same contract as `set()` — `ENV_LOCK` is held for
        // the whole test, so no concurrent env access is possible.
        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }
}

// ── ensure_workspace_file ──────────────────────────────────────

#[test]
fn ensure_workspace_file_creates_missing_file() {
    let tmp = tempdir().unwrap();
    let status = ensure_workspace_file(tmp.path(), "A.md", "hello", false).expect("should create");
    assert_eq!(status, "created");
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("A.md")).unwrap(),
        "hello"
    );
}

#[test]
fn ensure_workspace_file_leaves_existing_file_untouched_without_force() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("B.md"), "original").unwrap();
    let status = ensure_workspace_file(tmp.path(), "B.md", "new contents", false).expect("ok");
    assert_eq!(status, "existing");
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("B.md")).unwrap(),
        "original",
        "file must not be overwritten when force=false"
    );
}

#[test]
fn ensure_workspace_file_overwrites_when_forced() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("C.md"), "original").unwrap();
    let status = ensure_workspace_file(tmp.path(), "C.md", "new contents", true).expect("ok");
    assert_eq!(status, "overwritten");
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("C.md")).unwrap(),
        "new contents"
    );
}

#[test]
fn ensure_workspace_file_errors_when_directory_missing() {
    let tmp = tempdir().unwrap();
    let missing = tmp.path().join("does/not/exist");
    let err = ensure_workspace_file(&missing, "x.md", "y", false).unwrap_err();
    assert!(
        err.contains("failed to write"),
        "expected write-failure error, got: {err}"
    );
}

#[test]
fn bootstrap_files_contain_soul_and_identity() {
    // Lock in the contract so `init_workspace` doesn't silently stop
    // shipping a required prompt. These are the canonical prompt
    // files the agent harness expects in every fresh workspace.
    let names: Vec<&str> = BOOTSTRAP_FILES.iter().map(|(n, _)| *n).collect();
    assert!(names.contains(&"SOUL.md"));
    assert!(names.contains(&"IDENTITY.md"));
    assert_eq!(BOOTSTRAP_FILES.len(), 2);
    // Bundled contents must be non-empty — a packaging regression
    // that empties one would otherwise silently ship a broken agent.
    for (_, contents) in BOOTSTRAP_FILES {
        assert!(!contents.trim().is_empty());
    }
}

// ── init_workspace ────────────────────────────────────────────

#[tokio::test]
async fn init_workspace_creates_dirs_and_files_in_fresh_workspace() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempdir().unwrap();
    let _env = WorkspaceEnvGuard::set(tmp.path());

    let value = init_workspace(false)
        .await
        .expect("init_workspace on empty temp should succeed");

    let workspace_dir = value["result"]["workspace_dir"]
        .as_str()
        .expect("workspace_dir string");
    let workspace_dir = std::path::PathBuf::from(workspace_dir);
    for rel in ["memory", "sessions", "state", "cron"] {
        assert!(
            workspace_dir.join(rel).is_dir(),
            "expected {rel} directory under {}",
            workspace_dir.display()
        );
    }
    assert!(workspace_dir.join("SOUL.md").is_file());
    assert!(workspace_dir.join("IDENTITY.md").is_file());

    let created: Vec<&str> = value["result"]["files"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(created.iter().any(|s| s.ends_with("SOUL.md")));
    assert!(created.iter().any(|s| s.ends_with("IDENTITY.md")));

    let logs = value["logs"].as_array().expect("logs array");
    assert!(logs.iter().any(|l| l
        .as_str()
        .unwrap_or("")
        .contains("workspace initialization completed")));
}

#[tokio::test]
async fn init_workspace_reports_existing_entries_on_second_call_without_force() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempdir().unwrap();
    let _env = WorkspaceEnvGuard::set(tmp.path());

    // First call populates the workspace.
    init_workspace(false).await.expect("first init ok");
    // Second call without force should report everything as existing
    // and nothing as created / overwritten.
    let value = init_workspace(false).await.expect("second init ok");

    let created = value["result"]["files"]["created"].as_array().unwrap();
    let overwritten = value["result"]["files"]["overwritten"].as_array().unwrap();
    let existing = value["result"]["files"]["existing"].as_array().unwrap();
    assert!(created.is_empty(), "no files should be re-created");
    assert!(overwritten.is_empty(), "no files should be overwritten");
    assert!(
        existing
            .iter()
            .any(|v| v.as_str().unwrap_or("").ends_with("SOUL.md")),
        "SOUL.md should appear in the existing list"
    );

    let created_dirs = value["result"]["directories"]["created"]
        .as_array()
        .unwrap();
    let existing_dirs = value["result"]["directories"]["existing"]
        .as_array()
        .unwrap();
    assert!(created_dirs.is_empty());
    assert!(!existing_dirs.is_empty());
}

#[tokio::test]
async fn init_workspace_with_force_overwrites_existing_bootstrap_files() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempdir().unwrap();
    let _env = WorkspaceEnvGuard::set(tmp.path());

    let first = init_workspace(false).await.expect("initial init");
    // The config loader may place the workspace at a subpath of the
    // env override (e.g. `{tmp}/workspace`), so discover the real
    // location from the first result rather than assuming it is
    // `tmp.path()` itself.
    let workspace_dir = std::path::PathBuf::from(
        first["result"]["workspace_dir"]
            .as_str()
            .expect("workspace_dir string"),
    );
    let soul = workspace_dir.join("SOUL.md");
    std::fs::write(&soul, "corrupted").unwrap();

    let value = init_workspace(true).await.expect("forced init");

    let overwritten: Vec<&str> = value["result"]["files"]["overwritten"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(overwritten.iter().any(|s| s.ends_with("SOUL.md")));
    // And the on-disk contents must no longer be "corrupted".
    let restored = std::fs::read_to_string(&soul).unwrap();
    assert_ne!(restored, "corrupted");
    assert!(!restored.trim().is_empty());
}
