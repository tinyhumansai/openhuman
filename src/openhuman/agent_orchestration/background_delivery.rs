//! Delivery subsystem for finished detached background sub-agents.
//!
//! Surfaces results recorded in [`super::background_completions`] back into the
//! originating chat as a single **system-injected** turn:
//!   * **idle-gated** — never mid-turn; defers while a user turn is in flight,
//!   * **debounced** — a burst of completions batches into one turn,
//!   * **batched** — every result ready at delivery time goes in one turn,
//!     each tagged by its sub-agent process id.
//!
//! The turn is run via [`task_dispatcher::run_system_turn_on_thread`], which
//! streams it into the thread exactly like a chat turn (the same bridge cron /
//! welcome agents use), so it renders in the desktop UI.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;

use crate::core::event_bus::{subscribe_global, DomainEvent, EventHandler, SubscriptionHandle};

use super::background_completions;

/// Coalesce completions landing within this window into one delivery turn.
const DEBOUNCE: Duration = Duration::from_secs(3);

/// Sessions with a user turn currently in flight — delivery defers while busy.
fn busy() -> &'static Mutex<HashSet<String>> {
    static BUSY: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    BUSY.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Sessions whose delivery turn is in flight — prevents two concurrent turns.
fn delivering() -> &'static Mutex<HashSet<String>> {
    static D: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(HashSet::new()))
}

fn is_busy(session: &str) -> bool {
    busy()
        .lock()
        .expect("background_delivery busy poisoned")
        .contains(session)
}

struct BackgroundDeliveryHandler;

#[async_trait]
impl EventHandler for BackgroundDeliveryHandler {
    fn name(&self) -> &str {
        "agent_orchestration::background_delivery"
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::AgentTurnStarted { session_id, .. } => {
                busy()
                    .lock()
                    .expect("busy poisoned")
                    .insert(session_id.clone());
            }
            DomainEvent::AgentTurnCompleted { session_id, .. } => {
                busy().lock().expect("busy poisoned").remove(session_id);
                // A user turn just ended — drain anything that finished while it ran.
                schedule_delivery(session_id.clone(), Duration::from_millis(300));
            }
            DomainEvent::AgentError { session_id, .. } => {
                // A failed turn may not emit AgentTurnCompleted — clear busy so
                // delivery isn't stuck, then try to drain.
                busy().lock().expect("busy poisoned").remove(session_id);
                schedule_delivery(session_id.clone(), Duration::from_millis(300));
            }
            DomainEvent::SubagentCompleted { parent_session, .. } => {
                // Debounce so a burst of completions batches into a single turn.
                schedule_delivery(parent_session.clone(), DEBOUNCE);
            }
            _ => {}
        }
    }
}

/// Schedule a debounced delivery attempt for a session.
fn schedule_delivery(session: String, delay: Duration) {
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        try_deliver(session).await;
    });
}

/// Snapshot the ready batch for a session **right now** (sync, testable): if the
/// session is idle, drain all ready results. Returns `None` (queue untouched)
/// when busy or nothing is pending. Headless filtering + delivery happen in the
/// caller, which can requeue the batch if the turn fails.
fn plan_delivery(session: &str) -> Option<Vec<background_completions::CompletedBackgroundAgent>> {
    if is_busy(session) {
        return None;
    }
    let batch = background_completions::take_pending(session);
    if batch.is_empty() {
        None
    } else {
        Some(batch)
    }
}

/// Re-queue a drained batch (after a failed delivery) so it retries on the next
/// idle drain rather than being lost.
fn requeue(session: &str, batch: Vec<background_completions::CompletedBackgroundAgent>) {
    for c in batch {
        background_completions::record_completion(
            session,
            c.task_id,
            c.agent_id,
            c.summary,
            c.parent_thread_id,
        );
    }
}

/// Drain + deliver pending completions for a session — if idle and not already
/// delivering. Batches everything ready at this instant into one system turn.
async fn try_deliver(session: String) {
    if is_busy(&session) || !background_completions::has_pending(&session) {
        return;
    }
    // Claim the delivery slot — held for the WHOLE delivery (including the
    // awaited turn) so a concurrent completion can't start a second delivery
    // turn on the same thread. Skip if a delivery is already in flight.
    {
        let mut d = delivering().lock().expect("delivering poisoned");
        if !d.insert(session.clone()) {
            return;
        }
    }

    if let Some(batch) = plan_delivery(&session) {
        // A user turn can start (AgentTurnStarted -> busy) between plan_delivery's
        // gate and the awaited turn below. Re-check here so we don't stream a
        // *system* turn concurrently with a freshly-started user turn on the same
        // thread — requeue the drained batch and let the next idle drain retry.
        // (Narrows the window; a turn starting mid-await is still possible, but
        // both append into the thread and delivery is keyed to its own run id.)
        if is_busy(&session) {
            requeue(&session, batch);
            delivering()
                .lock()
                .expect("delivering poisoned")
                .remove(&session);
            return;
        }
        match (
            background_completions::batch_thread_id(&batch),
            background_completions::build_batched_notice(&batch),
        ) {
            (Some(thread_id), Some(notice)) => {
                log::info!(
                    "[background_delivery] delivering {} batched background result(s) \
                     session={session} thread_id={thread_id}",
                    batch.len()
                );
                if let Err(e) = crate::openhuman::agent::task_dispatcher::run_system_turn_on_thread(
                    thread_id, notice,
                )
                .await
                {
                    log::warn!(
                        "[background_delivery] delivery turn failed session={session} error={e}"
                    );
                    requeue(&session, batch); // don't lose results on a failed turn
                }
            }
            // Headless (no originating thread to stream into) — drop the batch.
            _ => {}
        }
    }

    // Release the slot only AFTER the turn settles.
    delivering()
        .lock()
        .expect("delivering poisoned")
        .remove(&session);
}

