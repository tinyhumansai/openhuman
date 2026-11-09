//! Operator-owned limits for hosted deployments (`[cron] agent_jobs_only`).
//!
//! With the mode on, a profile may hold agent jobs bound to the conversation
//! that created them and the memory domain's two trusted maintenance jobs.
//! User-owned shell and flow jobs are refused whatever the autonomy level;
//! delivery is limited to `origin` and `none` (no model- or
//! user-chosen destination), recurring jobs keep `min_interval_secs` apart and
//! at most `max_active_jobs` jobs are enabled at once, counted in the job
//! store rather than in this process. Each check runs where a job is added or
//! changed ([`super::store`]) and again where it runs, so a row written before
//! the mode was switched on is refused too.

use anyhow::{bail, Result};
use chrono::{Duration, Utc};

use crate::config::Config;
use crate::cron::system_jobs::TrustedSystemJobOwner;
use crate::cron::{delivery_mode, CronJob, CronJobPatch, DeliveryConfig, JobType, Schedule};

/// Whether `config` restricts cron to agent jobs.
pub fn enforced(config: &Config) -> bool {
    config.cron.agent_jobs_only
}

fn check_type(job_type: &JobType) -> Result<()> {
    if matches!(job_type, JobType::Agent) {
        return Ok(());
    }
    bail!("only agent cron jobs are allowed here; shell and user flow jobs are disabled")
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

fn install_capacity_limit(config: &Config) -> Result<()> {
    let max = config.cron.max_active_jobs as usize;
    crate::cron::client::store_blocking::<()>(
        config,
        tinyflows_bus::schedule::StoreOperation::SetMaxActiveJobs { limit: Some(max) },
    )?;
    Ok(())
}

/// Admit a job for an owning system domain. The typed owner comes from the
/// internal seeding call, never from a request or stored metadata.
pub(crate) fn check_trusted_system_job(
    config: &Config,
    owner: TrustedSystemJobOwner,
    name: &str,
    schedule: &Schedule,
) -> Result<()> {
    if !owner.owns(name) {
        bail!("system job name is outside its owning domain");
    }
    if !enforced(config) {
        clear_capacity_limit_if_supported(config)?;
        return Ok(());
    }
    check_schedule(config, schedule)?;
    install_capacity_limit(config)
}

/// Clear a prior operator cap when enforcement is off. Backends without
/// transactions cannot clear it, so skip the unsupported write; TinyFlows'
/// admission ledger guard refuses mutations if an earlier policy is present.
fn clear_capacity_limit_if_supported(config: &Config) -> Result<()> {
    let scope = super::documents_port::DocScope::current(config)?;
    if !scope.supports_transactions()? {
        return Ok(());
    }
    crate::cron::client::blocking(config, move |config| async move {
        crate::cron::client::store_in::<()>(
            &config,
            &scope,
            tinyflows_bus::schedule::StoreOperation::SetMaxActiveJobs { limit: None },
        )
        .await
    })
}

/// Enforce the limits for a new job, or clear a stored cap while they are off.
pub(crate) fn check_new(
    config: &Config,
    job_type: &JobType,
    schedule: &Schedule,
    delivery: Option<&DeliveryConfig>,
) -> Result<()> {
    if !enforced(config) {
        clear_capacity_limit_if_supported(config)?;
        return Ok(());
    }
    check_type(job_type)?;
    check_schedule(config, schedule)?;
    if let Some(delivery) = delivery {
        check_delivery(delivery)?;
    }
    install_capacity_limit(config)
}

/// Enforce update limits, or clear a stored cap while they are off.
pub(crate) fn check_patch(config: &Config, patch: &CronJobPatch) -> Result<()> {
    if !enforced(config) {
        clear_capacity_limit_if_supported(config)?;
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
        install_capacity_limit(config)?;
    }
    Ok(())
}

/// Refuse to run a job that breaks the limits, including rows stored before
/// the mode was on. No-op unless [`enforced`].
pub(crate) fn check_run(config: &Config, job: &CronJob) -> Result<()> {
    if !enforced(config) {
        return Ok(());
    }
    if let Some(owner) = crate::cron::system_jobs::trusted_system_job_owner(job) {
        let name = crate::cron::system_jobs::system_job_name(job)
            .expect("trusted system rows have a system job name");
        if owner.owns(name) {
            check_schedule(config, &job.schedule)?;
            check_delivery(&job.delivery)?;
            return Ok(());
        }
    }
    check_type(&job.job_type)?;
    check_schedule(config, &job.schedule)?;
    check_delivery(&job.delivery)
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
