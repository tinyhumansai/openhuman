//! Delivery subsystem for finished detached background sub-agents.
//!
//! Surfaces results recorded in [`super::background_completions`] back into the
//! originating chat as a single **system-injected** turn:
//!   * **idle-gated** — never mid-turn; defers while a user turn is in flight,
//!   * **debounced** — a burst of completions batches into one turn,
//!   * **batched** — every result ready at delivery time goes in one turn,
//!     each tagged by its sub-agent process id,
//!   * **bounded** — a failed turn requeues its batch, but only
//!     [`MAX_DELIVERY_ATTEMPTS`] times; a turn that can never succeed would
//!     otherwise retry forever, because a failed turn re-triggers this module,
//!   * **never silently lost** — once the retries are spent the results are
//!     written into the thread verbatim, with an explicit notice that delivery
//!     failed and why, instead of being discarded.
//!
//! The delivery turn runs on the originating thread's own chat session
//! (`web_chat::run_system_turn_on_thread`) so it sees the conversation and
//! appends to the thread's transcript. It persists its reply before announcing
//! `chat_done`, so a reconnect cannot lose a completed delegated result.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use tinybus::EventHandler;
use tinybus::SubscriptionHandle;

use super::background_completions;

/// Coalesce completions landing within this window into one delivery turn.
const DEBOUNCE: Duration = Duration::from_secs(3);

/// Upper bound on consecutive failed delivery turns for one session before the
/// loop stops retrying and writes the results into the thread instead.
///
/// The requeue below exists so a *transient* failure doesn't lose a result
/// (#4896), but on its own it is unbounded — and this module re-triggers
/// itself: a failed delivery turn publishes `DomainEvent::AgentError`, which
/// our own handler turns straight back into a scheduled drain 300 ms later.
/// Against a turn that can *never* succeed (refused inference, expired
/// session, revoked integration, backend outage) that is a self-sustaining
/// loop, not a retry. Observed in production 2026-09-21: 178 delivery attempts
/// per minute against a single thread, 4,389 in one day, and a 432 MB log.
///
/// Five attempts keeps the transient case working. Past that the retries stop,
/// but the results are **not** discarded: they are handed to the give-up sink,
/// which persists them into the thread with an explicit failed-delivery notice.
/// Ending the storm must not cost the user the result that caused it.
const MAX_DELIVERY_ATTEMPTS: u32 = 5;

/// Consecutive failed delivery turns per session. Cleared on a delivered batch
/// and once a batch has been handed to the give-up sink, so the budget tracks a
/// single failure chain and a later batch always starts fresh.
fn attempts() -> &'static Mutex<HashMap<String, u32>> {
    static A: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
    A.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record a failed delivery turn; returns the new consecutive-failure count.
fn note_failed_attempt(session: &str) -> u32 {
    let mut a = attempts().lock().expect("delivery attempts poisoned");
    let n = a.entry(session.to_string()).or_insert(0);
    *n += 1;
    *n
}

/// Reset a session's failure budget — on a delivered batch, or once a batch has
/// been handed to the give-up sink and the chain is over.
fn clear_attempts(session: &str) {
    attempts()
        .lock()
        .expect("delivery attempts poisoned")
        .remove(session);
}

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
    try_deliver_with(
        session,
        |thread_id, notice| async move { run_system_turn_on_thread(thread_id, notice).await },
        |thread_id, notice| async move { persist_undelivered(thread_id, notice).await },
    )
    .await;
}

/// Last resort when the delivery turn has failed [`MAX_DELIVERY_ATTEMPTS`]
/// times: append the results to the thread directly, with no model turn.
///
/// This is the whole point of the ceiling — retries stop, but the user is still
/// told. If even this append fails there is nowhere left to put the result, so
/// it is logged at error and the chain ends; that is the one path on which a
/// result is genuinely lost, and it requires the conversation store to be
/// failing as well as the agent.
async fn persist_undelivered(thread_id: String, notice: String) {
    let run_id = format!("bgdeliver-undelivered-{}", uuid::Uuid::new_v4());
    let config = match crate::config::Config::load_or_init().await {
        Ok(config) => config,
        Err(error) => {
            log::error!(
                "[background_delivery] undelivered results LOST — could not load config to \
                 persist them thread_id={thread_id} run_id={run_id} error={error:#}"
            );
            return;
        }
    };
    match persist_delivery_reply(
        config.workspace_dir.clone(),
        &thread_id,
        &run_id,
        notice,
        false,
    ) {
        Ok(()) => log::warn!(
            "[background_delivery] delivery turn gave up; wrote results into the thread \
             verbatim instead thread_id={thread_id} run_id={run_id}"
        ),
        Err(error) => log::error!(
            "[background_delivery] undelivered results LOST — could not append them to the \
             thread thread_id={thread_id} run_id={run_id} error={error}"
        ),
    }
}

