//! The host half of the schedule contract: the `ScheduleHost` callback object.
//!
//! The `tinyflows` module drives a scheduler (its own poll loop, or ticks the
//! host sends) and, for every job it decides to run, calls back into the host
//! on `tinyflows_bus::schedule::CALLBACK_INTERFACE`. One object is served per
//! scheduler scope (the operator, and each live embedded agent or hosted
//! profile), at its own object path, bound to that scope's [`Binding`]: the
//! config it acts on, its security policy and its `CoreContext`. The module
//! therefore never names a tenant; the path it was given is the identity.
//!
//! Only the four members the module needs when it owns the store are served:
//!
//! | Member | Host side |
//! | --- | --- |
//! | `Execute(job, run_id)` | one attempt through the job-kind registry ([`super::scheduler::executor`]) |
//! | `Failed(job, run_id, attempt)` | the retries-exhausted report |
//! | `Deliver(job, run_id, attempt)` | output delivery and the run's completion events |
//! | `ScheduleTrigger(flow_id, run_id)` | a flow tick or a `system:<name>` job |
//!
//! The job-store members (`DueJobs`, `ClaimSlot`, `RecordRun`, `Complete`) are
//! never called: the module reads and writes jobs through the document port
//! ([`super::documents_port`]), which is why this object has none of them.
//!
//! Work is run on the runtime that created the binding, not on the module bus
//! thread that delivered the callback: an agent turn is deep and long, and the
//! bus runtime is for serving. The binding's context is re-entered around it.

use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, RwLock, Weak};

use tinybus::Result as BusResult;
use tinyflows_bus::schedule::types::{CronJob, DeliveryStatus, JobType};
use tinyflows_bus::schedule::{Attempt, CallbackAddress, DeliveryProjection};

use super::scheduler::executor::HostExecutor;
use super::scheduler::run_delivery;
use crate::config::Config;
use crate::core::runtime::{CoreContext, DispatchCtx};
use crate::modules::tinyflows as module;
use crate::security::SecurityPolicy;

/// What every callback of one scheduler shares: the config it acts on, the
/// security policy, and the context it was built in.
#[derive(Clone)]
pub(crate) struct Binding {
    config: Arc<RwLock<Config>>,
    pub(crate) security: Arc<SecurityPolicy>,
    context: Option<Weak<CoreContext>>,
    /// The hosted-profile sweep runs agent jobs plus trusted memory system
    /// jobs; shell and user-owned flow rows are refused.
    agent_only: bool,
    runtime: Option<tokio::runtime::Handle>,
    #[cfg(feature = "flows")]
    flows: Arc<std::sync::OnceLock<crate::flows::bus::FlowTriggerSubscriber>>,
    triggered_runs: Arc<std::sync::Mutex<HashSet<String>>>,
}

impl Binding {
    /// A binding for the context `ctx` carries (none when it is unbound).
    pub(crate) fn new(ctx: &DispatchCtx, config: &Config, security: Arc<SecurityPolicy>) -> Self {
        Self {
            config: Arc::new(RwLock::new(config.clone())),
            security,
            context: ctx.core().map(Arc::downgrade),
            agent_only: false,
            runtime: tokio::runtime::Handle::try_current().ok(),
            #[cfg(feature = "flows")]
            flows: Arc::new(std::sync::OnceLock::new()),
            triggered_runs: Arc::new(std::sync::Mutex::new(HashSet::new())),
        }
    }

    /// The same binding restricted to agent jobs.
    #[must_use]
    pub(crate) fn agent_only(mut self) -> Self {
        self.agent_only = true;
        self
    }

    /// Whether only agent jobs may run under this binding.
    pub(crate) fn is_agent_only(&self) -> bool {
        self.agent_only
    }

