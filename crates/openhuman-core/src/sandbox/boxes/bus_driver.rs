//! [`BusBoxDriver`]: boxes through the TinyBox module's `openshell` or
//! `agent37` backend.
//!
//! A retained box (agent37) is created with the contract 2.4
//! [`contract::Retention`]: the module adopts the box it already keeps under
//! the owner's key, or creates one that sleeps when idle. A module older than
//! 2.4 never sees the request: every call first requires a compatible
//! contract, and this host is built against 2.4. Forgetting a retained box is
//! the module's `Close`, which for a retained box drops only the module's
//! handle and leaves the instance, so the module's resource slots are freed
//! without deleting anything.
//!
//! Each call reaches the module with the configuration of the host call it
//! serves ([`BusBoxDriver::serve`]), not of whichever call built the
//! process's registry. A box's close or forget reaches the module with the
//! configuration it was created under, so a later call cannot redirect the
//! cleanup of an earlier box; work outside any call (the idle reaper) uses the
//! most recent module-enabled call's.
//!
//! A reservation is closed if the create it was for is cancelled before the
//! module answers ([`AbandonGuard`]), so a cancelled first command never
//! leaves a module resource slot (or a box) that nothing tracks.
//!
//! Each step is one bus call (`Reserve` + `Create`, `Exec`, `Close`). The
//! module's error names do not cross the bus client (it reports only
//! sanitized categories), so a failed exec is followed by an `Inspect`: a box
//! the module no longer knows, or one that can no longer take commands, is
//! [`BoxError::UnknownResource`], which the registry answers with a new box.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use async_trait::async_trait;

use super::{BoxBackend, BoxDriver, BoxError, BoxExec, BoxOutput, BoxSettings, Retention};
use crate::config::Config;
use crate::modules::tinybox::{self, contract};

/// The TinyBox backend name for OpenShell boxes.
pub(crate) const OPENSHELL_BACKEND: &str = "openshell";

/// The TinyBox backend name for agent37 boxes.
pub(crate) const AGENT37_BACKEND: &str = "agent37";

/// The TinyBox backend name `backend` creates its boxes with.
pub(crate) fn module_backend(backend: BoxBackend) -> &'static str {
    match backend {
        BoxBackend::OpenShell => OPENSHELL_BACKEND,
        BoxBackend::Agent37 => AGENT37_BACKEND,
    }
}

/// How long past the registry's own deadline the bus waits for an exec reply,
/// so the registry's backstop always decides first.
pub(crate) const BUS_REPLY_GRACE: Duration = Duration::from_secs(5);

/// Box states that still take commands.
const LIVE_STATES: &[&str] = &["ready", "running"];

tokio::task_local! {
    /// The configuration of the host call a driver step runs for.
    static CALL_CONFIG: Arc<Config>;
}

/// A [`BoxDriver`] over the TinyBox module, for one backend, reaching the
/// module with the configuration of the call it serves.
pub struct BusBoxDriver {
    /// The configuration of the most recent call that has modules enabled,
    /// for work outside any call; the initial configuration until such a call.
    latest: RwLock<Arc<Config>>,
    /// The configuration each box was created under, by resource id. Its
    /// cleanup reaches the module with that configuration, not with whichever
    /// call replaced `latest` since.
    created_with: Mutex<HashMap<String, Arc<Config>>>,
    backend: BoxBackend,
}

impl BusBoxDriver {
    /// A driver that creates `backend`'s boxes, reaching the module with
    /// `config` until a call brings its own.
    pub fn new(config: Config, backend: BoxBackend) -> Self {
        Self {
            latest: RwLock::new(Arc::new(config)),
            created_with: Mutex::new(HashMap::new()),
            backend,
        }
    }

    /// Record that `resource` was created under `config`, the configuration
    /// its create was made with.
    fn remember(&self, resource: &str, config: &Arc<Config>) {
        self.created_with
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(resource.to_owned(), Arc::clone(config));
    }

