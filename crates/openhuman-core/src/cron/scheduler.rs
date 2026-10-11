//! Background scheduler loop for cron jobs: polls the store for due jobs,
//! dispatches each onto its own task with bounded concurrency (so a long job
//! never holds up the next poll), persists results, and emits health
//! signals. Job-type execution, retry, delivery, and persistence live in the
//! submodules below.

mod agent_run;
mod delivery;
mod dispatch;
mod failure_classification;
mod origin_context;
mod origin_delivery;
mod retry;
mod run_record;
pub(crate) mod shell_job;
mod slot;

#[cfg(test)]
#[path = "scheduler_tests.rs"]
mod tests;

#[allow(unused_imports)]
use agent_run::*;
#[allow(unused_imports)]
use delivery::*;
#[allow(unused_imports)]
use failure_classification::*;
#[allow(unused_imports)]
use retry::*;
#[allow(unused_imports)]
use run_record::*;
#[allow(unused_imports)]
use shell_job::*;

pub use delivery::deliver_job;
pub(crate) use dispatch::JobDispatcher;
pub use retry::execute_job_now;
pub use slot::is_running;
pub(crate) use slot::SchedulerSlot;

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::security::SecurityPolicy;
use anyhow::Result;
use chrono::Utc;
use std::sync::Arc;
use tokio::time::{self, Duration};
use {crate::cron::due_jobs, tinyflows_schedule::types::CronJob};

const MIN_POLL_SECONDS: u64 = 5;

pub async fn run(config: Config) -> Result<()> {
    // One poll loop per process: two would each see the same due rows.
    let Some(_slot) = SchedulerSlot::acquire() else {
        tracing::warn!(
            "[cron:scheduler] a scheduler loop is already running; not starting another"
        );
        return Ok(());
    };
    // Ensure the global event bus is initialized so cron delivery events
    // are not silently dropped. This is a no-op if already initialized.
    crate::core::bus::init().await.expect("bus init");
    crate::platform::health::bus::register_health_subscriber();

    let poll_secs = config.reliability.scheduler_poll_secs.max(MIN_POLL_SECONDS);
    let mut interval = time::interval(Duration::from_secs(poll_secs));
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));

    BUS.publish(DomainEvent::SystemStartup {
        component: "scheduler".to_string(),
    });

    // Track the most recently *emitted* scheduler health so we only
    // publish `HealthChanged` on a state transition. Without this the
    // bus would carry a steady `healthy: true` event every poll
    // interval — typically 30 s, forever — churn for any subscriber
    // that logs / persists / reacts to health events. `None` means
    // "nothing emitted yet for this run", so the first successful tick
    // is treated as a transition and emits.
    let mut last_emitted_health: Option<bool> = None;
    let mut dispatcher = JobDispatcher::new(config.scheduler.max_concurrent);

    loop {
        interval.tick().await;
        tick_once(
            &config,
            &security,
            &mut last_emitted_health,
            &mut dispatcher,
        )
        .await;
        tick_live_agents(&mut dispatcher).await;
    }
}

/// Runs the due jobs of every live embedded agent, each under that agent's
/// own context: its config, provider route, policy and job store. An agent
/// that is not live has no context to run under — no host tools or prompt
/// for its agent jobs — so its jobs stay dormant until it is instantiated
/// again, including when a storage backend still records it
/// (`crate::storage::agents`).
pub(crate) async fn tick_live_agents(dispatcher: &mut JobDispatcher) {
    for (agent_id, ctx) in crate::core::runtime::AgentContextRegistry::live() {
        let agent = agent_id.clone();
        let dispatcher = &mut *dispatcher;
        crate::core::runtime::CoreContext::scope(ctx, async move {
            let config = match crate::config::ops::load_current_or_init().await {
                Ok(config) => config,
                Err(error) => {
                    tracing::debug!(agent = %agent, %error, "[cron:scheduler] agent config unavailable");
                    return;
                }
            };
            if !agent_jobs_may_exist(&config) {
                return;
            }
            let jobs = match due_jobs(&config, Utc::now()) {
                Ok(jobs) => jobs,
                Err(error) => {
                    tracing::warn!(agent = %agent, "[cron:scheduler] agent poll db_error: {error}");
                    return;
                }
            };
            if jobs.is_empty() {
                return;
            }
            let Some(security) = crate::security::live_policy::effective() else {
                tracing::warn!(agent = %agent, "[cron:scheduler] agent has no policy; jobs skipped");
                return;
            };
            tracing::debug!(agent = %agent, due_count = jobs.len(), "[cron:scheduler] running agent jobs");
            dispatcher.dispatch(&config, &security, jobs).await;
        })
        .await;
    }
}

/// One pass over the live agents' due jobs, run to completion: what the
/// scheduler loop does after each operator tick, for hosts and end-to-end
/// tests that drive the agent pass without owning the loop's interval.
pub async fn run_live_agent_pass(max_concurrent: usize) {
    tracing::debug!(max_concurrent, "[cron:scheduler] live agent pass begin");
    let mut dispatcher = JobDispatcher::new(max_concurrent.max(1));
    tick_live_agents(&mut dispatcher).await;
    dispatcher.drain().await;
}

