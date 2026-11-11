//! Boxes kept per tenant: the OpenShell and agent37 sandbox backends.
//!
//! With `[security.sandbox] backend = "openshell"` or `"agent37"`, a
//! sandboxed shell command runs in a box the TinyBox module creates, on an
//! NVIDIA OpenShell gateway (`[runtime.openshell]`) or on the hosted agent37
//! service (`[runtime.agent37]`). Unlike the Docker backend, which starts a
//! container per command, a box outlives the command: [`BoxRegistry`] keeps
//! one per [`Tenant`], so files a command writes are there for the next one.
//! [`BoxBackend`] names the two and what differs between them.
//!
//! - **Keyed on the whole tenant, within its scope.** An embedded host runs
//!   one agent per end user with no profile, so keying on the profile alone
//!   would put every user in one box. A call with no tenant ([`NoTenant`]) is
//!   refused, never sent to a shared box. Boxes are also kept per
//!   [`OwnerScope`]: the active workspace (which is per signed-in user) and,
//!   on agent37, `[runtime.agent37] retention_namespace`, hashed. Equal
//!   tenants of two users or two hosts never share a box. Two deployments
//!   share a box only when they share an agent37 key, tenant and workspace
//!   path *and* leave `retention_namespace` blank; distinct nonblank
//!   namespaces keep them apart.
//! - **Created once, reused, let go of.** Concurrent first commands make one
//!   box. A box idle for the registry's TTL is let go of by a background
//!   reaper, and `max_live` caps the boxes held at once.
//! - **OpenShell closes, agent37 retains.** An OpenShell box let go of is
//!   closed and its files are gone. An agent37 box is created with the
//!   scoped tenant's [`owner_key`] and a [`RetainPolicy`] (box contract 2.4): letting
//!   go only forgets it, the service sleeps it when idle, and the next command
//!   (from this process or after a restart) finds the same box, files
//!   included, until it has slept for `delete_after_secs`.
//! - **The box enforces the deadline.** Commands run as
//!   `timeout -k 5 <secs> sh -c <command>` in the backend's work folder
//!   ([`BoxBackend::workdir`]), with only the variables the request carries.
//!   A box that does not answer shortly after the deadline is let go of and
//!   the command reported as timed out. A command whose caller gives up is
//!   stopped the same way: its box is let go of.
//! - **Files cross over commands too.** [`write_file`] and [`read_file`] move a
//!   file between the host and a tenant's box with plain `exec`s (base64 in
//!   chunks), relative to the work folder, on whichever backend is configured.
//!
//! [`BoxDriver`] is the seam: [`BusBoxDriver`] reaches the TinyBox module
//! (behind the `modules` feature); tests use a scripted stand-in. The process
//! holds one registry per backend, whose TTL, `max_live` and retention come
//! from the config of the first call that needs it, so changes to them take
//! effect on restart. Each call still reaches the module with its own
//! configuration ([`BusBoxDriver::serve`]).

#[cfg(feature = "modules")]
mod bus_driver;
mod cleanup;
mod files;
mod registry;
mod types;

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::time::Duration;

#[cfg(feature = "modules")]
pub use bus_driver::BusBoxDriver;
pub use files::{read_file_in, write_file_in, MAX_FILE_BYTES};
pub use registry::{owner_key, BoxRegistry};
pub use types::{
    BoxDriver, BoxError, BoxExec, BoxOutput, BoxRequest, BoxSettings, OwnerScope, RegistryLimits,
    RetainPolicy, Retention, AGENT37_WORKDIR, MAX_IDLE_SLEEP_SECS, MIN_IDLE_SLEEP_SECS,
    OPENSHELL_WORKDIR,
};

use super::types::{SandboxBackendKind, SandboxExecResult};
use crate::config::Config;
use crate::core::runtime::{DispatchCtx, NoTenant, Tenant};

/// A sandbox backend that keeps a box per tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoxBackend {
    /// An NVIDIA OpenShell gateway; boxes are closed when let go of.
    OpenShell,
    /// The hosted agent37 service; boxes are retained for their tenant.
    Agent37,
}

impl BoxBackend {
    /// The box-keeping backend `kind` names, if it is one.
    pub fn of(kind: SandboxBackendKind) -> Option<Self> {
        match kind {
            SandboxBackendKind::OpenShell => Some(Self::OpenShell),
            SandboxBackendKind::Agent37 => Some(Self::Agent37),
            SandboxBackendKind::None | SandboxBackendKind::Local | SandboxBackendKind::Docker => {
                None
            }
        }
    }

    /// The box-keeping backend `config` names, unless the config turns the
    /// sandbox off.
    pub fn configured(config: &Config) -> Option<Self> {
        if crate::sandbox::ops::config_disables_sandbox(config) {
            return None;
        }
        Self::of(crate::sandbox::policy::named_backend(
            &config.sandbox,
            &config.runtime,
            false,
        ))
    }

