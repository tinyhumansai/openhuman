//! One lock for every desktop unit test that reads or changes process environment.
//! Separate module-local locks cannot protect a core boot from another test's
//! temporary workspace, RPC URL, or executable search path.

use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn env_lock() -> MutexGuard<'static, ()> {
    // Environment guards restore on unwind, so poisoning carries no state
    // that could make the next test unsafe.
    ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
