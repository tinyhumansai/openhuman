//! Which agent a paired device belongs to.
//!
//! A device paired from inside an agent's context is stored in that agent's
//! storage scope (`crate::storage`), and the RPCs it sends through the tunnel
//! must run as that agent — not as the process default, which would read and
//! write somebody else's records. The tunnel subscriber runs outside any
//! agent, so it asks here, per frame:
//!
//! 1. the pending pairing session, which recorded the agent that started it;
//! 2. the owners this process has already resolved;
//! 3. otherwise each storage scope, for a device paired by an earlier process
//!    (`crate::storage::agents::for_each_scope`).
//!
//! `None` means the `local` scope (a single-user host, or no backend).

use crate::core::runtime::DispatchCtx;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use super::types::PairingSession;

/// Channels whose owner this process has resolved: `Some(agent)`, or `None`
/// for `local`.
static OWNERS: LazyLock<Mutex<HashMap<String, Option<String>>>> = LazyLock::new(Default::default);

/// Records that `channel_id` belongs to `agent` (`None` = `local`).
pub(super) fn remember(channel_id: &str, agent: Option<String>) {
    OWNERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(channel_id.to_string(), agent);
}

fn cached(channel_id: &str) -> Option<Option<String>> {
    OWNERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(channel_id)
        .cloned()
}

/// Whether this process holds a post-handshake session cipher for the channel.
fn has_active_session(channel_id: &str) -> bool {
    super::rpc::ACTIVE_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains_key(channel_id)
}

fn forget(channel_id: &str) {
    OWNERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(channel_id);
}

/// A failed owner lookup (`crate::storage::agents::LookupFailed`): no scope
/// has the device and a scope's lookup failed, so its owner is unknown. The
/// frame is dropped rather than handled as `local`: guessing could run a
/// paired device's RPCs as the wrong agent.
pub(super) type OwnerLookupFailed = crate::storage::agents::LookupFailed;

/// Whether the current scope holds `channel_id` as a live (not revoked)
/// paired device. The configuration is loaded inside the scope: in SaaS mode
/// loading it needs an acting agent, which the tunnel task does not have.
async fn has_live_device(ctx: &DispatchCtx, channel_id: &str) -> Result<bool, String> {
    let config = ctx
        .load_config()
        .await
        .map_err(|error| format!("load config: {error}"))?;
    super::store::get_device(&config, channel_id)
        .map(|device| device.is_some_and(|device| !device.revoked))
        .map_err(|error| error.to_string())
}

/// Whether the current scope holds `channel_id` as a revoked device (a missing
/// row is not revoked).
async fn is_revoked(ctx: &DispatchCtx, channel_id: &str) -> Result<bool, String> {
    let config = ctx
        .load_config()
        .await
        .map_err(|error| format!("load config: {error}"))?;
    super::store::get_device(&config, channel_id)
        .map(|device| device.is_some_and(|device| device.revoked))
        .map_err(|error| error.to_string())
}

/// The agent `channel_id` belongs to, resolved as described above: `None`
/// for `local`, which is also where a channel no scope knows yet (a handshake
/// still in flight) is handled. A channel no scope holds live but whose session
/// cipher this process still has (a device revoked elsewhere) is an error.
///
/// A remembered owner is re-checked in its scope each time, so a revoked,
/// deleted or re-paired channel is resolved afresh.
///
/// # Errors
///
/// [`OwnerLookupFailed`] when no scope has the device and at least one
/// scope's lookup failed.
pub(super) async fn owner_of(
    ctx: &DispatchCtx,
    channel_id: &str,
    pending: Option<&PairingSession>,
) -> Result<Option<String>, OwnerLookupFailed> {
    // A pending pairing names its agent. Once this process holds the channel's
    // session cipher the handshake has completed, and the session may be a
    // leftover: another process sharing the backend can have revoked the
    // device since. The session's own scope then decides — a revoked row drops
    // the frame; no row yet (the first frame can race the row being persisted
    // right after the handshake ACK) keeps the session's agent.
    if let Some(session) = pending {
        if !has_active_session(channel_id) {
            return Ok(session.agent.clone());
        }
        let revoked = crate::storage::agents::within_agent(
            ctx,
            session.agent.as_deref(),
            is_revoked(ctx, channel_id),
        )
        .await;
        return match revoked {
            Some(Ok(false)) => Ok(session.agent.clone()),
            Some(Ok(true)) => Err(OwnerLookupFailed {
                agent: session.agent.clone(),
                error: "device is revoked".to_string(),
            }),
            Some(Err(error)) => Err(OwnerLookupFailed {
                agent: session.agent.clone(),
                error,
            }),
            None => Err(OwnerLookupFailed {
                agent: session.agent.clone(),
                error: "no context can act for the pairing agent".to_string(),
            }),
        };
    }
    if let Some(owner) = cached(channel_id) {
        match crate::storage::agents::within_agent(
            ctx,
            owner.as_deref(),
            has_live_device(ctx, channel_id),
        )
        .await
        {
            Some(Ok(true)) => return Ok(owner),
            Some(Err(error)) => {
                return Err(OwnerLookupFailed {
                    agent: owner,
                    error,
                })
            }
            Some(Ok(false)) | None => forget(channel_id),
        }
    }
    let lookups =
        crate::storage::agents::for_each_scope(ctx, "device owner", |scope_ctx| async move {
            has_live_device(&scope_ctx, channel_id).await
        })
        .await;
    let owner = decide(lookups)?;
    if let Some(owner) = &owner {
        log::debug!(
            "[devices/owner] channel_id={channel_id} belongs to agent={}",
            owner.as_deref().unwrap_or("local")
        );
        remember(channel_id, owner.clone());
    }
    match owner {
        Some(owner) => Ok(owner),
        // No scope holds the device live, yet this process still has its
        // session cipher: it was revoked or removed elsewhere (another process
        // sharing the backend). Dropping the frame, rather than handling it as
        // `local`, keeps the revoked device from gaining the operator's scope.
        None if has_active_session(channel_id) => Err(OwnerLookupFailed {
            agent: None,
            error: "device is revoked or no longer paired".to_string(),
        }),
        None => Ok(None),
    }
}

/// The owner from each scope's lookup (`crate::storage::agents::decide`).
fn decide(
    lookups: Vec<(Option<String>, Result<bool, String>)>,
) -> Result<Option<Option<String>>, OwnerLookupFailed> {
    crate::storage::agents::decide(lookups)
}

#[cfg(test)]
#[path = "owner_tests.rs"]
mod tests;
