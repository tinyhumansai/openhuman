//! The OpenShell sandbox backend: one long-lived box per tenant.
//!
//! With `[security.sandbox] backend = "openshell"`, a sandboxed shell command
//! runs in a box the TinyBox module creates on an NVIDIA OpenShell gateway
//! (`[runtime.openshell]`). Unlike the Docker backend, which starts a
//! container per command, a box outlives the command: [`BoxRegistry`] keeps
//! one per [`Tenant`], so files a command writes are there for the next one.
//!
//! - **Keyed on the whole tenant.** An embedded host runs one agent per end
//!   user with no profile, so keying on the profile alone would put every user
//!   in one box. A call with no tenant ([`NoTenant`]) is refused, never sent
//!   to a shared box.
//! - **Created once, reused, reaped.** Concurrent first commands make one box.
//!   A box idle for `idle_ttl_secs` is closed by a background reaper, and
//!   `max_live` caps the boxes alive at once.
//! - **The box enforces the deadline.** Commands run as
//!   `timeout -k 5 <secs> sh -c <command>` in `/sandbox`, with only the
//!   variables the request carries. A box that does not answer shortly after
//!   the deadline is closed and the command reported as timed out.
//!
//! [`BoxDriver`] is the seam: [`BusBoxDriver`] reaches the TinyBox module
//! (behind the `modules` feature); tests use a scripted stand-in. The process
//! holds one registry, built from the config of the first command that needs
//! it, so `idle_ttl_secs` and `max_live` changes take effect on restart.

#[cfg(feature = "modules")]
mod bus_driver;
mod registry;
mod types;

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::time::Duration;

#[cfg(feature = "modules")]
pub use bus_driver::BusBoxDriver;
pub use registry::BoxRegistry;
pub use types::{BoxDriver, BoxError, BoxExec, BoxOutput, BoxRequest, BoxSettings, RegistryLimits};

use super::types::SandboxExecResult;
use crate::config::{Config, OpenShellRuntimeConfig};
use crate::core::runtime::{DispatchCtx, NoTenant, Tenant};

/// The tenant a box is kept for: the call's whole tenant, or a refusal.
///
/// # Errors
///
/// [`BoxError::NoTenant`] when the call carries no tenant.
pub fn tenant_for_box(tenant: Result<Tenant, NoTenant>) -> Result<Tenant, BoxError> {
    tenant.map_err(|NoTenant| {
        tracing::warn!("[sandbox:openshell] refusing a command with no tenant scope");
        BoxError::NoTenant
    })
}

/// The command's variables as UTF-8, refusing any that are not.
///
/// # Errors
///
/// [`BoxError::NonUtf8Env`] when a name or value is not UTF-8.
pub fn box_env(
    extra_env: &HashMap<OsString, OsString>,
) -> Result<BTreeMap<String, String>, BoxError> {
    extra_env
        .iter()
        .map(|(name, value)| match (name.to_str(), value.to_str()) {
            (Some(name), Some(value)) => Ok((name.to_owned(), value.to_owned())),
            _ => Err(BoxError::NonUtf8Env),
        })
        .collect()
}

/// Run `command` in the box of the tenant `tenant` resolves to.
///
/// # Errors
///
/// [`BoxError::NoTenant`] before any box is touched when there is no tenant;
/// otherwise the settings, environment and registry errors.
pub async fn run_command<D: BoxDriver>(
    registry: &BoxRegistry<D>,
    tenant: Result<Tenant, NoTenant>,
    openshell: &OpenShellRuntimeConfig,
    command: &str,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
) -> Result<SandboxExecResult, BoxError> {
    let tenant = tenant_for_box(tenant)?;
    let settings = BoxSettings::from_config(openshell)?;
    let request = BoxRequest {
        command: command.to_owned(),
        env: box_env(extra_env)?,
        timeout,
    };
    registry.exec(&tenant, &settings, &request).await
}

#[cfg(feature = "modules")]
static REGISTRY: std::sync::OnceLock<std::sync::Arc<BoxRegistry<BusBoxDriver>>> =
    std::sync::OnceLock::new();

/// The process's registry, built (and its reaper started) on first use.
#[cfg(feature = "modules")]
fn registry(config: &Config) -> &'static std::sync::Arc<BoxRegistry<BusBoxDriver>> {
    let registry = REGISTRY.get_or_init(|| {
        BoxRegistry::new(
            std::sync::Arc::new(BusBoxDriver::new(config.clone())),
            RegistryLimits::from_config(&config.runtime.openshell),
        )
    });
    registry.start_reaper();
    registry
}

/// Run a sandboxed shell command in the calling tenant's OpenShell box.
///
/// # Errors
///
/// Every [`BoxError`], and a refusal when this build has no module support.
pub(crate) async fn execute(
    ctx: &DispatchCtx,
    config: &Config,
    command: &str,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
) -> anyhow::Result<SandboxExecResult> {
    #[cfg(feature = "modules")]
    {
        tracing::debug!(
            timeout_secs = timeout.as_secs(),
            "[sandbox:openshell] running a command"
        );
        run_command(
            registry(config),
            ctx.tenant(),
            &config.runtime.openshell,
            command,
            extra_env,
            timeout,
        )
        .await
        .map_err(anyhow::Error::from)
    }
    #[cfg(not(feature = "modules"))]
    {
        let _ = (ctx, config, command, extra_env, timeout);
        anyhow::bail!(
            "TinyBox module support is disabled; OpenShell sandbox execution is unavailable"
        )
    }
}

/// Close `tenant`'s box, if the process has one.
pub async fn release(tenant: &Tenant) {
    #[cfg(feature = "modules")]
    if let Some(registry) = REGISTRY.get() {
        registry.release(tenant).await;
    }
    #[cfg(not(feature = "modules"))]
    let _ = tenant;
}

/// Close every box the process holds.
pub async fn shutdown() {
    #[cfg(feature = "modules")]
    if let Some(registry) = REGISTRY.get() {
        registry.shutdown().await;
    }
}

#[cfg(test)]
#[path = "scripted_driver_fixture.rs"]
pub(crate) mod scripted_driver;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
