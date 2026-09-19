//! Process-level installation: give a core booted by *any* host — the desktop
//! shell, the TUI, the CLI, a test — its TinyHumans backend connection.
//!
//! Hosts that build the core through [`crate::RuntimeBuilder`] do not need
//! this; it is for hosts that boot the core themselves
//! (`run_server_embedded_with_ready`, `run_core_from_args`, `CoreBuilder`)
//! and for test fixtures. Idempotent: calling it again re-installs an
//! equivalent transport and is harmless.

use std::sync::{Arc, Mutex, OnceLock};

use openhuman_core::api::transport::{install_backend_transport, installed_backend_transport};
use openhuman_core::api::{set_product_identity, ProductIdentity};
use openhuman_core::core::all::register_controller_extension;

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
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            product_identity: None,
            hosted_controllers: true,
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

static INSTALLED: OnceLock<Mutex<Option<Arc<SdkBackendTransport>>>> = OnceLock::new();

/// Install the SDK-backed backend transport as the process-global transport
/// (and optionally set the product identity first).
///
/// Returns the transport so a host that also builds the core through
/// `CoreBuilder` can bind it there explicitly with
/// `CoreBuilder::backend_transport`; binding is optional because the core
/// resolves the process global when a context carries none.
pub fn install(options: InstallOptions) -> Result<Arc<SdkBackendTransport>, InstallError> {
    let slot = INSTALLED.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(identity) = options.product_identity.clone() {
        log::debug!(
            "[tinyhumans] install: product identity {}",
            identity.as_str()
        );
        set_product_identity(identity);
        // A new identity means new attribution headers; rebuild below.
        *guard = None;
    }

    if options.hosted_controllers {
        // Idempotent in the core: an identical re-registration is a no-op.
        register_controller_extension(crate::hosted::extension())
            .map_err(InstallError::Registry)?;
    }

    if let Some(existing) = guard.as_ref() {
        // Re-install into the core slot in case something cleared it (tests).
        if installed_backend_transport().is_none() {
            install_backend_transport(existing.clone());
        }
        log::trace!("[tinyhumans] install: already installed");
        return Ok(existing.clone());
    }

    let transport = Arc::new(SdkBackendTransport::new().map_err(InstallError::Transport)?);
    install_backend_transport(transport.clone());
    *guard = Some(transport.clone());
    log::info!("[tinyhumans] install: backend transport installed");
    Ok(transport)
}

/// Whether [`install`] has run in this process (and its transport is still
/// the core's global one).
pub fn is_installed() -> bool {
    INSTALLED
        .get()
        .and_then(|slot| {
            slot.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
                .map(|_| installed_backend_transport().is_some())
        })
        .unwrap_or(false)
}
