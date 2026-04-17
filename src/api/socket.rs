//! Socket.IO (Engine.IO v4) WebSocket URL for the TinyHumans backend.

/// Build a Socket.IO WebSocket URL from an HTTP(S) API base (e.g. `https://api.tinyhumans.ai`).
pub fn websocket_url(http_or_https_base: &str) -> String {
    let base = http_or_https_base.trim_end_matches('/');
    let ws_base = if base.starts_with("https://") {
        base.replacen("https://", "wss://", 1)
    } else if base.starts_with("http://") {
        base.replacen("http://", "ws://", 1)
    } else {
        base.to_string()
    };
    format!("{}/socket.io/?EIO=4&transport=websocket", ws_base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_https_to_wss() {
        let url = websocket_url("https://api.tinyhumans.ai");
        assert_eq!(
            url,
            "wss://api.tinyhumans.ai/socket.io/?EIO=4&transport=websocket"
        );
    }

    #[test]
    fn converts_http_to_ws() {
        let url = websocket_url("http://localhost:3000");
        assert_eq!(
            url,
            "ws://localhost:3000/socket.io/?EIO=4&transport=websocket"
        );
    }

    #[test]
    fn passes_through_unknown_scheme() {
        let url = websocket_url("ftp://example.com");
        assert_eq!(
            url,
            "ftp://example.com/socket.io/?EIO=4&transport=websocket"
        );
    }

    #[test]
    fn strips_trailing_slash() {
        let url = websocket_url("https://api.tinyhumans.ai/");
        assert_eq!(
            url,
            "wss://api.tinyhumans.ai/socket.io/?EIO=4&transport=websocket"
        );
    }

    #[test]
    fn strips_multiple_trailing_slashes() {
        let url = websocket_url("https://api.tinyhumans.ai///");
        assert_eq!(
            url,
            "wss://api.tinyhumans.ai/socket.io/?EIO=4&transport=websocket"
        );
    }
}
