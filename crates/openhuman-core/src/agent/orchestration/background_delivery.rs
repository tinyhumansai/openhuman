//! Delivery subsystem for finished detached background sub-agents.
//!
//! Surfaces completions held by the harness `CompletionRouter`
//! ([`super::background_completions`]) back into the originating chat as a
//! single **system-injected** turn:
//!   * **idle-gated** — never mid-turn; defers while a user turn is in flight,
//!   * **debounced** — a burst of completions batches into one turn,
//!   * **batched** — every result ready at delivery time goes in one turn,
//!     each tagged by its sub-agent process id,
//!   * **at-least-once** — a claimed batch stays leased until the turn has
//!     landed (`mark_delivered`); a failed turn releases it for the next drain,
//!     and a completion that was never delivered (a restart in between) is
//!     claimed again on boot ([`recover_on_boot`]),
//!   * **bounded** — the router counts attempts per record and returns the ones
//!     that gave up after [`DEFAULT_MAX_ATTEMPTS`] (`mark_failed`); a turn that
//!     can never succeed would otherwise retry forever, because a failed turn
//!     re-triggers this module,
//!   * **never silently lost** — a record that gave up is written into the
//!     thread verbatim, with an explicit notice that delivery failed and why.
//!
//! The queue, dedupe, tombstones and attempt counting are the router's. What
//! stays here is the host policy: when a thread is idle, the delivery turn and
//! the web_chat events, and the give-up writer.
//!
//! The delivery turn runs on the originating thread's own chat session
//! (`web_chat::run_system_turn_on_thread`) so it sees the conversation and
//! appends to the thread's transcript. It persists its reply before announcing
//! `chat_done`, so a reconnect cannot lose a completed delegated result.

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tinyagents_tasks::{CompletionRecord, CompletionRouter};

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use tinybus::EventHandler;
use tinybus::SubscriptionHandle;

use super::background_completions;
use super::busy_guard::is_busy;
#[cfg(test)]
use super::busy_guard::{busy, clear_busy_for_thread, TurnBusy};
use super::completion_notice::build_undelivered_notice;
use super::completion_owners;
use super::delivery_drain::{run_drain, DrainFence, DrainOutcome, RECOVERY_BUSY_POLL};
use crate::core::runtime::tenant;
use crate::core::runtime::CoreContext;

/// Coalesce completions landing within this window into one delivery turn.
const DEBOUNCE: Duration = Duration::from_secs(3);

/// How long after boot a recovered, never-delivered completion waits before its
/// delivery turn, so providers and the session store are up first.
const RECOVERY_DELAY: Duration = Duration::from_secs(15);

