//! Host-authored turns on an existing conversation thread.
//!
//! Background-delivery notices and goal continuations are turns the *host*
//! injects into a user's thread. They must run on the thread's own session —
//! the cached agent, or one cold-boot resumed from the thread's transcript —
//! for two reasons:
//!
//! * the model needs the conversation to present a delegated result in
//!   context ("here's the inbox summary you asked for", not "what are we doing
//!   with the inbox?"), and
//! * the turn must land in the thread's durable transcript. A throwaway
//!   `OpenHumanSessionHost` bound to the thread wrote a competing root
//!   transcript with a newer `created`, and the next cold-boot resume picked
//!   that one — three rows — over the real history, so after a restart the
//!   agent had forgotten everything but the delivery notice.

use std::sync::Arc;
use tinyagents_harness::run_queue::RunQueue;

use crate::agent::turn_origin::{with_origin, AgentTurnOrigin};

use super::super::run_task::turn_error_discards_session;
use super::super::session::{
    checkin_session_agent_if_vacant, checkout_session_agent, CheckedOutSession, CheckoutPolicy,
};

/// Client id stamped on host-authored turns. Never a real socket, so it never
/// collides with a user's cancel/interrupt routing.
pub const SYSTEM_CLIENT_ID: &str = "system";

/// Prefix on the error a system turn returns when the thread's session could
/// not even be checked out (config load or agent build), as opposed to a turn
/// that ran and failed. Callers that track "did a turn run" key off it.
pub const SESSION_CHECKOUT_FAILURE: &str = "session checkout failed: ";

/// Run one host-authored turn on `thread_id` through the thread's session and
/// return the reply text. Best-effort: the caller decides how to surface the
/// reply (background delivery persists and announces it; goal continuation
/// only logs).
///
/// The turn is not registered in `IN_FLIGHT`: it is not interruptible by a
/// newer user message and does not interrupt one. Callers gate on idleness
/// themselves (background delivery defers while a user turn is in flight).
pub async fn run_system_turn_on_thread(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    run_id: &str,
    prompt: &str,
    origin: AgentTurnOrigin,
) -> Result<String, String> {
    let config = ctx
        .load_config()
        .await
        .map_err(|error| format!("{SESSION_CHECKOUT_FAILURE}{error}"))?;
    let CheckedOutSession {
        mut agent,
        fingerprint,
    } = checkout_session_agent(
        ctx,
        &config,
        SYSTEM_CLIENT_ID,
        thread_id,
        None,
        None,
        None,
        CheckoutPolicy::AdoptCached,
    )
    .await
    .map_err(|error| format!("{SESSION_CHECKOUT_FAILURE}{error}"))?;

    log::info!(
        "[web-channel] running system turn on thread={} run_id={} origin={} prompt_chars={}",
        thread_id,
        run_id,
        origin.class(),
        prompt.chars().count()
    );

    // The hosted harness only retains streamed terminal text for an observed
    // turn. A system turn has no UI progress consumer, so drain a local sink
    // solely to preserve the generated reply; otherwise a successful provider
    // response is replaced with the empty-turn fallback.
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(128);
    agent.set_on_progress(Some(progress_tx));
    let progress_drain = tokio::spawn(async move { while progress_rx.recv().await.is_some() {} });
    agent.set_run_queue(Some(Arc::new(RunQueue::new())));
    agent.set_thread_id(Some(thread_id));

    let result = with_origin(origin, agent.run_single(prompt))
        .await
        .map_err(|error| format!("{error:#}"));

    agent.set_on_progress(None);
    progress_drain.abort();

    // Same rule as a user turn: a failed harness turn can leave the session
    // partially advanced, so let it drop and cold-boot from the durable
    // transcript on the next request.
    if matches!(&result, Err(err) if turn_error_discards_session(err)) {
        log::warn!(
            "[web-channel] dropping session agent after failed system \
             turn thread={} run_id={}",
            thread_id,
            run_id
        );
    } else {
        checkin_session_agent_if_vacant(ctx, thread_id, agent, fingerprint).await;
    }

    result
}