/// Delivery-loop core with an injected turn executor and an injected
/// give-up sink. Keeping the queue and retry boundary independent from host
/// execution lets tests prove a failed durable append is requeued before any
/// terminal announcement is observable, and that a batch which exhausts its
/// retries is handed to `on_undeliverable` rather than dropped.
async fn try_deliver_with<F, Fut, G, GFut>(session: String, mut deliver: F, on_undeliverable: G)
where
    F: FnMut(String, String) -> Fut,
    Fut: Future<Output = Result<String, String>>,
    G: FnOnce(String, String) -> GFut,
    GFut: Future<Output = ()>,
{
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
            match deliver(thread_id.clone(), notice).await {
                Ok(_) => clear_attempts(&session),
                Err(e) => {
                    // Count only a failed *turn*. The busy re-check above also
                    // requeues, but that is a deferral, not a failure — letting
                    // it burn the budget would drop results just because the
                    // user kept typing.
                    //
                    // A `SESSION_CHECKOUT_FAILURE`-prefixed error (the session
                    // could not be checked out, so no turn ran at all) counts
                    // the same as a turn that ran and failed. That is
                    // deliberate: the budget measures "the result was not
                    // delivered", which is equally true either way, and giving
                    // up is no longer lossy — the batch is written into the
                    // thread rather than discarded. Note a checkout failure
                    // publishes no `AgentError`, so it cannot drive the
                    // self-retrigger loop on its own; it only ever spends the
                    // budget, never extends it.
                    let attempt = note_failed_attempt(&session);
                    if attempt >= MAX_DELIVERY_ATTEMPTS {
                        // Stop retrying, but do NOT drop: the results exist and
                        // the user is owed them. Hand them to the give-up sink,
                        // which writes them into the thread verbatim along with
                        // an explicit statement that delivery failed and why.
                        log::warn!(
                            "[background_delivery] giving up on the delivery turn after \
                             {attempt} consecutive failures; writing {} result(s) into the \
                             thread instead session={session} thread_id={thread_id} \
                             tasks=[{}] error={e}",
                            batch.len(),
                            batch
                                .iter()
                                .map(|c| c.task_id.as_str())
                                .collect::<Vec<_>>()
                                .join(","),
                        );
                        clear_attempts(&session);
                        if let Some(undelivered) =
                            background_completions::build_undelivered_notice(&batch, attempt, &e)
                        {
                            on_undeliverable(thread_id, undelivered).await;
                        }
                    } else {
                        log::warn!(
                            "[background_delivery] delivery turn failed session={session} \
                             attempt={attempt}/{MAX_DELIVERY_ATTEMPTS} error={e}"
                        );
                        requeue(&session, batch); // don't lose results on a failed turn
                    }
                }
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

/// Run one system-authored delivery turn on an existing conversation thread.
/// It only delivers a detached sub-agent result already produced by
/// `background_completions`.
///
/// The turn runs on the thread's own session (`web_chat::run_system_turn_on_thread`),
/// never on a throwaway host: the model presents the result in the context of
/// what the user asked, the warm session learns the result was delivered, and
/// the turn lands in the thread's transcript instead of a competing one that a
/// later cold-boot resume would prefer — which is how a restart used to drop
/// every turn before the delivery notice.
async fn run_system_turn_on_thread(thread_id: String, prompt: String) -> Result<String, String> {
    let config = crate::config::Config::load_or_init()
        .await
        .map_err(|error| format!("load config: {error:#}"))?;
    let run_id = format!("bgdeliver-{}", uuid::Uuid::new_v4());
    let result = crate::web_chat::run_system_turn_on_thread(
        &thread_id,
        &run_id,
        &prompt,
        crate::agent::turn_origin::AgentTurnOrigin::Cli,
    )
    .await;

    persist_then_announce(
        result,
        |content, success| {
            persist_delivery_reply(
                config.workspace_dir.clone(),
                &thread_id,
                &run_id,
                content.to_string(),
                success,
            )
            .map_err(|error| {
                tracing::warn!(%thread_id, %run_id, %error, "[background_delivery] could not persist reply before announcement");
                error
            })
        },
        |result| match result {
            Ok(response) => crate::web_chat::presentation::deliver_response_single_bubble(
                "system", &thread_id, &run_id, response, None,
            ),
            Err(error) => {
                crate::web_chat::publish_web_channel_event(crate::web_chat::WebChannelEvent {
                    event: "chat_error".to_string(),
                    client_id: "system".to_string(),
                    thread_id: thread_id.clone(),
                    request_id: run_id.clone(),
                    message: Some(error.clone()),
                    error_type: Some("agent_error".to_string()),
                    ..Default::default()
                })
            }
        },
    )
}

/// Persist a terminal reply, then announce it. A persistence failure returns
/// before `announce` runs, allowing the outer delivery loop to requeue the
/// completed background batch without publishing a phantom terminal event.
fn persist_then_announce<P, A>(
    result: Result<String, String>,
    persist: P,
    announce: A,
) -> Result<String, String>
where
    P: FnOnce(&str, bool) -> Result<(), String>,
    A: FnOnce(&Result<String, String>),
{
    let (content, success) = match &result {
        Ok(text) => (text.trim().to_string(), true),
        Err(error) => (format!("Run failed: {error}"), false),
    };
    if !content.is_empty() {
        persist(&content, success)?;
    }
    announce(&result);
    result
}

/// Durably append a background-delivery reply before publishing its terminal
/// chat event. Callers must propagate failures: publishing `chat_done` or
/// `chat_error` without a stored row loses the result across reconnects.
fn persist_delivery_reply(
    workspace_dir: std::path::PathBuf,
    thread_id: &str,
    run_id: &str,
    content: String,
    success: bool,
) -> Result<(), String> {
    crate::threads::store::append_message(
        workspace_dir,
        thread_id,
        crate::threads::store::ConversationMessage {
            id: crate::threads::store::run_reply_message_id(run_id),
            content,
            message_type: "text".to_string(),
            extra_metadata: json!({
                "scope": "background_delivery",
                "success": success,
                "requestId": run_id,
            }),
            sender: "agent".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .map(|_| ())
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
