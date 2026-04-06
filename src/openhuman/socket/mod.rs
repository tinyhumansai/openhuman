//! Socket domain — persistent Socket.IO client connection to the backend.
//!
//! Provides the `SocketManager` for WebSocket-based communication with
//! automatic reconnection, MCP tool dispatch, webhook routing, and channel
//! inbound message handling.

mod event_handlers;
pub mod manager;
mod schemas;
pub mod types;
pub(crate) mod ws_loop;

pub use manager::{global_socket_manager, set_global_socket_manager, SocketManager};
pub use schemas::{
    all_controller_schemas as all_socket_controller_schemas,
    all_registered_controllers as all_socket_registered_controllers,
};
