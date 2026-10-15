//! The host side of the cron scheduler.
//!
//! The runtime — poll pass, bounded concurrency, process-wide run claims,
//! retry with backoff, rescheduling and single-flight skips — is
//! `tinyflows_schedule::runtime::Scheduler`. This module builds it from config
//! and supplies the ports (see [`host`]): the document store, the shell/agent
//! executor with its failure classification ([`executor`]), delivery and the
//! flow trigger sink ([`run_delivery`]). It owns the loop that drives one
//! scheduler for the operator and one for every live embedded agent, and the
//! health signals the loop emits.

mod agent_run;
mod delivery;
mod executor;
mod failure_classification;
mod host;
mod origin_context;
mod origin_delivery;
mod run_delivery;
mod run_record;
pub(crate) mod shell_job;

#[cfg(test)]
#[path = "scheduler_tests.rs"]
mod tests;

#[allow(unused_imports)]
use crate::core::runtime::DispatchCtx;
use agent_run::*;
#[allow(unused_imports)]
use delivery::*;
#[allow(unused_imports)]
use executor::*;
#[allow(unused_imports)]
use failure_classification::*;
#[allow(unused_imports)]
use run_record::*;
#[allow(unused_imports)]
use shell_job::*;

pub use delivery::deliver_job;
pub(crate) use host::{try_acquire_run, JobDispatcher};

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::security::SecurityPolicy;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tinyflows_schedule::runtime::{execute_with_retry, SchedulerSlot};
use tinyflows_schedule::types::CronJob;
use tokio::time::{self, Duration};

const MIN_POLL_SECONDS: u64 = 5;

/// The poll-loop slot: one loop per process, or two would each see the same
/// due rows.
const LOOP_SCOPE: &str = "openhuman:cron-loop";

/// Whether a cron poll loop is running in this process.
pub fn is_running() -> bool {
    SchedulerSlot::is_running(LOOP_SCOPE)
}

