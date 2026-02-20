//! Client IP parsing and rate-limit key derivation.

use axum::http::HeaderMap;
use std::net::{IpAddr, SocketAddr};

/// Parse a client IP from a header value, tolerating quotes and socket formats.
pub fn parse_client_ip(value: &str) -> Option<IpAddr> {
    let value = value.trim().trim_matches('"').trim();
    if value.is_empty() {
        return None;
    }

    if let Ok(ip) = value.parse::<IpAddr>() {
        return Some(ip);
    }

    if let Ok(addr) = value.parse::<SocketAddr>() {
        return Some(addr.ip());
    }

    let value = value.trim_matches(['[', ']']);
    value.parse::<IpAddr>().ok()
}

/// Extract the first valid client IP from forwarding headers.
pub fn forwarded_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    if let Some(xff) = headers.get("X-Forwarded-For").and_then(|v| v.to_str().ok()) {
        for candidate in xff.split(',') {
            if let Some(ip) = parse_client_ip(candidate) {
                return Some(ip);
            }
        }
    }

    headers
        .get("X-Real-IP")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_client_ip)
}

/// Resolve a stable client key for rate limiting.
pub fn client_key_from_request(
    connect_info: Option<SocketAddr>,
    headers: &HeaderMap,
    trust_forwarded_headers: bool,
) -> String {
    if trust_forwarded_headers {
        if let Some(forwarded) = forwarded_client_ip(headers) {
            return forwarded.to_string();
        }
    }

    connect_info
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Normalize configured key counts, ensuring a non-zero default.
pub fn normalize_max_keys(configured: usize, fallback: usize) -> usize {
    if configured == 0 {
        fallback.max(1)
    } else {
        configured
    }
}
