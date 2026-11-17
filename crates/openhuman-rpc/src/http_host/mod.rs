//! Static directory hosting over ad-hoc HTTP listeners.
//!
//! This domain lets trusted callers start, inspect, list, and stop lightweight
//! file servers that expose a chosen directory on a chosen TCP port. Each
//! server runs in-process, shares the core's lifetime, and defaults to HTTP
//! Basic authentication using the active user's identity plus a generated
//! password.
//!
//! The domain is HTTP transport, so it lives with the JSON-RPC server rather
//! than in the core. Its controllers join the core's registry as an extension
//! ([`register_controllers`]), which the server entry points install.

mod auth;
mod handlers;
pub mod ops;
mod path_utils;
pub mod rpc;
mod schemas;
#[cfg(test)]
#[path = "http_host_tests.rs"]
mod tests;
mod types;

pub use schemas::{
    all_controller_schemas as all_http_host_controller_schemas,
    all_registered_controllers as all_http_host_registered_controllers,
};

pub(crate) const LOG_PREFIX: &str = "[http_host]";

use crate::core_host::core::all::{
    register_controller_extension, ControllerExtension, DomainGroup,
};

const NAMESPACES: &[(&str, &str)] = &[(
    "http_host",
    "Serve a local directory over an ad-hoc, Basic-auth-protected HTTP listener.",
)];

/// Registers the `http_host.*` controllers with the core's registry.
///
/// Idempotent: the core treats an identical re-registration as a no-op, so
/// every server entry point may call it. Kernel surface with no family of its
/// own, so it is gated as [`DomainGroup::Platform`].
pub fn register_controllers() -> Result<(), String> {
    log::debug!("{LOG_PREFIX} registering controller extension");
    register_controller_extension(extension())
}

/// The `http_host.*` controllers as a controller extension, for a runtime
/// builder's `controller_extension` option (what [`crate::host`] wires).
/// Kernel surface with no family of its own, so gated as
/// [`DomainGroup::Platform`].
pub fn extension() -> ControllerExtension {
    ControllerExtension {
        group: DomainGroup::Platform,
        controllers: all_http_host_registered_controllers(),
        namespaces: NAMESPACES,
    }
}

/// [`register_controllers`] for server setup paths that cannot return an
/// error. A registration failure is a broken host invariant, so fail before
/// installing the server or building its router.
pub(crate) fn ensure_registered() {
    register_controllers().unwrap_or_else(|error| {
        panic!("{LOG_PREFIX} controller registration failed: {error}");
    });
}
