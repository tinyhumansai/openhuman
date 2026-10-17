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

use std::future::Future;
use std::sync::{Arc, RwLock, Weak};

use tinybus::Result as BusResult;
use tinyflows_bus::schedule::types::{CronJob, DeliveryStatus, JobType};
use tinyflows_bus::schedule::{Attempt, CallbackAddress};

use super::scheduler::executor::HostExecutor;
use super::scheduler::run_delivery;
use crate::config::Config;
use crate::core::runtime::{CoreContext, DispatchCtx};
use crate::modules::tinyflows as module;
use crate::security::SecurityPolicy;

/// The claim namespace every host scheduler and every manual run shares: job
/// ids are unique across agents, so Run Now, the run tool and the poll loop of
/// any agent refuse to start a job another of them is running.
pub(crate) const CLAIM_SCOPE: &str = "openhuman";

/// What every callback of one scheduler shares: the config it acts on, the
/// security policy, and the context it was built in.
#[derive(Clone)]
pub(crate) struct Binding {
    config: Arc<RwLock<Config>>,
    pub(crate) security: Arc<SecurityPolicy>,
    context: Option<Weak<CoreContext>>,
    /// Only agent jobs run (the hosted-profile sweep): shell and flow rows are
    /// refused whatever the config says.
    agent_only: bool,
    runtime: Option<tokio::runtime::Handle>,
    #[cfg(feature = "flows")]
    flows: Arc<std::sync::OnceLock<crate::flows::bus::FlowTriggerSubscriber>>,
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

    /// The dispatch context the bound context stands for: the bound one, or
    /// (when it has since been dropped, or none was bound) the calling task's.
    pub(crate) fn dispatch_ctx(&self) -> DispatchCtx {
        DispatchCtx::from_weak(self.context.as_ref())
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
        self.flows
            .get_or_init(|| crate::flows::bus::FlowTriggerSubscriber::new(Arc::new(self.config())))
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
        let executor = Arc::clone(&self.executor);
        self.binding
            .offload(async move { executor.execute(&job, &run_id).await })
            .await
            .map_err(|e| bus_error("Execute", e))
    }

    /// The final failure of a run: reported once.
    async fn failed(&self, job: CronJob, run_id: String, attempt: Attempt) -> BusResult<()> {
        tracing::debug!(job_id = %job.id, %run_id, "[cron:bus] Failed");
        let executor = Arc::clone(&self.executor);
        self.binding
            .offload(async move { executor.failed(&job, &run_id, &attempt).await })
            .await
            .map_err(|e| bus_error("Failed", e))
    }

    /// Delivery of a finished run's output; the reply is the delivery status.
    async fn deliver(&self, job: CronJob, run_id: String, attempt: Attempt) -> BusResult<String> {
        tracing::debug!(job_id = %job.id, %run_id, "[cron:bus] Deliver");
        let binding = self.binding.clone();
        let status: Result<DeliveryStatus, String> = self
            .binding
            .offload(async move {
                run_delivery::deliver(&binding, &job, &run_id, &attempt)
                    .await
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|e| bus_error("Deliver", e))?;
        status
            .map(|status| status.as_str().to_string())
            .map_err(|e| bus_error("Deliver", e))
    }

    /// A flow row's schedule tick (or a `system:<name>` job).
    async fn schedule_trigger(&self, flow_id: String, run_id: String) -> BusResult<Attempt> {
        tracing::debug!(%flow_id, %run_id, "[cron:bus] ScheduleTrigger");
        let binding = self.binding.clone();
        self.binding
            .offload(async move { run_delivery::trigger(&binding, &flow_id, &run_id).await })
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

/// Whether a job of `job_type` may run under `binding`, and the refusal text
/// if not.
pub(crate) fn kind_refusal(binding: &Binding, job_type: &JobType) -> Option<String> {
    (binding.is_agent_only() && !matches!(job_type, JobType::Agent)).then(|| {
        "blocked by hosting policy: only agent cron jobs run here; shell and flow jobs are disabled"
            .to_string()
    })
}

#[cfg(all(test, feature = "modules"))]
#[path = "bus_host_tests.rs"]
mod tests;
