//! Shared in-memory state for the web-channel turn dispatcher: the per-thread
//! session cache, the in-flight turn tables, and the small helpers that key
//! and mutate them.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;
use tokio::sync::Mutex;

use crate::core::runtime::DispatchCtx;

use super::super::runner::{runtime, scope_of};
use super::super::types::SessionEntry;

type ThreadSessions = Mutex<HashMap<String, SessionEntry>>;

/// The current agent context's per-thread session cache.
pub(crate) fn thread_sessions(ctx: &DispatchCtx) -> Arc<ThreadSessions> {
    crate::core::runtime::slot_for::<ThreadSessions>(ctx)
}

/// The map key for `thread_id` in the calling scope.
///
/// Thread ids are chosen by callers and are only unique per tenant, so the
/// key is [`tenant_key`](crate::core::runtime::tenant_key) of the calling
/// tenant: a SaaS profile and/or an embedded agent
/// ([`CoreContext::session_agent`]) prefix the thread id, so two users (or
/// two agents) that pick the same thread id get two cache entries and two
/// in-flight slots instead of sharing one live session. On the desktop,
/// outside an agent scope, the key stays the bare thread id.
///
/// [`CoreContext::session_agent`]: crate::core::runtime::CoreContext::session_agent
pub(crate) fn key_for(ctx: &DispatchCtx, thread_id: &str) -> String {
    key_in(&tenant_of(ctx), thread_id)
}

/// `key` with the calling scope's tenant prefix removed: the id the caller
/// chose, for handing back to it. A key of another tenant is returned as is.
pub(crate) fn unscope(ctx: &DispatchCtx, key: &str) -> String {
    unscope_in(&tenant_of(ctx), key)
}

/// The tenant `ctx` acts for; a SaaS context with none gets an isolated
/// tenant no other work shares, so the key can only miss.
pub(crate) fn tenant_of(ctx: &DispatchCtx) -> crate::core::runtime::Tenant {
    crate::core::runtime::tenant::isolated_unless_tenant(ctx.tenant(), "web_chat")
}

/// [`key_for`] under an explicit tenant.
pub(crate) fn key_in(tenant: &crate::core::runtime::Tenant, thread_id: &str) -> String {
    crate::core::runtime::tenant_key(tenant, thread_id)
}

/// [`unscope`] under an explicit tenant.
pub(crate) fn unscope_in(tenant: &crate::core::runtime::Tenant, key: &str) -> String {
    match crate::core::runtime::tenant::split_key(key) {
        Some((owner, id)) if &owner == tenant => id.to_string(),
        _ => key.to_string(),
    }
}

/// Injective encoding of `(session_agent, thread_id)` with no profile: the
/// desktop's [`tenant_key`](crate::core::runtime::tenant_key). Both parts are
/// caller controlled, so a plain `agent::thread` join would let
/// `("a", "b::c")` and `("a::b", "c")` (or an unscoped `"a::b"`) share one
/// slot.
pub(crate) fn scoped_key(session_agent: Option<&str>, thread_id: &str) -> String {
    key_in(
        &crate::core::runtime::Tenant {
            profile: None,
            agent: session_agent.map(str::to_owned),
        },
        thread_id,
    )
}

/// The thread id a [`key_for`] key was built from, ignoring its tenant.
fn thread_id_of_key(key: &str) -> Option<&str> {
    crate::core::runtime::tenant::id_of_key(key)
}

pub(crate) fn event_session_id_for(client_id: &str, thread_id: &str) -> String {
    json!({
        "client_id": client_id,
        "thread_id": thread_id,
    })
    .to_string()
}

pub async fn invalidate_thread_sessions(ctx: &DispatchCtx, thread_id: &str) {
    // Under an embedded agent or a profile only that tenant's entry goes.
    // Outside both every entry for the thread in the current table goes.
    let tenant = tenant_of(ctx);
    let mut sessions = thread_sessions(ctx).lock_owned().await;
    let keys_to_remove: Vec<String> = match tenant.agent.as_ref().or(tenant.profile.as_ref()) {
        Some(_) => {
            let key = key_for(ctx, thread_id);
            sessions
                .contains_key(&key)
                .then_some(key)
                .into_iter()
                .collect()
        }
        None => sessions
            .keys()
            .filter(|k| thread_id_of_key(k) == Some(thread_id))
            .cloned()
            .collect(),
    };
    for key in &keys_to_remove {
        sessions.remove(key);
    }
    if !keys_to_remove.is_empty() {
        log::debug!(
            "[web-channel] invalidated {} cached session(s) for thread_id={}",
            keys_to_remove.len(),
            thread_id
        );
    }
}

