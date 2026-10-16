//! Process-level installation: give a core booted by *any* host — the desktop
//! shell, the TUI, the CLI, a test — its TinyHumans backend connection.
//!
//! Hosts that build the core through [`crate::RuntimeBuilder`] do not need
//! this; it is for hosts that boot the core themselves
//! (`run_core_from_args`, `CoreBuilder`) and for test fixtures. The shared host
//! boot (`openhuman_rpc::host`) connects the backend on its own. Idempotent: calling it again re-installs an
//! equivalent transport and is harmless.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use openhuman_embed::__host::agent::tinyagents::discovery::install_tool_ranker;
use openhuman_embed::__host::core::all::register_controller_extension;
use openhuman_embed::seams::ControllerExtension;
use tinytools::ToolRanker;

use crate::backend::{set_product_identity, ProductIdentity};
use crate::transport::SdkBackendTransport;

/// What [`install`] sets up.
#[derive(Debug, Clone)]
pub struct InstallOptions {
    /// The `x-sdk-name` this process reports on every backend request. `None`
    /// keeps whatever identity is already set (the core's default is
    /// `"openhuman"`). Set it here, before the transport is built, because the
    /// transport captures the attribution headers once.
    pub product_identity: Option<ProductIdentity>,
    /// Register the hosted-backend RPC proxies (`billing`, `team`, `referral`,
    /// `announcements`) with the core's controller registry. Default `true`;
    /// a host whose `DomainSet` excludes `hosted` may leave it on — the group
    /// gate hides them — or turn it off to keep them out of the registry
    /// entirely.
    pub hosted_controllers: bool,
    /// Install the Jev-backed `tool_search` ranker (`crate::jev`) as the
    /// core's process-wide ranker. Default `true`; only meaningful with the
    /// `jev` feature, and only used when `agent.tool_search.ranker` lets it.
    /// Without it the harness ranks `tool_search` with BM25 alone.
    pub tool_ranker: bool,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            product_identity: None,
            hosted_controllers: true,
            tool_ranker: true,
        }
    }
}

impl InstallOptions {
    /// Report `identity` as this process's product on every backend request.
    pub fn product_identity(mut self, identity: ProductIdentity) -> Self {
        self.product_identity = Some(identity);
        self
    }

    /// Whether to register the hosted RPC proxies (default `true`).
    pub fn hosted_controllers(mut self, enabled: bool) -> Self {
        self.hosted_controllers = enabled;
        self
    }

    /// Whether to install the Jev `tool_search` ranker (default `true`).
    pub fn tool_ranker(mut self, enabled: bool) -> Self {
        self.tool_ranker = enabled;
        self
    }
}

/// Why [`install`] could not set the process up.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    /// The SDK transport's HTTP client could not be built.
    #[error("failed to build the TinyHumans backend transport: {0:#}")]
    Transport(anyhow::Error),
    /// The hosted controllers collided with the core's registry.
    #[error("failed to register the hosted RPC controllers: {0}")]
    Registry(String),
}

static INSTALLED: AtomicBool = AtomicBool::new(false);

/// What a TinyHumans connection contributes to a core, resolved from
/// [`InstallOptions`]. The one code path both entry points share:
/// [`install`] applies it to the process globals, and
/// [`RuntimeBuilder`](crate::RuntimeBuilder) hands it to the embed builder's
/// seam options (`controller_extension`, `tool_ranker`).
pub(crate) struct Wiring {
    /// The process-wide SDK transport ([`crate::client::shared_transport`]).
    /// Hosted callers fetch it themselves; kept so tests can pin that every
    /// connect shares one instance.
    #[cfg(test)]
    pub(crate) transport: Arc<SdkBackendTransport>,
    /// The hosted RPC proxies, unless [`InstallOptions::hosted_controllers`]
    /// is off.
    pub(crate) controllers: Option<ControllerExtension>,
    /// The Jev `tool_search` ranker, unless [`InstallOptions::tool_ranker`]
    /// is off or the `jev` feature is compiled out.
    pub(crate) ranker: Option<Arc<dyn ToolRanker>>,
}

impl std::fmt::Debug for Wiring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wiring")
            .field(
                "controllers",
                &self.controllers.as_ref().map(|ext| ext.group),
            )
            .field("ranker", &self.ranker.as_ref().map(|ranker| ranker.kind()))
            .finish_non_exhaustive()
    }
}

