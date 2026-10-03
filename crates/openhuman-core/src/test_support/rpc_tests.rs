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
