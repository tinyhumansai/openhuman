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
pub(crate) mod executor;
mod failure_classification;
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
use crate::cron::bus_host::{self, Binding, HostRegistration, CLAIM_SCOPE};
use crate::cron::documents_port::DocScope;
use crate::modules::tinyflows as module;
use crate::security::SecurityPolicy;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use tinyflows_bus::schedule::types::CronJob;
use tinyflows_bus::schedule::{members, Attempt, StartScheduler, StoreBackend, StoreConfig};
use tokio::time::{self, Duration};

const MIN_POLL_SECONDS: u64 = 5;

/// What the module answers when another run of the job holds its claim.
pub(crate) const ALREADY_RUNNING: &str = "job is already running";

/// Whether the host's cron poll loop is running in this process.
static LOOP_RUNNING: AtomicBool = AtomicBool::new(false);

/// The poll-loop slot: one loop per process, or two would each see the same
/// due rows. Released when dropped.
struct LoopSlot;

impl LoopSlot {
    fn acquire() -> Option<Self> {
        LOOP_RUNNING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self)
    }
}

impl Drop for LoopSlot {
    fn drop(&mut self) {
        LOOP_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// Whether a cron poll loop is running in this process.
pub fn is_running() -> bool {
    LOOP_RUNNING.load(Ordering::SeqCst)
}

/// Job ids a manual run (Run Now, the run tool) holds in this process.
static MANUAL_RUNS: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);

/// A held manual run. Released on drop, including on panic or future
/// cancellation, so an aborted task can never lock a job id.
pub(crate) struct RunGuard(String);

impl Drop for RunGuard {
    fn drop(&mut self) {
        MANUAL_RUNS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.0);
    }
}

/// Claims `job_id` for a manual run, or `None` when another manual run holds
/// it. A scheduled run of the same job is refused later, by the module's own
/// claim (`ExecuteJob` answers [`ALREADY_RUNNING`]).
pub(crate) fn try_acquire_run(job_id: &str) -> Option<RunGuard> {
    MANUAL_RUNS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(job_id.to_string())
        .then(|| RunGuard(job_id.to_string()))
}

/// The module's retry settings from the operator's reliability config.
pub(crate) fn scheduler_config(config: &Config) -> Value {
    json!({
        "poll_secs": config.reliability.scheduler_poll_secs,
        "max_concurrent": config.scheduler.max_concurrent,
        "retries": config.reliability.scheduler_retries,
        "backoff_ms": config.reliability.provider_backoff_ms,
    })
}

