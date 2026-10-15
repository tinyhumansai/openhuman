//! Serving the due agent jobs of hosted profiles.
//!
//! A hosted profile has no scheduler of its own. Each sweep visits every
//! provisioned profile in the shared storage backend, reads its due agent jobs
//! straight from the store (so a closed profile is seen without being opened),
//! and for each occurrence:
//!
//! 1. opens the profile, taking its lease, so only the hosting node runs it;
//! 2. claims the `(job, scheduled time)` occurrence in the store
//!    ([`crate::cron::claims`]), so two sweepers, or a node that took the
//!    profile over, never both run it;
//! 3. runs the claimed jobs under that profile's own context with the ordinary
//!    dispatcher, so schedules, time zones, one-shot removal, retries and run
//!    history behave as for any cron job.
//!
//! The reply goes where the job's delivery says; with a
//! [`CronDelivery`](crate::cron::host_delivery::CronDelivery) installed that is
//! the host. Shell and flow jobs are never run here.
//!
//! # Shutdown
//!
//! A sweep in flight is not awaited when the process exits. Its claims lapse
//! after [`CLAIM_LEASE_MS`](crate::cron::claims::CLAIM_LEASE_MS), after which
//! another node may run the interrupted occurrence again.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use tinyflows_drivers::schedule::CronDocuments;

use super::host::ProfileHost;
use crate::core::runtime::CoreContext;
use crate::cron::scheduler::JobDispatcher;
use crate::cron::{claims, JobType};

/// What one sweep did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SweepReport {
    /// Occurrences this node claimed and ran.
    pub ran: usize,
    /// Due occurrences left alone: another node holds the profile or the claim.
    pub skipped: usize,
}

/// Sweep every provisioned profile for due agent jobs as of `now`.
pub async fn sweep(host: &ProfileHost, now: DateTime<Utc>) -> SweepReport {
    let mut report = SweepReport::default();
    let Some(backend) = crate::storage::installed() else {
        log::debug!("[profiles][cron] no shared storage backend; nothing to sweep");
        return report;
    };
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
        let scoped = match backend.for_scope(&crate::storage::scope_for_profile(id.as_str())) {
            Ok(scoped) => scoped,
            Err(error) => {
                log::warn!("[profiles][cron] profile={id} storage scope refused: {error}");
                continue;
            }
        };
        let docs = Arc::clone(scoped.documents());
        let store = CronDocuments::new(Arc::clone(&docs))
            .with_limits(config.cron.max_run_history, config.scheduler.max_tasks);
        let due: Vec<_> = match store.due_jobs(now).await {
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
        let mut claimed = Vec::new();
        for job in due {
            match claims::claim_occurrence(docs.as_ref(), &job, host.node(), now).await {
                Ok(true) => claimed.push(job),
                Ok(false) => report.skipped += 1,
                Err(error) => {
                    log::warn!(
                        "[profiles][cron] profile={id} job={} claim: {error}",
                        job.id
                    );
                    report.skipped += 1;
                }
            }
        }
        if let Err(error) = claims::prune(docs.as_ref(), now).await {
            log::debug!("[profiles][cron] profile={id} pruning claims: {error}");
        }
        if claimed.is_empty() {
            continue;
        }
        let count = claimed.len();
        let profile_config = profile.config.clone();
        let outcome = CoreContext::scope(Arc::clone(profile.context()), async move {
            let security = Arc::new(crate::security::SecurityPolicy::from_full_config(
                &profile_config,
            ));
            let mut dispatcher = JobDispatcher::new(&profile_config, &security);
            if let Err(error) = dispatcher.dispatch(claimed).await {
                log::warn!("[profiles][cron] dispatch failed: {error}");
            }
            // Stop at once if the profile is fenced while its jobs run.
            tokio::select! {
                biased;
                () = fence.fenced() => false,
                () = dispatcher.drain() => true,
            }
        })
        .await;
        drop(turn);
        if outcome {
            report.ran += count;
        } else {
            log::warn!("[profiles][cron] profile={id} fenced mid-run; jobs stopped");
            report.skipped += count;
        }
    }
    report
}

#[cfg(test)]
#[path = "cron_tests.rs"]
mod tests;
