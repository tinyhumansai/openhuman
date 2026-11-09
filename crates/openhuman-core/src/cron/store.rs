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

/// Adds an agent job with an optional built-in definition and enabled state.
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
    tracing::info!(job_id = %id, "removed cron job");
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

pub fn update_job(config: &Config, job_id: &str, patch: CronJobPatch) -> Result<CronJob> {
    update_job_with_origin(config, job_id, patch, None)
}

/// Update a job, using only a host-verified conversation origin for delivery
/// validation. Raw origins stored on jobs or supplied by RPC are not trusted.
pub(crate) fn update_job_with_origin(
    config: &Config,
    job_id: &str,
    mut patch: CronJobPatch,
    trusted_origin: Option<tinyflows_bus::schedule::types::JobOrigin>,
) -> Result<CronJob> {
    limits::check_patch(config, &patch)?;
    if patch.delivery.is_some() || patch.origin.is_some() {
        let existing = get_job(config, job_id)?;
        patch =
            validate_and_complete_route_patch(config, &existing, patch, trusted_origin.as_ref())?;
    }
    store_blocking(
        config,
        StoreOperation::Update {
            job_id: job_id.to_string(),
            patch,
        },
    )
}

/// Validate routing changes against the row snapshot and return the patch to
/// persist. Route completion is performed alongside validation so a delayed
/// store write cannot combine the caller's route with a newer counterpart.
fn validate_and_complete_route_patch(
    config: &Config,
    existing: &CronJob,
    mut patch: CronJobPatch,
    trusted_origin: Option<&tinyflows_bus::schedule::types::JobOrigin>,
) -> Result<CronJobPatch> {
    let delivery = patch
        .delivery
        .clone()
        .unwrap_or_else(|| existing.delivery.clone());
    let effective_origin = patch
        .origin
        .clone()
        .unwrap_or_else(|| existing.origin.clone());
    crate::cron::job_builder::validate_delivery_for_config_with_trusted_origin(
        config,
        &delivery,
        effective_origin.as_ref(),
        trusted_origin,
    )
    .map_err(anyhow::Error::msg)?;

    // Persist exactly the pair that was validated. A stale write containing
    // only one route field must not combine with a newer counterpart.
    patch.delivery = Some(delivery);
    patch.origin = Some(effective_origin);
    Ok(patch)
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
/// Legacy TinyFlows rows also carry `next_run` and `next_run_ns`; keep those
/// representations in sync when present so the module's claim decoder sees
/// the same deadline as the due-job query.
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
        for collection in ["cron.jobs", "cron_jobs"] {
            if let Some(stored) = docs.get(collection, &job_id).await? {
                let mut doc = stored.doc;
                doc["next_run_ms"] = serde_json::json!(at.timestamp_millis());
                if doc.get("next_run").is_some() {
                    doc["next_run"] = serde_json::json!(at.to_rfc3339());
                }
                if doc.get("next_run_ns").is_some() {
                    let next_run_ns = at
                        .timestamp_nanos_opt()
                        .ok_or_else(|| anyhow::anyhow!("next run is outside nanosecond range"))?;
                    doc["next_run_ns"] = serde_json::json!(next_run_ns);
                }
                docs.put(collection, &job_id, doc, Precondition::None)
                    .await?;
                return Ok(());
            }
        }
        Err(anyhow::anyhow!("no job {job_id}"))
    })
}

#[cfg(test)]
#[path = "store_tests.rs"]
pub(crate) mod tests;
