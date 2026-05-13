//! Socket.IO (Engine.IO v4) WebSocket URL for the TinyHumans backend.

use url::Url;

/// Build a Socket.IO WebSocket URL from an HTTP(S) API base (e.g. `https://api.tinyhumans.ai`).
pub fn websocket_url(http_or_https_base: &str) -> String {
    let Ok(mut url) = Url::parse(http_or_https_base) else {
        return http_or_https_base.to_string();
    };

    let scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        other => other,
    }
    .to_string();

    let _ = url.set_scheme(&scheme);

    // Ensure path ends with /socket.io/ and includes required query params
    url.set_path(&format!("{}/socket.io/", url.path().trim_end_matches('/')));
    url.set_query(Some("EIO=4&transport=websocket"));

    url.to_string()
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
