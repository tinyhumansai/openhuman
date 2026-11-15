//! Public-bind detection: [`serve`](super::serve) refuses a non-loopback
//! bind without an operator-supplied RPC token.

/// Check if a host string represents a non-localhost bind address.
pub fn is_public_bind(host: &str) -> bool {
    !matches!(
        host.trim(),
        "127.0.0.1" | "localhost" | "::1" | "[::1]" | "0:0:0:0:0:0:0:1"
    )
}

#[cfg(test)]
#[path = "public_bind_tests.rs"]
mod tests;