/// Whether an agent can have jobs to poll: with a storage backend its jobs
/// live in the backend under its scope (`crate::storage`), so there is always
/// something to ask; without one, only once its own `jobs.db` exists.
pub(crate) fn agent_jobs_may_exist(config: &Config) -> bool {
    crate::storage::installed().is_some() || crate::cron::store::db_path(config).exists()
}

/// Single poll cycle of the scheduler loop, extracted so tests can drive
/// it without owning `tokio::time::interval`.
///
/// Emits a `scheduler` health signal in three cases:
/// - Poll itself failed (DB read) → `healthy: false` with the DB error.
/// - Poll succeeded, queue empty or not → `healthy: true` (#3312
///   recovery signal). Without this, a single transient job failure
///   that flipped the component to `error` by a dispatched job (`dispatch.rs`)
///   would stay there indefinitely while the queue was idle — no later
///   event would clear it, the health endpoint would keep returning
///   503, and Docker would mark the container `unhealthy` for hours
///   until a manual restart. Tick-level "still polling" beats
///   job-level success as the recovery signal because the queue is
///   empty most of the time.
/// - Per-job results (published by each dispatched job) continue to
///   flip the component back to `healthy: false` on a failure; the
///   next tick that survives the DB read will re-flip it to
///   `healthy: true`, exactly the auto-recovery behaviour the Docker
///   health check needs.
pub(crate) async fn tick_once(
    config: &Config,
    security: &Arc<SecurityPolicy>,
    last_emitted_health: &mut Option<bool>,
    dispatcher: &mut JobDispatcher,
) {
    tracing::debug!("[cron:scheduler] tick poll begin");
    let jobs = match due_jobs(config, Utc::now()) {
        Ok(jobs) => jobs,
        Err(e) => {
            tracing::warn!("[cron:scheduler] tick poll db_error: {e}");
            // Transition-only emission: only publish on the first
            // failure after a previous healthy (or unknown) state.
            // Repeat DB failures stay quiet so subscribers don't see
            // an event-storm during a long outage.
            if *last_emitted_health != Some(false) {
                BUS.publish(DomainEvent::HealthChanged {
                    component: "scheduler".to_string(),
                    healthy: false,
                    message: Some(e.to_string()),
                });
                *last_emitted_health = Some(false);
            }
            return;
        }
    };

    let due_count = jobs.len();
    // Transition-only emission for the recovery / healthy signal: a
    // long idle stretch with no transitions stays silent on the bus,
    // so subscribers don't pay per-poll work for a steady `healthy:
    // true` event every poll interval — the nit oxoxDev caught on
    // #3329. The very first successful tick after boot (or after a
    // failure) is the one that fires; subsequent successful ticks
    // are no-ops on the wire.
    if *last_emitted_health != Some(true) {
        tracing::debug!(
            "[cron:scheduler] tick poll ok due_count={due_count} (recovery signal: healthy=true)"
        );
        BUS.publish(DomainEvent::HealthChanged {
            component: "scheduler".to_string(),
            healthy: true,
            message: None,
        });
        *last_emitted_health = Some(true);
    } else {
        tracing::trace!(
            "[cron:scheduler] tick poll ok due_count={due_count} (steady state, no event)"
        );
    }

    if due_count == 0 {
        tracing::trace!("[cron:scheduler] tick end (no due jobs)");
        return;
    }

    dispatcher.dispatch(config, security, jobs).await;
    tracing::debug!("[cron:scheduler] tick end due_count={due_count} (jobs processed)");

    // A dispatched job may have published `healthy: false` on
    // a job failure, but it does so directly on the bus without
    // touching our local tracker. Reset so the next successful tick
    // is again treated as a transition and re-emits `healthy: true` —
    // exactly the auto-recovery behaviour #3312 requires.
    *last_emitted_health = None;
}

pub(super) async fn execute_and_persist_job(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (String, bool, Option<String>) {
    warn_if_high_frequency_agent_job(job);

    let started_at = Utc::now();

    BUS.publish(DomainEvent::CronJobTriggered {
        job_id: job.id.clone(),
        job_name: job.name.clone().unwrap_or_default(),
        job_type: format!("{:?}", job.job_type),
    });

    let run_id = uuid::Uuid::new_v4().to_string();
    let (execution_success, output) =
        execute_job_with_retry_for_run(config, security, job, &run_id).await;
    let finished_at = Utc::now();
    let success = persist_job_result_for_run(
        config,
        job,
        &run_id,
        execution_success,
        &output,
        started_at,
        finished_at,
    )
    .await;

    // The job ran as its agent (when it had one); a one-shot job is already
    // deleted, so subscribers learn the owner here, not from the store.
    crate::cron::completion_owner::note(&job.id);
    BUS.publish(DomainEvent::CronJobCompleted {
        job_id: job.id.clone(),
        success,
        output: crate::util::truncate_with_ellipsis(&output, 512),
    });
    let failure_message = (!success).then(|| crate::util::truncate_with_ellipsis(&output, 256));

    (job.id.clone(), success, failure_message)
}