/// Threads whose delivery turn is in flight — prevents two concurrent turns.
/// Keyed by [`tenant::profile_key`] of the thread id, for the same reason.
fn delivering() -> &'static Mutex<HashSet<String>> {
    static D: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Ask for a delivery attempt on `thread_id` soon (after a Stop cleared a stale
/// busy mark, or anything else that may have left a drain without its trigger).
pub(crate) fn kick_delivery(thread_id: &str) {
    schedule_delivery(thread_id.to_string(), Duration::from_millis(300));
}

struct BackgroundDeliveryHandler;

#[async_trait]
impl EventHandler<DomainEvent> for BackgroundDeliveryHandler {
    fn name(&self) -> &str {
        "agent_orchestration::background_delivery"
    }

    async fn handle(&self, event: &DomainEvent) {
        // Busy state is not tracked here (off-task, no tenant): see `TurnBusy`.
        let saas = crate::core::runtime::is_saas();
        // Opening a profile is async (it may take its lease), so resolve the
        // owning profiles' contexts first and hand the sync scheduler a lookup.
        let mut live = std::collections::HashMap::new();
        if saas {
            for profile in owning_profiles(event) {
                if let Some(ctx) = completion_owners::context_for_profile(&profile).await {
                    live.insert(profile, ctx);
                }
            }
        }
        for drain in drain_schedule_in(saas, event, |p| live.get(p).cloned()) {
            match drain.owner {
                // Re-enter the owner's scope so the scheduled task inherits it.
                Some(ctx) => {
                    CoreContext::sync_scope(ctx, || schedule_delivery(drain.thread_id, drain.delay))
                }
                None => schedule_delivery(drain.thread_id, drain.delay),
            }
        }
    }
}

/// One drain an event asks for: the thread, how soon, and the context of the
/// profile that owns it (`None` on the desktop, where nothing is per profile).
pub(super) struct Drain {
    pub(super) owner: Option<Arc<CoreContext>>,
    pub(super) thread_id: String,
    pub(super) delay: Duration,
}

/// Which thread to drain, and after how long, for an event, outside any
/// profile. A session that maps to no thread (cron, voice, skills) has nothing
/// to deliver into.
#[cfg(test)]
fn drain_schedule(event: &DomainEvent) -> Option<(String, Duration)> {
    let drain = drain_schedule_in(false, event, |_| None)
        .into_iter()
        .next()?;
    Some((drain.thread_id, drain.delay))
}

/// The session, task and delay an event drains on, if it asks for a drain.
fn drain_target(event: &DomainEvent) -> Option<(&String, Option<&String>, Duration)> {
    match event {
        // A user turn just ended (or failed) — drain anything that finished while
        // it ran.
        DomainEvent::AgentTurnCompleted { session_id, .. }
        | DomainEvent::AgentError { session_id, .. } => {
            Some((session_id, None, Duration::from_millis(300)))
        }
        // Any subagent terminal state — completed, failed, or awaiting-user — can
        // arrive after the parent turn already went idle. Schedule a debounced
        // drain for all three so the pending result is delivered promptly instead
        // of sitting until some unrelated later turn. Only `SubagentCompleted`
        // used to trigger a drain, so a failure (or an awaiting-user pause) after
        // the parent turn went idle left the chat stuck on the original
        // "Accepted" response (#4896). Debounce so a burst batches into a single
        // turn.
        DomainEvent::SubagentCompleted {
            parent_session,
            task_id,
            ..
        }
        | DomainEvent::SubagentFailed {
            parent_session,
            task_id,
            ..
        }
        | DomainEvent::SubagentAwaitingUser {
            parent_session,
            task_id,
            ..
        } => Some((parent_session, Some(task_id), DEBOUNCE)),
        _ => None,
    }
}

/// Profiles recorded as owning the task (else the session).
fn owners_of(session: &str, task: Option<&String>) -> Vec<String> {
    let mut profiles = task
        .map(|t| completion_owners::profiles_of(t))
        .unwrap_or_default();
    if profiles.is_empty() {
        profiles = completion_owners::profiles_of(session);
    }
    profiles
}

/// The profiles whose contexts [`drain_schedule_in`] will ask for.
fn owning_profiles(event: &DomainEvent) -> Vec<String> {
    drain_target(event)
        .map(|(session, task, _)| owners_of(session, task))
        .unwrap_or_default()
}

/// The drains for `event`. This subscriber runs off-task, with no tenant scope,
/// but the thread tables are keyed per profile: the profiles that recorded the
/// completion (by task id) or ran the session are looked up in
/// [`completion_owners`] and each is resolved through `resolve` and drained in
/// its own scope, where it reaches only its own tables. In SaaS (`saas`) an id
/// with no owner is dropped; elsewhere it drains unscoped, as on the desktop.
pub(super) fn drain_schedule_in(
    saas: bool,
    event: &DomainEvent,
    resolve: impl Fn(&str) -> Option<Arc<CoreContext>>,
) -> Vec<Drain> {
    let Some((session, task, delay)) = drain_target(event) else {
        return Vec::new();
    };
    // A task id is core-minted and unique, so it names its one owner; a
    // session id can be shared by profiles, so it may name several.
    let profiles = owners_of(session, task);
    let thread_in = |owner: Option<Arc<CoreContext>>| {
        let thread_id = match &owner {
            Some(ctx) => CoreContext::sync_scope(Arc::clone(ctx), || {
                background_completions::thread_for_session(session)
            }),
            None => background_completions::thread_for_session(session),
        };
        if thread_id.is_none() {
            log::trace!("[background_delivery] session has no delivery thread; not scheduling");
        }
        thread_id.map(|thread_id| Drain {
            owner,
            thread_id,
            delay,
        })
    };
    if profiles.is_empty() {
        if saas {
            log::debug!(
                "[background_delivery] no owning profile for the session or task; dropping \
                 the drain (fails closed in SaaS)"
            );
            return Vec::new();
        }
        return thread_in(None).into_iter().collect();
    }
    profiles
        .iter()
        .filter_map(|profile| match resolve(profile) {
            Some(ctx) => thread_in(Some(ctx)),
            None => {
                log::debug!("[background_delivery] owning profile has no live context; dropping");
                None
            }
        })
        .collect()
}

/// Schedule a debounced delivery attempt for a thread, fenced by the calling
/// profile's lease ([`DrainFence::current`]).
fn schedule_delivery(thread_id: String, delay: Duration) {
    schedule_delivery_inner(thread_id, delay, false, DrainFence::current());
}

/// `wait_idle`: after the delay, keep waiting while the thread's user turn is in
/// flight instead of giving up (see [`run_drain`]). `fence` drops the drain
/// once this node loses the owning profile's lease.
fn schedule_delivery_inner(thread_id: String, delay: Duration, wait_idle: bool, fence: DrainFence) {
    #[cfg(test)]
    scheduled_for_test()
        .lock()
        .expect("scheduled")
        .push((thread_id.clone(), delay));
    // Scoped: the thread -> workspace table is keyed per profile, so the
    // delayed task must run under the profile that scheduled it.
    crate::core::runtime::spawn_scoped(async move {
        let turn_fence = fence.clone();
        let turn_thread = thread_id.clone();
        let outcome = run_drain(
            &thread_id,
            delay,
            wait_idle,
            &fence,
            RECOVERY_BUSY_POLL,
            || try_deliver(turn_thread, turn_fence),
        )
        .await;
        if outcome == DrainOutcome::StillBusy {
            // Still running past the ceiling: leave the record pending and
            // wait again rather than overlap the user's turn.
            log::debug!(
                "[background_delivery] recovered drain still busy; waiting again \
                 thread_id={thread_id}"
            );
            schedule_delivery_inner(thread_id, RECOVERY_BUSY_POLL, true, fence);
        }
    });
}

/// Every drain the handler scheduled, so a test can assert the event ->
/// schedule wiring without running a delivery turn.
#[cfg(test)]
fn scheduled_for_test() -> &'static Mutex<Vec<(String, Duration)>> {
    static SCHEDULED: OnceLock<Mutex<Vec<(String, Duration)>>> = OnceLock::new();
    SCHEDULED.get_or_init(|| Mutex::new(Vec::new()))
}

