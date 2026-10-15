//! The one sync-to-async seam to the TinySecurity module.
//!
//! The module proxy is async, but several host security entry points are
//! synchronous by contract (the log and Sentry scrubber, URL helpers, the
//! keyring and encrypted store). Every such caller goes through [`run`] instead
//! of building its own bridge. Rules:
//!
//! * No `block_on` on a tokio worker thread. The future is *spawned* on the
//!   runtime that owns the module connection and the caller waits on a std
//!   channel. On a multi-thread runtime the wait is wrapped in `block_in_place`
//!   so the worker's other tasks move off first; a current-thread runtime that
//!   owns the module cannot be waited on without deadlocking, so that call is
//!   refused.
//! * Bounded: every call has a deadline, and at most [`MAX_IN_FLIGHT`] callers
//!   may wait at once. Over the limit is an error, never a queue.
//! * Fail closed: no module runtime, a timeout, a panicked or dropped task and
//!   a module error are all `Err`. Callers translate `Err` into denial or
//!   suppression; nothing here ever invents an allowing answer.
//! * Scoped decisions are cached by `scoped_client::DecisionCache` (keyed by
//!   policy generation, stored only when the module marked the decision
//!   cacheable); this seam adds no cache of its own, so it cannot extend what
//!   the contract allows.
//!
//! Where a sync boundary must remain: `log` macros, the Sentry `before_send`
//! hook and `std::fs`-style URL/credential helpers cannot be `async`; they are
//! the only legitimate users.

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use tokio::runtime::{Handle, RuntimeFlavor};

/// Most synchronous callers allowed to wait on the module at once.
pub(crate) const MAX_IN_FLIGHT: usize = 64;
/// Slack past the call deadline for the spawned task to report back.
const GRACE: Duration = Duration::from_millis(100);

/// Why a synchronous module call produced no answer. Always a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SeamError {
    /// No runtime owns the module connection yet.
    Unavailable,
    /// The deadline passed.
    TimedOut,
    /// Too many synchronous callers are already waiting.
    Saturated,
    /// The caller sits on the runtime that must run the call.
    WouldDeadlock,
    /// The task panicked, was cancelled, or the module answered with an error.
    Failed(String),
}

impl std::fmt::Display for SeamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => f.write_str("TinySecurity runtime unavailable"),
            Self::TimedOut => f.write_str("TinySecurity synchronous call timed out"),
            Self::Saturated => f.write_str("TinySecurity synchronous call limit reached"),
            Self::WouldDeadlock => {
                f.write_str("TinySecurity synchronous call refused on the module runtime")
            }
            Self::Failed(reason) => write!(f, "TinySecurity synchronous call failed: {reason}"),
        }
    }
}

impl From<SeamError> for String {
    fn from(error: SeamError) -> Self {
        error.to_string()
    }
}

static TARGET: OnceLock<Handle> = OnceLock::new();
/// The module bus's own runtime. When armed it is preferred over [`TARGET`]:
/// it lives for the process, so it cannot be a caller runtime that has since
/// shut down, and it is the runtime the module connection belongs to.
static MODULE_TARGET: OnceLock<Handle> = OnceLock::new();
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// Remember the runtime the module connection lives on. The first runtime wins.
/// Returns false outside a tokio runtime.
pub(crate) fn arm_current() -> bool {
    match Handle::try_current() {
        Ok(handle) => {
            let _ = TARGET.set(handle);
            true
        }
        Err(_) => false,
    }
}

/// Target the module bus's own runtime, once it is up. Returns false while the
/// module runtime has not started or does not answer in time.
pub(crate) fn arm_module_runtime() -> bool {
    super::super::runtime::started().is_some_and(|runtime| arm_slot_from(&MODULE_TARGET, runtime))
}

/// Fill `slot` with the handle of `runtime`'s own tokio runtime.
fn arm_slot_from(slot: &OnceLock<Handle>, runtime: &tinybus::module::ModuleRuntime) -> bool {
    if slot.get().is_some() {
        return true;
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    runtime.spawn(async move {
        let _ = tx.send(Handle::current());
    });
    match rx.recv_timeout(Duration::from_secs(1)) {
        Ok(handle) => {
            let _ = slot.set(handle);
            true
        }
        Err(_) => false,
    }
}

/// Run `future` on the module runtime and wait at most `timeout` for it.
pub(crate) fn run<T, F>(timeout: Duration, future: F) -> Result<T, SeamError>
where
    T: Send + 'static,
    F: Future<Output = Result<T, String>> + Send + 'static,
{
    let slot = if MODULE_TARGET.get().is_some() {
        &MODULE_TARGET
    } else {
        &TARGET
    };
    run_slot(slot, &IN_FLIGHT, MAX_IN_FLIGHT, timeout, future)
}

fn run_slot<T, F>(
    slot: &OnceLock<Handle>,
    in_flight: &AtomicUsize,
    limit: usize,
    timeout: Duration,
    future: F,
) -> Result<T, SeamError>
where
    T: Send + 'static,
    F: Future<Output = Result<T, String>> + Send + 'static,
{
    let target = slot.get().ok_or(SeamError::Unavailable)?;
    run_on(in_flight, limit, target, timeout, future)
}

pub(crate) fn run_on<T, F>(
    in_flight: &AtomicUsize,
    limit: usize,
    target: &Handle,
    timeout: Duration,
    future: F,
) -> Result<T, SeamError>
where
    T: Send + 'static,
    F: Future<Output = Result<T, String>> + Send + 'static,
{
    struct Permit<'a>(&'a AtomicUsize);
    impl Drop for Permit<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    if in_flight.fetch_add(1, Ordering::SeqCst) >= limit {
        in_flight.fetch_sub(1, Ordering::SeqCst);
        log::debug!("[security][sync] refused: {limit} callers already waiting");
        return Err(SeamError::Saturated);
    }
    let _permit = Permit(in_flight);

    let nested = Handle::try_current().ok();
    if let Some(current) = &nested {
        if current.runtime_flavor() == RuntimeFlavor::CurrentThread && current.id() == target.id() {
            return Err(SeamError::WouldDeadlock);
        }
    }

    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let task = target.spawn(async move {
        let outcome = match tokio::time::timeout(timeout, future).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(reason)) => Err(SeamError::Failed(reason)),
            Err(_) => Err(SeamError::TimedOut),
        };
        let _ = tx.send(outcome);
    });
    let wait = || rx.recv_timeout(timeout + GRACE);
    let received = match &nested {
        Some(current) if current.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(wait)
        }
        _ => wait(),
    };
    match received {
        Ok(outcome) => outcome,
        Err(error) => {
            task.abort();
            Err(match error {
                std::sync::mpsc::RecvTimeoutError::Timeout => SeamError::TimedOut,
                std::sync::mpsc::RecvTimeoutError::Disconnected => {
                    SeamError::Failed("task ended without an answer".into())
                }
            })
        }
    }
}

#[cfg(test)]
#[path = "security_sync_tests.rs"]
mod tests;
