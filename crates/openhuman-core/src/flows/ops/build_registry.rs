//! Tracks the OpenHuman-owned in-flight workflow-builder turns that stream
//! into a chat thread. The builder runs inline on its host task, so the bus
//! run registry does not own this cancellation lifecycle.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use tokio_util::sync::CancellationToken;

/// One in-flight builder turn keyed by its chat thread.
struct BuildTurn {
    request_id: Option<String>,
    token: CancellationToken,
}

/// Current builder turn for each chat thread.
static BUILD_TURNS: LazyLock<Mutex<HashMap<String, BuildTurn>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Registers an in-flight `flows_build` turn.
///
/// A newer turn replaces the old thread entry and cancels its token so the
/// displaced model call cannot continue after losing its Stop control.
pub(super) fn register_build_turn(
    thread_id: String,
    request_id: Option<String>,
    token: CancellationToken,
) {
    tracing::debug!(
        target: "flows",
        thread_id = %thread_id,
        request_id = request_id.as_deref().unwrap_or("<none>"),
        "[flows] build turn registered"
    );
    let displaced = BUILD_TURNS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(thread_id.clone(), BuildTurn { request_id, token });
    if let Some(displaced) = displaced.filter(|turn| !turn.token.is_cancelled()) {
        tracing::debug!(
            target: "flows",
            thread_id = %thread_id,
            "[flows] cancelling displaced builder turn"
        );
        displaced.token.cancel();
    }
}

/// Signals the current builder turn, optionally matching its request id.
/// A stale Stop for an earlier request cannot cancel its replacement.
pub(super) fn cancel_build_turn_scoped(thread_id: &str, request_id: Option<&str>) -> bool {
    let turns = BUILD_TURNS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let Some(turn) = turns.get(thread_id) else {
        tracing::debug!(
            target: "flows",
            thread_id,
            "[flows] builder cancellation ignored: no turn is registered"
        );
        return false;
    };
    if request_id.is_some_and(|request_id| turn.request_id.as_deref() != Some(request_id)) {
        tracing::debug!(
            target: "flows",
            thread_id,
            request_id = request_id.unwrap_or_default(),
            active_request_id = turn.request_id.as_deref().unwrap_or("<none>"),
            "[flows] builder cancellation ignored: request id does not match"
        );
        return false;
    }
    turn.token.cancel();
    tracing::info!(
        target: "flows",
        thread_id,
        "[flows] builder turn cancellation signalled"
    );
    true
}

/// Removes a turn only if it still belongs to this request.
///
/// An older turn's cleanup may run after a newer turn has registered on the
/// same thread; matching the request id prevents that cleanup from removing
/// the newer Stop target.
pub(super) fn unregister_build_turn(thread_id: &str, request_id: Option<&str>) {
    let mut turns = BUILD_TURNS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(request_id) = request_id {
        match turns.get(thread_id) {
            Some(turn) if turn.request_id.as_deref() == Some(request_id) => {}
            _ => {
                tracing::debug!(
                    target: "flows",
                    thread_id,
                    request_id,
                    "[flows] builder unregister ignored: thread has a different request"
                );
                return;
            }
        }
    }
    turns.remove(thread_id);
}

#[cfg(test)]
#[path = "build_registry_tests.rs"]
mod tests;
