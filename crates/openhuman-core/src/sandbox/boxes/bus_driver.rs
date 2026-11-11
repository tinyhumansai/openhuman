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
//! Each step is one bus call (`Reserve` + `Create`, `Exec`, `Close`). The
//! module's error names do not cross the bus client (it reports only
//! sanitized categories), so a failed exec is followed by an `Inspect`: a box
//! the module no longer knows, or one that can no longer take commands, is
//! [`BoxError::UnknownResource`], which the registry answers with a new box.

use std::time::Duration;

use async_trait::async_trait;

use super::{BoxBackend, BoxDriver, BoxError, BoxExec, BoxOutput, BoxRetention, BoxSettings};
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

/// A [`BoxDriver`] over the TinyBox module, using the module configuration it
/// was built with, for one backend.
pub struct BusBoxDriver {
    config: Config,
    backend: BoxBackend,
}

impl BusBoxDriver {
    /// A driver that reaches the module with `config`'s module settings and
    /// creates `backend`'s boxes.
    pub fn new(config: Config, backend: BoxBackend) -> Self {
        Self { config, backend }
    }
}

#[async_trait]
impl BoxDriver for BusBoxDriver {
    async fn create(
        &self,
        settings: &BoxSettings,
        retention: Option<&BoxRetention>,
    ) -> Result<String, BoxError> {
        let resource = tinybox::reserve(&self.config, contract::ReserveRequest::Resource)
            .await
            .map_err(BoxError::Driver)?;
        let request = create_request(resource, self.backend, settings, retention);
        let info = tinybox::create(&self.config, request)
            .await
            .map_err(BoxError::Driver)?;
        Ok(info.resource.0)
    }

    async fn exec(&self, resource: &str, exec: &BoxExec) -> Result<BoxOutput, BoxError> {
        match tinybox::exec_within(
            &self.config,
            exec_request(resource, exec),
            bus_deadline(exec.deadline),
        )
        .await
        {
            Ok(output) => Ok(box_output(output)),
            Err(error) => {
                let probe =
                    tinybox::inspect(&self.config, contract::ResourceId(resource.to_owned())).await;
                Err(exec_failure(error, probe))
            }
        }
    }

    async fn close(&self, resource: &str) -> Result<(), BoxError> {
        tinybox::close(&self.config, contract::ResourceId(resource.to_owned()))
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
    retention: Option<&BoxRetention>,
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
        retention: retention.map(|retention| contract::Retention {
            key: retention.key.clone(),
            idle_sleep_secs: retention.idle_sleep_secs,
            delete_after_secs: retention.delete_after_secs,
        }),
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
