//! Bounded synchronous bridge for security calls made by sync log/storage seams.

use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::runtime::{Handle, RuntimeFlavor};

static TARGET: OnceLock<Handle> = OnceLock::new();

/// How long arming waits for the module bus to report its executor.
const ARM_TIMEOUT: Duration = Duration::from_secs(5);

/// Point the bridge at the module bus's own executor.
///
/// Not the caller's runtime: that may be single-threaded, where every bridged
/// call would be refused as a deadlock, or may end long before the process
/// does. The bus executor lives for the process and is never the thread a host
/// blocks in here, unless a module callback does, and that executor is
/// multi-threaded.
pub(crate) fn arm_current() -> bool {
    if TARGET.get().is_some() {
        return true;
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let spawned = crate::modules::runtime::spawn_on_bus(async move {
        let _ = tx.send(Handle::current());
    });
    if spawned.is_err() {
        return false;
    }
    let wait = || rx.recv_timeout(ARM_TIMEOUT);
    let reported = match Handle::try_current().map(|handle| handle.runtime_flavor()) {
        Ok(RuntimeFlavor::MultiThread) => tokio::task::block_in_place(wait),
        _ => wait(),
    };
    reported.is_ok_and(|handle| TARGET.set(handle).is_ok() || TARGET.get().is_some())
}

pub(crate) fn run<T, F>(timeout: Duration, future: F) -> Result<T, String>
where
    T: Send + 'static,
    F: Future<Output = Result<T, String>> + Send + 'static,
{
    run_on(TARGET.get(), timeout, future)
}

fn run_on<T, F>(target: Option<&Handle>, timeout: Duration, future: F) -> Result<T, String>
where
    T: Send + 'static,
    F: Future<Output = Result<T, String>> + Send + 'static,
{
    let target = target.ok_or_else(|| "TinySecurity runtime unavailable".to_owned())?;
    let current = Handle::try_current().ok();
    if current.as_ref().is_some_and(|handle| {
        handle.id() == target.id() && handle.runtime_flavor() == RuntimeFlavor::CurrentThread
    }) {
        return Err("TinySecurity sync call would deadlock its current-thread runtime".into());
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let task = target.spawn(async move {
        let result = tokio::time::timeout(timeout, future)
            .await
            .map_err(|_| "TinySecurity sync call timed out".to_owned())
            .and_then(|result| result);
        let _ = tx.send(result);
    });
    let wait = || rx.recv_timeout(timeout + Duration::from_millis(100));
    let result = match current.as_ref().map(Handle::runtime_flavor) {
        Some(RuntimeFlavor::MultiThread) => tokio::task::block_in_place(wait),
        _ => wait(),
    };
    match result {
        Ok(result) => result,
        Err(error) => {
            task.abort();
            Err(format!("TinySecurity sync call failed: {error}"))
        }
    }
}

#[cfg(test)]
#[path = "sync_seam_tests.rs"]
mod tests;
