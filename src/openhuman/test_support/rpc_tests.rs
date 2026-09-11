use super::*;
use std::sync::{Mutex, OnceLock};
use tempfile::TempDir;

static E2E_MODE_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    E2E_MODE_ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[tokio::test]
async fn reset_rejects_when_e2e_mode_unset() {
    let _guard = env_lock();
    let prior = std::env::var(E2E_MODE_ENV_VAR).ok();
    std::env::remove_var(E2E_MODE_ENV_VAR);

    let err = reset()
        .await
        .expect_err("unset E2E mode must reject test_reset");

    match prior {
        Some(value) => std::env::set_var(E2E_MODE_ENV_VAR, value),
        None => std::env::remove_var(E2E_MODE_ENV_VAR),
    }

    assert!(
        err.contains("OPENHUMAN_E2E_MODE") && err.contains("is set to one of"),
        "unexpected guard error: {err}"
    );
}

#[test]
fn reset_guard_accepts_explicit_e2e_mode() {
    ensure_e2e_mode_value(Some("1")).expect("1 enables E2E mode");
    ensure_e2e_mode_value(Some("true")).expect("true enables E2E mode");
    ensure_e2e_mode_value(Some("yes")).expect("yes enables E2E mode");
}

#[tokio::test]
async fn wipe_memory_tree_removes_content_dirs_and_reports_summary() {
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");

    let content_root = config.memory_tree_content_root();
    let raw_dir = content_root.join("raw");
    let wiki_dir = content_root.join("wiki");
    std::fs::create_dir_all(&raw_dir).unwrap();
    std::fs::create_dir_all(&wiki_dir).unwrap();
    std::fs::write(raw_dir.join("chunk.md"), "test chunk").unwrap();
    std::fs::write(wiki_dir.join("summary.md"), "test summary").unwrap();

    let summary = wipe_memory_tree(&config).await.unwrap();

    assert_eq!(summary.rows_deleted, 0);
    assert_eq!(summary.sync_state_cleared, 0);
    assert!(summary.dirs_removed.contains(&"raw".to_string()));
    assert!(summary.dirs_removed.contains(&"wiki".to_string()));
    assert!(!raw_dir.exists());
    assert!(!wiki_dir.exists());
}