    /// The configuration a step on `resource` reaches the module with: the
    /// one it was created under, or the current one for a box not recorded.
    fn config_of(&self, resource: &str) -> Arc<Config> {
        self.created_with
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(resource)
            .cloned()
            .unwrap_or_else(|| self.config())
    }

    /// Run `call` with every step of this driver inside it reaching the
    /// module with `config`, which also becomes the configuration for work
    /// outside any call.
    pub fn serve<F: Future>(&self, config: &Config, call: F) -> impl Future<Output = F::Output> {
        let config = Arc::new(config.clone());
        // A call that has the module switched off must not become the
        // configuration for later cleanups: they would be refused before they
        // reach TinyBox, and the box would be left untracked.
        if config.modules.enabled {
            *self.latest.write().unwrap_or_else(PoisonError::into_inner) = Arc::clone(&config);
        }
        CALL_CONFIG.scope(config, call)
    }

    /// The configuration for a step now: the serving call's, or the most
    /// recent call's outside one.
    pub(crate) fn config(&self) -> Arc<Config> {
        CALL_CONFIG.try_with(Arc::clone).unwrap_or_else(|_| {
            Arc::clone(&self.latest.read().unwrap_or_else(PoisonError::into_inner))
        })
    }
}

/// Holds `value` for a step that may be cancelled: dropped while armed, it
/// hands the value to `abandon`; [`Self::keep`] disarms it.
pub(crate) struct AbandonGuard<T, F: FnOnce(T)> {
    armed: Option<(T, F)>,
}

impl<T, F: FnOnce(T)> AbandonGuard<T, F> {
    pub(crate) fn new(value: T, abandon: F) -> Self {
        Self {
            armed: Some((value, abandon)),
        }
    }

    /// The step finished: abandon nothing.
    pub(crate) fn keep(mut self) {
        self.armed = None;
    }
}

impl<T, F: FnOnce(T)> Drop for AbandonGuard<T, F> {
    fn drop(&mut self) {
        if let Some((value, abandon)) = self.armed.take() {
            abandon(value);
        }
    }
}

/// Close a reservation (or the box created on it) whose create was
/// cancelled, on a detached task, since `Drop` cannot await. Closing a
/// retained box only drops the module's handle and keeps the instance.
fn close_abandoned(config: Arc<Config>, resource: contract::ResourceId) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    tracing::debug!("[sandbox:box] create cancelled; closing its reservation");
    // Cleanup of a module id this host no longer tracks; it acts under no
    // tenant's context, so it carries none.
    runtime.spawn(async move {
        if let Err(error) = tinybox::close(&config, resource).await {
            tracing::warn!(%error, "[sandbox:box] closing a cancelled reservation failed");
        }
    });
}

/// Settle a create's answer against its reservation. A created box is handed
/// to the registry and the reservation disarmed. A failed create keeps the
/// reservation armed, so it is closed on drop: the module may have kept the
/// allocation even though the create reported an error, and the registry holds
/// no id for it to clean up later.
pub(crate) fn settle_create<T, F: FnOnce(T)>(
    reservation: AbandonGuard<T, F>,
    created: Result<contract::ResourceInfo, String>,
) -> Result<String, BoxError> {
    match created {
        Ok(info) => {
            reservation.keep();
            Ok(info.resource.0)
        }
        Err(error) => {
            drop(reservation);
            Err(BoxError::Driver(error))
        }
    }
}