pub async fn run(ctx: &crate::core::runtime::DispatchCtx, config: Config) -> Result<()> {
    let Some(_slot) = LoopSlot::acquire() else {
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
                let Some(security) = crate::security::live_policy::effective_in(&dispatch) else {
                    tracing::warn!(agent = %agent_id, "[cron:scheduler] agent has no policy; jobs skipped");
                    return;
                };
                let dispatcher = scheduler
                    .entry(agent_id.clone())
                    .or_insert_with(|| JobDispatcher::new(&dispatch, &config, &security));
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
/// classification as a scheduled run: the module's `ExecuteJob` runs the
/// retry loop and calls back into a host object bound to `ctx`. The caller
/// holds the job's manual-run guard; the module holds the cross-scheduler
/// claim and answers [`ALREADY_RUNNING`] when a scheduled run has the job.
pub async fn execute_job_now(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    job: &CronJob,
    run_id: &str,
) -> (bool, String) {
    match execute_via_bus(ctx, config, job, run_id).await {
        Ok(attempt) => (attempt.success, attempt.output),
        Err(error) => {
            tracing::debug!(job_id = %job.id, %run_id, %error, "[cron:scheduler] ExecuteJob failed");
            (false, error.to_string())
        }
    }
}

async fn execute_via_bus(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    job: &CronJob,
    run_id: &str,
) -> Result<Attempt> {
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    let binding = Binding::new(ctx, config, security);
    let host = bus_host::register(binding).await.map_err(|e| anyhow!(e))?;
    let proxy = module::run_proxy(config).await.map_err(|e| anyhow!(e))?;
    tracing::debug!(job_id = %job.id, %run_id, "[cron:scheduler] ExecuteJob");
    let attempt: Attempt = proxy
        .call(
            members::EXECUTE_JOB,
            (
                CLAIM_SCOPE,
                host.address(),
                scheduler_config(config),
                job,
                run_id,
            ),
        )
        .await
        .map_err(|error| {
            let message = error.wire_message();
            if message.contains(ALREADY_RUNNING) {
                anyhow!("cron job '{}' is already running", job.id)
            } else {
                anyhow!(message)
            }
        })?;
    drop(host);
    Ok(attempt)
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

/// One scope's scheduler: a manual-tick scheduler in the module, polled by the
/// host, dispatching under its context. Stopped when dropped.
pub(crate) struct JobDispatcher {
    binding: Binding,
    started: Option<Started>,
}

/// A scheduler running in the module, and the host object it calls back.
struct Started {
    handle: String,
    host: HostRegistration,
}

impl JobDispatcher {
    /// A dispatcher for the context `ctx` carries. Nothing is started until the
    /// first tick.
    pub(crate) fn new(
        ctx: &crate::core::runtime::DispatchCtx,
        config: &Config,
        security: &Arc<SecurityPolicy>,
    ) -> Self {
        Self {
            binding: Binding::new(ctx, config, Arc::clone(security)),
            started: None,
        }
    }

    /// Restrict the dispatcher to agent jobs (the hosted-profile sweep).
    #[must_use]
    pub(crate) fn agent_only(mut self) -> Self {
        self.binding = self.binding.clone().agent_only();
        self
    }

    /// The dispatch context the scheduler's callbacks act under.
    #[cfg(test)]
    pub(crate) fn dispatch_ctx(&self) -> crate::core::runtime::DispatchCtx {
        self.binding.dispatch_ctx()
    }

    async fn start(&self, config: &Config) -> Result<Started> {
        let host = bus_host::register(self.binding.clone())
            .await
            .map_err(|e| anyhow!(e))?;
        // The documents of the scope this dispatcher acts in: resolved under
        // its context, because the call below is made from a task that may
        // not carry it.
        let scope = self
            .binding
            .within(async { DocScope::current(config) })
            .await?;
        let storage = StoreConfig {
            backend: StoreBackend::Documents {
                callback: crate::cron::documents_port::address(&scope).await?,
            },
            max_run_history: config.cron.max_run_history,
            max_tasks: config.scheduler.max_tasks,
        };
        let request = StartScheduler {
            scope: CLAIM_SCOPE.to_string(),
            callback: host.address(),
            config: scheduler_config(config),
            manual_ticks: true,
            storage: Some(storage),
        };
        let proxy = module::proxy(config).await.map_err(|e| anyhow!(e))?;
        let handle: String = proxy
            .call(members::START_SCHEDULER, (request,))
            .await
            .map_err(|error| anyhow!(error.wire_message()))?;
        tracing::debug!("[cron:scheduler] scheduler started in the tinyflows module");
        Ok(Started { handle, host })
    }

    async fn call<R: serde::de::DeserializeOwned>(
        &mut self,
        config: &Config,
        member: &'static str,
    ) -> Result<R> {
        if self.started.is_none() {
            self.started = Some(self.start(config).await?);
        }
        let handle = self
            .started
            .as_ref()
            .map(|started| started.handle.clone())
            .ok_or_else(|| anyhow!("scheduler not started"))?;
        let proxy = module::run_proxy(config).await.map_err(|e| anyhow!(e))?;
        proxy
            .call(member, (handle,))
            .await
            .map_err(|error| anyhow!(error.wire_message()))
    }

    /// Read the due jobs and dispatch them; the count dispatched. Errors when
    /// the store cannot be read.
    pub(crate) async fn tick(&mut self, config: &Config) -> Result<usize> {
        self.binding.set_config(config);
        let dispatched: usize = self.call(config, members::TICK_SCHEDULER).await?;
        tracing::trace!(dispatched, "[cron:scheduler] TickScheduler");
        Ok(dispatched)
    }

    /// Wait for every dispatched job to finish.
    pub(crate) async fn drain(&mut self) {
        let config = self.binding.config();
        if self.started.is_none() {
            return;
        }
        if let Err(error) = self.call::<()>(&config, members::DRAIN_SCHEDULER).await {
            tracing::warn!(%error, "[cron:dispatch] a dispatched job failed");
        }
    }
}

impl Drop for JobDispatcher {
    fn drop(&mut self) {
        let Some(started) = self.started.take() else {
            return;
        };
        let config = self.binding.config();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        // Stop the module's scheduler (releasing its claims) before the
        // callback object it would call goes away.
        runtime.spawn(async move {
            match module::proxy(&config).await {
                Ok(proxy) => {
                    let stopped = proxy
                        .call::<()>(members::STOP_SCHEDULER, (started.handle.clone(),))
                        .await;
                    if let Err(error) = stopped {
                        tracing::debug!(error = %error.wire_message(), "[cron:scheduler] StopScheduler failed");
                    }
                }
                Err(error) => tracing::debug!(%error, "[cron:scheduler] StopScheduler skipped"),
            }
            drop(started.host);
        });
    }
}
