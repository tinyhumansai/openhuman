//! Cron persistence: a thin host wrapper over `tinyflows_sqlite::schedule`.
//!
//! With a storage backend configured ([`crate::storage`]) every function is
//! served by `tinyflows_drivers::schedule::CronDocuments` instead, in the
//! acting agent's storage scope, with the same limits.
//!
//! The SQLite job store and run history live upstream (schema, CRUD, output
//! truncation, pruning). This module only turns the host [`Config`] into the
//! store's [`CronStoreOptions`] — database path under the workspace, run-history
//! cap, and due-job batch size — so callers keep passing `&Config`.

use crate::config::Config;
use anyhow::Result;
use chrono::{DateTime, Utc};
use tinyflows_drivers::schedule::CronDocuments;
use tinyflows_schedule::DeliveryStatus;
use tinyflows_schedule::{
    CronJob, CronJobPatch, CronRun, DeliveryConfig, JobType, Schedule, SessionTarget,
};

use super::limits;
use tinyflows_sqlite::schedule::{self as upstream, AgentJobSpec, CronStoreOptions};

/// The job database for `config`: `<workspace>/cron/jobs.db`, or the agent's
/// own `<workspace>/agents/<id>/cron/jobs.db` under an embedded agent's
/// context, so an agent lists, edits and runs only the jobs it created.
pub fn db_path(config: &Config) -> std::path::PathBuf {
    crate::core::runtime::agent_scope_dir(config)
        .join("cron")
        .join("jobs.db")
}

/// Builds the store options from the host config: [`db_path`],
/// `cron.max_run_history`, `scheduler.max_tasks`.
fn opts(config: &Config) -> CronStoreOptions {
    CronStoreOptions {
        db_path: db_path(config),
        max_run_history: config.cron.max_run_history,
        max_tasks: config.scheduler.max_tasks,
    }
}

/// The document store for this call when the host configured a backend.
fn documents(config: &Config) -> Result<Option<CronDocuments>> {
    Ok(crate::storage::current_scoped()?.map(|scoped| {
        CronDocuments::new(std::sync::Arc::clone(scoped.documents()))
            .with_limits(config.cron.max_run_history, config.scheduler.max_tasks)
    }))
}

/// Runs a document-store call from this synchronous API.
fn run<T: Send + 'static>(
    future: impl std::future::Future<Output = Result<T>> + Send + 'static,
) -> Result<T> {
    crate::storage::block_on_anyhow(future)
}

pub fn add_job(config: &Config, expression: &str, command: &str) -> Result<CronJob> {
    let schedule = Schedule::Cron {
        expr: expression.to_string(),
        tz: None,
        active_hours: None,
    };
    limits::check_new(config, &JobType::Shell, &schedule, None)?;
    if let Some(docs) = documents(config)? {
        let (expression, command) = (expression.to_string(), command.to_string());
        return run(async move { docs.add_job(&expression, &command).await });
    }
    upstream::add_job(&opts(config), expression, command)
}

