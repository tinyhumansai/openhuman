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
//! serves (carried in [`BoxSettings::module`] and [`BoxExec::module`]), not of whichever call built the
//! process's registry; work outside any call (the idle reaper, cleanups of
//! cancelled commands) uses the most recent call's.
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

use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use async_trait::async_trait;

use super::{
    BoxBackend, BoxDriver, BoxError, BoxExec, BoxOutput, BoxSettings, ModuleConfig, Retention,
};
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

/// A [`BoxDriver`] over the TinyBox module, for one backend, reaching the
/// module with the configuration of the call each step belongs to.
pub struct BusBoxDriver {
    /// The most recent call's configuration, for work outside any call.
    latest: RwLock<Arc<Config>>,
    backend: BoxBackend,
}

impl BusBoxDriver {
    /// A driver that creates `backend`'s boxes, reaching the module with
    /// `config` until a call brings its own.
    pub fn new(config: Config, backend: BoxBackend) -> Self {
        Self {
            latest: RwLock::new(Arc::new(config)),
            backend,
        }
    }

    /// The configuration for a step now: the call's own, which also becomes
    /// the configuration for work outside any call, or the most recent
    /// call's when the step brought none.
    pub(crate) fn config(&self, module: &ModuleConfig) -> Arc<Config> {
        match module.get() {
            Some(config) => {
                *self.latest.write().unwrap_or_else(PoisonError::into_inner) = Arc::clone(config);
                Arc::clone(config)
            }
            None => Arc::clone(&self.latest.read().unwrap_or_else(PoisonError::into_inner)),
        }
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

#[async_trait]
impl BoxDriver for BusBoxDriver {
    async fn create(
        &self,
        settings: &BoxSettings,
        retention: Option<&Retention>,
    ) -> Result<String, BoxError> {
        let config = self.config(&settings.module);
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
        reservation.keep();
        let info = created.map_err(BoxError::Driver)?;
        Ok(info.resource.0)
    }

    async fn exec(&self, resource: &str, exec: &BoxExec) -> Result<BoxOutput, BoxError> {
        let config = self.config(&exec.module);
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
        tinybox::close(
            &self.config(&ModuleConfig::default()),
            contract::ResourceId(resource.to_owned()),
        )
        .await
        .map_err(BoxError::Driver)
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