/// Boot recovery: redeliver completions a previous process finished but never
/// delivered. Each thread holding undelivered results gets one delivery attempt
/// after [`RECOVERY_DELAY`], through the normal idle-gated path, fenced by the
/// calling profile's lease when there is one. Returns the number of threads
/// scheduled.
pub(crate) fn recover_on_boot(workspace_dir: &Path) -> usize {
    recover_fenced(workspace_dir, DrainFence::current())
}

fn recover_fenced(workspace_dir: &Path, fence: DrainFence) -> usize {
    // Scheduling needs a runtime; check before claiming so a later call retries.
    if tokio::runtime::Handle::try_current().is_err() {
        log::warn!(
            "[background_delivery] no async runtime for boot recovery; will retry on the next call"
        );
        return 0;
    }
    // Once per workspace per process: the bootstrap workspace at startup, any
    // other the first time a spawn opens it.
    if !background_completions::claim_recovery(workspace_dir) {
        return 0;
    }
    let threads = background_completions::recover_pending_threads(workspace_dir);
    for thread_id in &threads {
        log::info!(
            "[background_delivery] scheduling redelivery of undelivered completions after restart \
             thread_id={thread_id}"
        );
        schedule_delivery_inner(thread_id.clone(), RECOVERY_DELAY, true, fence.clone());
    }
    threads.len()
}