    /// The folder commands run in and file paths are relative to.
    pub fn workdir(self) -> &'static str {
        match self {
            Self::OpenShell => OPENSHELL_WORKDIR,
            Self::Agent37 => AGENT37_WORKDIR,
        }
    }

    /// What a new box is created with, from the backend's runtime table, in
    /// the scope of `config`'s workspace ([`Self::owner_scope`]).
    ///
    /// # Errors
    ///
    /// [`BoxError::InvalidConfig`] and [`BoxError::InvalidMemory`] for an
    /// unusable table.
    pub fn settings(self, config: &Config) -> Result<BoxSettings, BoxError> {
        let settings = match self {
            Self::OpenShell => BoxSettings::from_config(&config.runtime.openshell),
            Self::Agent37 => BoxSettings::from_agent37(&config.runtime.agent37),
        }?;
        Ok(BoxSettings {
            owner_scope: self.owner_scope(config),
            ..settings
        })
    }

    /// The scope a call's boxes belong to: the active workspace, which is
    /// per signed-in user, and on agent37 the deployment's
    /// `[runtime.agent37] retention_namespace`, so hosts and deployments
    /// sharing one agent37 key keep apart. OpenShell boxes are never
    /// retained, so its scope is the workspace alone.
    pub fn owner_scope(self, config: &Config) -> OwnerScope {
        let namespace = match self {
            Self::OpenShell => "",
            Self::Agent37 => config.runtime.agent37.retention_namespace.trim(),
        };
        OwnerScope::new(namespace, &config.workspace_dir)
    }

    /// The registry bounds, and retention, from the backend's runtime table.
    pub fn limits(self, config: &Config) -> RegistryLimits {
        match self {
            Self::OpenShell => RegistryLimits::from_config(&config.runtime.openshell),
            Self::Agent37 => RegistryLimits::from_agent37(&config.runtime.agent37),
        }
    }

    /// Refuse a backend the config does not allow: agent37 sends commands,
    /// their environment and files to a hosted service, which
    /// `[privacy] mode = "local_only"` forbids. This holds whether or not
    /// the security module is on.
    ///
    /// # Errors
    ///
    /// [`BoxError::LocalOnly`] for agent37 under local-only privacy.
    pub fn check_allowed(self, config: &Config) -> Result<(), BoxError> {
        if self == Self::Agent37 && config.privacy.mode == crate::config::PrivacyMode::LocalOnly {
            tracing::warn!("[sandbox:box] refusing agent37 under local-only privacy");
            return Err(BoxError::LocalOnly);
        }
        Ok(())
    }

    fn label(self) -> &'static str {
        match self {
            Self::OpenShell => "OpenShell",
            Self::Agent37 => "agent37",
        }
    }
}

