//! Where the process finds its [`BackendTransport`].
//!
//! Resolution order, first hit wins:
//!
//! 1. the transport carried by the ambient [`CoreContext`] (installed through
//!    `CoreBuilder::backend_transport`; inherited by every derived context);
//! 2. the process-global transport installed with [`install_backend_transport`]
//!    — the path the desktop shell and CLI use, because they boot the core
//!    through `run_server_embedded_with_ready` / `run_core_from_args` rather
//!    than through the builder;
//! 3. under `cfg(test)` only, a plain `reqwest` transport so the crate's
//!    wiremock unit tests need no host crate;
//! 4. otherwise [`BackendTransportError::Unavailable`].
//!
//! Production builds have **no** implicit fallback: a core that nobody gave a
//! transport genuinely has no backend, and every caller degrades to a typed
//! "backend unavailable" error.

use std::sync::{Arc, OnceLock, RwLock};

use super::{BackendTransport, BackendTransportError};

static GLOBAL: OnceLock<RwLock<Option<Arc<dyn BackendTransport>>>> = OnceLock::new();

fn slot() -> &'static RwLock<Option<Arc<dyn BackendTransport>>> {
    GLOBAL.get_or_init(|| RwLock::new(None))
}

/// Install `transport` as the process-wide backend transport, replacing any
/// previous one. Idempotent from the caller's point of view: installing the
/// same implementation twice is harmless.
pub fn install_backend_transport(transport: Arc<dyn BackendTransport>) {
    let name = transport.name();
    let previous = slot()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .replace(transport)
        .map(|t| t.name());
    match previous {
        Some(prev) if prev != name => {
            log::info!("[backend-transport] replaced global transport {prev} with {name}")
        }
        Some(_) => log::debug!("[backend-transport] re-installed global transport {name}"),
        None => log::info!("[backend-transport] installed global transport {name}"),
    }
}

/// The process-global transport, if one was installed.
pub fn installed_backend_transport() -> Option<Arc<dyn BackendTransport>> {
    slot()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Remove the process-global transport. Test hook.
pub fn clear_backend_transport() {
    let cleared = slot()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .is_some();
    if cleared {
        log::debug!("[backend-transport] cleared global transport");
    }
}

/// Resolve the transport a backend call should use right now (see the module
/// docs for the precedence).
pub fn resolve_backend_transport() -> Result<Arc<dyn BackendTransport>, BackendTransportError> {
    if let Some(transport) = crate::core::runtime::context::CoreContext::current()
        .and_then(|ctx| ctx.backend_transport())
    {
        return Ok(transport);
    }
    if let Some(transport) = installed_backend_transport() {
        return Ok(transport);
    }
    #[cfg(test)]
    {
        return Ok(super::plain::PlainHttpTransport::fresh());
    }
    #[cfg(not(test))]
    {
        log::debug!("[backend-transport] no transport installed; backend unavailable");
        Err(BackendTransportError::Unavailable)
    }
}

/// Whether a backend call made right now would find a transport (see
/// [`resolve_backend_transport`]). Paths that talk to the backend host
/// directly instead of through the port (the Langfuse proxy push, the socket,
/// channel reply delivery, browser-task module config) check this first, so a
/// core running without a TinyHumans connection skips them quietly instead of
/// reaching a backend it was never connected to.
pub fn is_installed() -> bool {
    resolve_backend_transport().is_ok()
}
