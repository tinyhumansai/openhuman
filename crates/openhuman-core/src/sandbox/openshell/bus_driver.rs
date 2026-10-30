//! [`BusBoxDriver`]: boxes through the TinyBox module's `openshell` backend.
//!
//! Each step is one bus call (`Reserve` + `Create`, `Exec`, `Close`). The
//! module's error names do not cross the bus client (it reports only
//! sanitized categories), so a failed exec is followed by an `Inspect`: a box
//! the module no longer knows, or one that can no longer take commands, is
//! [`BoxError::UnknownResource`], which the registry answers with a new box.

use std::time::Duration;

use async_trait::async_trait;

use super::{BoxDriver, BoxError, BoxExec, BoxOutput, BoxSettings};
use crate::config::Config;
use crate::modules::tinybox::{self, contract};

/// The TinyBox backend name for OpenShell boxes.
pub(crate) const BACKEND: &str = "openshell";

/// How long past the registry's own deadline the bus waits for an exec reply,
/// so the registry's backstop always decides first.
pub(crate) const BUS_REPLY_GRACE: Duration = Duration::from_secs(5);

/// Box states that still take commands.
const LIVE_STATES: &[&str] = &["ready", "running"];

/// A [`BoxDriver`] over the TinyBox module, using the module configuration it
/// was built with.
pub struct BusBoxDriver {
    config: Config,
}

impl BusBoxDriver {
    /// A driver that reaches the module with `config`'s module settings.
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

#[async_trait]
impl BoxDriver for BusBoxDriver {
    async fn create(&self, settings: &BoxSettings) -> Result<String, BoxError> {
        let resource = tinybox::reserve(&self.config, contract::ReserveRequest::Resource)
            .await
            .map_err(BoxError::Driver)?;
        let info = tinybox::create(&self.config, create_request(resource, settings))
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
}

/// An OpenShell box: the image, no network, no published ports, and no
/// environment from the host.
pub(crate) fn create_request(
    resource: contract::ResourceId,
    settings: &BoxSettings,
) -> contract::CreateRequest {
    contract::CreateRequest {
        resource,
        backend: BACKEND.to_owned(),
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
