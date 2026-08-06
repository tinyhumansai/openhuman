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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use tokio_util::sync::CancellationToken;

/// Monotonic registration id, so a [`RunGuard`] can prove an entry is still
/// *its own* before removing it. See [`RunGuard::drop`].
static NEXT_REGISTRATION: AtomicU64 = AtomicU64::new(1);

/// The live in-flight runs: `run_id` → (registration id, cancellation token).
static IN_FLIGHT: LazyLock<Mutex<HashMap<String, (u64, CancellationToken)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Registers `run_id` as in-flight and returns both a clone of its
/// cancellation token (to `select!` on) and a [`RunGuard`] that deregisters it
/// on drop. Hold the guard for the whole run.
///
/// A duplicate `run_id` replaces the prior token, and each registration carries
/// a unique id so a guard only ever removes the entry it installed.
///
/// This used to be documented as impossible ("thread ids are UUID-suffixed"),
/// which held while only `flows_run` / `flows_run_detached` registered — both
/// mint a fresh UUID `thread_id` per call, so two concurrent registrations could
/// never collide on a key. `flows_resume` is the first caller to register
/// against a **stable, pre-existing** id (the parked run's own), and nothing
/// serializes two concurrent resumes of the same run — a client double-submit or
/// a retry-on-timeout is enough. With removal keyed only by `run_id`, the loser
/// of that race would deregister the *winner's* live token on its way out, and a
/// later `flows_cancel_run` would then see `is_in_flight == false` for a run that
/// is genuinely executing, take its "parked/stale" branch, and drop the
/// checkpoint out from under it — the exact bug class this registry exists to
/// prevent.
pub(crate) fn register(run_id: &str) -> (CancellationToken, RunGuard) {
    let token = CancellationToken::new();
    let registration = NEXT_REGISTRATION.fetch_add(1, Ordering::Relaxed);
    let displaced = IN_FLIGHT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(run_id.to_string(), (registration, token.clone()));
    if displaced.is_some() {
        tracing::warn!(
            target: "flows",
            run_id,
            registration,
            "[flows] run_registry: duplicate registration for a run id already in flight — the displaced guard will no longer deregister this entry"
        );
    }
    tracing::debug!(target: "flows", run_id, registration, "[flows] run_registry: registered in-flight run");
    (
        token,
        RunGuard {
            run_id: run_id.to_string(),
            registration,
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
        Some((_registration, token)) => {
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

/// Returns `true` when `run_id` is currently registered as an in-flight run in
/// THIS process. Used by the boot-time orphan sweep (bug B42) to distinguish a
/// genuinely orphaned `running` row (left by a prior process — not in flight)
/// from one a freshly-started run in this process legitimately owns, so the
/// sweep never reconciles a live run out from under itself.
pub(crate) fn is_in_flight(run_id: &str) -> bool {
    IN_FLIGHT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(run_id)
}

/// RAII guard that removes a run's entry from the in-flight registry on drop.
pub(crate) struct RunGuard {
    run_id: String,
    registration: u64,
}

impl Drop for RunGuard {
    /// Removes this run's entry — but only if it is still the entry THIS guard
    /// installed. A plain `remove(&run_id)` would let the loser of a duplicate
    /// registration deregister the winner's live token (see [`register`]).
    fn drop(&mut self) {
        let mut map = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
        match map.get(&self.run_id) {
            Some((registration, _)) if *registration == self.registration => {
                map.remove(&self.run_id);
                tracing::debug!(target: "flows", run_id = %self.run_id, registration = self.registration, "[flows] run_registry: deregistered run");
            }
            Some(_) => {
                tracing::debug!(
                    target: "flows",
                    run_id = %self.run_id,
                    registration = self.registration,
                    "[flows] run_registry: entry belongs to a newer registration — leaving it in place"
                );
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    // NOTE: `duplicate_registration_*` below pin the identity-based removal —
    // see `register`'s doc for why `flows_resume` made this reachable.

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

    /// The loser of a duplicate registration must NOT deregister the winner's
    /// live token. Before removal compared registration identity, the loser's
    /// guard dropping would clear the map entry the winner still relies on, and
    /// a later `cancel` would report "not in flight" for a genuinely running
    /// run — letting the caller drop its checkpoint mid-execution.
    #[test]
    fn a_displaced_guard_does_not_deregister_the_live_registration() {
        let run_id = "dup-resume-run";
        let (_winner_token, winner_guard) = register(run_id);
        // A second concurrent resume of the SAME run id registers on top.
        let (_later_token, later_guard) = register(run_id);

        // The first (now displaced) guard drops, e.g. because it lost the
        // guarded `mark_run_resuming` race and returned early.
        drop(winner_guard);

        assert!(
            is_in_flight(run_id),
            "the newer, still-live registration must survive a displaced guard's drop"
        );
        assert!(cancel(run_id), "the live run must still be cancellable");

        drop(later_guard);
        assert!(
            !is_in_flight(run_id),
            "the owning guard must still deregister its own entry"
        );
    }

    /// The ordinary single-registration path is unchanged.
    #[test]
    fn the_owning_guard_still_deregisters_its_own_entry() {
        let run_id = "solo-run";
        let (_token, guard) = register(run_id);
        assert!(is_in_flight(run_id));
        drop(guard);
        assert!(!is_in_flight(run_id));
    }
}
