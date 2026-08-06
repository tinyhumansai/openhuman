//! Process-local registry of in-flight `flows_build` (Workflow Copilot)
//! authoring turns, keyed by `thread_id`, so the copilot's Stop button can
//! actually cancel the agent turn instead of merely hiding its own button
//! (issue: PR #5223 review found the FE-only Stop was cosmetic).
//!
//! `flows_build` runs the `workflow_builder` agent turn **awaited inline**
//! inside its RPC handler — wrapped in `with_origin` /
//! `APPROVAL_CHAT_CONTEXT.scope` / `APPROVAL_COPILOT_STREAM_CONTEXT.scope` /
//! `with_thread_id`, all task-local context that would be lost if the run were
//! `tokio::spawn`ed onto a fresh task. So it can't register in
//! `web_chat::IN_FLIGHT` (only tracks *spawned* interactive chat turns) or
//! `task_dispatcher::ACTIVE_RUNS` (aborts a detached tokio task via
//! `JoinHandle::abort`) — neither fits an inline-awaited future. This module
//! mirrors `flows::run_registry` (used by `flows_cancel_run` for live
//! `flows_run`/`flows_resume` executions — see that module's doc for the same
//! register-a-token / `tokio::select!` shape), but is scoped by `thread_id` +
//! `request_id` instead of `run_id`, matching
//! `task_dispatcher::cancel_session_scoped`'s stale-cancel guard (#4760): a
//! Stop click for a superseded/earlier request must not tear down a newer
//! builder turn that has since started on the same thread.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use tokio_util::sync::CancellationToken;

/// One in-flight builder turn: the `request_id` it was registered under (for
/// scoped-cancel matching) and the token `flows_build` selects against.
struct BuildTurn {
    request_id: Option<String>,
    token: CancellationToken,
}

/// The live in-flight builder turns: `thread_id` -> its turn entry.
static BUILD_TURNS: LazyLock<Mutex<HashMap<String, BuildTurn>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Registers an in-flight `flows_build` turn for `thread_id`, storing its
/// `request_id` (for scoped-cancel matching) and the `token` the turn selects
/// against.
///
/// Overwrites any prior entry on this thread unconditionally: `flows_build`
/// unregisters on every exit path (success, error, timeout, cancel), so a
/// still-present entry here means an earlier turn's cleanup hasn't run yet —
/// the newer registration should win either way, matching `run_registry`'s
/// duplicate-key handling.
pub(crate) fn register_build_turn(
    thread_id: String,
    request_id: Option<String>,
    token: CancellationToken,
) {
    tracing::debug!(
        target: "flows",
        thread_id = %thread_id,
        request_id = request_id.as_deref().unwrap_or("<none>"),
        "[flows] build_registry: registered in-flight build turn"
    );
    BUILD_TURNS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(thread_id, BuildTurn { request_id, token });
}

/// Signals the in-flight `flows_build` turn on `thread_id` to cancel, scoped by
/// `request_id` exactly like `task_dispatcher::cancel_session_scoped`: when
/// `request_id` is `Some`, the cancel fires only if it matches the turn
/// currently registered on the thread — a stale Stop for a superseded/unrelated
/// request is a no-op rather than killing a newer turn. `None` cancels
/// unscoped (whatever turn is on the thread). Returns whether a turn was found
/// and signalled.
pub(crate) fn cancel_build_turn_scoped(thread_id: &str, request_id: Option<&str>) -> bool {
    let guard = BUILD_TURNS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(turn) = guard.get(thread_id) else {
        tracing::debug!(
            target: "flows",
            thread_id = %thread_id,
            "[flows] build_registry: cancel ignored — no in-flight build turn on thread"
        );
        return false;
    };
    if let Some(rid) = request_id {
        if turn.request_id.as_deref() != Some(rid) {
            tracing::debug!(
                target: "flows",
                thread_id = %thread_id,
                request_id = %rid,
                active_request_id = turn.request_id.as_deref().unwrap_or("<none>"),
                "[flows] build_registry: cancel ignored — request_id mismatch \
                 (superseded/unrelated request)"
            );
            return false;
        }
    }
    turn.token.cancel();
    tracing::info!(
        target: "flows",
        thread_id = %thread_id,
        "[flows] build_registry: signalled in-flight build turn to cancel"
    );
    true
}