pub async fn run(config: Config) -> Result<()> {
    let Some(_slot) = SchedulerSlot::acquire(LOOP_SCOPE) else {
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
    let security = Arc::new(SecurityPolicy::from_full_config(&config));

    BUS.publish(DomainEvent::SystemStartup {
        component: "scheduler".to_string(),
    });

    // Track the most recently *emitted* scheduler health so we only
    // publish `HealthChanged` on a state transition. Without this the
    // bus would carry a steady `healthy: true` event every poll
    // interval, forever. `None` means "nothing emitted yet for this run",
    // so the first successful tick is treated as a transition and emits.
    let mut last_emitted_health: Option<bool> = None;
    let mut dispatcher = JobDispatcher::new(&config, &security);
    let mut agents = LiveAgentSchedulers::default();

    loop {
        interval.tick().await;
        tick_once(&config, &mut last_emitted_health, &mut dispatcher).await;
        agents.tick().await;
    }
}

/// The schedulers of the live embedded agents, one per agent id.
#[derive(Default)]
pub(crate) struct LiveAgentSchedulers {
    by_agent: HashMap<String, JobDispatcher>,
}

impl LiveAgentSchedulers {
    /// Runs the due jobs of every live embedded agent, each under that agent's
    /// own context: its config, provider route, policy and job store. An agent
    /// that is not live has no context to run under — no host tools or prompt
    /// for its agent jobs — so its jobs stay dormant until it is instantiated
    /// again, including when a storage backend still records it
    /// (`crate::storage::agents`).
    pub(crate) async fn tick(&mut self) {
        let live = crate::core::runtime::AgentContextRegistry::live();
        // A scheduler whose agent is gone aborts the jobs it still owns.
        self.by_agent
            .retain(|id, _| live.iter().any(|(live_id, _)| live_id == id));
        for (agent_id, ctx) in live {
            let scheduler = &mut self.by_agent;
            let dispatch = crate::core::runtime::DispatchCtx::new(ctx.clone());
            crate::core::runtime::CoreContext::scope(ctx, async move {
                let config = match crate::config::ops::load_current_or_init(&dispatch).await {
                    Ok(config) => config,
                    Err(error) => {
                        tracing::debug!(agent = %agent_id, %error, "[cron:scheduler] agent config unavailable");
                        return;
                    }
                };
                let Some(security) = crate::security::live_policy::effective() else {
                    tracing::warn!(agent = %agent_id, "[cron:scheduler] agent has no policy; jobs skipped");
                    return;
                };
                let dispatcher = scheduler
                    .entry(agent_id.clone())
                    .or_insert_with(|| JobDispatcher::new(&config, &security));
                match dispatcher.tick(&config).await {
                    Ok(0) => {}
                    Ok(count) => {
                        tracing::debug!(agent = %agent_id, count, "[cron:scheduler] agent jobs dispatched");
                    }
                    Err(error) => {
                        tracing::warn!(agent = %agent_id, "[cron:scheduler] agent poll failed: {error}");
                    }
                }
            })
            .await;
        }
    }

    /// Wait for every dispatched agent job to finish.
    pub(crate) async fn drain(&mut self) {
        for dispatcher in self.by_agent.values_mut() {
            dispatcher.drain().await;
        }
    }
}

/// One pass over the live agents' due jobs, run to completion: what the
/// scheduler loop does after each operator tick, for hosts and end-to-end
/// tests that drive the agent pass without owning the loop's interval.
pub async fn run_live_agent_pass(max_concurrent: usize) {
    tracing::debug!(max_concurrent, "[cron:scheduler] live agent pass begin");
    let mut agents = LiveAgentSchedulers::default();
    agents.tick().await;
    agents.drain().await;
}

/// Run `job` now under the caller's `run_id` (also the id its delivery and
/// run record use), with the same executor, retry budget and failure
/// classification as a scheduled run. The caller holds the job's run claim.
pub async fn execute_job_now(config: &Config, job: &CronJob, run_id: &str) -> (bool, String) {
    let security = Arc::new(SecurityPolicy::from_full_config(config));
    let binding = host::Binding::current(config, security);
    let ports = host::ports(&binding);
    match execute_with_retry(&ports, &host::scheduler_config(config), job, run_id).await {
        Ok(attempt) => (attempt.success, attempt.output),
        Err(error) => (false, error.to_string()),
    }
}

/// Single poll cycle of the scheduler loop, extracted so tests can drive
/// it without owning `tokio::time::interval`.
///
/// Emits a `scheduler` health signal in two cases:
/// - Poll itself failed (DB read) → `healthy: false` with the DB error.
/// - Poll succeeded, queue empty or not → `healthy: true` (#3312
///   recovery signal). Without this, a single transient job failure
///   that flipped the component to `error` by a dispatched job would stay
///   there indefinitely while the queue was idle — the health endpoint would
///   keep returning 503 and Docker would mark the container `unhealthy` for
///   hours until a manual restart.
/// - Per-job results (published by each job's delivery) continue to flip the
///   component back to `healthy: false` on a failure; the next tick that
///   survives the DB read will re-flip it to `healthy: true`.
pub(crate) async fn tick_once(
    config: &Config,
    last_emitted_health: &mut Option<bool>,
    dispatcher: &mut JobDispatcher,
) {
    tracing::debug!("[cron:scheduler] tick poll begin");
    let dispatched = match dispatcher.tick(config).await {
        Ok(count) => count,
        Err(e) => {
            tracing::warn!("[cron:scheduler] tick poll failed: {e}");
            // Transition-only emission: repeat failures stay quiet so
            // subscribers don't see an event-storm during a long outage.
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

    // Transition-only emission for the recovery signal: the very first
    // successful tick after boot (or after a failure) fires; later successful
    // ticks are no-ops on the wire.
    if *last_emitted_health != Some(true) {
        tracing::debug!(
            "[cron:scheduler] tick poll ok dispatched={dispatched} (recovery signal: healthy=true)"
        );
        BUS.publish(DomainEvent::HealthChanged {
            component: "scheduler".to_string(),
            healthy: true,
            message: None,
        });
        *last_emitted_health = Some(true);
    } else {
        tracing::trace!(
            "[cron:scheduler] tick poll ok dispatched={dispatched} (steady state, no event)"
        );
    }

    if dispatched > 0 {
        // A dispatched job publishes `healthy: false` on failure directly on
        // the bus, without touching this tracker. Reset so the next
        // successful tick is again a transition and re-emits `healthy: true`.
        *last_emitted_health = None;
    }
}
