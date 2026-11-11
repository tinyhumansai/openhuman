//! Bounded synchronous bridge for security calls made by sync log/storage seams.

use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::runtime::{Handle, RuntimeFlavor};

static TARGET: OnceLock<Handle> = OnceLock::new();

pub(crate) fn arm_current() -> bool {
    Handle::try_current().is_ok_and(|handle| TARGET.set(handle).is_ok() || TARGET.get().is_some())
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