/// Recovery for a profile that was just opened: forget the workspace's earlier
/// claim (a profile released and re-leased may have results another node left
/// pending since), then recover. Duplicate drains are harmless: delivery is
/// lease-claimed. `fence` is the grant the profile was just opened under (the
/// host has not published the profile yet, so it cannot be looked up): a drain
/// that comes due after this node lost that lease is dropped, and the records
/// stay pending for the node that holds it now.
pub(crate) fn recover_on_open(workspace_dir: &Path, fence: DrainFence) -> usize {
    background_completions::forget_recovery(workspace_dir);
    let scheduled = recover_fenced(workspace_dir, fence);
    if scheduled == 0 {
        // Nothing pending: do not keep this profile's log handle cached for a
        // workspace that may never complete anything on this node.
        background_completions::forget_recovery(workspace_dir);
    }
    scheduled
}

/// Claim everything ready for a thread **right now** (sync, testable): `None`
/// (queue untouched) when the thread is busy or nothing is pending. A claimed
/// batch stays leased until the caller settles it with `mark_delivered` /
/// `mark_failed` / `release`.
fn claim_ready(router: &CompletionRouter, thread_id: &str) -> Option<Vec<CompletionRecord>> {
    if is_busy(thread_id) {
        return None;
    }
    match router.claim_pending(thread_id, usize::MAX) {
        Ok(batch) if !batch.is_empty() => Some(batch),
        Ok(_) => None,
        Err(error) => {
            log::error!("[background_delivery] claim failed thread_id={thread_id} error={error}");
            None
        }
    }
}

/// Drain + deliver pending completions for a thread — if idle and not already
/// delivering. Batches everything ready at this instant into one system turn.
async fn try_deliver(thread_id: String, fence: DrainFence) {
    if !fence.admits() {
        log::info!(
            "[background_delivery] profile lease lost; not delivering thread_id={thread_id}"
        );
        return;
    }
    let Some(workspace_dir) = background_completions::workspace_for_thread(&thread_id) else {
        log::debug!("[background_delivery] no workspace for thread_id={thread_id}");
        return;
    };
    let router = background_completions::router_for_workspace(&workspace_dir);
    let turn_workspace = workspace_dir.clone();
    let retry_after = try_deliver_with(
        thread_id.clone(),
        router,
        move |thread_id, notice| {
            let workspace_dir = turn_workspace.clone();
            async move { run_system_turn_on_thread(workspace_dir, thread_id, notice).await }
        },
        move |thread_id, notice| async move {
            persist_undelivered(workspace_dir, thread_id, notice).await
        },
    )
    .await;
    // A failed turn that published no `AgentError` (a session checkout failure,
    // or the only post-boot attempt) re-triggers nothing, so a quiet thread would
    // keep its record pending for the rest of the process. The attempt ceiling
    // bounds this: after the last attempt the record is handed to the give-up
    // writer instead of retried.
    if let Some(delay) = retry_after {
        log::debug!(
            "[background_delivery] rescheduling a failed delivery thread_id={thread_id} \
             delay_ms={}",
            delay.as_millis()
        );
        schedule_delivery_inner(thread_id, delay, false, fence);
    }
}

/// One pass of the delivery loop whose turn only counts itself, for tests
/// outside this module.
#[cfg(test)]
pub(super) async fn try_deliver_for_test(
    thread_id: String,
    router: Arc<CompletionRouter>,
    turns: Arc<std::sync::atomic::AtomicU32>,
) -> Option<Duration> {
    try_deliver_with(
        thread_id,
        router,
        move |_, _| {
            turns.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { Ok(String::new()) }
        },
        |_, _| async {},
    )
    .await
}

/// Backoff before retrying a delivery whose record has now failed `attempts` times.
fn retry_backoff(attempts: u32) -> Duration {
    Duration::from_secs(2u64.saturating_pow(attempts.min(5)))
}

