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

use crate::core::runtime::DispatchCtx;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use tinyagents_tasks::{
    CompletionDelivery, CompletionDeliveryConfig, CompletionRecord, DeliveryOutcome,
    DeliveryTarget, ThreadDeliveryLeases,
};

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use tinybus::EventHandler;
use tinybus::SubscriptionHandle;

mod persistence;
use persistence::{persist_delivery_reply, persist_then_announce};

use super::background_completions;
use super::busy_guard::is_busy;
use super::completion_notice::build_undelivered_notice;
use super::completion_owners;
use super::delivery_drain::DrainFence;
#[path = "delivery_events.rs"]
mod delivery_events;
use crate::core::runtime::CoreContext;
#[cfg(test)]
pub(super) use delivery_events::Drain;
pub(super) use delivery_events::{drain_schedule_in, owning_profiles};

/// Coalesce completions landing within this window into one delivery turn.
const DEBOUNCE: Duration = Duration::from_secs(3);

/// How long after boot a recovered, never-delivered completion waits before its
/// delivery turn, so providers and the session store are up first.
const RECOVERY_DELAY: Duration = Duration::from_secs(15);

/// Ask for a delivery attempt on `thread_id` soon (after a Stop cleared a stale
/// busy mark, or anything else that may have left a drain without its trigger).
pub(crate) fn kick_delivery(ctx: &DispatchCtx, thread_id: &str) {
    schedule_delivery(ctx, thread_id.to_string(), Duration::from_millis(300));
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
                    let dispatch = DispatchCtx::new(Arc::clone(&ctx));
                    CoreContext::sync_scope(ctx, || {
                        schedule_delivery(&dispatch, drain.thread_id, drain.delay)
                    })
                }
                None => {
                    schedule_delivery(&DispatchCtx::host_default(), drain.thread_id, drain.delay)
                }
            }
        }
    }
}

/// Schedule a debounced delivery attempt for a thread, fenced by the calling
/// profile's lease ([`DrainFence::current`]).
fn schedule_delivery(ctx: &DispatchCtx, thread_id: String, delay: Duration) {
    schedule_delivery_inner(
        ctx.clone(),
        thread_id,
        delay,
        false,
        DrainFence::current(ctx),
    );
}

/// `wait_idle`: after the delay, keep waiting while the thread's user turn is in
/// flight instead of giving up (see [`run_drain`]). `fence` drops the drain
/// once this node loses the owning profile's lease.
fn schedule_delivery_inner(
    ctx: DispatchCtx,
    thread_id: String,
    delay: Duration,
    wait_idle: bool,
    fence: DrainFence,
) {
    // Scoped: the thread -> workspace table is keyed per profile, so the delayed
    // task must run under the profile that scheduled it.
    ctx.clone().spawn(async move {
        let outcome = try_deliver(
            ctx.clone(),
            thread_id.clone(),
            fence.clone(),
            wait_idle,
            delay,
            |ctx, workspace, thread_id, notice| async move {
                run_system_turn_on_thread(&ctx, workspace, thread_id, notice)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            },
        )
        .await;
        if outcome == DeliveryOutcome::ParentBusy {
            // Still running past the ceiling: leave the record pending and
            // wait again rather than overlap the user's turn.
            log::debug!(
                "[background_delivery] recovered drain still busy; waiting again \
                 thread_id={thread_id}"
            );
            schedule_delivery_inner(
                ctx,
                thread_id,
                CompletionDeliveryConfig::default().busy_poll,
                true,
                fence,
            );
        }
    });
}

/// Boot recovery: redeliver completions a previous process finished but never
/// delivered. Each thread holding undelivered results gets one delivery attempt
/// after [`RECOVERY_DELAY`], through the normal idle-gated path, fenced by the
/// calling profile's lease when there is one. Returns the number of threads
/// scheduled.
pub(crate) fn recover_on_boot(ctx: &DispatchCtx, workspace_dir: &Path) -> usize {
    recover_fenced(ctx, workspace_dir, DrainFence::current(ctx))
}

