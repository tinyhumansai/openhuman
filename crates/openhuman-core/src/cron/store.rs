//! Cron persistence, reached over the schedule bus contract.
//!
//! Jobs, run history and per-job policy live in the `tinyflows` module's cron
//! store (`CronDocuments`: validation, schedule maths, output limits, history
//! pruning, claim generations), which persists through the acting scope's
//! documents on the storage ports via the host's document port
//! ([`super::documents_port`]). tinyflows no longer has a SQLite store of its
//! own: the `jobs.db` it wrote is imported once, read-only, when the backend is
//! installed ([`crate::storage::import`]).
//!
//! This module keeps the synchronous API the rest of the core calls. Each
//! function applies the operator limits ([`super::limits`]) that only the host
//! knows, then sends one typed [`StoreOperation`] through
//! [`super::client::store_blocking`]. The host's [`Config`] supplies the store's
//! limits (run-history cap, due-job batch size) per call.

use crate::config::Config;
use anyhow::Result;
use chrono::{DateTime, Utc};
use tinyflows_bus::schedule::types::DeliveryStatus;
use tinyflows_bus::schedule::types::{
    CronJob, CronJobPatch, CronRun, DeliveryConfig, JobType, Schedule, SessionTarget,
};
use tinyflows_bus::schedule::{AgentJobSpec, StoreOperation};

use super::client::store_blocking;
use super::limits;

/// The cron database tinyflows used to write, imported on first use.
pub fn legacy_db_path(config: &Config) -> std::path::PathBuf {
    crate::storage::import::tinyflows::legacy_jobs_db(&crate::core::runtime::agent_scope_dir(
        config,
    ))
}

pub fn add_job(config: &Config, expression: &str, command: &str) -> Result<CronJob> {
    let schedule = Schedule::Cron {
        expr: expression.to_string(),
        tz: None,
        active_hours: None,
    };
    limits::check_new(config, &JobType::Shell, &schedule, None)?;
    store_blocking(
        config,
        StoreOperation::Add {
            expression: expression.to_string(),
            command: command.to_string(),
        },
    )
}