#[async_trait]
impl BoxDriver for BusBoxDriver {
    async fn create(
        &self,
        settings: &BoxSettings,
        retention: Option<&Retention>,
    ) -> Result<String, BoxError> {
        let config = self.config();
        let resource = tinybox::reserve(&config, contract::ReserveRequest::Resource)
            .await
            .map_err(BoxError::Driver)?;
        // Until Create answers, the reservation is only ours: close it if this
        // future is dropped. Once Create answers, the module has either
        // created the box (handed to the registry below) or torn a failed one
        // down itself.
        let reservation = AbandonGuard::new(resource.clone(), {
            let config = Arc::clone(&config);
            move |resource| close_abandoned(config, resource)
        });
        let request = create_request(resource, self.backend, settings, retention);
        let created = tinybox::create(&config, request).await;
        let resource = settle_create(reservation, created)?;
        self.remember(&resource, &config);
        Ok(resource)
    }

    async fn exec(&self, resource: &str, exec: &BoxExec) -> Result<BoxOutput, BoxError> {
        let config = self.config();
        match tinybox::exec_within(
            &config,
            exec_request(resource, exec),
            bus_deadline(exec.deadline),
        )
        .await
        {
            Ok(output) => Ok(box_output(output)),
            Err(error) => {
                let probe =
                    tinybox::inspect(&config, contract::ResourceId(resource.to_owned())).await;
                Err(exec_failure(error, probe))
            }
        }
    }

    async fn close(&self, resource: &str) -> Result<(), BoxError> {
        let config = self.config_of(resource);
        let outcome = tinybox::close(&config, contract::ResourceId(resource.to_owned()))
            .await
            .map_err(BoxError::Driver);
        // The box is no longer ours to track once closed or forgotten.
        self.created_with
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(resource);
        outcome
    }

    async fn forget(&self, resource: &str) -> Result<(), BoxError> {
        // The module's Close on a retained box drops its handle and keeps the
        // instance (contract 2.4); see the module docs above.
        self.close(resource).await
    }
}

/// A box on `backend`: the image (a template name on agent37), no network,
/// no published ports, no environment from the host, and the retention a
/// retaining registry asked for.
pub(crate) fn create_request(
    resource: contract::ResourceId,
    backend: BoxBackend,
    settings: &BoxSettings,
    retention: Option<&Retention>,
) -> contract::CreateRequest {
    contract::CreateRequest {
        resource,
        backend: module_backend(backend).to_owned(),
        host: contract::HostConfig::Local,
        workspace: contract::Workspace::Image(settings.image.clone()),
        network: contract::NetworkPolicy::Denied,
        resources: contract::ResourceLimits {
            cpu_millis: settings.cpu_millis,
            memory_bytes: settings.memory_bytes,
            ..contract::ResourceLimits::default()
        },
        ports: Vec::new(),
        env: std::collections::BTreeMap::new(),
        retention: retention.cloned(),
    }
}

pub(crate) fn exec_request(resource: &str, exec: &BoxExec) -> contract::ExecRequest {
    contract::ExecRequest {
        resource: contract::ResourceId(resource.to_owned()),
        argv: exec.argv.clone(),
        cwd: Some(exec.cwd.clone()),
        env: exec.env.clone(),
        stdin: None,
    }
}

pub(crate) fn bus_deadline(registry_deadline: Duration) -> Duration {
    registry_deadline.saturating_add(BUS_REPLY_GRACE)
}

/// Classify a failed exec by what `Inspect` says about its box afterwards.
pub(crate) fn exec_failure(
    exec_error: String,
    probe: Result<contract::ResourceInfo, String>,
) -> BoxError {
    let gone = match probe {
        Ok(info) => !LIVE_STATES.contains(&info.state.as_str()),
        // The module answered and refused: it holds no such box.
        Err(probe_error) => {
            probe_error == tinybus::module::ModuleCallError::ModuleFault.to_string()
        }
    };
    if gone {
        BoxError::UnknownResource
    } else {
        BoxError::Driver(exec_error)
    }
}

pub(crate) fn box_output(output: contract::ExecOutput) -> BoxOutput {
    BoxOutput {
        exit_code: output.exit_code,
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

#[cfg(test)]
#[path = "bus_driver_tests.rs"]
mod tests;
