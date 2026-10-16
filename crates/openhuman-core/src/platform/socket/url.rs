//! Socket.IO (Engine.IO v4) WebSocket URL for the TinyHumans backend.

use url::Url;

/// Permit credential-bearing socket connections only over TLS or loopback.
pub(super) fn is_safe_socket_endpoint(endpoint: &str) -> bool {
    let Ok(mut url) = Url::parse(endpoint) else {
        return false;
    };
    match url.scheme() {
        "wss" => url.set_scheme("https").is_ok(),
        "ws" => {
            if url.set_scheme("http").is_err() {
                return false;
            }
            crate::inference::provider::openhuman_backend_model::is_safe_endpoint_for_managed_bearer(
                url.as_str(),
            )
        }
        _ => false,
    }
}

/// Build a Socket.IO WebSocket URL from an HTTP(S) API base (e.g. `https://api.example.com`).
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
#[path = "url_tests.rs"]
mod tests;
