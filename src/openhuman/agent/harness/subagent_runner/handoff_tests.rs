use super::*;
use std::sync::{Mutex, OnceLock};

/// `OPENHUMAN_TEST_HANDOFF_THRESHOLD_TOKENS` is process-global and this
/// suite runs ~11.6k tests in one process, so serialize the two tests in
/// this module that touch it and restore the prior value on drop.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, val: &str) -> Self {
        let prev = std::env::var(key).ok();
        // SAFETY: caller holds `env_lock()` for the duration of the test.
        unsafe { std::env::set_var(key, val) };
        Self { key, prev }
    }

    fn remove(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        // SAFETY: caller holds `env_lock()` for the duration of the test.
        unsafe { std::env::remove_var(key) };
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            // SAFETY: caller holds `env_lock()` for the duration of the test.
            Some(val) => unsafe { std::env::set_var(self.key, val) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

#[test]
fn apply_handoff_uses_the_test_env_threshold_when_set() {
    let _guard = env_lock();
    let _env = EnvGuard::set("OPENHUMAN_TEST_HANDOFF_THRESHOLD_TOKENS", "5");
    let cache = ResultHandoffCache::new();

    // 40 bytes / 4 => 10 estimated tokens, comfortably above the
    // env-lowered threshold of 5 but nowhere near the real default of
    // 50_000 — this only exercises the handoff path because the env var
    // was actually read and honoured.
    let oversized = "x".repeat(40);
    let out = apply_handoff(&cache, "some_tool", "task-1", "agent-1", oversized.clone());

    assert_ne!(
        out, oversized,
        "result above the env-lowered threshold should be replaced with a placeholder"
    );
    assert!(
        out.contains("result_id"),
        "placeholder should mention how to retrieve the cached result: {out}"
    );
}

#[test]
fn apply_handoff_falls_back_to_the_default_threshold_when_env_unset() {
    let _guard = env_lock();
    let _env = EnvGuard::remove("OPENHUMAN_TEST_HANDOFF_THRESHOLD_TOKENS");
    let cache = ResultHandoffCache::new();

    // Comfortably below HANDOFF_OVERSIZE_THRESHOLD_TOKENS (50_000 tokens
    // / 200_000 bytes), so with the env var unset (falling back to the
    // real default) the text passes through unchanged.
    let small = "hello world".to_string();
    let out = apply_handoff(&cache, "some_tool", "task-1", "agent-1", small.clone());

    assert_eq!(
        out, small,
        "a small result under the default threshold must pass through unchanged"
    );
}
