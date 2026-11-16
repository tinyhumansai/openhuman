//! Process-env lock for this crate's unit tests.
//!
//! The service mock reads `OPENHUMAN_SERVICE_MOCK` and
//! `OPENHUMAN_SERVICE_MOCK_STATE_FILE` from the process environment, which is
//! global, so every test that sets either serializes on one lock.

use std::sync::{Mutex, MutexGuard, PoisonError};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Take the env lock. A test that panicked while holding it leaves nothing
/// half-done that the next test must see, so a poisoned lock is reused.
pub(crate) fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}
