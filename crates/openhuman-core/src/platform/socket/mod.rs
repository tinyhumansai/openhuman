//! Core-facing socket port and RPC contract for the hosted Socket.IO connection.
//!
//! The transport implementation is installed by `openhuman-tinyhumans`; core
//! keeps the stable state DTOs, token seam, RPC handlers, and event sink.

mod event_sink;
pub mod models;
mod ops;
mod port;
mod schemas;
pub(crate) mod token_provider;

pub use event_sink::dispatch_socket_event;
pub use port::{
    global_socket_manager, global_socket_port, set_global_socket_manager, set_global_socket_port,
    SocketIdentityRebindGuard, SocketPort, TokenProvider,
};
pub use schemas::{
    all_controller_schemas as all_socket_controller_schemas,
    all_registered_controllers as all_socket_registered_controllers,
};

/// Event names preserved for the desktop observability bridge.
pub mod events {
    pub const SOCKET_STATE_CHANGED: &str = "runtime:socket-state-changed";
    pub const SERVER_EVENT: &str = "server:event";
}

pub(crate) use schemas::all_controller_entries;

#[cfg(test)]
pub(crate) use schemas::socket_parameter_contracts;
