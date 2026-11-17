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
use tinyagents_harness::progress::AgentProgress;
use tinyagents_harness::run_queue::RunQueue;

use crate::core::runtime::turn_origin::{with_origin, AgentTurnOrigin};

use super::super::session::{
    checkin_session_agent_if_vacant, checkout_session_agent, CheckedOutSession, CheckoutPolicy,
};
use super::super::turn_host::{LateSubagentSink, PROGRESS_DRAIN_TIMEOUT};
use super::super::{Surface, TurnIds};

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
    //
    // The drain is also the turn's only progress consumer, so a sub-agent
    // spawned inside it reported on a channel nobody read (#8458): its
    // `subagent_spawned` event was addressed to `system`, which has no socket
    // and no thread room. Forward the sub-agent events to the thread's web
    // surface instead, addressed to the client that owns the thread's
    // session where the checkout knows one.
    let (progress_tx, progress_rx) = tokio::sync::mpsc::channel(128);
    agent.set_on_progress(Some(progress_tx));
    let client_id = forward_client_id(agent.event_session_id(), thread_id);
    let surface = Surface::new(
        ctx,
        TurnIds {
            client_id: client_id.clone(),
            thread_id: thread_id.to_owned(),
            request_id: run_id.to_owned(),
        },
        config.clone(),
    )
    .await;
    // The forwarder signals `drained` once it has projected the turn's
    // terminal event, switched to its late sink, or the stream ends.
    // `turn_over` tells the pump the turn is done when no terminal event
    // ever arrives; the pump places a FIFO marker behind whatever the turn
    // queued, so the switch waits for those events. It is never aborted:
    // after the drain it keeps the receiver as a late sink for detached children
    // and exits when the last progress sender drops, like a user turn's tap.
    let (drained_tx, mut drained_rx) = tokio::sync::watch::channel(false);
    let turn_over = Arc::new(tokio::sync::Notify::new());
    let late_sink = LateSubagentSink {
        client_id,
        thread_id: thread_id.to_owned(),
        request_id: run_id.to_owned(),
        config,
    };
    let _progress_forward = ctx.spawn(forward_system_turn_subagents(
        ctx.clone(),
        surface,
        progress_rx,
        drained_tx,
        Arc::clone(&turn_over),
        late_sink,
    ));
    agent.set_run_queue(Some(Arc::new(RunQueue::new())));
    agent.set_thread_id(Some(thread_id));

    let result = with_origin(origin, agent.run_single(prompt))
        .await
        .map_err(|error| format!("{error:#}"));

    agent.set_on_progress(None);

    // The turn surface now belongs to the forwarder: it closes the
    // projection when it switches to its late sink. Signal the switch first
    // so a turn that ended without a `TurnCompleted` still gets one, then
    // wait for the drain: the runner publishes the terminal event as soon
    // as this returns, so let the forwarder publish everything the turn
    // queued first — a spawn or completion must not be lost to the surface
    // close or overtaken by the delivery notice. Bounded (see
    // `PROGRESS_DRAIN_TIMEOUT`): a detached sub-agent can hold a sender
    // clone and keep the channel open past the turn, so the drain waits for
    // the terminal event (or the switch), not for the channel to close.
    turn_over.notify_one();
    if tokio::time::timeout(PROGRESS_DRAIN_TIMEOUT, drained_rx.wait_for(|done| *done))
        .await
        .is_err()
    {
        log::warn!(
            "[web-channel] system turn progress did not drain within {PROGRESS_DRAIN_TIMEOUT:?}; delivering anyway thread={thread_id} run_id={run_id}"
        );
    }

    // Same rule as a user turn: a failed harness turn can leave the session
    // partially advanced, so let it drop and cold-boot from the durable
    // transcript on the next request.
    if result.is_err() {
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

/// The client a host-authored turn addresses its sub-agent progress to.
///
/// The checked-out agent's event context names the web client that built
/// the session. A cold-booted agent (or one built for cron/skill/voice,
/// whose context is a plain id like `cron:<job>` or `standalone`) names no
/// web client — and the fallback must not be `system`: every socket
/// auto-joins the `system` room, so that would publish the thread's
/// sub-agent activity (child text, tool arguments, results) to every
/// connected client. Without an originating client, address to the thread
/// itself instead: no socket joins a bare thread-id room, so the emit lands
/// only in `thread:<id>`, where the thread's watchers are.
fn forward_client_id(event_session_id: &str, thread_id: &str) -> String {
    originating_client_id(event_session_id).unwrap_or_else(|| thread_id.to_owned())
}

/// The client stamped into the checked-out agent's event context. A cached
/// web-chat agent names the web client that built it; anything else names
/// no web client, so the caller falls back to thread-scoped addressing.
fn originating_client_id(event_session_id: &str) -> Option<String> {
    let client_id = serde_json::from_str::<serde_json::Value>(event_session_id)
        .ok()?
        .get("client_id")?
        .as_str()?
        .trim()
        .to_owned();
    (!client_id.is_empty() && client_id != SYSTEM_CLIENT_ID).then_some(client_id)
}

/// Project one progress event in full mode: the sub-agent events cross to
/// the thread's web surface, everything else is turn-scoped output the
/// delivery notice carries instead. Returns true when the turn's terminal
/// event was projected: everything after it belongs to a detached child.
async fn project_system_turn_event(surface: &Surface, event: AgentProgress) -> bool {
    let completed = matches!(event, AgentProgress::TurnCompleted { .. });
    if let Some(output) = crate::agent::turn_progress::subagent_event(event, 0, None) {
        surface.output(output).await;
    }
    completed
}

/// Forward the sub-agent events of a host-authored turn to the thread's web
/// surface, addressed to the turn's originating client.
///
/// This is the turn's progress drain as well: consuming the channel is what
/// preserves the generated reply, so the forwarder replaces the drop-drain
/// rather than running beside it. Only sub-agent events cross — the turn's
/// own narration stays with the delivery notice.
///
/// Receiving and projecting run on separate stages: the pump recvs from the
/// turn's bounded channel as fast as it allows and hands events to an
/// unbounded queue, so a slow projection can never fill the channel and
/// stall the turn's spawn-path sends (which block on capacity). One pump
/// and one projector keep the order.
///
/// Once the turn's terminal event is projected — or the pump places the
/// FIFO turn-over marker the caller signalled — the forwarder switches to
/// a late sink: the turn's surface closes, and only the terminal sub-agent
/// events of a detached child still cross, each through a fresh unsequenced
/// surface. It exits when the last progress sender drops.
async fn forward_system_turn_subagents(
    ctx: crate::core::runtime::DispatchCtx,
    surface: Surface,
    mut progress_rx: tokio::sync::mpsc::Receiver<AgentProgress>,
    drained: tokio::sync::watch::Sender<bool>,
    turn_over: Arc<tokio::sync::Notify>,
    late: LateSubagentSink,
) {
    let (queue_tx, mut queue_rx) = tokio::sync::mpsc::unbounded_channel::<QueuedProgress>();
    ctx.spawn(async move {
        // Forward events as fast as the turn publishes them. When the turn
        // ends, drain whatever it already queued and place the turn-over
        // marker behind it in FIFO order: the forwarder switches to its
        // late sink only on the marker, so a spawn still in the bounded
        // channel when the turn ends cannot be overtaken by the switch and
        // dropped there (the late sink settles completions only).
        loop {
            tokio::select! {
                event = progress_rx.recv() => {
                    let Some(event) = event else { break };
                    if queue_tx.send(QueuedProgress::Event(event)).is_err() {
                        break;
                    }
                }
                _ = turn_over.notified() => {
                    while let Ok(event) = progress_rx.try_recv() {
                        if queue_tx.send(QueuedProgress::Event(event)).is_err() {
                            break;
                        }
                    }
                    if queue_tx.send(QueuedProgress::TurnOver).is_err() {
                        break;
                    }
                    // Keep pumping post-turn events from detached children.
                    while let Some(event) = progress_rx.recv().await {
                        if queue_tx.send(QueuedProgress::Event(event)).is_err() {
                            break;
                        }
                    }
                    break;
                }
            }
        }
    });

    let mut late_mode = false;
    while let Some(item) = queue_rx.recv().await {
        match item {
            QueuedProgress::Event(event) => {
                if late_mode {
                    late.publish(&ctx, event).await;
                } else if project_system_turn_event(&surface, event).await {
                    let _ = drained.send(true);
                    surface.close().await;
                    late_mode = true;
                }
            }
            QueuedProgress::TurnOver => {
                if !late_mode {
                    // The turn ended without a terminal event (error path).
                    // The marker is FIFO behind everything the turn queued,
                    // so that stream is fully projected: close the surface
                    // and switch to the late sink for detached children.
                    surface.close().await;
                    late_mode = true;
                    let _ = drained.send(true);
                }
            }
        }
    }
    surface.close().await;
    let _ = drained.send(true);
}

/// One item in the forwarder's queue: a progress event, or the marker that
/// the turn ended. The marker is FIFO behind every event the turn queued,
/// so the forwarder only switches to its late sink once those are projected.
enum QueuedProgress {
    Event(AgentProgress),
    TurnOver,
}

#[cfg(test)]
#[path = "system_turn_tests.rs"]
mod tests;