/// Last resort when the delivery turn has failed [`DEFAULT_MAX_ATTEMPTS`]
/// times: append the results to the thread directly, with no model turn.
///
/// This is the whole point of the ceiling — retries stop, but the user is still
/// told. If even this append fails there is nowhere left to put the result, so
/// it is logged at error and the chain ends; that is the one path on which a
/// result is genuinely lost, and it requires the conversation store to be
/// failing as well as the agent.
async fn persist_undelivered(workspace_dir: PathBuf, thread_id: String, notice: String) {
    let run_id = format!("bgdeliver-undelivered-{}", uuid::Uuid::new_v4());
    match persist_delivery_reply(workspace_dir, &thread_id, &run_id, notice, false) {
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

/// Delivery-loop core with an injected router, turn executor and give-up
/// sink. Keeping the queue and retry boundary independent from host execution
/// lets tests prove a failed durable append releases the batch before any
/// terminal announcement is observable, and that a record which exhausts its
/// attempts is handed to `on_undeliverable` rather than dropped.
async fn try_deliver_with<F, Fut, G, GFut>(
    thread_id: String,
    router: Arc<CompletionRouter>,
    mut deliver: F,
    on_undeliverable: G,
) -> Option<Duration>
where
    F: FnMut(String, String) -> Fut,
    Fut: Future<Output = Result<String, String>>,
    G: FnOnce(String, String) -> GFut,
    GFut: Future<Output = ()>,
{
    let mut retry_after = None;
    if is_busy(&thread_id) {
        return None;
    }
    // Claim the delivery slot — held for the WHOLE delivery (including the
    // awaited turn) so a concurrent completion can't start a second delivery
    // turn on the same thread. Skip if a delivery is already in flight. The
    // guard frees the slot, and any lease still held, even if this future is
    // dropped mid-turn.
    let mut slot = DeliverySlot::claim(&thread_id, router.clone())?;

    // A busy thread defers *before* the claim: the claim counts a delivery
    // attempt, and a user who keeps typing must not burn a record's budget.
    if let Some(batch) = claim_ready(&router, &thread_id) {
        let task_ids: Vec<String> = batch.iter().map(|c| c.task_id.clone()).collect();
        slot.hold(&task_ids);
        // A user turn can start between the idle check inside `claim_ready` and
        // the awaited turn below; re-check so a system turn is never streamed
        // concurrently with it. Dropping the slot releases the lease (the claim
        // already counted one attempt; this window is narrow).
        if is_busy(&thread_id) {
            log::debug!(
                "[background_delivery] thread became busy after the claim; deferring \
                 thread_id={thread_id}"
            );
            return None;
        }
        let notice = router.formatter().format_batch(&batch);
        log::info!(
            "[background_delivery] delivering {} batched background result(s) thread_id={thread_id}",
            batch.len()
        );
        match deliver(thread_id.clone(), notice).await {
            Ok(_) => {
                if let Err(error) = router.mark_delivered(&task_ids) {
                    // The reply is already in the thread; the record stays
                    // pending, so a later drain or a restart delivers it again
                    // (at-least-once).
                    log::error!(
                        "[background_delivery] delivered but could not settle records \
                         thread_id={thread_id} tasks=[{}] error={error}",
                        task_ids.join(",")
                    );
                }
            }
            Err(e) => {
                // A `SESSION_CHECKOUT_FAILURE`-prefixed error (the session could
                // not be checked out, so no turn ran at all) counts the same as a
                // turn that ran and failed. That is deliberate: the budget
                // measures "the result was not delivered", which is equally true
                // either way, and giving up is not lossy — the record is written
                // into the thread rather than discarded. A checkout failure
                // publishes no `AgentError`, so it cannot drive the self-retrigger
                // loop on its own; it only ever spends the budget.
                match router.mark_failed(&task_ids) {
                    Ok(gave_up) if !gave_up.is_empty() => {
                        // Stop retrying, but do NOT drop: the results exist and
                        // the user is owed them. Hand them to the give-up sink,
                        // which writes them into the thread verbatim along with
                        // an explicit statement that delivery failed and why.
                        let attempts = gave_up.iter().map(|r| r.attempts).max().unwrap_or(0);
                        log::warn!(
                            "[background_delivery] giving up on the delivery turn after \
                             {attempts} attempts; writing {} result(s) into the thread \
                             instead thread_id={thread_id} tasks=[{}] error={e}",
                            gave_up.len(),
                            gave_up
                                .iter()
                                .map(|c| c.task_id.as_str())
                                .collect::<Vec<_>>()
                                .join(","),
                        );
                        if let Some(undelivered) = build_undelivered_notice(&gave_up, attempts, &e)
                        {
                            on_undeliverable(thread_id.clone(), undelivered).await;
                        }
                    }
                    Ok(_) => {
                        log::warn!(
                            "[background_delivery] delivery turn failed thread_id={thread_id} \
                             tasks=[{}] max_attempts={} error={e}",
                            task_ids.join(","),
                            router.max_attempts()
                        );
                        let attempts = router
                            .pending_for(&thread_id)
                            .iter()
                            .map(|r| r.attempts)
                            .max()
                            .unwrap_or(1);
                        retry_after = Some(retry_backoff(attempts));
                    }
                    Err(error) => {
                        log::error!(
                            "[background_delivery] could not record the failed delivery \
                             thread_id={thread_id} error={error}"
                        );
                        router.release(&task_ids);
                    }
                }
                // A batch can mix records at different attempt counts: some gave
                // up above, the rest are still pending. Nothing else would wake a
                // quiet thread for them, so ask for a retry.
                if retry_after.is_none() {
                    if let Some(attempts) = router
                        .pending_for(&thread_id)
                        .iter()
                        .map(|r| r.attempts)
                        .max()
                    {
                        retry_after = Some(retry_backoff(attempts));
                    }
                }
            }
        }
    }

    // The slot is released only AFTER the turn settles (on drop).
    retry_after
}

/// The per-thread (per profile) delivery slot, plus the batch lease.
/// Dropping it frees both: `release` on an already-settled record is a no-op,
/// so a settled batch is untouched and an abandoned one is claimable again.
struct DeliverySlot {
    /// [`tenant::profile_key`] of the thread id.
    key: String,
    router: Arc<CompletionRouter>,
    held: Vec<String>,
}

impl DeliverySlot {
    /// `None` when a delivery is already in flight for the thread.
    fn claim(thread_id: &str, router: Arc<CompletionRouter>) -> Option<Self> {
        let key = tenant::profile_key(thread_id);
        let mut d = delivering().lock().expect("delivering poisoned");
        if !d.insert(key.clone()) {
            return None;
        }
        Some(Self {
            key,
            router,
            held: Vec::new(),
        })
    }

    fn hold(&mut self, task_ids: &[String]) {
        self.held = task_ids.to_vec();
    }
}

impl Drop for DeliverySlot {
    fn drop(&mut self) {
        if !self.held.is_empty() {
            self.router.release(&self.held);
        }
        if let Ok(mut d) = delivering().lock() {
            d.remove(&self.key);
        }
    }
}

/// Run one system-authored delivery turn on an existing conversation thread.
/// It only delivers a detached sub-agent result already produced by
/// the completion router.
///
/// The turn runs on the thread's own session (`web_chat::run_system_turn_on_thread`),
/// never on a throwaway host: the model presents the result in the context of
/// what the user asked, the warm session learns the result was delivered, and
/// the turn lands in the thread's transcript instead of a competing one that a
/// later cold-boot resume would prefer — which is how a restart used to drop
/// every turn before the delivery notice.
async fn run_system_turn_on_thread(
    workspace_dir: PathBuf,
    thread_id: String,
    prompt: String,
) -> Result<String, String> {
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
                workspace_dir.clone(),
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

#[cfg(test)]
#[path = "background_delivery_slots_tests.rs"]
mod slots_tests;