/// Removes the registered turn for `thread_id`, but only if it still matches
/// `request_id` (when `Some`) — guards the same race
/// `task_dispatcher::take_active_run_if` guards against: a newer turn may have
/// already re-registered on this thread by the time an older turn's own
/// cleanup runs, and unregistering unconditionally would clobber that newer
/// turn's entry out from under it. `None` unregisters unconditionally. Call on
/// every `flows_build` exit path (success, error, timeout, cancel).
pub(crate) fn unregister_build_turn(thread_id: &str, request_id: Option<&str>) {
    let mut guard = BUILD_TURNS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(rid) = request_id {
        match guard.get(thread_id) {
            Some(turn) if turn.request_id.as_deref() == Some(rid) => {}
            _ => {
                tracing::debug!(
                    target: "flows",
                    thread_id = %thread_id,
                    request_id = %rid,
                    "[flows] build_registry: unregister skipped — thread now owned by a \
                     different (or no) turn"
                );
                return;
            }
        }
    }
    guard.remove(thread_id);
    tracing::debug!(
        target: "flows",
        thread_id = %thread_id,
        "[flows] build_registry: deregistered build turn"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_build_turn_scoped_cancels_a_registered_token() {
        let thread_id = "build-registry-test:thread-1";
        let token = CancellationToken::new();
        register_build_turn(
            thread_id.to_string(),
            Some("req-1".to_string()),
            token.clone(),
        );

        assert!(!token.is_cancelled());
        assert!(cancel_build_turn_scoped(thread_id, Some("req-1")));
        assert!(token.is_cancelled());

        unregister_build_turn(thread_id, Some("req-1"));
    }

    #[test]
    fn cancel_build_turn_scoped_returns_false_when_nothing_registered() {
        assert!(!cancel_build_turn_scoped(
            "build-registry-test:never-registered",
            Some("req-x")
        ));
        assert!(!cancel_build_turn_scoped(
            "build-registry-test:never-registered",
            None
        ));
    }

    #[test]
    fn cancel_build_turn_scoped_ignores_a_mismatched_request_id() {
        let thread_id = "build-registry-test:thread-2";
        let token = CancellationToken::new();
        register_build_turn(
            thread_id.to_string(),
            Some("req-1".to_string()),
            token.clone(),
        );

        // A stale/unrelated request_id must not cancel a still-live turn.
        assert!(!cancel_build_turn_scoped(
            thread_id,
            Some("some-other-request")
        ));
        assert!(!token.is_cancelled());

        // The real request_id still works.
        assert!(cancel_build_turn_scoped(thread_id, Some("req-1")));
        assert!(token.is_cancelled());

        unregister_build_turn(thread_id, Some("req-1"));
    }

    #[test]
    fn cancel_build_turn_scoped_unscoped_none_cancels_whatever_is_registered() {
        let thread_id = "build-registry-test:thread-3";
        let token = CancellationToken::new();
        register_build_turn(
            thread_id.to_string(),
            Some("req-1".to_string()),
            token.clone(),
        );

        assert!(cancel_build_turn_scoped(thread_id, None));
        assert!(token.is_cancelled());

        unregister_build_turn(thread_id, None);
    }

    #[test]
    fn unregister_build_turn_is_match_guarded() {
        let thread_id = "build-registry-test:thread-4";
        let token_a = CancellationToken::new();
        register_build_turn(
            thread_id.to_string(),
            Some("req-a".to_string()),
            token_a.clone(),
        );

        // A newer turn supersedes the entry (mirrors `flows_build` starting a
        // second turn on the same thread before the first one's cleanup ran).
        let token_b = CancellationToken::new();
        register_build_turn(
            thread_id.to_string(),
            Some("req-b".to_string()),
            token_b.clone(),
        );

        // An unregister scoped to the OLD (superseded) request must not clobber
        // the newer turn's entry.
        unregister_build_turn(thread_id, Some("req-a"));
        assert!(
            cancel_build_turn_scoped(thread_id, Some("req-b")),
            "the newer turn's entry must still be registered after a stale unregister"
        );
        assert!(token_b.is_cancelled());
        assert!(!token_a.is_cancelled());

        unregister_build_turn(thread_id, Some("req-b"));
        assert!(!cancel_build_turn_scoped(thread_id, None));
    }

    #[test]
    fn unregister_build_turn_none_removes_unconditionally() {
        let thread_id = "build-registry-test:thread-5";
        let token = CancellationToken::new();
        register_build_turn(thread_id.to_string(), Some("req-1".to_string()), token);

        unregister_build_turn(thread_id, None);
        assert!(!cancel_build_turn_scoped(thread_id, None));
    }
}