pub fn add_shell_job(
    config: &Config,
    name: Option<String>,
    schedule: Schedule,
    command: &str,
) -> Result<CronJob> {
    limits::check_new(config, &JobType::Shell, &schedule, None)?;
    if let Some(docs) = documents(config)? {
        let command = command.to_string();
        return run(async move { docs.add_shell_job(name, schedule, &command).await });
    }
    upstream::add_shell_job(&opts(config), name, schedule, command)
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
    limits::check_new(config, &JobType::Agent, &schedule, delivery.as_ref())?;
    if let Some(docs) = documents(config)? {
        let prompt = prompt.to_string();
        return run(async move {
            docs.add_agent_job(
                name,
                schedule,
                &prompt,
                session_target,
                model,
                delivery,
                delete_after_run,
            )
            .await
        });
    }
    upstream::add_agent_job(
        &opts(config),
        name,
        schedule,
        prompt,
        session_target,
        model,
        delivery,
        delete_after_run,
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
    limits::check_new(config, &JobType::Agent, &schedule, delivery.as_ref())?;
    if let Some(docs) = documents(config)? {
        let prompt = prompt.to_string();
        return run(async move {
            docs.add_agent_job_with_definition(
                name,
                schedule,
                &prompt,
                session_target,
                model,
                delivery,
                delete_after_run,
                agent_id,
                enabled,
            )
            .await
        });
    }
    upstream::add_agent_job_with_definition(
        &opts(config),
        name,
        schedule,
        prompt,
        session_target,
        model,
        delivery,
        delete_after_run,
        agent_id,
        enabled,
    )
}

/// Adds an agent job described by `spec`, including its origin conversation.
pub fn add_agent_job_from_spec(config: &Config, spec: AgentJobSpec) -> Result<CronJob> {
    limits::check_new(
        config,
        &JobType::Agent,
        &spec.schedule,
        spec.delivery.as_ref(),
    )?;
    if let Some(docs) = documents(config)? {
        return run(async move { docs.add_agent_job_from_spec(spec).await });
    }
    upstream::add_agent_job_from_spec(&opts(config), spec)
}

/// Registers (idempotently) the cron job that fires a flow's `schedule`
/// trigger; see `tinyflows_sqlite::schedule::add_flow_schedule_job`.
pub fn add_flow_schedule_job(
    config: &Config,
    flow_id: &str,
    schedule: Schedule,
) -> Result<CronJob> {
    limits::check_new(config, &JobType::Flow, &schedule, None)?;
    if let Some(docs) = documents(config)? {
        let flow_id = flow_id.to_string();
        return run(async move { docs.add_flow_schedule_job(&flow_id, schedule).await });
    }
    upstream::add_flow_schedule_job(&opts(config), flow_id, schedule)
}

pub fn find_flow_schedule_job(config: &Config, flow_id: &str) -> Result<Option<CronJob>> {
    if let Some(docs) = documents(config)? {
        let flow_id = flow_id.to_string();
        return run(async move { docs.find_flow_schedule_job(&flow_id).await });
    }
    upstream::find_flow_schedule_job(&opts(config), flow_id)
}

pub fn list_jobs(config: &Config) -> Result<Vec<CronJob>> {
    if let Some(docs) = documents(config)? {
        return run(async move { docs.list_jobs().await });
    }
    upstream::list_jobs(&opts(config))
}

pub fn get_job(config: &Config, job_id: &str) -> Result<CronJob> {
    if let Some(docs) = documents(config)? {
        let job_id = job_id.to_string();
        return run(async move { docs.get_job(&job_id).await });
    }
    upstream::get_job(&opts(config), job_id)
}

pub fn remove_job(config: &Config, id: &str) -> Result<()> {
    if let Some(docs) = documents(config)? {
        let owned = id.to_string();
        run(async move { docs.remove_job(&owned).await })?;
        if let Err(error) = super::policy::clear_policy(config, id) {
            tracing::warn!(job_id = id, %error, "[cron:store] removing job policy failed");
        }
        println!("✅ Removed cron job {id}");
        return Ok(());
    }
    upstream::remove_job(&opts(config), id)?;
    if let Err(error) = super::policy::clear_policy(config, id) {
        tracing::warn!(job_id = id, %error, "[cron:store] removing job policy failed");
    }
    println!("✅ Removed cron job {id}");
    Ok(())
}

/// Deletes every cron job in the workspace (E2E `openhuman.test_reset`).
pub fn clear_all_jobs(config: &Config) -> Result<usize> {
    let removed = if let Some(docs) = documents(config)? {
        run(async move { docs.clear_all_jobs().await })?
    } else {
        upstream::clear_all_jobs(&opts(config))?
    };
    if let Err(error) = super::policy::clear_all_policies(config) {
        tracing::warn!(%error, "[cron:store] clearing job policies failed");
    }
    Ok(removed)
}

/// Removes duplicate jobs sharing a `name`, keeping the one with most history.
pub fn dedup_named_jobs(config: &Config) -> Result<usize> {
    if let Some(docs) = documents(config)? {
        return run(async move { docs.dedup_named_jobs().await });
    }
    upstream::dedup_named_jobs(&opts(config))
}

pub fn due_jobs(config: &Config, now: DateTime<Utc>) -> Result<Vec<CronJob>> {
    if let Some(docs) = documents(config)? {
        return run(async move { docs.due_jobs(now).await });
    }
    upstream::due_jobs(&opts(config), now)
}

pub fn update_job(config: &Config, job_id: &str, patch: CronJobPatch) -> Result<CronJob> {
    limits::check_patch(config, &patch)?;
    if let Some(docs) = documents(config)? {
        let job_id = job_id.to_string();
        return run(async move { docs.update_job(&job_id, patch).await });
    }
    upstream::update_job(&opts(config), job_id, patch)
}

pub fn record_last_run(
    config: &Config,
    job_id: &str,
    finished_at: DateTime<Utc>,
    success: bool,
    output: &str,
) -> Result<()> {
    if let Some(docs) = documents(config)? {
        let (job_id, output) = (job_id.to_string(), output.to_string());
        return run(async move {
            docs.record_last_run(&job_id, finished_at, success, &output)
                .await
        });
    }
    upstream::record_last_run(&opts(config), job_id, finished_at, success, output)
}

pub fn reschedule_after_run(
    config: &Config,
    job: &CronJob,
    success: bool,
    output: &str,
) -> Result<()> {
    if let Some(docs) = documents(config)? {
        let (job, output) = (job.clone(), output.to_string());
        return run(async move { docs.reschedule_after_run(&job, success, &output).await });
    }
    upstream::reschedule_after_run(&opts(config), job, success, output)
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
    if let Some(docs) = documents(config)? {
        let (job_id, status, output) = (
            job_id.to_string(),
            status.to_string(),
            output.map(str::to_string),
        );
        return run(async move {
            docs.record_run(
                &job_id,
                started_at,
                finished_at,
                &status,
                output.as_deref(),
                duration_ms,
            )
            .await
        });
    }
    upstream::record_run(
        &opts(config),
        job_id,
        started_at,
        finished_at,
        status,
        output,
        duration_ms,
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
    if let Some(docs) = documents(config)? {
        let (job_id, status, output) = (
            job_id.to_string(),
            status.to_string(),
            output.map(str::to_string),
        );
        return run(async move {
            docs.record_run_with_delivery(
                &job_id,
                started_at,
                finished_at,
                &status,
                output.as_deref(),
                duration_ms,
                delivery_status,
            )
            .await
        });
    }
    upstream::record_run_with_delivery(
        &opts(config),
        job_id,
        started_at,
        finished_at,
        status,
        output,
        duration_ms,
        delivery_status,
    )
}

/// Removes "queued" placeholder rows so only the real result row remains.
pub fn delete_queued_runs(config: &Config, job_id: &str) -> Result<usize> {
    if let Some(docs) = documents(config)? {
        let job_id = job_id.to_string();
        return run(async move { docs.delete_queued_runs(&job_id).await });
    }
    upstream::delete_queued_runs(&opts(config), job_id)
}

pub fn list_runs(config: &Config, job_id: &str, limit: usize) -> Result<Vec<CronRun>> {
    if let Some(docs) = documents(config)? {
        let job_id = job_id.to_string();
        return run(async move { docs.list_runs(&job_id, limit).await });
    }
    upstream::list_runs(&opts(config), job_id, limit)
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