/// Register the delivery subscriber on the global event bus. Keeps the
/// subscription alive for the process lifetime. Idempotent.
pub fn register_background_delivery() {
    static HANDLE: OnceLock<Option<SubscriptionHandle>> = OnceLock::new();
    HANDLE.get_or_init(|| subscribe_global(Arc::new(BackgroundDeliveryHandler)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent_orchestration::background_completions::record_completion;

    #[test]
    fn plan_drains_ready_batch_when_idle() {
        let s = "bd-ready";
        record_completion(s, "sub-1", "researcher", "alpha", Some("thread-9".into()));
        record_completion(s, "sub-2", "researcher", "beta", Some("thread-9".into()));

        let batch = plan_delivery(s).expect("plans a delivery");
        assert_eq!(batch.len(), 2);
        assert_eq!(
            background_completions::batch_thread_id(&batch).as_deref(),
            Some("thread-9")
        );
        let notice = background_completions::build_batched_notice(&batch).unwrap();
        assert!(notice.contains("sub-1") && notice.contains("sub-2"));
        assert!(!background_completions::has_pending(s)); // drained
    }

    #[test]
    fn plan_skips_when_busy_and_leaves_queue_intact() {
        let s = "bd-busy";
        record_completion(s, "sub-1", "researcher", "x", Some("t".into()));
        busy().lock().expect("busy").insert(s.to_string());

        assert!(plan_delivery(s).is_none());
        assert!(background_completions::has_pending(s)); // NOT drained while busy

        busy().lock().expect("busy").remove(s);
        let _ = background_completions::take_pending(s); // cleanup
    }

    #[test]
    fn plan_none_when_nothing_pending() {
        assert!(plan_delivery("bd-empty-unique").is_none());
    }

    #[test]
    fn headless_batch_has_no_thread_so_caller_drops_it() {
        let s = "bd-headless";
        record_completion(s, "sub-1", "researcher", "x", None);
        let batch = plan_delivery(s).expect("batch present");
        // No originating thread → batch_thread_id is None, so try_deliver drops it.
        assert!(background_completions::batch_thread_id(&batch).is_none());
    }

    #[test]
    fn requeue_restores_a_failed_batch() {
        let s = "bd-requeue";
        record_completion(s, "sub-1", "researcher", "alpha", Some("t".into()));
        let batch = plan_delivery(s).expect("batch");
        assert!(!background_completions::has_pending(s)); // drained
        requeue(s, batch);
        assert!(background_completions::has_pending(s)); // restored for retry
        let _ = background_completions::take_pending(s); // cleanup
    }

    #[test]
    fn interleave_recheck_requeues_when_user_turn_starts_after_drain() {
        // Mirrors try_deliver's M1 guard: a user turn can start between
        // plan_delivery draining the batch and the awaited system turn. The
        // re-check must requeue the drained batch rather than stream concurrently.
        let s = "bd-interleave";
        record_completion(s, "sub-1", "researcher", "alpha", Some("t".into()));

        let batch = plan_delivery(s).expect("batch drained");
        assert!(!background_completions::has_pending(s)); // drained

        // User turn starts after the drain, before the (would-be) await.
        busy().lock().expect("busy").insert(s.to_string());
        if is_busy(s) {
            requeue(s, batch); // the guard's action
        }
        assert!(background_completions::has_pending(s)); // preserved for next drain

        busy().lock().expect("busy").remove(s);
        let _ = background_completions::take_pending(s); // cleanup
    }

    #[tokio::test]
    async fn handler_tracks_busy_across_turn_and_error_events() {
        let h = BackgroundDeliveryHandler;
        let sid = "bd-turn".to_string();

        h.handle(&DomainEvent::AgentTurnStarted {
            session_id: sid.clone(),
            channel: "test".into(),
        })
        .await;
        assert!(is_busy(&sid));

        h.handle(&DomainEvent::AgentTurnCompleted {
            session_id: sid.clone(),
            text_chars: 0,
            iterations: 0,
        })
        .await;
        assert!(!is_busy(&sid));

        // A failed turn (AgentError) must also clear busy so delivery isn't stuck.
        busy().lock().expect("busy").insert(sid.clone());
        h.handle(&DomainEvent::AgentError {
            session_id: sid.clone(),
            message: "boom".into(),
            recoverable: true,
        })
        .await;
        assert!(!is_busy(&sid));
    }
}
