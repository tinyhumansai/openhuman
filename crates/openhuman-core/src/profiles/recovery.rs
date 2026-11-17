//! Recovery after an unclean hand-over: when a profile's lease is taken
//! over from a holder that never released it, that holder's turns may have
//! died mid-flight. [`ProfileHost::open`](super::ProfileHost::open) runs
//! these sweeps before the profile opens.

use std::sync::Arc;

use super::types::ProfileId;
use crate::core::runtime::CoreContext;

/// Settle what a previous holder left in profile `id`'s workspace: turn-state
/// files that were mid-flight become interrupted. The run ledger and the
/// stored turn states live in the storage backend and are swept by
/// [`recover_session_store`]. Run when a profile's lease is taken over from a
/// holder that never released it. Failures are logged; the profile still
/// opens.
pub(crate) fn recover_workspace(id: &ProfileId, workspace_dir: &std::path::Path) {
    let now = chrono::Utc::now().to_rfc3339();
    match crate::storage::sessions::default_turn_states_for(workspace_dir)
        .mark_all_interrupted(&now)
    {
        Ok(0) => {}
        Ok(turns) => log::info!("[profiles] profile={id} recovered {turns} interrupted turn(s)"),
        Err(error) => log::warn!("[profiles] profile={id} turn recovery failed: {error}"),
    }
}

/// [`recover_workspace`] for turn states kept in a storage-backed session
/// store, which the workspace sweep cannot see: the profile's default
/// session key is swept through the installed provider.
pub(crate) fn recover_session_store(id: &ProfileId, context: &Arc<CoreContext>) {
    let Some(provider) = crate::storage::sessions::installed_in(
        &crate::core::runtime::DispatchCtx::new(Arc::clone(context)),
    ) else {
        return;
    };
    if provider.workspace_dir().is_some() {
        // File-backed: the workspace sweep already covered it.
        return;
    }
    let key = crate::core::runtime::session_key(&crate::core::runtime::Tenant::of(context));
    let now = chrono::Utc::now().to_rfc3339();
    let recovered = CoreContext::sync_scope(Arc::clone(context), || {
        provider
            .for_agent(&key)
            .turn_states
            .mark_all_interrupted(&now)
    });
    match recovered {
        Ok(0) => {}
        Ok(turns) => {
            log::info!("[profiles] profile={id} recovered {turns} interrupted stored turn(s)");
        }
        Err(error) => log::warn!("[profiles] profile={id} stored turn recovery failed: {error}"),
    }
    if crate::storage::installed_is_shared() {
        // Another replica may still be driving a run in a shared backend.
        return;
    }
    let ledger =
        CoreContext::sync_scope(Arc::clone(context), || provider.run_ledger_for_agent(&key));
    match ledger {
        Ok(Some(ledger)) => {
            let swept = crate::storage::block_on(async move {
                Ok(ledger.interrupt_orphaned_agent_runs().await)
            });
            match swept {
                Ok(Ok(0)) => {}
                Ok(Ok(runs)) => {
                    log::info!("[profiles] profile={id} settled {runs} orphaned run(s)");
                }
                Ok(Err(error)) => {
                    log::warn!("[profiles] profile={id} run recovery failed: {error:#}");
                }
                Err(error) => log::warn!("[profiles] profile={id} run recovery failed: {error}"),
            }
        }
        Ok(None) => {}
        Err(error) => log::warn!("[profiles] profile={id} run ledger unavailable: {error}"),
    }
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
