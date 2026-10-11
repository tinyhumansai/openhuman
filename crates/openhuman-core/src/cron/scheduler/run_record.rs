//! Persisting a finished run: delivery, run history, one-shot termination,
//! rescheduling, and the high-frequency agent-job warning.

use super::delivery::deliver_run;
use crate::config::Config;
use chrono::{DateTime, Utc};
use {
    crate::cron::record_last_run, crate::cron::record_run_with_delivery, crate::cron::remove_job,
    crate::cron::reschedule_after_run, crate::cron::update_job,
    tinyflows_schedule::schedule::runs_closer_than, tinyflows_schedule::schedule::TooFrequent,
    tinyflows_schedule::schedule::MIN_AGENT_JOB_INTERVAL, tinyflows_schedule::types::CronJob,
    tinyflows_schedule::types::CronJobPatch, tinyflows_schedule::types::DeliveryStatus,
    tinyflows_schedule::types::JobType, tinyflows_schedule::types::Schedule,
};

/// [`persist_job_result_for_run`] under a fresh run id.
#[cfg(test)]
pub(super) async fn persist_job_result(
    config: &Config,
    job: &CronJob,
    success: bool,
    output: &str,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
) -> bool {
    let run_id = uuid::Uuid::new_v4().to_string();
    persist_job_result_for_run(
        config,
        job,
        &run_id,
        success,
        output,
        started_at,
        finished_at,
    )
    .await
}

pub(super) async fn persist_job_result_for_run(
    config: &Config,
    job: &CronJob,
    run_id: &str,
    mut success: bool,
    output: &str,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
) -> bool {
    let duration_ms = (finished_at - started_at).num_milliseconds();

    let delivery_status = match deliver_run(config, job, run_id, output, success).await {
        Ok(status) => status,
        Err(e) => {
            if job.delivery.best_effort {
                tracing::warn!("Cron delivery failed (best_effort): {e}");
            } else {
                success = false;
                tracing::warn!("Cron delivery failed: {e}");
            }
            DeliveryStatus::Failed
        }
    };

    let _ = record_run_with_delivery(
        config,
        &job.id,
        started_at,
        finished_at,
        if success { "ok" } else { "error" },
        Some(output),
        duration_ms,
        Some(delivery_status),
    );

    // A fixed-instant (`Schedule::At`) job is inherently one-shot: its `at` is in
    // the past the moment it runs, so `reschedule_after_run` (which writes
    // next_run = at for an `At` schedule) leaves next_run <= now and the job is
    // re-selected by `due_jobs` on every poll, re-executing forever. Terminate
    // every `At` job after a single run, regardless of `delete_after_run`. Only an
    // auto-delete job that succeeded is removed; everything else is kept disabled
    // so its run history stays inspectable. (Inside this `At` branch
    // `is_one_shot_auto_delete` reduces to `job.delete_after_run`.)
    if matches!(job.schedule, Schedule::At { .. }) {
        if is_one_shot_auto_delete(job) && success {
            if let Err(e) = remove_job(config, &job.id) {
                tracing::warn!("Failed to remove one-shot cron job after success: {e}");
            }
        } else {
            let _ = record_last_run(config, &job.id, finished_at, success, output);
            if let Err(e) = update_job(
                config,
                &job.id,
                CronJobPatch {
                    enabled: Some(false),
                    ..CronJobPatch::default()
                },
            ) {
                tracing::warn!("Failed to disable one-shot cron job: {e}");
            }
        }
        return success;
    }

    if let Err(e) = reschedule_after_run(config, job, success, output) {
        tracing::warn!("Failed to persist scheduler run result: {e}");
    }

    success
}

pub(super) fn is_one_shot_auto_delete(job: &CronJob) -> bool {
    job.delete_after_run && matches!(job.schedule, Schedule::At { .. })
}

/// Why an agent job counts as scheduled more often than every five minutes,
/// if it does. `None` for shell and flow jobs, for one-shot `At` schedules,
/// for anything at or above [`MIN_AGENT_JOB_INTERVAL`], and for an expression
/// that cannot be read (that is not evidence of anything).
///
/// Returns the verdict rather than only logging it, because the verdict is
/// the part worth testing: the `Schedule::Cron` arm used to measure the gap
/// between the next run after now and the next run after now plus one
/// second — the same instant unless a run fell inside that second — so every
/// cron-scheduled agent job warned, and no test could see it because a
/// function that only warns has nothing to assert. The gap is now the
/// shortest one between consecutive runs (`runs_closer_than`), so an
/// irregular expression is judged by its tightest pair and not by whichever
/// pair happens to follow the instant of the check.
pub(super) fn agent_job_too_frequent(job: &CronJob) -> Option<TooFrequent> {
    if !matches!(job.job_type, JobType::Agent) {
        return None;
    }
    runs_closer_than(&job.schedule, Utc::now(), MIN_AGENT_JOB_INTERVAL)
}

/// New agent jobs cannot be created below the floor (`validate_agent_schedule`
/// rejects them), so a job that trips this predates the rule. It keeps
/// running — silently skipping runs would turn its schedule into a lie — and
/// this line is the operator's signal to edit it.
pub(super) fn warn_if_high_frequency_agent_job(job: &CronJob) {
    if let Some(too_frequent) = agent_job_too_frequent(job) {
        tracing::warn!(
            "Cron agent job '{}' is scheduled more frequently than every 5 minutes: it \
             {too_frequent}. It was created before that floor was enforced; edit its \
             schedule to silence this.",
            job.id
        );
    }
}
