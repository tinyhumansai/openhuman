//! Dispatching due jobs without holding the poll loop.
//!
//! The poll loop used to await its whole batch (`buffer_unordered`), so one
//! long agent turn kept every other job — including ones that came due during
//! it — waiting for the next poll that could only happen once it finished.
//! Now each due job is spawned onto the [`JobDispatcher`]'s task set and the
//! poll returns at once; a semaphore keeps at most `scheduler.max_concurrent`
//! jobs executing. Dropping the dispatcher (the loop's task is aborted on
//! shutdown) aborts the jobs it still owns.
//!
//! Because the loop no longer waits, two guards replace the old implicit one:
//!
//! - **Run claim**: a job is claimed (`cron::ops::try_acquire_run`, the same
//!   claim Run Now and the run tool take) from dispatch until its run is
//!   persisted, and a claimed job is never dispatched again.
//! - **Claimed slot**: a recurring job's `next_run` is advanced to its next
//!   occurrence at dispatch, so a running job is not "due" on every poll. When
//!   it finishes, the usual reschedule recomputes `next_run` from that moment,
//!   exactly as before. A job that comes due again while still running has
//!   genuinely missed a slot: a default job skips it silently (the historical
//!   behaviour, when such slots were swallowed by the blocked loop); a
//!   single-flight job ([`crate::cron::policy::JobPolicy::single_flight`])
//!   records it as a `skipped` run.

use std::sync::Arc;

use chrono::Utc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::core::runtime::CoreContext;
use crate::cron::policy::get_policy;
use crate::security::SecurityPolicy;
use {
    crate::cron::record_run, crate::cron::update_job,
    tinyflows_schedule::schedule::next_run_for_schedule, tinyflows_schedule::types::CronJob,
    tinyflows_schedule::types::CronJobPatch, tinyflows_schedule::types::Schedule,
};

/// Output recorded for a single-flight slot missed while a run was in flight.
pub(crate) const SKIPPED_OUTPUT: &str = "skipped: the previous run was still in flight";

/// Owns the tasks of the jobs the poll loop dispatched.
pub(crate) struct JobDispatcher {
    tasks: JoinSet<()>,
    permits: Arc<Semaphore>,
}

impl JobDispatcher {
    /// A dispatcher running at most `max_concurrent` jobs at once.
    pub(crate) fn new(max_concurrent: usize) -> Self {
        Self {
            tasks: JoinSet::new(),
            permits: Arc::new(Semaphore::new(max_concurrent.max(1))),
        }
    }

    /// Start every job in `jobs` that is not already running and return.
    pub(crate) async fn dispatch(
        &mut self,
        config: &Config,
        security: &Arc<SecurityPolicy>,
        jobs: Vec<CronJob>,
    ) {
        self.reap();
        for job in jobs {
            // One run per job at a time, shared with Run Now and the run tool.
            let Some(guard) = crate::cron::ops::try_acquire_run(&job.id) else {
                skip_in_flight(config, &job);
                continue;
            };
            // Fail closed: an unreadable policy refuses the job before its slot
            // is advanced, so the occurrence is retried once the read recovers.
            if get_policy(config, &job.id).is_err() {
                continue;
            }
            // Operator limits (`cron::limits`) also bind rows stored before
            // they were switched on.
            if let Err(error) = crate::cron::limits::check_run(config, &job) {
                tracing::warn!(job_id = %job.id, %error, "[cron:dispatch] job refused by operator limits");
                continue;
            }
            claim_slot(config, &job);
            let config = config.clone();
            let security = Arc::clone(security);
            let permits = Arc::clone(&self.permits);
            tracing::debug!(job_id = %job.id, "[cron:dispatch] job dispatched");
            self.tasks.spawn(CoreContext::propagate(async move {
                let _guard = guard;
                let Ok(_permit) = permits.acquire_owned().await else {
                    return;
                };
                let (job_id, success, failure_message) =
                    super::execute_and_persist_job(&config, security.as_ref(), &job).await;
                BUS.publish(DomainEvent::HealthChanged {
                    component: "scheduler".to_string(),
                    healthy: success,
                    message: (!success)
                        .then(|| failure_message.unwrap_or_else(|| format!("job {job_id} failed"))),
                });
            }));
        }
    }

    /// Wait for every dispatched job to finish.
    pub(crate) async fn drain(&mut self) {
        while let Some(result) = self.tasks.join_next().await {
            log_join(result);
        }
    }

    /// Collect the jobs that already finished, so the set does not grow.
    fn reap(&mut self) {
        while let Some(result) = self.tasks.try_join_next() {
            log_join(result);
        }
    }
}

fn log_join(result: Result<(), tokio::task::JoinError>) {
    if let Err(error) = result {
        if error.is_panic() {
            tracing::error!("[cron:dispatch] a dispatched job panicked: {error}");
        }
    }
}

/// Advance a recurring job's `next_run` past now, so it is not due again
/// while it runs. Best effort: on failure the in-flight guard still keeps the
/// job from being dispatched twice.
fn claim_slot(config: &Config, job: &CronJob) {
    if matches!(job.schedule, Schedule::At { .. }) {
        return;
    }
    match update_job(
        config,
        &job.id,
        CronJobPatch {
            schedule: Some(job.schedule.clone()),
            ..CronJobPatch::default()
        },
    ) {
        Ok(claimed) => tracing::trace!(
            job_id = %job.id,
            next_run = %claimed.next_run,
            "[cron:dispatch] slot claimed"
        ),
        Err(error) => tracing::debug!(
            job_id = %job.id,
            %error,
            "[cron:dispatch] could not claim the slot; relying on the run claim"
        ),
    }
}

/// A due job whose previous run is still going.
fn skip_in_flight(config: &Config, job: &CronJob) {
    let single_flight = get_policy(config, &job.id).is_ok_and(|policy| policy.single_flight);
    if !single_flight || matches!(job.schedule, Schedule::At { .. }) {
        tracing::trace!(job_id = %job.id, "[cron:dispatch] still running; not dispatched again");
        return;
    }
    tracing::info!(job_id = %job.id, "[cron:dispatch] single-flight job still running; slot skipped");
    let now = Utc::now();
    if let Err(error) = record_run(
        config,
        &job.id,
        now,
        now,
        "skipped",
        Some(SKIPPED_OUTPUT),
        0,
    ) {
        tracing::warn!(job_id = %job.id, %error, "[cron:dispatch] recording the skipped run failed");
    }
    if next_run_for_schedule(&job.schedule, now).is_ok() {
        claim_slot(config, job);
    }
}