/// The tenant a box is kept for: the call's whole tenant, or a refusal.
///
/// # Errors
///
/// [`BoxError::NoTenant`] when the call carries no tenant.
pub fn tenant_for_box(tenant: Result<Tenant, NoTenant>) -> Result<Tenant, BoxError> {
    tenant.map_err(|NoTenant| {
        tracing::warn!("[sandbox:box] refusing a box call with no tenant scope");
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
/// otherwise the environment and registry errors.
pub async fn run_command<D: BoxDriver>(
    registry: &BoxRegistry<D>,
    tenant: Result<Tenant, NoTenant>,
    settings: &BoxSettings,
    command: &str,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
) -> Result<SandboxExecResult, BoxError> {
    let tenant = tenant_for_box(tenant)?;
    let request = BoxRequest {
        command: command.to_owned(),
        env: box_env(extra_env)?,
        timeout,
    };
    registry.exec(&tenant, settings, &request).await
}

#[cfg(feature = "modules")]
type ProcessRegistry = std::sync::Arc<BoxRegistry<BusBoxDriver>>;

#[cfg(feature = "modules")]
static OPENSHELL_REGISTRY: std::sync::OnceLock<ProcessRegistry> = std::sync::OnceLock::new();

#[cfg(feature = "modules")]
static AGENT37_REGISTRY: std::sync::OnceLock<ProcessRegistry> = std::sync::OnceLock::new();

#[cfg(feature = "modules")]
fn registry_cell(backend: BoxBackend) -> &'static std::sync::OnceLock<ProcessRegistry> {
    match backend {
        BoxBackend::OpenShell => &OPENSHELL_REGISTRY,
        BoxBackend::Agent37 => &AGENT37_REGISTRY,
    }
}

/// The process's registry for `backend`, built (and its reaper started) on
/// first use.
#[cfg(feature = "modules")]
fn registry(backend: BoxBackend, config: &Config) -> &'static ProcessRegistry {
    let registry = registry_cell(backend).get_or_init(|| {
        BoxRegistry::new(
            std::sync::Arc::new(BusBoxDriver::new(config.clone(), backend)),
            backend.limits(config),
        )
    });
    registry.start_reaper();
    registry
}

/// Run a sandboxed shell command in the calling tenant's box on `backend`.
///
/// # Errors
///
/// Every [`BoxError`], and a refusal when this build has no module support.
pub(crate) async fn execute(
    ctx: &DispatchCtx,
    config: &Config,
    backend: BoxBackend,
    command: &str,
    extra_env: &HashMap<OsString, OsString>,
    timeout: Duration,
) -> anyhow::Result<SandboxExecResult> {
    backend.check_allowed(config)?;
    #[cfg(feature = "modules")]
    {
        tracing::debug!(
            backend = backend.label(),
            timeout_secs = timeout.as_secs(),
            "[sandbox:box] running a command"
        );
        let tenant = tenant_for_box(ctx.tenant())?;
        let settings = backend.settings(config)?;
        let registry = registry(backend, config);
        registry
            .driver()
            .serve(
                config,
                run_command(registry, Ok(tenant), &settings, command, extra_env, timeout),
            )
            .await
            .map_err(anyhow::Error::from)
    }
    #[cfg(not(feature = "modules"))]
    {
        let _ = (ctx, command, extra_env, timeout);
        anyhow::bail!(
            "TinyBox module support is disabled; {} sandbox execution is unavailable",
            backend.label()
        )
    }
}

/// The box-keeping backend for a file call: the configured one, if it keeps
/// boxes and is allowed.
fn file_backend(config: &Config) -> Result<BoxBackend, BoxError> {
    let backend = BoxBackend::configured(config).ok_or(BoxError::NoBoxBackend)?;
    backend.check_allowed(config)?;
    Ok(backend)
}

/// Write `bytes` to `path` (relative to the box's work folder) in the box of
/// `tenant`, replacing any file there and creating its folders. The box is
/// on the backend `config` names and comes from the process's registry for
/// it, built from `config` on first use.
///
/// # Errors
///
/// [`BoxError::NoTenant`] first; [`BoxError::NoBoxBackend`] when the
/// configured backend keeps no boxes; [`BoxError::LocalOnly`] for agent37
/// under local-only privacy; every [`BoxError`] [`write_file_in`] returns;
/// and a refusal when this build has no module support.
pub async fn write_file(
    tenant: Result<Tenant, NoTenant>,
    config: &Config,
    path: &str,
    bytes: &[u8],
) -> Result<(), BoxError> {
    let tenant: Result<Tenant, NoTenant> = Ok(tenant_for_box(tenant)?);
    let backend = file_backend(config)?;
    #[cfg(feature = "modules")]
    {
        let settings = backend.settings(config)?;
        let registry = registry(backend, config);
        registry
            .driver()
            .serve(
                config,
                write_file_in(registry, tenant, &settings, path, bytes),
            )
            .await
    }
    #[cfg(not(feature = "modules"))]
    {
        let _ = (tenant, backend, path, bytes);
        Err(modules_disabled())
    }
}

/// Read the file at `path` (relative to the box's work folder) from the box of
/// `tenant`. A file over `max_bytes`, or over [`MAX_FILE_BYTES`], is refused
/// before any of it is transferred. The box is on the backend `config` names
/// and comes from the process's registry for it, built from `config` on
/// first use.
///
/// # Errors
///
/// As [`write_file`], with [`read_file_in`]'s errors.
pub async fn read_file(
    tenant: Result<Tenant, NoTenant>,
    config: &Config,
    path: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, BoxError> {
    let tenant: Result<Tenant, NoTenant> = Ok(tenant_for_box(tenant)?);
    let backend = file_backend(config)?;
    #[cfg(feature = "modules")]
    {
        let settings = backend.settings(config)?;
        let registry = registry(backend, config);
        registry
            .driver()
            .serve(
                config,
                read_file_in(registry, tenant, &settings, path, max_bytes),
            )
            .await
    }
    #[cfg(not(feature = "modules"))]
    {
        let _ = (tenant, backend, path, max_bytes);
        Err(modules_disabled())
    }
}

#[cfg(not(feature = "modules"))]
fn modules_disabled() -> BoxError {
    BoxError::Driver(
        "TinyBox module support is disabled; sandbox box files are unavailable".to_owned(),
    )
}

/// Let go of `tenant`'s boxes, in every scope, on every backend the process
/// holds boxes on: an OpenShell box is closed, an agent37 box only forgotten
/// (it sleeps).
pub async fn release(tenant: &Tenant) {
    #[cfg(feature = "modules")]
    for cell in [&OPENSHELL_REGISTRY, &AGENT37_REGISTRY] {
        if let Some(registry) = cell.get() {
            registry.release(tenant).await;
        }
    }
    #[cfg(not(feature = "modules"))]
    let _ = tenant;
}

/// Let go of every box the process holds: OpenShell boxes are closed, agent37
/// boxes only forgotten, so they are still there after a restart.
pub async fn shutdown() {
    #[cfg(feature = "modules")]
    for cell in [&OPENSHELL_REGISTRY, &AGENT37_REGISTRY] {
        if let Some(registry) = cell.get() {
            registry.shutdown().await;
        }
    }
}

#[cfg(test)]
#[path = "scripted_driver_fixture.rs"]
pub(crate) mod scripted_driver;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
