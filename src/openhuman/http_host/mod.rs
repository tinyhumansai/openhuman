//! Static directory hosting over ad-hoc HTTP listeners owned by the core.
//!
//! This domain lets trusted callers start, inspect, list, and stop lightweight
//! file servers that expose a chosen directory on a chosen TCP port. Each
//! server runs in-process, shares the core's lifetime, and defaults to HTTP
//! Basic authentication using the active user's identity plus a generated
//! password.

mod auth;
mod handlers;
pub mod ops;
mod path_utils;
pub mod rpc;
mod schemas;
#[cfg(test)]
mod tests;
mod types;

pub use schemas::{
    all_controller_schemas as all_http_host_controller_schemas,
    all_registered_controllers as all_http_host_registered_controllers,
};

pub(crate) const LOG_PREFIX: &str = "[http_host]";
