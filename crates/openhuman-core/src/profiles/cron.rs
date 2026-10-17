//! Serving the due agent jobs of hosted profiles.
//!
//! A hosted profile has no scheduler of its own. Each sweep visits every
//! provisioned profile in the shared storage backend, reads its due agent jobs
//! straight from the store (so a closed profile is seen without being opened),
//! and for each profile with something due:
//!
//! 1. opens the profile, taking its lease, so only the hosting node runs it;
//! 2. runs one agent-only scheduler for it under its own context: the
//!    `tinyflows` module claims each `(job, scheduled time)` occurrence in the
//!    store (`claim_slot`, a compare-and-swap on the job's claim generation), so
//!    two sweepers, or a node that took the profile over, never both run it, and
//!    schedules, time zones, one-shot removal, retries and run history behave as
//!    for any cron job.
//!
//! The reply goes where the job's delivery says; with a
//! [`CronDelivery`](crate::cron::host_delivery::CronDelivery) installed that is
//! the host. Shell and flow jobs are never run here: the sweep's scheduler is
//! agent-only, so a due shell or flow row of a profile is refused (and recorded
//! as such) rather than executed.
//!
//! # Shutdown
//!
//! A sweep in flight is not awaited when the process exits. Its occurrence
//! claims are generation-fenced in the store; another node may run an
//! interrupted occurrence again once the module's claim lapses.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use tinyflows_bus::schedule::StoreOperation;

use super::host::ProfileHost;
use crate::core::runtime::CoreContext;
use crate::cron::client::store_in;
use crate::cron::documents_port::DocScope;
use crate::cron::scheduler::JobDispatcher;
use crate::cron::{CronJob, JobType};

/// What one sweep did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SweepReport {
    /// Occurrences this node claimed and started.
    pub ran: usize,
    /// Due occurrences left alone: another node holds the profile or the claim.
    pub skipped: usize,
}

/// Sweep every provisioned profile for due agent jobs as of `now`.
pub async fn sweep(host: &ProfileHost, now: DateTime<Utc>) -> SweepReport {
    let mut report = SweepReport::default();
    if crate::storage::installed().is_none() {
        log::debug!("[profiles][cron] no shared storage backend; nothing to sweep");
        return report;
    }
    let profiles = match host.list().await {
        Ok(profiles) => profiles,
        Err(error) => {
            log::warn!("[profiles][cron] listing profiles failed: {error}");
            return report;
        }
    };
    for summary in profiles {
        let id = summary.profile_id;
        let Ok(config) = host.provisioned_config(&id).await else {
            continue;
        };
        let scope = DocScope::for_scope(&config, crate::storage::scope_for_profile(id.as_str()));
        let due: Vec<CronJob> =
            match store_in::<Vec<CronJob>>(&config, &scope, StoreOperation::Due { now }).await {
                Ok(jobs) => jobs
                    .into_iter()
                    .filter(|job| matches!(job.job_type, JobType::Agent))
                    .collect(),
                Err(error) => {
                    log::warn!("[profiles][cron] profile={id} due jobs unreadable: {error}");
                    continue;
                }
            };
        if due.is_empty() {
            continue;
        }
        let profile = match host.open(&id).await {
            Ok(profile) => profile,
            Err(error) => {
                log::debug!("[profiles][cron] profile={id} not opened: {error}");
                report.skipped += due.len();
                continue;
            }
        };
        let fence = Arc::clone(profile.lease_fence());
        if let Err(error) = fence.check(super::lease::now_ms()).await {
            log::warn!("[profiles][cron] profile={id} not run, lease fence: {error}");
            report.skipped += due.len();
            continue;
        }
        // The profile's turn gate, taken before the occurrences are claimed so
        // a wait behind a live chat turn does not eat into the claim lease.
        let turn = super::turn_gate::acquire_profile(id.as_str()).await;
        let due_count = due.len();
        let profile_config = profile.config.clone();
        let profile_ctx = Arc::clone(profile.context());
        let outcome = CoreContext::scope(Arc::clone(&profile_ctx), async move {
            let security = Arc::new(crate::security::SecurityPolicy::from_config(
                &profile_config.autonomy,
                &profile_config.workspace_dir,
                &profile_config.action_dir,
            ));
            let dispatch = crate::core::runtime::DispatchCtx::new(Arc::clone(&profile_ctx));
            let mut dispatcher =
                JobDispatcher::new(&dispatch, &profile_config, &security).agent_only();
            let started = match dispatcher.tick(&profile_config).await {
                Ok(started) => started,
                Err(error) => {
                    log::warn!("[profiles][cron] dispatch failed: {error}");
                    0
                }
            };
            // Stop at once if the profile is fenced while its jobs run.
            let finished = tokio::select! {
                biased;
                () = fence.fenced() => false,
                () = dispatcher.drain() => true,
            };
            (started, finished)
        })
        .await;
        drop(turn);
        let (started, finished) = outcome;
        report.ran += started;
        report.skipped += due_count.saturating_sub(started);
        if !finished {
            log::warn!("[profiles][cron] profile={id} fenced mid-run; jobs stopped");
        }
    }
    report
}

#[cfg(test)]
#[path = "cron_tests.rs"]
mod tests;
