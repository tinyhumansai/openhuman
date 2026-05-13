//! Generic structured error envelope for the JSON-RPC controller boundary.
//!
//! Domains expose typed errors (e.g. `ThreadsError::NotFound`) and convert
//! them — at their own boundary — into [`StructuredRpcError`]s. The envelope
//! is then encoded as a sentinel-prefixed string so it can travel through
//! the existing `Result<Value, String>` channel that controller handlers
//! already use, without changing every handler signature.
//!
//! The JSON-RPC transport layer (`src/core/jsonrpc.rs`) decodes the envelope
//! transparently — it has zero knowledge of which domain produced the error,
//! and never branches on the RPC method name. New domains that want
//! structured RPC errors just emit a [`StructuredRpcError`] at their
//! controller boundary; the transport handles the rest.
//!
//! Wire shape is unchanged: the human-readable `message` populates
//! `RpcError.message` and the typed `data` populates `RpcError.data`. The
//! `expected_user_state` flag is consumed by the boundary itself to decide
//! whether to forward the error to Sentry; it does not appear on the wire.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Sentinel prefix that marks a [`StructuredRpcError`] encoded as a `String`.
///
/// The prefix is intentionally noisy and unlikely to collide with a real
/// human-readable error message. If a controller error string starts with
/// this prefix, the boundary decodes the rest as JSON.
pub const STRUCTURED_RPC_ERROR_SENTINEL: &str = "__OPENHUMAN_STRUCTURED_RPC_ERROR_V1__:";

/// Generic structured error emitted by a controller / domain boundary.
///
/// The transport layer decodes this without inspecting the RPC method name
/// or the message contents, so new domains can adopt it without touching
/// `src/core/jsonrpc.rs`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredRpcError {
    /// Human-readable error text for the JSON-RPC `error.message` field.
    pub message: String,
    /// Typed payload for the JSON-RPC `error.data` field. Must include a
    /// stable `kind` discriminator so frontends can branch on it.
    pub data: Option<Value>,
    /// When `true`, the boundary skips Sentry reporting — this is an
    /// expected user-visible state (stale thread, missing resource, etc.)
    /// not an internal failure.
    #[serde(default)]
    pub expected_user_state: bool,
}

impl StructuredRpcError {
    /// Encode into a sentinel-prefixed string suitable for the controller
    /// `Result<_, String>` error channel.
    pub fn encode(&self) -> String {
        // serde_json::to_string on a struct of String/Option<Value>/bool
        // cannot fail in practice, so unwrap is acceptable.
        let json = serde_json::to_string(self)
            .expect("StructuredRpcError serialization cannot fail: struct contains only String, Option<Value>, and bool");
        format!("{STRUCTURED_RPC_ERROR_SENTINEL}{json}")
    }

    /// Decode a controller error string if it carries the sentinel prefix.
    /// Returns `None` for plain error strings.
    pub fn decode(raw: &str) -> Option<Self> {
        let body = raw.strip_prefix(STRUCTURED_RPC_ERROR_SENTINEL)?;
        serde_json::from_str(body).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn encode_then_decode_round_trips() {
        let original = StructuredRpcError {
            message: "thread thread-123 not found".to_string(),
            data: Some(json!({ "kind": "ThreadNotFound", "thread_id": "thread-123" })),
            expected_user_state: true,
        };
        let encoded = original.encode();
        assert!(
            encoded.starts_with(STRUCTURED_RPC_ERROR_SENTINEL),
            "encoded string must carry the sentinel prefix"
        );
        let decoded = StructuredRpcError::decode(&encoded).expect("decoded");
        assert_eq!(decoded, original);
    }

    #[test]
    fn decode_returns_none_for_plain_strings() {
        assert!(StructuredRpcError::decode("plain error").is_none());
        assert!(StructuredRpcError::decode("").is_none());
        assert!(StructuredRpcError::decode("__OPENHUMAN_STRUCTURED_RPC_ERROR_V1__").is_none());
    }

    #[test]
    fn decode_returns_none_for_corrupt_envelope() {
        let bad = format!("{STRUCTURED_RPC_ERROR_SENTINEL}not-json");
        assert!(StructuredRpcError::decode(&bad).is_none());
    }

    #[test]
    fn expected_user_state_defaults_to_false_when_absent() {
        let raw = format!("{STRUCTURED_RPC_ERROR_SENTINEL}{{\"message\":\"x\",\"data\":null}}");
        let decoded = StructuredRpcError::decode(&raw).expect("decoded");
        assert!(!decoded.expected_user_state);
    }
}
