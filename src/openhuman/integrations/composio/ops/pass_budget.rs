//! Drain a connection within one invocation's item budget, one bounded pass
//! at a time (openhuman#6025).
//!
//! `SYNC_PASS_MAX_ITEMS` dropped from 500 to 200 so that a single
//! `AcceptSourceItems` call fits the host's 15-minute deadline. The entry
//! points that make one sync call per invocation — the periodic tick, the
//! manual provider sync, the `connection_created` initial sync and the Slack
//! ingest RPC — used to hand the whole 500 to that one pass. Handing them 200
//! would stop an account with 201–500 records short and still report the run
//! complete (Codex review finding on #6026). So they keep the per-invocation
//! budget they had, and spend it in passes the deadline can hold: each pass
//! is sized by [`next_pass_budget`], and the loop ends when the connector
//! reports the end or the budget is spent.

use std::future::Future;

use super::providers_ops::{next_pass_budget, run_sync_pass, SyncPassOutcome};
use crate::openhuman::config::Config;

/// Records one single-call entry point may read per invocation — the ceiling
/// those callers had when a pass was 500 items. Read, not written: the old
/// call's budget was what the module fetched, and a record the driver already
/// held still spent it. The Sources-row button is not bound by this: it loops
/// `composio_sync_budgeted` under its own cap.
pub(crate) const SINGLE_CALL_ITEM_BUDGET: u32 = 500;

/// Passes one call may make before it stops regardless — the same bound the
/// Sources-row loop uses. Unreachable with full pages (three passes spend the
/// budget); it exists so a module that keeps answering "more pending" with a
/// trickle cannot pin the caller.
const SINGLE_CALL_MAX_PASSES: usize = 50;

/// Run passes until the connector reports the end, `budget` records have been
/// read, a pass reads nothing, or [`SINGLE_CALL_MAX_PASSES`] is reached; each
/// pass is capped at `SYNC_PASS_MAX_ITEMS` through [`next_pass_budget`].
///
/// The outcome sums `records_read` and `written` across passes, is
/// `already_ingested` only when every pass was a no-op, and carries the last
/// pass's `more_pending` and `message` — the same "last pass's word wins" rule
/// the Sources-row loop applies. A pass error ends the run with that error;
/// what earlier passes wrote is already committed by the driver.
pub(crate) async fn run_passes_within_budget<F, Fut>(
    budget: u32,
    mut run_pass: F,
) -> Result<SyncPassOutcome, String>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Result<SyncPassOutcome, String>>,
{
    let mut total = SyncPassOutcome {
        already_ingested: true,
        ..SyncPassOutcome::default()
    };
    let mut read: u64 = 0;
    let mut passes = 0usize;
    while let Some(pass_budget) = next_pass_budget(Some(budget), read) {
        if passes >= SINGLE_CALL_MAX_PASSES {
            break;
        }
        let pass = run_pass(pass_budget).await?;
        passes += 1;
        read = read.saturating_add(u64::try_from(pass.records_read).unwrap_or(u64::MAX));
        total.records_read = total.records_read.saturating_add(pass.records_read);
        total.written = total.written.saturating_add(pass.written);
        total.already_ingested = total.already_ingested && pass.already_ingested;
        total.more_pending = pass.more_pending;
        total.message = pass.message;
        // A pass that read nothing cannot make progress; "more pending" from
        // it would only be asked again for the same nothing.
        if !pass.more_pending || pass.records_read == 0 {
            break;
        }
    }
    if passes > 1 {
        tracing::debug!(
            passes,
            written = total.written,
            more_pending = total.more_pending,
            "[composio] single-call sync spent its budget across passes"
        );
    }
    Ok(total)
}

/// [`run_sync_pass`] for the single-call entry points: the same
/// tinyconnectors-mediated pass, repeated within [`SINGLE_CALL_ITEM_BUDGET`].
pub(crate) async fn run_sync_within_budget(
    config: &Config,
    toolkit: &str,
    connection_id: &str,
    reason: &str,
) -> Result<SyncPassOutcome, String> {
    run_passes_within_budget(SINGLE_CALL_ITEM_BUDGET, |pass_budget| {
        run_sync_pass(config, toolkit, connection_id, reason, pass_budget)
    })
    .await
}

#[cfg(test)]
#[path = "pass_budget_tests.rs"]
mod tests;
