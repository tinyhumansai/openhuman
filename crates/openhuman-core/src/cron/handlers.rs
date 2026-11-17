//! The job-kind handler registry: what runs when the scheduler fires a job.
//!
//! The `tinyflows` module decides *when* a job runs (claims, retries,
//! completion) and calls back into the host to run it. Which host code answers
//! is decided here, per [`JobType`], so the split is a table and not a `match`
//! welded into the executor:
//!
//! | Kind | Default handler | Reached through |
//! | --- | --- | --- |
//! | `shell` | the sandboxed shell runner (`scheduler::shell_job`) | `Execute` |
//! | `agent` | a full agent turn (`scheduler::agent_run`) | `Execute` |
//! | `flow` | a flow schedule tick, or a `system:<name>` job | `ScheduleTrigger` |
//!
//! A host (an embedder, a hosted profile) may [`register_kind`] its own handler
//! for any kind, or add one for a `system:<name>` job with
//! [`register_system`]; both replace the default for the life of the returned
//! guard and restore it on drop. The guard of a later registration wins, and
//! dropping an older guard never disturbs a newer handler.
//!
//! What stays in the executor and is *not* pluggable: operator limits, the
//! per-run bookkeeping and the permanent-failure classification of a
//! shell/agent attempt (`scheduler::executor`). A handler reports the raw
//! result of one attempt as a [`JobOutcome`]; the host classifies it.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, PoisonError, RwLock};

use async_trait::async_trait;
use tinyflows_bus::schedule::types::{CronJob, JobType};

use super::bus_host::Binding;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::security::SecurityPolicy;

/// What a handler is given besides the job: the scope the run belongs to.
#[derive(Clone)]
pub struct JobRunContext {
    binding: Binding,
}

impl JobRunContext {
    pub(crate) fn new(binding: Binding) -> Self {
        Self { binding }
    }

    pub(crate) fn binding(&self) -> &Binding {
        &self.binding
    }

    /// The config of the scope the job belongs to, as of the latest tick.
    #[must_use]
    pub fn config(&self) -> Config {
        self.binding.config()
    }

    /// The dispatch context the run acts under.
    #[must_use]
    pub fn dispatch_ctx(&self) -> DispatchCtx {
        self.binding.dispatch_ctx()
    }

    /// The security policy the scope's shell and agent runs are held to.
    #[must_use]
    pub fn security(&self) -> &Arc<SecurityPolicy> {
        &self.binding.security
    }
}

/// The raw result of one attempt of a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobOutcome {
    /// Whether the attempt succeeded.
    pub success: bool,
    /// Text recorded as the run's output (and delivered).
    pub output: String,
    /// The agent's own error text, when the attempt was an agent turn that
    /// failed. The host classifies permanent failures (expired session, no
    /// credits, ...) from it; it is never shown to the user.
    pub agent_error: Option<String>,
    /// Stop retrying this run now, whatever the retry budget says.
    pub permanent: bool,
}

impl JobOutcome {
    /// A successful attempt.
    #[must_use]
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            success: true,
            output: output.into(),
            agent_error: None,
            permanent: false,
        }
    }

    /// A failed attempt the retry budget may retry.
    #[must_use]
    pub fn failed(output: impl Into<String>) -> Self {
        Self {
            success: false,
            output: output.into(),
            agent_error: None,
            permanent: false,
        }
    }

    /// Mark the attempt as not worth retrying.
    #[must_use]
    pub fn permanent(mut self) -> Self {
        self.permanent = true;
        self
    }
}

/// Runs one attempt of every job of the kind it is registered for.
#[async_trait]
pub trait JobKindHandler: Send + Sync {
    /// One attempt of `job` under run `run_id`. The same `run_id` names every
    /// attempt of a run, its delivery key and its run record.
    async fn run(&self, cx: &JobRunContext, job: &CronJob, run_id: &str) -> JobOutcome;
}

use super::system_job_handlers::SystemJobHandler as SystemHandler;

