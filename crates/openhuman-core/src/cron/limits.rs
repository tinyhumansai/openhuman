//! Operator-owned limits for hosted deployments (`[cron] agent_jobs_only`).
//!
//! With the mode on, a profile may only hold agent jobs bound to the
//! conversation that created them: shell and flow jobs are refused whatever the
//! autonomy level, delivery is limited to `origin` and `none` (no model- or
//! user-chosen destination), recurring jobs keep `min_interval_secs` apart and
//! at most `max_active_jobs` jobs are enabled at once, counted in the job
//! store rather than in this process. Each check runs where a job is added or
//! changed ([`super::store`]) and again where it runs, so a row written before
//! the mode was switched on is refused too.

use anyhow::{bail, Result};
use chrono::{Duration, Utc};

use crate::config::Config;
use crate::cron::{
    delivery_mode, list_jobs, CronJob, CronJobPatch, DeliveryConfig, JobType, Schedule,
};

/// Whether `config` restricts cron to agent jobs.
pub fn enforced(config: &Config) -> bool {
    config.cron.agent_jobs_only
}

fn check_type(job_type: &JobType) -> Result<()> {
    if matches!(job_type, JobType::Agent) {
        return Ok(());
    }
    bail!("only agent cron jobs are allowed here; shell and flow jobs are disabled")
}

fn check_schedule(config: &Config, schedule: &Schedule) -> Result<()> {
    let min = Duration::seconds(i64::try_from(config.cron.min_interval_secs).unwrap_or(i64::MAX));
    // The cadence walk runs in the module (`RunsCloserThan`); a module that
    // cannot answer fails the check closed rather than waving a job through.
    let schedule = schedule.clone();
    let too_frequent = crate::cron::client::blocking(config, move |config| async move {
        crate::cron::client::runs_closer_than(&config, &schedule, Utc::now(), min).await
    })?;
    if let Some(too_frequent) = too_frequent {
        bail!(
            "recurring cron jobs must run at least {}s apart, but this schedule {too_frequent}",
            config.cron.min_interval_secs
        );
    }
    Ok(())
}

fn check_delivery(delivery: &DeliveryConfig) -> Result<()> {
    let mode = delivery.mode.trim().to_ascii_lowercase();
    if mode == delivery_mode::ORIGIN || mode == delivery_mode::NONE {
        return Ok(());
    }
    bail!("cron output can only return to the conversation that created the job")
}

/// Enabled jobs in the current scope, read from the store.
fn active_jobs(config: &Config) -> Result<usize> {
    Ok(list_jobs(config)?.iter().filter(|job| job.enabled).count())
}

fn check_capacity(config: &Config) -> Result<()> {
    let max = config.cron.max_active_jobs as usize;
    if active_jobs(config)? >= max {
        bail!("this profile already has {max} active cron jobs; remove one first");
    }
    Ok(())
}

/// Refuse a new job that breaks the limits. No-op unless [`enforced`].
pub(crate) fn check_new(
    config: &Config,
    job_type: &JobType,
    schedule: &Schedule,
    delivery: Option<&DeliveryConfig>,
) -> Result<()> {
    if !enforced(config) {
        return Ok(());
    }
    check_type(job_type)?;
    check_schedule(config, schedule)?;
    if let Some(delivery) = delivery {
        check_delivery(delivery)?;
    }
    check_capacity(config)
}

/// Refuse an update that breaks the limits. No-op unless [`enforced`].
pub(crate) fn check_patch(config: &Config, patch: &CronJobPatch) -> Result<()> {
    if !enforced(config) {
        return Ok(());
    }
    if patch.command.is_some() {
        bail!("shell commands are disabled here");
    }
    if patch.origin.is_some() {
        bail!("a cron job's conversation is fixed when it is created");
    }
    if let Some(schedule) = &patch.schedule {
        check_schedule(config, schedule)?;
    }
    if let Some(delivery) = &patch.delivery {
        check_delivery(delivery)?;
    }
    if patch.enabled == Some(true) {
        check_capacity(config)?;
    }
    Ok(())
}

/// Refuse to run a job that breaks the limits, including rows stored before
/// the mode was on. No-op unless [`enforced`].
pub(crate) fn check_run(config: &Config, job: &CronJob) -> Result<()> {
    if !enforced(config) {
        return Ok(());
    }
    check_type(&job.job_type)?;
    check_schedule(config, &job.schedule)?;
    check_delivery(&job.delivery)
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