/// Set the product identity (if any) and resolve the seams `options` asks
/// for, without touching the controller registry or the ranker slot.
///
/// A new identity drops the cached transport, because a transport captures
/// its attribution headers once; [`connect_transport`] then rebuilds it.
fn prepare(options: &InstallOptions) -> (Option<ControllerExtension>, Option<Arc<dyn ToolRanker>>) {
    if let Some(identity) = options.product_identity.clone() {
        log::debug!(
            "[tinyhumans] install: product identity {}",
            identity.as_str()
        );
        set_product_identity(identity);
        // A new identity means new attribution headers: the shared transport
        // captured the old ones, so drop it and rebuild on connect.
        crate::client::reset_shared_transport();
    }

    // Plug every area into the core's generic registries (idempotent).
    crate::register::register_all(options);

    let controllers = options.hosted_controllers.then(crate::hosted::extension);

    #[cfg(feature = "jev")]
    let ranker = options.tool_ranker.then(|| {
        // The ranker resolves the credential per search, so nothing here
        // needs a login.
        Arc::new(crate::jev::TinyHumansJevRanker::new()) as Arc<dyn ToolRanker>
    });
    #[cfg(not(feature = "jev"))]
    let ranker = None;

    log::trace!(
        "[tinyhumans] install: prepared hosted_controllers={} tool_ranker={}",
        controllers.is_some(),
        ranker.is_some()
    );
    (controllers, ranker)
}

/// The process's SDK transport, built once and shared by every hosted caller.
fn connect_transport() -> Result<Arc<SdkBackendTransport>, InstallError> {
    let transport = crate::client::shared_transport().map_err(InstallError::Transport)?;
    if !INSTALLED.swap(true, Ordering::SeqCst) {
        log::info!("[tinyhumans] install: backend transport ready");
    }
    Ok(transport)
}

/// Resolve everything a TinyHumans connection wires in and connect the
/// transport. The builder's path; [`install`] runs the same two steps with
/// the registry and ranker applied in between, in its historical order.
pub(crate) fn wiring(options: &InstallOptions) -> Result<Wiring, InstallError> {
    let (controllers, ranker) = prepare(options);
    // Connecting builds the process transport; only tests read it back.
    #[cfg_attr(not(test), allow(unused_variables))]
    let transport = connect_transport()?;
    Ok(Wiring {
        #[cfg(test)]
        transport,
        controllers,
        ranker,
    })
}

/// Connect the TinyHumans layer for this process: set the product identity
/// (if any), plug every area into the core's generic registries and build the
/// shared SDK transport.
///
/// The compatibility entry point for hosts that boot the core themselves;
/// [`RuntimeBuilder`](crate::RuntimeBuilder) wires the same pieces through
/// the embed builder instead. Order: product identity, hosted controllers
/// (process registry), Jev ranker (process slot), transport.
///
/// The core holds no transport of its own; the returned handle is the one
/// every hosted caller in this crate sends through.
pub fn install(options: InstallOptions) -> Result<Arc<SdkBackendTransport>, InstallError> {
    let (controllers, ranker) = prepare(&options);

    if let Some(extension) = controllers {
        // Idempotent in the core: an identical re-registration is a no-op.
        register_controller_extension(extension).map_err(InstallError::Registry)?;
    }

    if let Some(ranker) = ranker {
        // Idempotent in the core: the slot is replaced in place.
        install_tool_ranker(ranker);
    }

    connect_transport()
}

/// Register the hooks a config *load* depends on, and nothing else.
///
/// A host that reads configuration before it connects (the CLI and TUI open
/// their storage-backed session store first) must call this before any
/// config read: the config enricher is what tells the load-time migrations
/// which endpoints are managed, and with none registered a legacy managed
/// config is mistaken for a custom one. Cheap and idempotent;
/// [`install`] and the builder's connect register the same hook again.
pub fn register_hooks() {
    log::trace!("[tinyhumans] register_hooks: config enricher");
    crate::client::register(&InstallOptions::default());
}

/// Whether [`install`] (or the builder's connect) has run in this process.
pub fn is_installed() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

#[cfg(test)]
#[path = "install_tests.rs"]
mod tests;
