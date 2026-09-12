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

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use tinybus::EventHandler;
use tinybus::SubscriptionHandle;

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
impl EventHandler<DomainEvent> for BackgroundDeliveryHandler {
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
            // Any subagent terminal state — completed, failed, or awaiting-user —
            // can arrive after the parent turn already went idle. Schedule a
            // debounced drain for all three so the pending result is delivered
            // promptly instead of sitting until some unrelated later turn. Only
            // `SubagentCompleted` used to trigger a drain, so a failure (or an
            // awaiting-user pause) after the parent turn went idle left the chat
            // stuck on the original "Accepted" response (#4896). Debounce so a
            // burst batches into a single turn.
            DomainEvent::SubagentCompleted { parent_session, .. }
            | DomainEvent::SubagentFailed { parent_session, .. }
            | DomainEvent::SubagentAwaitingUser { parent_session, .. } => {
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
        // Preserve the terminal outcome on requeue so a failed / awaiting-input
        // result isn't downgraded to a success when a delivery turn fails (#4896).
        background_completions::record_outcome(
            session,
            c.task_id,
            c.agent_id,
            c.summary,
            c.parent_thread_id,
            c.outcome,
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
        if let (Some(thread_id), Some(notice)) = (
            background_completions::batch_thread_id(&batch),
            background_completions::build_batched_notice(&batch),
        ) {
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
        } else {
            log::warn!(
                "[background_delivery] dropping headless batch session={session} count={}",
                batch.len()
            );
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
pub(crate) fn register_background_delivery() {
    static HANDLE: OnceLock<Option<SubscriptionHandle>> = OnceLock::new();
    HANDLE.get_or_init(|| BUS.subscribe(Arc::new(BackgroundDeliveryHandler)));
}

#[cfg(test)]
#[path = "background_delivery_tests.rs"]
mod tests;
