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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_are_stable_grep_anchors() {
        // The frontend subscribes to these exact strings — a rename here
        // silently breaks the Tauri event bridge. Lock them in.
        assert_eq!(events::SOCKET_STATE_CHANGED, "runtime:socket-state-changed");
        assert_eq!(events::SERVER_EVENT, "server:event");
    }

    #[test]
    fn connection_outcome_variants_can_be_constructed() {
        // Sanity-check that the enum variants match what `ws_loop` expects
        // when deciding whether to reset or grow backoff.
        let a = ConnectionOutcome::Shutdown;
        let b = ConnectionOutcome::Lost("net".into());
        let c = ConnectionOutcome::Failed("tls".into());
        for outcome in [a, b, c] {
            match outcome {
                ConnectionOutcome::Shutdown => {}
                ConnectionOutcome::Lost(reason) => assert!(!reason.is_empty()),
                ConnectionOutcome::Failed(reason) => assert!(!reason.is_empty()),
            }
        }
    }

    #[test]
    fn connection_outcome_reason_strings_are_preserved() {
        if let ConnectionOutcome::Lost(r) = ConnectionOutcome::Lost("timeout".into()) {
            assert_eq!(r, "timeout");
        } else {
            panic!("expected Lost");
        }
        if let ConnectionOutcome::Failed(r) = ConnectionOutcome::Failed("hs".into()) {
            assert_eq!(r, "hs");
        } else {
            panic!("expected Failed");
        }
    }
}