fn recover_fenced(ctx: &DispatchCtx, workspace_dir: &Path, fence: DrainFence) -> usize {
    // Scheduling needs a runtime; check before claiming so a later call retries.
    if tokio::runtime::Handle::try_current().is_err() {
        log::warn!(
            "[background_delivery] no async runtime for boot recovery; will retry on the next call"
        );
        return 0;
    }
    // Once per workspace per process: the bootstrap workspace at startup, any
    // other the first time a spawn opens it.
    if !background_completions::claim_recovery_ctx(ctx, workspace_dir) {
        return 0;
    }
    let threads = background_completions::recover_pending_threads(ctx, workspace_dir);
    for thread_id in &threads {
        log::info!(
            "[background_delivery] scheduling redelivery of undelivered completions after restart \
             thread_id={thread_id}"
        );
        schedule_delivery_inner(
            ctx.clone(),
            thread_id.clone(),
            RECOVERY_DELAY,
            true,
            fence.clone(),
        );
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
pub(crate) fn recover_on_open(ctx: &DispatchCtx, workspace_dir: &Path, fence: DrainFence) -> usize {
    background_completions::forget_recovery_ctx(ctx, workspace_dir);
    let scheduled = recover_fenced(ctx, workspace_dir, fence);
    if scheduled == 0 {
        // Nothing pending: do not keep this profile's log handle cached for a
        // workspace that may never complete anything on this node.
        background_completions::forget_recovery_ctx(ctx, workspace_dir);
    }
    scheduled
}

/// Drain + deliver pending completions for a thread — if idle and not already
/// delivering. Batches everything ready at this instant into one system turn.
async fn try_deliver<D, Fut>(
    busy_ctx: DispatchCtx,
    thread_id: String,
    fence: DrainFence,
    recovering: bool,
    initial_delay: Duration,
    run_turn: D,
) -> DeliveryOutcome
where
    D: Fn(DispatchCtx, PathBuf, String, String) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    if !fence.admits() {
        log::info!(
            "[background_delivery] profile lease lost; not delivering thread_id={thread_id}"
        );
        return DeliveryOutcome::Fenced;
    }
    let Some(workspace_dir) = background_completions::workspace_for_thread(&busy_ctx, &thread_id)
    else {
        log::debug!("[background_delivery] no workspace for thread_id={thread_id}");
        return DeliveryOutcome::Empty;
    };
    let router = background_completions::router_for_workspace_ctx(&busy_ctx, &workspace_dir);
    let delivery = CompletionDelivery::new(
        CompletionDeliveryConfig {
            debounce: initial_delay,
            recovery_delay: RECOVERY_DELAY,
            retry_backoff: Duration::from_secs(2),
            busy_poll: CompletionDeliveryConfig::default().busy_poll,
            max_busy_polls: CompletionDeliveryConfig::default().max_busy_polls,
            batch_size: usize::MAX,
        },
        delivery_leases().clone(),
    );
    let ctx = busy_ctx.clone();
    let turn_workspace = workspace_dir.clone();
    let give_up_ctx = busy_ctx.clone();
    let format_router = router.clone();
    let turn_thread_id = thread_id.clone();
    let give_up_thread_id = thread_id.clone();
    let give_up_log_thread_id = thread_id.clone();
    let owner = match busy_ctx.tenant() {
        Ok(tenant) => tenant.profile_only(),
        Err(error) => {
            log::debug!("[background_delivery] no tenant for completion delivery: {error}");
            return DeliveryOutcome::Fenced;
        }
    };
    let tenant_key = crate::core::runtime::tenant::profile_key_in(&owner, "");
    let scoped_thread = crate::core::runtime::tenant::profile_key_in(&owner, &thread_id);
    let run_turn = &run_turn;
    let deliver = move |records: &[CompletionRecord]| {
        let notice = format_router.formatter().format_batch(records);
        let ctx = ctx.clone();
        let workspace_dir = turn_workspace.clone();
        let thread_id = turn_thread_id.clone();
        run_turn(ctx, workspace_dir, thread_id, notice)
    };
    let write_undelivered = move |records: &[CompletionRecord], error: &str| {
        let attempts = records
            .iter()
            .map(|record| record.attempts)
            .max()
            .unwrap_or(1);
        let notice = build_undelivered_notice(records, attempts, error);
        if let Some(notice) = notice {
            let run_id = format!("bgdeliver-undelivered-{}", uuid::Uuid::new_v4());
            if let Err(error) = persist_delivery_reply(
                &give_up_ctx,
                workspace_dir.clone(),
                &give_up_thread_id,
                &run_id,
                notice,
                false,
            ) {
                log::error!(
                    "[background_delivery] undelivered results LOST thread_id={give_up_log_thread_id} run_id={run_id} error={error}"
                );
            }
        }
    };
    let target = DeliveryTarget {
        router: &router,
        parent_key: &thread_id,
        tenant_key: &tenant_key,
        thread_key: &scoped_thread,
    };
    let outcome = if recovering {
        delivery
            .recover_on_boot_with_error(
                target,
                || is_busy(&busy_ctx, &thread_id),
                fence.clone(),
                deliver,
                write_undelivered,
            )
            .await
    } else {
        delivery
            .deliver_once_with_error(
                target,
                || is_busy(&busy_ctx, &thread_id),
                fence.clone(),
                deliver,
                write_undelivered,
            )
            .await
    };
    let retry_after = match outcome {
        DeliveryOutcome::Failed { count, gave_up } if count > gave_up => {
            Some(Duration::from_secs(2))
        }
        _ => None,
    };
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
        schedule_delivery_inner(busy_ctx, thread_id, delay, false, fence);
    }
    outcome
}

fn delivery_leases() -> &'static ThreadDeliveryLeases {
    static LEASES: OnceLock<ThreadDeliveryLeases> = OnceLock::new();
    LEASES.get_or_init(ThreadDeliveryLeases::default)
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
    ctx: &DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: String,
    prompt: String,
) -> Result<String, String> {
    let run_id = format!("bgdeliver-{}", uuid::Uuid::new_v4());
    let result = crate::web_chat::run_system_turn_on_thread(
        ctx,
        &thread_id,
        &run_id,
        &prompt,
        crate::core::runtime::turn_origin::AgentTurnOrigin::Cli,
    )
    .await;

    persist_then_announce(
        result,
        |content, success| {
            persist_delivery_reply(
                ctx,
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
                ctx, "system", &thread_id, &run_id, response, None,
            ),
            Err(error) => crate::web_chat::publish_web_channel_event(
                crate::web_chat::WebChannelEvent::ChatError {
                    context: crate::web_chat::event_context(
                        "system",
                        thread_id.clone(),
                        run_id.clone(),
                    ),
                    message: Some(error.clone()),
                    error_type: Some("agent_error".to_string()),
                    error_source: None,
                    error_retryable: None,
                    error_retry_after_ms: None,
                    error_provider: None,
                    error_fallback_available: None,
                    copy_key: None,
                    copy_params: None,
                    guardrail: None,
                },
            ),
        },
    )
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
