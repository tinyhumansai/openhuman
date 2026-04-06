//! Socket domain types, constants, and re-exports.

pub use crate::api::models::socket::{ConnectionStatus, SocketState};

/// Events emitted for observability / frontend bridging.
#[allow(dead_code)]
pub mod events {
    /// Socket state changed (status, socket_id, error).
    pub const SOCKET_STATE_CHANGED: &str = "runtime:socket-state-changed";
    /// A server event was received and forwarded.
    pub const SERVER_EVENT: &str = "server:event";
}

/// Type alias for the underlying WebSocket stream.
pub(super) type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Result of a single connection attempt in the `ws_loop`.
pub(super) enum ConnectionOutcome {
    /// Clean shutdown requested by the user.
    Shutdown,
    /// Connection was established then lost (triggers reset of backoff).
    Lost(String),
    /// Connection failed during handshake (triggers increment of backoff).
    Failed(String),
}