#[derive(Default)]
struct Registry {
    kinds: HashMap<&'static str, Arc<dyn JobKindHandler>>,
    systems: HashMap<String, SystemHandler>,
}

static REGISTRY: LazyLock<RwLock<Registry>> = LazyLock::new(|| {
    let mut registry = Registry::default();
    for (kind, handler) in super::scheduler::builtin::defaults() {
        registry.kinds.insert(kind.as_str(), handler);
    }
    RwLock::new(registry)
});

fn read() -> std::sync::RwLockReadGuard<'static, Registry> {
    REGISTRY.read().unwrap_or_else(PoisonError::into_inner)
}

fn write() -> std::sync::RwLockWriteGuard<'static, Registry> {
    REGISTRY.write().unwrap_or_else(PoisonError::into_inner)
}

/// Serialises the tests that swap a kind handler in the process-wide registry.
///
/// A test registers a decorator that hands every other job to the handler it
/// replaced, reading that handler with [`kind_handler`] before it calls
/// [`register_kind`]. Two such tests running in parallel can both read the same
/// handler, so the second one's decorator bypasses the first's and a job lands
/// on the wrong handler. Hold this for the whole read, register and use, and
/// declare it before the [`KindRegistration`] so the registration drops first.
#[cfg(test)]
pub(crate) static KIND_REGISTRY_TEST_LOCK: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());

/// Keeps a kind handler registered while held; dropping it restores the handler
/// it replaced, unless a later registration has replaced this one.
#[must_use = "the handler is unregistered when the registration drops"]
pub struct KindRegistration {
    kind: JobType,
    handler: Arc<dyn JobKindHandler>,
    previous: Option<Arc<dyn JobKindHandler>>,
}

impl Drop for KindRegistration {
    fn drop(&mut self) {
        let mut registry = write();
        let current = registry.kinds.get(self.kind.as_str());
        if current.is_some_and(|current| Arc::ptr_eq(current, &self.handler)) {
            match self.previous.take() {
                Some(previous) => {
                    registry.kinds.insert(self.kind.as_str(), previous);
                }
                None => {
                    registry.kinds.remove(self.kind.as_str());
                }
            }
            tracing::debug!(
                kind = self.kind.as_str(),
                "[cron:handlers] kind handler restored"
            );
        }
    }
}

impl std::fmt::Debug for KindRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KindRegistration")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// Register `handler` for jobs of `kind`, replacing the current handler.
pub fn register_kind(kind: JobType, handler: Arc<dyn JobKindHandler>) -> KindRegistration {
    let previous = write().kinds.insert(kind.as_str(), Arc::clone(&handler));
    tracing::debug!(
        kind = kind.as_str(),
        replaced = previous.is_some(),
        "[cron:handlers] kind handler registered"
    );
    KindRegistration {
        kind,
        handler,
        previous,
    }
}

/// The handler for `kind`, if one is registered.
#[must_use]
pub fn kind_handler(kind: &JobType) -> Option<Arc<dyn JobKindHandler>> {
    read().kinds.get(kind.as_str()).cloned()
}

/// Register the handler of system job `name`, replacing any earlier one.
pub(crate) fn register_system(name: &str, handler: SystemHandler) {
    write().systems.insert(name.to_string(), handler);
    tracing::debug!(job = %name, "[cron:handlers] system handler registered");
}

/// Remove system job `name`'s handler if it is still `handler`.
pub(crate) fn unregister_system(name: &str, handler: &SystemHandler) {
    let mut registry = write();
    if registry
        .systems
        .get(name)
        .is_some_and(|current| Arc::ptr_eq(current, handler))
    {
        registry.systems.remove(name);
        tracing::debug!(job = %name, "[cron:handlers] system handler unregistered");
    }
}

/// Whether system job `name` has a handler.
pub(crate) fn has_system(name: &str) -> bool {
    read().systems.contains_key(name)
}

/// The handler of system job `name`.
pub(crate) fn system_handler(name: &str) -> Option<SystemHandler> {
    read().systems.get(name).cloned()
}

#[cfg(test)]
#[path = "handlers_tests.rs"]
mod tests;
