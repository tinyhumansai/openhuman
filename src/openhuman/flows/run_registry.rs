//! Process-local registry of in-flight flow runs, keyed by `run_id`
//! (== the run's checkpointer `thread_id`), so `flows_cancel_run` (issue G4)
//! can signal a synchronously-executing run to abort.
//!
//! A `flows_run` / `flows_resume` executes inline inside its RPC await (or a
//! fire-and-forget `tokio::spawn` from `flows::bus`), so there is no
//! `JoinHandle` a caller can reach. Instead each active run [`register`]s a
//! [`tokio_util::sync::CancellationToken`] here for the duration of the run and
//! `tokio::select!`s its future against the token's `cancelled()`. A separate
//! `flows_cancel_run` RPC looks the token up by `run_id` and [`cancel`]s it,
//! tripping the run's select arm.
//!
//! The registration is RAII: [`register`] returns a [`RunGuard`] that removes
//! the entry on `Drop` (including on panic / early return), so a finished run
//! can never leave a stale token wedged in the map.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use tokio_util::sync::CancellationToken;

/// The live in-flight runs: `run_id` → its cancellation token.
static IN_FLIGHT: LazyLock<Mutex<HashMap<String, CancellationToken>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Registers `run_id` as in-flight and returns both a clone of its
/// cancellation token (to `select!` on) and a [`RunGuard`] that deregisters it
/// on drop. Hold the guard for the whole run.
///
/// A duplicate `run_id` (should not happen — thread ids are UUID-suffixed)
/// replaces the prior token; the returned guard still removes exactly this
/// `run_id` on drop.
pub(crate) fn register(run_id: &str) -> (CancellationToken, RunGuard) {
    let token = CancellationToken::new();
    IN_FLIGHT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(run_id.to_string(), token.clone());
    tracing::debug!(target: "flows", run_id, "[flows] run_registry: registered in-flight run");
    (
        token,
        RunGuard {
            run_id: run_id.to_string(),
        },
    )
}

/// Signals the in-flight run keyed by `run_id` to cancel, if one is registered.
/// Returns `true` when a live run was signalled, `false` when no run with that
/// id is currently in flight (e.g. it already settled, or is a parked
/// `pending_approval` row with no executing task).
pub(crate) fn cancel(run_id: &str) -> bool {
    let guard = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
    match guard.get(run_id) {
        Some(token) => {
            token.cancel();
            tracing::info!(target: "flows", run_id, "[flows] run_registry: signalled in-flight run to cancel");
            true
        }
        None => {
            tracing::debug!(target: "flows", run_id, "[flows] run_registry: no in-flight run to cancel");
            false
        }
    }
}

/// RAII guard that removes a run's entry from the in-flight registry on drop.
pub(crate) struct RunGuard {
    run_id: String,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        IN_FLIGHT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.run_id);
        tracing::debug!(target: "flows", run_id = %self.run_id, "[flows] run_registry: deregistered run");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_signals_a_registered_run_then_deregisters_on_drop() {
        let run_id = "flow:reg-test:run-1";
        let (token, guard) = register(run_id);
        assert!(!token.is_cancelled());

        // A live run is signalled and its token trips.
        assert!(cancel(run_id), "a registered run must be signalled");
        assert!(token.is_cancelled(), "the run's token must be cancelled");

        // Dropping the guard removes it; a second cancel finds nothing.
        drop(guard);
        assert!(
            !cancel(run_id),
            "after the guard drops the run must no longer be in flight"
        );
    }

    #[test]
    fn cancel_of_unknown_run_is_false() {
        assert!(!cancel("flow:never-registered:run-x"));
    }
}
