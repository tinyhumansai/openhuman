//! Which agent a flow belongs to, for the subscribers that act on a flow
//! named only by id (a schedule trigger, `FlowRunFinished`).
//!
//! A flow saved from inside an agent's context lives in that agent's storage
//! scope (`crate::storage`). The subscribers run outside any agent, so they
//! look the flow up here and handle the event as its owner
//! (`crate::storage::agents::within_agent`); without a storage backend every
//! flow is `local` and this is a no-op.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::storage::agents::{find_owner, within_agent, within_agent_ctx, LookupFailed};

/// Flows whose owner this process has resolved (`None` = `local`).
static OWNERS: LazyLock<Mutex<HashMap<String, Option<String>>>> = LazyLock::new(Default::default);

/// The agent `flow_id` belongs to: `None` for `local`, or when no scope has
/// the flow (the handler then reports the flow as unknown, as before).
///
/// # Errors
///
/// When no scope reported the flow and a scope's lookup failed: the event
/// is then not handled, rather than handled in the wrong scope.
pub(super) async fn flow_owner(
    ctx: &DispatchCtx,
    config: &Config,
    flow_id: &str,
) -> Result<Option<String>, LookupFailed> {
    // No backend: every record is `local`.
    if crate::storage::installed().is_none() {
        return Ok(None);
    }
    resolve(ctx, config, flow_id).await
}

/// Whether the current scope has `flow_id`.
async fn has_flow(config: &Config, flow_id: &str) -> Result<bool, String> {
    crate::flows::store::get_flow(config, flow_id)
        .map(|flow| flow.is_some())
        .map_err(|error| error.to_string())
}

/// [`flow_owner`]'s cache and lookup, across whatever scopes exist.
async fn resolve(
    ctx: &DispatchCtx,
    config: &Config,
    flow_id: &str,
) -> Result<Option<String>, LookupFailed> {
    let cached = OWNERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(flow_id)
        .cloned();
    if let Some(owner) = cached {
        // Still there? A removed flow's id may be reused, or the flow moved:
        // re-check the cached scope and resolve again when it lost the flow.
        match within_agent(ctx, owner.as_deref(), has_flow(config, flow_id)).await {
            Some(Ok(true)) => return Ok(owner),
            Some(Err(error)) => {
                return Err(LookupFailed {
                    agent: owner,
                    error,
                })
            }
            Some(Ok(false)) | None => forget(flow_id),
        }
    }
    let Some(owner) = find_owner(ctx, "flow owner", |_| has_flow(config, flow_id)).await? else {
        return Ok(None);
    };
    tracing::debug!(
        target: "flows",
        %flow_id,
        agent = owner.as_deref().unwrap_or("local"),
        "[flows] resolved the flow's owner"
    );
    OWNERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(flow_id.to_string(), owner.clone());
    Ok(owner)
}

/// Runs `step`, handed the owner's context, as `flow_id`'s owner; logs and skips it when the owner is
/// unknown (a failed lookup) or no context can act for it.
pub(super) async fn as_owner<F, Fut>(ctx: &DispatchCtx, config: &Config, flow_id: &str, step: F)
where
    F: FnOnce(DispatchCtx) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    match flow_owner(ctx, config, flow_id).await {
        Ok(owner) => {
            within_agent_ctx(ctx, owner.as_deref(), step).await;
        }
        Err(failed) => tracing::warn!(
            target: "flows",
            %flow_id,
            "[flows] flow owner unknown ({failed}); event not handled"
        ),
    }
}

/// The configuration to handle a flow event under: the acting agent's (or SaaS profile's) own
/// when the handler runs as one ([`as_owner`]) — its provider, access
/// policy, memory and action directory — else `registered`, the one the
/// subscriber was registered with.
pub(super) fn config_for_scope(
    ctx: &crate::core::runtime::DispatchCtx,
    registered: &std::sync::Arc<Config>,
) -> std::sync::Arc<Config> {
    // A SaaS profile acts as its own tenant even with no agent inside it.
    let acting = ctx
        .tenant()
        .is_ok_and(|tenant| tenant.agent.is_some() || tenant.profile.is_some());
    match acting.then(|| ctx.embedder_config()) {
        Some(Some(config)) => std::sync::Arc::new(config),
        _ => std::sync::Arc::clone(registered),
    }
}

/// Forgets a cached owner (a removed flow's id may be reused).
pub(super) fn forget(flow_id: &str) {
    OWNERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(flow_id);
}

#[cfg(test)]
#[path = "owner_tests.rs"]
mod tests;