pub fn add_shell_job(
    config: &Config,
    name: Option<String>,
    schedule: Schedule,
    command: &str,
) -> Result<CronJob> {
    limits::check_new(config, &JobType::Shell, &schedule, None)?;
    store_blocking(
        config,
        StoreOperation::AddShell {
            name,
            schedule,
            command: command.to_string(),
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn add_agent_job(
    config: &Config,
    name: Option<String>,
    schedule: Schedule,
    prompt: &str,
    session_target: SessionTarget,
    model: Option<String>,
    delivery: Option<DeliveryConfig>,
    delete_after_run: bool,
) -> Result<CronJob> {
    add_agent_job_with_definition(
        config,
        name,
        schedule,
        prompt,
        session_target,
        model,
        delivery,
        delete_after_run,
        None,
        true,
    )
}

/// Like [`add_agent_job`] but accepts an optional built-in agent definition
/// ID and the initial enabled state.
#[allow(clippy::too_many_arguments)]
pub fn add_agent_job_with_definition(
    config: &Config,
    name: Option<String>,
    schedule: Schedule,
    prompt: &str,
    session_target: SessionTarget,
    model: Option<String>,
    delivery: Option<DeliveryConfig>,
    delete_after_run: bool,
    agent_id: Option<String>,
    enabled: bool,
) -> Result<CronJob> {
    let mut spec = AgentJobSpec::new(schedule, prompt);
    spec.name = name;
    spec.session_target = session_target;
    spec.model = model;
    spec.delivery = delivery;
    spec.delete_after_run = delete_after_run;
    spec.agent_id = agent_id;
    spec.enabled = enabled;
    add_agent_job_from_spec(config, spec)
}

/// Adds an agent job described by `spec`, including its origin conversation.
pub fn add_agent_job_from_spec(config: &Config, spec: AgentJobSpec) -> Result<CronJob> {
    limits::check_new(
        config,
        &JobType::Agent,
        &spec.schedule,
        spec.delivery.as_ref(),
    )?;
    store_blocking(config, StoreOperation::AddAgent { spec })
}

/// Registers (idempotently) the cron job that fires a flow's `schedule`
/// trigger; see the module's `add_flow` store operation.
pub fn add_flow_schedule_job(
    config: &Config,
    flow_id: &str,
    schedule: Schedule,
) -> Result<CronJob> {
    limits::check_new(config, &JobType::Flow, &schedule, None)?;
    store_blocking(
        config,
        StoreOperation::AddFlow {
            flow_id: flow_id.to_string(),
            schedule,
        },
    )
}

pub fn find_flow_schedule_job(config: &Config, flow_id: &str) -> Result<Option<CronJob>> {
    store_blocking(
        config,
        StoreOperation::FindFlow {
            flow_id: flow_id.to_string(),
        },
    )
}

pub fn list_jobs(config: &Config) -> Result<Vec<CronJob>> {
    store_blocking(config, StoreOperation::List)
}

pub fn get_job(config: &Config, job_id: &str) -> Result<CronJob> {
    store_blocking(
        config,
        StoreOperation::Get {
            job_id: job_id.to_string(),
        },
    )
}

pub fn remove_job(config: &Config, id: &str) -> Result<()> {
    // The job's policy is a field of the job, so it goes with it.
    store_blocking::<serde_json::Value>(
        config,
        StoreOperation::Remove {
            job_id: id.to_string(),
        },
    )?;
    println!("✅ Removed cron job {id}");
    Ok(())
}

/// Deletes every cron job in the workspace (E2E `openhuman.test_reset`).
pub fn clear_all_jobs(config: &Config) -> Result<usize> {
    store_blocking(config, StoreOperation::Clear)
}

/// Removes duplicate jobs sharing a `name`, keeping the one with most history.
pub fn dedup_named_jobs(config: &Config) -> Result<usize> {
    store_blocking(config, StoreOperation::Dedup)
}

pub fn due_jobs(config: &Config, now: DateTime<Utc>) -> Result<Vec<CronJob>> {
    store_blocking(config, StoreOperation::Due { now })
}

pub fn update_job(config: &Config, job_id: &str, patch: CronJobPatch) -> Result<CronJob> {
    limits::check_patch(config, &patch)?;
    store_blocking(
        config,
        StoreOperation::Update {
            job_id: job_id.to_string(),
            patch,
        },
    )
}

pub fn record_last_run(
    config: &Config,
    job_id: &str,
    finished_at: DateTime<Utc>,
    success: bool,
    output: &str,
) -> Result<()> {
    store_blocking::<serde_json::Value>(
        config,
        StoreOperation::RecordLastRun {
            job_id: job_id.to_string(),
            finished_at,
            success,
            output: output.to_string(),
        },
    )
    .map(|_| ())
}

pub fn reschedule_after_run(
    config: &Config,
    job: &CronJob,
    success: bool,
    output: &str,
) -> Result<()> {
    store_blocking::<serde_json::Value>(
        config,
        StoreOperation::Reschedule {
            job: job.clone(),
            success,
            output: output.to_string(),
        },
    )
    .map(|_| ())
}

pub fn record_run(
    config: &Config,
    job_id: &str,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    status: &str,
    output: Option<&str>,
    duration_ms: i64,
) -> Result<()> {
    record_run_with_delivery(
        config,
        job_id,
        started_at,
        finished_at,
        status,
        output,
        duration_ms,
        None,
    )
}

/// [`record_run`] plus the outcome of delivering the run's result.
#[allow(clippy::too_many_arguments)]
pub fn record_run_with_delivery(
    config: &Config,
    job_id: &str,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    status: &str,
    output: Option<&str>,
    duration_ms: i64,
    delivery_status: Option<DeliveryStatus>,
) -> Result<()> {
    store_blocking::<serde_json::Value>(
        config,
        StoreOperation::RecordRun {
            job_id: job_id.to_string(),
            started_at,
            finished_at,
            status: status.to_string(),
            output: output.map(str::to_string),
            duration_ms,
            delivery_status,
        },
    )
    .map(|_| ())
}

/// Removes "queued" placeholder rows so only the real result row remains.
pub fn delete_queued_runs(config: &Config, job_id: &str) -> Result<usize> {
    store_blocking(
        config,
        StoreOperation::DeleteQueuedRuns {
            job_id: job_id.to_string(),
        },
    )
}

pub fn list_runs(config: &Config, job_id: &str, limit: usize) -> Result<Vec<CronRun>> {
    store_blocking(
        config,
        StoreOperation::ListRuns {
            job_id: job_id.to_string(),
            limit,
        },
    )
}

/// Moves a job's next run to `at`, which no production path can do directly
/// (a patch reschedules from the schedule, not to a given instant). Writes the
/// instant the store keeps (`next_run_ms`), so the due-job query sees it.
#[cfg(test)]
pub(crate) fn force_next_run_for_test(
    config: &Config,
    job_id: &str,
    at: DateTime<Utc>,
) -> Result<()> {
    use tinystoragedrivers::Precondition;
    let opened = crate::storage::local::open("cron::store", &config.workspace_dir)?;
    let docs = std::sync::Arc::clone(opened.scoped()?.documents());
    let job_id = job_id.to_string();
    crate::storage::block_on_anyhow(async move {
        let stored = docs
            .get("cron.jobs", &job_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no job {job_id}"))?;
        let mut doc = stored.doc;
        doc["next_run_ms"] = serde_json::json!(at.timestamp_millis());
        docs.put("cron.jobs", &job_id, doc, Precondition::None)
            .await?;
        Ok(())
    })
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