    /// The config as of the latest tick.
    pub(crate) fn config(&self) -> Config {
        self.config
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn set_config(&self, config: &Config) {
        *self
            .config
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = config.clone();
    }

    /// Publish the start event once per logical scheduler run, across Execute
    /// retries and the separate ScheduleTrigger callback path.
    pub(crate) fn publish_triggered_once(&self, job: &CronJob, run_id: &str) {
        let mut runs = self
            .triggered_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if runs.contains(run_id) {
            return;
        }
        runs.insert(run_id.to_string());
        crate::core::bus::BUS.publish(crate::core::events::DomainEvent::CronJobTriggered {
            job_id: job.id.clone(),
            job_name: job.name.clone().unwrap_or_default(),
            job_type: format!("{:?}", job.job_type),
        });
    }

    pub(crate) fn finish_triggered_run(&self, run_id: &str) {
        self.triggered_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(run_id);
    }

    /// The dispatch context the bound context stands for: the bound one, or
    /// (when it has since been dropped, or none was bound) the calling task's.
    pub(crate) fn dispatch_ctx(&self) -> DispatchCtx {
        DispatchCtx::from_weak(self.context.as_ref())
    }

    /// Pin the bound context for a finite module lifecycle handoff. The host
    /// registration itself keeps only the weak reference, avoiding a cycle
    /// through long-lived scheduler state.
    pub(crate) fn pin_context(&self) -> Option<Arc<CoreContext>> {
        self.context.as_ref().and_then(Weak::upgrade)
    }

    /// Acquire the bound context for one callback. A missing weak reference is
    /// an intentional unbound test/pre-boot context; an expired bound context
    /// fails closed instead of borrowing the callback runtime's ambient one.
    fn callback_context(&self) -> Result<Option<Arc<CoreContext>>, String> {
        match &self.context {
            Some(context) => context
                .upgrade()
                .map(Some)
                .ok_or_else(|| "the schedule callback's bound context has expired".to_string()),
            None => Ok(None),
        }
    }

    /// Runs `fut` under the bound context. A context that has since been
    /// dropped (its agent is no longer live) runs `fut` as the caller is.
    pub(crate) async fn within<F: Future>(&self, fut: F) -> F::Output {
        match self.context.as_ref().and_then(Weak::upgrade) {
            Some(ctx) => CoreContext::scope(ctx, fut).await,
            None => fut.await,
        }
    }

    /// Runs `fut` on the runtime that built this binding and waits for it.
    /// Without one (a binding built outside any runtime) it runs inline.
    ///
    /// # Errors
    ///
    /// When the task panicked or its runtime shut down.
    pub(crate) async fn offload<T, F>(&self, fut: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        match &self.runtime {
            Some(handle) => handle
                .spawn(fut)
                .await
                .map_err(|error| format!("the scheduled job's task did not finish: {error}")),
            None => Ok(fut.await),
        }
    }

    /// The flow-trigger subscriber this scheduler's flow ticks go through; it
    /// holds the per-flow in-flight dedupe across ticks.
    #[cfg(feature = "flows")]
    pub(crate) fn flows(&self) -> &crate::flows::bus::FlowTriggerSubscriber {
        self.flows.get_or_init(|| {
            crate::flows::bus::FlowTriggerSubscriber::with_config_source(Arc::clone(&self.config))
        })
    }
}

/// The `ScheduleHost` object for one scope.
pub(crate) struct ScheduleHost {
    binding: Binding,
    executor: Arc<HostExecutor>,
}

fn bus_error(what: &str, error: impl std::fmt::Display) -> tinybus::Error {
    tracing::debug!(callback = what, %error, "[cron:bus] callback failed");
    tinybus::Error::failed(error)
}

#[tinybus::interface(name = "ai.tinyhumans.tinyflows.ScheduleHost")]
impl ScheduleHost {
    /// One attempt of a shell or agent job, classified `retryable` or permanent.
    async fn execute(&self, job: CronJob, run_id: String) -> BusResult<Attempt> {
        tracing::debug!(job_id = %job.id, %run_id, "[cron:bus] Execute");
        let context_pin = self
            .binding
            .callback_context()
            .map_err(|error| bus_error("Execute", error))?;
        let executor = Arc::clone(&self.executor);
        self.binding
            .offload(async move {
                let _context_pin = context_pin;
                executor.execute(&job, &run_id).await
            })
            .await
            .map_err(|e| bus_error("Execute", e))
    }

    /// The final failure of a run: reported once.
    async fn failed(&self, job: CronJob, run_id: String, attempt: Attempt) -> BusResult<()> {
        tracing::debug!(job_id = %job.id, %run_id, "[cron:bus] Failed");
        let context_pin = self
            .binding
            .callback_context()
            .map_err(|error| bus_error("Failed", error))?;
        let executor = Arc::clone(&self.executor);
        self.binding
            .offload(async move {
                let _context_pin = context_pin;
                executor.failed(&job, &run_id, &attempt).await
            })
            .await
            .map_err(|e| bus_error("Failed", e))
    }

