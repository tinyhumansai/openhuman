//! Host-owned system cron jobs.
//!
//! The scheduler's job types (`shell`, `agent`, `flow`) come from
//! `tinyflows-schedule`. A system job is a `flow`-type row whose command is
//! `system:<job>`: when it comes due the scheduler publishes
//! [`DomainEvent::CronSystemJobDue`](crate::core::events::DomainEvent::CronSystemJobDue)
//! instead of a flow tick, and the owning domain runs it. Cron stays agnostic
//! of what the job does; the rows show up in the routines list like any other.
//!
//! Memory owns two (`memory::bus`): `memory_background`, every
//! [`BACKGROUND_INTERVAL_MINS`] (the queued belief builds and deferred
//! ingests), and `memory_sources_sync`, every [`SOURCES_SYNC_INTERVAL_MINS`].
//! [`ensure_memory_jobs`] is idempotent: it creates a missing row, reschedules
//! one whose interval drifted, and removes the retired
//! `memory_context_refresh` row.

use anyhow::Result;

use crate::config::Config;
use crate::memory::bus::{RETIRED_CONTEXT_REFRESH_JOB, SOURCES_SYNC_JOB};
use crate::memory::lifecycle::jobs::{BACKGROUND_INTERVAL_MINS, BACKGROUND_JOB};

use {
    super::add_flow_schedule_job, super::list_jobs, super::remove_job, super::update_job,
    tinyflows_bus::schedule::types::CronJob, tinyflows_bus::schedule::types::CronJobPatch,
    tinyflows_bus::schedule::types::JobType, tinyflows_bus::schedule::types::Schedule,
};

/// Command prefix that marks a `flow` row as a system job.
pub const SYSTEM_COMMAND_PREFIX: &str = "system:";

/// Minutes between scheduled source-sync sweeps.
pub const SOURCES_SYNC_INTERVAL_MINS: u32 = 15;

/// The system job a row runs, when it is one.
#[must_use]
pub fn system_job_name(job: &CronJob) -> Option<&str> {
    if !matches!(job.job_type, JobType::Flow) {
        return None;
    }
    system_command_name(&job.command)
}

/// The system job a `flow` row's command names, when it names one.
#[must_use]
pub fn system_command_name(command: &str) -> Option<&str> {
    command
        .strip_prefix(SYSTEM_COMMAND_PREFIX)
        .filter(|name| !name.is_empty())
}

fn every(mins: u32) -> Schedule {
    Schedule::Every {
        every_ms: u64::from(mins.max(1)) * 60_000,
    }
}

/// Creates or reschedules system job `name` to run on `schedule`.
pub fn ensure_system_job(config: &Config, name: &str, schedule: Schedule) -> Result<CronJob> {
    let command = format!("{SYSTEM_COMMAND_PREFIX}{name}");
    let existing = list_jobs(config)?
        .into_iter()
        .find(|job| matches!(job.job_type, JobType::Flow) && job.command == command);
    match existing {
        Some(job) if job.schedule == schedule && job.name.as_deref() == Some(name) => Ok(job),
        Some(job) => {
            tracing::debug!(job = %name, "[cron::system_jobs] rescheduling");
            update_job(
                config,
                &job.id,
                CronJobPatch {
                    schedule: Some(schedule),
                    name: Some(name.to_string()),
                    ..CronJobPatch::default()
                },
            )
        }
        None => {
            tracing::info!(job = %name, "[cron::system_jobs] seeding");
            let job = add_flow_schedule_job(config, &command, schedule)?;
            update_job(
                config,
                &job.id,
                CronJobPatch {
                    name: Some(name.to_string()),
                    ..CronJobPatch::default()
                },
            )
        }
    }
}

/// Seeds (or reschedules) memory's system jobs for `config`, and removes
/// the ones memory retired.
pub fn ensure_memory_jobs(config: &Config) -> Result<()> {
    let background_mins = u32::try_from(BACKGROUND_INTERVAL_MINS).unwrap_or(u32::MAX);
    ensure_system_job(config, BACKGROUND_JOB, every(background_mins))?;
    ensure_system_job(config, SOURCES_SYNC_JOB, every(SOURCES_SYNC_INTERVAL_MINS))?;
    for job in list_jobs(config)? {
        if system_job_name(&job) == Some(RETIRED_CONTEXT_REFRESH_JOB) {
            tracing::info!(id = %job.id, "[cron:system] removing the retired context refresh job");
            remove_job(config, &job.id)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "system_jobs_tests.rs"]
mod tests;
