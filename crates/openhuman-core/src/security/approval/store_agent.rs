//! Approval rows by the embedded agent that parked them, and whether the
//! store exists to hold any.

use anyhow::Result;

use crate::core::runtime::DispatchCtx;

use super::{Config, PendingApproval};

/// Whether the approval store exists for `config`'s workspace. The document
/// store is created on first use, so it always does.
pub fn exists(_config: &Config) -> bool {
    true
}

/// [`list_pending`](super::list_pending) narrowed to the rows of `agent` (see
/// [`PendingApproval::belongs_to`]).
///
/// An embedded agent's requests are kept in its own storage scope, so the rows
/// of `Some(agent)` are read there as well as in the caller's scope, whichever
/// scope the caller runs in (under a SaaS profile every agent shares the
/// profile's scope, which is the caller's).
pub fn list_pending_for_agent(
    ctx: &DispatchCtx,
    config: &Config,
    agent: Option<&str>,
) -> Result<Vec<PendingApproval>> {
    let mut rows = super::list_pending(config)?;
    if let Some(agent) = agent.filter(|_| !acts_for_a_profile(ctx)) {
        let docs = super::super::store_documents::for_agent(config, agent)?;
        super::publish_expired(&docs.expire_stale(chrono::Utc::now())?);
        for row in docs.list_pending()? {
            if !rows.iter().any(|seen| seen.request_id == row.request_id) {
                rows.push(row);
            }
        }
    }
    Ok(rows
        .into_iter()
        .filter(|row| row.belongs_to(agent))
        .collect())
}

/// Whether the calling task serves a SaaS profile, whose agents share one scope.
fn acts_for_a_profile(ctx: &DispatchCtx) -> bool {
    ctx.tenant().is_ok_and(|tenant| tenant.profile.is_some())
}

/// The agent that parked the still-undecided `request_id`: `Ok(None)` when no
/// such row exists, `Ok(Some(None))` for a row the process parked itself.
pub fn pending_agent(config: &Config, request_id: &str) -> Result<Option<Option<String>>> {
    super::super::store_documents::current(config)?.pending_agent(request_id)
}

#[cfg(test)]
#[path = "store_agent_tests.rs"]
mod tests;
