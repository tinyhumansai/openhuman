//! The host side of the cron scheduler.
//!
//! The runtime (poll pass, bounded concurrency, process-wide run claims, retry
//! with backoff, rescheduling and single-flight skips) is the `tinyflows`
//! module's, reached over `tinyflows_bus::schedule` (`StartScheduler`,
//! `TickScheduler`, `DrainScheduler`, `StopScheduler`, `ExecuteJob`). This
//! module owns what the module cannot: the poll cadence and the health signals
//! it emits, one scheduler per scope (the operator, and every live embedded
//! agent), and the callbacks the module runs jobs through
//! ([`crate::cron::bus_host`]): the job-kind registry ([`executor`],
//! [`builtin`]), delivery and the flow trigger ([`run_delivery`]).
//!
//! The host drives its schedulers with manual ticks rather than letting the
//! module run its own loop: it keeps the poll interval, the transition-only
//! `HealthChanged` signals and the live-agent reconcile in one place, and a
//! test or an embedding host can run one deterministic pass
//! ([`run_live_agent_pass`]) without sleeping.

mod agent_run;
pub(crate) mod builtin;
mod delivery;
mod dispatcher;
pub(crate) mod executor;
mod failure_classification;
mod live_agents;
mod origin_context;
mod origin_delivery;
pub(crate) mod run_delivery;
pub(crate) mod shell_job;

#[cfg(test)]
#[path = "scheduler_tests.rs"]
mod tests;

#[allow(unused_imports)]
use crate::core::runtime::DispatchCtx;
#[allow(unused_imports)]
use delivery::*;
#[allow(unused_imports)]
use executor::*;
#[allow(unused_imports)]
use failure_classification::*;
#[allow(unused_imports)]
use shell_job::*;

pub use delivery::deliver_job;

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::cron::bus_host::{self, Binding, HostRegistration};
use crate::cron::documents_port::DocScope;
use crate::modules::tinyflows as module;
use crate::security::SecurityPolicy;
use anyhow::{anyhow, Result};
use std::sync::Arc;
use tinyflows_bus::schedule::types::CronJob;
use tinyflows_bus::schedule::{
    members, ManualDelivery, RunJobReply, StartScheduler, StoreBackend, StoreConfig,
};
use tokio::time::{self, Duration};

pub async fn run(ctx: &crate::core::runtime::DispatchCtx, config: Config) -> Result<()> {
    // Ensure the global event bus is initialized so cron delivery events
    // are not silently dropped. This is a no-op if already initialized.
    crate::core::bus::init().await.expect("bus init");
    crate::platform::health::bus::register_health_subscriber();

    let poll_secs = config.reliability.scheduler_poll_secs.max(5);
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
    let mut dispatcher = JobDispatcher::new(ctx, &config, &security);
    let mut agents = LiveAgentSchedulers::default();

    loop {
        interval.tick().await;
        tick_once(&config, &mut last_emitted_health, &mut dispatcher).await;
        agents.tick().await;
    }
}

pub use dispatcher::execute_job_now;
#[cfg(test)]
pub(crate) use dispatcher::lose_next_ack_reply_after_commit_for_test;
pub(crate) use dispatcher::run_job_now;
pub(crate) use dispatcher::{tick_once, JobDispatcher};
pub use live_agents::run_live_agent_pass;
pub(crate) use live_agents::LiveAgentSchedulers;
pub(crate) fn scheduler_config(config: &Config) -> serde_json::Value {
    serde_json::json!({
        "poll_secs": config.reliability.scheduler_poll_secs,
        "max_concurrent": config.scheduler.max_concurrent,
        "retries": config.reliability.scheduler_retries,
        "backoff_ms": config.reliability.provider_backoff_ms,
    })
}