/// Invalidate only the captured tenant, unless its channel session ended.
///
/// Call from the captured core context so the original session table is used.
/// Cancellation while waiting for that table never mutates a replacement session.
pub(crate) async fn invalidate_thread_sessions_for_tenant(
    ctx: &DispatchCtx,
    tenant: &crate::core::runtime::Tenant,
    thread_id: &str,
    cancellation: &tokio_util::sync::CancellationToken,
) -> bool {
    let table = thread_sessions(ctx);
    let mut sessions = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return false,
        sessions = table.lock_owned() => sessions,
    };
    if cancellation.is_cancelled() {
        return false;
    }
    sessions.remove(&key_in(tenant, thread_id));
    true
}

/// Hold the real session table while exercising callback cancellation.
#[cfg(test)]
pub(crate) async fn lock_thread_sessions_for_test(ctx: &DispatchCtx) -> impl Send + 'static {
    thread_sessions(ctx).lock_owned().await
}

/// The thread ids (as the caller chose them) with a primary or parallel turn
/// in the calling context's runner.
pub(crate) async fn live_thread_ids(ctx: &DispatchCtx) -> Vec<String> {
    let scope = scope_of(ctx);
    let mut threads: Vec<String> = runtime(ctx)
        .runner
        .live_turns()
        .await
        .into_iter()
        .filter(|turn| turn.scope == scope)
        .map(|turn| turn.thread_id)
        .collect();
    threads.sort();
    threads.dedup();
    threads
}

/// Test accessor: `(thread key, request_id)` of every primary turn in flight
/// in the calling context.
pub async fn in_flight_entries_for_test(ctx: &DispatchCtx) -> Vec<(String, String)> {
    let scope = scope_of(ctx);
    runtime(ctx)
        .runner
        .live_turns()
        .await
        .into_iter()
        .filter(|turn| turn.scope == scope && !turn.parallel)
        .map(|turn| (key_for(ctx, &turn.thread_id), turn.request_id))
        .collect()
}

/// Test accessor: `(request_id, thread key)` for every in-flight parallel turn.
#[cfg(any(test, debug_assertions))]
pub async fn parallel_in_flight_entries_for_test(ctx: &DispatchCtx) -> Vec<(String, String)> {
    let scope = scope_of(ctx);
    runtime(ctx)
        .runner
        .live_turns()
        .await
        .into_iter()
        .filter(|turn| turn.scope == scope && turn.parallel)
        .map(|turn| (turn.request_id, key_for(ctx, &turn.thread_id)))
        .collect()
}

/// Test-only host-routing seam: take one lane from the active turn's real
/// TinyAgents queue so acceptance tests can assert that web metadata survives
/// queue admission. Production code only observes this queue through its
/// status and terminal dispatch paths.
#[cfg(test)]
pub async fn drain_queued_turns_for_test(
    ctx: &DispatchCtx,
    thread_id: &str,
    lane: tinyagents_harness::run_queue::QueueLane,
) -> Vec<crate::agent::queued_turn::QueuedTurn> {
    runtime(ctx)
        .runner
        .take_queued(
            &scope_of(ctx),
            thread_id,
            lane,
            tinyagents_harness::run_queue::QueueMode::All,
        )
        .await
}

/// Whether a cancel request should tear down the turn currently in flight for a
/// thread.
///
/// `requested` is the `request_id` the caller is cancelling; `None` means an
/// unscoped stop ("cancel whatever is running", e.g. a Stop button or a session
/// teardown). `in_flight` is the `request_id` currently registered for the
/// thread.
///
/// A *scoped* cancel matches only its own request. This is the fix for #4760: a
/// client that times out on request A and then sends request B — which
/// supersedes A on the same thread — must not have A's late-arriving cancel tear
/// down B. Scoping the cancel to A makes it a no-op once B is in flight, so the
/// newer turn survives instead of being killed at t=0.
pub fn cancel_should_target(requested: Option<&str>, in_flight: &str) -> bool {
    match requested {
        Some(rid) => rid == in_flight,
        None => true,
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
