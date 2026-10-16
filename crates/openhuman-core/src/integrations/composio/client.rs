//! Host-side Composio route resolution and the direct-mode v3 reads.
//!
//! The Composio HTTP client itself lives in the `tinyconnectors` module and is
//! reached through [`super::module_client`]. What remains here is the part the
//! module does not decide for the host: which route the config selects
//! ([`resolve_composio_route`]) and the two direct-mode v3 reads
//! ([`direct_list_connections`], [`direct_list_tools`]), which run in the module
//! as stateless members under the host's key-health gate, proxy and TLS policy.
//!
//! Logging uses the `[composio]` grep-prefix so all sidecar output for
//! this domain can be filtered in one shot.

mod credential;
mod direct;
mod factory;
mod network;

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;

pub use credential::DirectCredential;
pub use direct::direct_list_connections;
#[cfg(test)]
pub(crate) use direct::direct_list_tools;
pub(crate) use factory::create_direct_client_for_api_key;
pub use factory::{
    register_composio_route_builder, resolve_composio_route, unregister_composio_route_builder,
    ComposioRoute, ComposioRouteBuilder,
};
pub(crate) use network::module_transport_with_proxy;

// Brought into this module's own namespace (private `use`, not `pub use`)
// so `client_tests.rs` — declared as a direct child module of `client`
// above — can still reach these via a plain `use super::*;`.
#[cfg(test)]
use std::sync::Arc;