    /// Send a finished run's output exactly once; the reply is its status.
    async fn send(&self, job: CronJob, run_id: String, attempt: Attempt) -> BusResult<String> {
        tracing::debug!(job_id = %job.id, %run_id, "[cron:bus] Send");
        let context_pin = self
            .binding
            .callback_context()
            .map_err(|error| bus_error("Send", error))?;
        let binding = self.binding.clone();
        let status: Result<DeliveryStatus, String> = self
            .binding
            .offload(async move {
                let _context_pin = context_pin;
                run_delivery::send(&binding, &job, &run_id, &attempt)
                    .await
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|e| bus_error("Send", e))?;
        status
            .map(|status| status.as_str().to_string())
            .map_err(|e| bus_error("Send", e))
    }

    /// Project the module's terminal decision without sending a second time.
    async fn deliver(&self, projection: DeliveryProjection) -> BusResult<()> {
        tracing::debug!(job_id = %projection.job.id, run_id = %projection.run_id, "[cron:bus] Deliver");
        let context_pin = self
            .binding
            .callback_context()
            .map_err(|error| bus_error("Deliver", error))?;
        let binding = self.binding.clone();
        self.binding
            .offload(async move {
                let _context_pin = context_pin;
                run_delivery::deliver(&binding, &projection).await
            })
            .await
            .map_err(|e| bus_error("Deliver", e))?;
        Ok(())
    }

    /// A flow row's schedule tick (or a `system:<name>` job).
    async fn schedule_trigger(&self, job: CronJob, run_id: String) -> BusResult<Attempt> {
        tracing::debug!(job_id = %job.id, %run_id, "[cron:bus] ScheduleTrigger");
        let context_pin = self
            .binding
            .callback_context()
            .map_err(|error| bus_error("ScheduleTrigger", error))?;
        let binding = self.binding.clone();
        self.binding
            .offload(async move {
                let _context_pin = context_pin;
                run_delivery::trigger(&binding, &job, &run_id).await
            })
            .await
            .map_err(|e| bus_error("ScheduleTrigger", e))
    }
}

/// A served `ScheduleHost` object: the address to give the module, unserved on
/// drop.
pub(crate) struct HostRegistration {
    path: String,
    address: CallbackAddress,
}

impl HostRegistration {
    /// The address the module calls back.
    pub(crate) fn address(&self) -> CallbackAddress {
        self.address.clone()
    }
}

impl Drop for HostRegistration {
    fn drop(&mut self) {
        module::unserve(std::mem::take(&mut self.path));
    }
}

/// Serve a `ScheduleHost` bound to `binding` and return its registration.
///
/// # Errors
///
/// When the module bus is unavailable.
pub(crate) async fn register(binding: Binding) -> Result<HostRegistration, String> {
    let path = format!(
        "/ai/tinyhumans/openhuman/ScheduleHost/h{}",
        uuid::Uuid::new_v4().simple()
    );
    let host = ScheduleHost {
        executor: Arc::new(HostExecutor::new(binding.clone())),
        binding,
    };
    module::serve(path.clone(), host).await?;
    tracing::debug!(%path, "[cron:bus] host object served");
    Ok(HostRegistration {
        address: CallbackAddress {
            destination: module::HOST_BUS_NAME.to_string(),
            object_path: path.clone(),
        },
        path,
    })
}

/// Whether a job may run under `binding`, and the refusal text
/// if not.
pub(crate) fn kind_refusal(binding: &Binding, job: &CronJob) -> Option<String> {
    let trusted_memory_job = crate::cron::system_jobs::trusted_system_job_owner(job).is_some();
    (binding.is_agent_only()
        && !matches!(job.job_type, JobType::Agent)
        && !trusted_memory_job)
        .then(|| {
        "blocked by hosting policy: only agent cron jobs and trusted memory maintenance run here; shell and user flow jobs are disabled"
            .to_string()
        })
}

#[cfg(all(test, feature = "modules"))]
#[path = "bus_host_tests.rs"]
mod tests;

#[cfg(all(test, feature = "modules"))]
#[path = "bus_host_context_tests.rs"]
mod context_tests;

// Boots the module runtime (`crate::modules::runtime`), which only exists
// with the `modules` feature.
#[cfg(all(test, feature = "modules"))]
#[path = "bus_host_trusted_system_tests.rs"]
mod trusted_system_tests;
