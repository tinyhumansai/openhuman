//! Host-owned dispatch for plaintext tunnel RPC requests.
//!
//! Framing and authenticated decryption belong to TinySecurity. This module
//! keeps only the host's controller invocation and response semantics.

use crate::core::runtime::DispatchCtx;
use crate::profiles::surface::USER_METHODS;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct TunnelEnvelope {
    #[serde(rename = "requestId")]
    pub(super) request_id: String,
    pub(super) kind: String,
    pub(super) seq: u64,
    pub(super) payload: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct TunnelRpcPayload {
    pub(super) method: String,
    #[serde(default)]
    pub(super) params: Value,
}

pub(super) fn parse_tunnel_rpc_payload(payload: Value) -> Result<TunnelRpcPayload, String> {
    serde_json::from_value(payload).map_err(|error| format!("invalid tunnel RPC payload: {error}"))
}

pub(super) fn response_envelope(request_id: &str, kind: &str, payload: Value) -> TunnelEnvelope {
    TunnelEnvelope {
        request_id: request_id.to_string(),
        kind: kind.to_string(),
        seq: 0,
        payload,
    }
}

pub(super) fn is_tunnel_rpc_method_allowed(method: &str) -> bool {
    method == "openhuman.ping" || USER_METHODS.contains(&method)
}

pub(super) async fn dispatch_tunnel_rpc(
    ctx: &DispatchCtx,
    request: TunnelRpcPayload,
) -> Result<Value, String> {
    if !is_tunnel_rpc_method_allowed(&request.method) {
        return Err(format!(
            "tunnel RPC method is not allowed: {}",
            request.method
        ));
    }

    crate::core::invoke::invoke_method(
        ctx.clone(),
        crate::core::invoke::default_state(),
        &request.method,
        request.params,
    )
    .await
}

#[cfg(test)]
#[path = "tunnel_rpc_tests.rs"]
mod tests;
