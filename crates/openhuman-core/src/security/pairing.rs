//! Public-bind detection used by the RPC host before serving.

/// Check if a host string represents a non-localhost bind address.
pub fn is_public_bind(host: &str) -> bool {
    !matches!(
        host.trim(),
        "127.0.0.1" | "localhost" | "::1" | "[::1]" | "0:0:0:0:0:0:0:1"
    )
}
