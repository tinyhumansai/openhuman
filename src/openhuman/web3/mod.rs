//! High-level web3 surface built on top of the [`crate::openhuman::wallet`]
//! signing primitives. Focuses on EVM/Solana dapp interactions: swaps, bridges,
//! and generic contract calls.
//!
//! Quotes + unsigned transactions come from the backend deBridge proxy
//! (`/agent-integrations/crypto/*`); signing/broadcast is delegated to the
//! wallet's crate-internal `sign_and_broadcast_*` primitives so private keys
//! never leave the wallet. Three sub-modules expose distinct RPC namespaces +
//! tool families:
//! - [`swap`] (`web3_swap`) — single-chain swaps (cross-chain → `web3_bridge`).
//! - [`bridge`] (`web3_bridge`) — cross-chain DLN bridges.
//! - [`dapp`] (`web3_dapp`) — generic EVM contract calls.

//! ## Compile-time gate (`web3` feature)
//!
//! `pub mod web3;` is ALWAYS compiled — it is a facade. The real swap/bridge/
//! dapp implementation is gated behind the default-ON `web3` Cargo feature
//! (shared with `openhuman::wallet` + `openhuman::x402`). When the feature is
//! off, [`stub`] takes its place and exposes the controller/agent-tool
//! registration entry points (`all_web3_registered_controllers`,
//! `all_web3_controller_schemas`, `all_web3_agent_tools`) returning empty
//! collections, so `core/all.rs` + `tools/ops.rs` need no per-call `#[cfg]`.

#[cfg(feature = "web3")]
pub mod bridge;
#[cfg(feature = "web3")]
pub mod client;
#[cfg(feature = "web3")]
pub mod dapp;
#[cfg(feature = "web3")]
pub mod ops;
#[cfg(feature = "web3")]
pub mod store;
#[cfg(feature = "web3")]
pub mod swap;
#[cfg(feature = "web3")]
pub mod types;

#[cfg(all(test, feature = "web3"))]
#[path = "web3_tests.rs"]
mod tests;

#[cfg(feature = "web3")]
use serde::Serialize;

#[cfg(feature = "web3")]
use crate::core::all::RegisteredController;
#[cfg(feature = "web3")]
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
#[cfg(feature = "web3")]
use crate::openhuman::tools::traits::{Tool, ToolResult};
#[cfg(feature = "web3")]
use crate::rpc::RpcOutcome;

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `web3` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "web3"))]
mod stub;
#[cfg(not(feature = "web3"))]
pub use stub::*;

/// A required JSON-typed controller input.
#[cfg(feature = "web3")]
pub(crate) fn req_json(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

/// An optional string controller input.
#[cfg(feature = "web3")]
pub(crate) fn opt_str(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

/// Standard `result` output field.
#[cfg(feature = "web3")]
pub(crate) fn json_result(comment: &'static str) -> FieldSchema {
    FieldSchema {
        name: "result",
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

/// Shared `quoteId` + `confirmed` inputs for the execute controllers.
#[cfg(feature = "web3")]
pub(crate) fn execute_inputs() -> Vec<FieldSchema> {
    vec![
        req_json("quoteId", "quoteId returned by a prior web3 quote/call."),
        req_json(
            "confirmed",
            "Must be true; explicit boundary between quote and execute.",
        ),
    ]
}

/// Shared agent-tool JSON Schema for the execute tools.
#[cfg(feature = "web3")]
pub(crate) fn execute_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "quoteId": {"type": "string", "description": "quoteId from a prior web3 quote/call."},
            "confirmed": {"type": "boolean", "description": "Must be true to execute."}
        },
        "required": ["quoteId", "confirmed"],
        "additionalProperties": false
    })
}

/// Convert an op result into a `ToolResult` with pretty-printed JSON on success.
#[cfg(feature = "web3")]
pub(crate) fn to_tool_result<T: Serialize>(result: Result<RpcOutcome<T>, String>) -> ToolResult {
    match result {
        Ok(outcome) => match serde_json::to_string_pretty(&outcome.value) {
            Ok(s) => ToolResult::success(s),
            Err(e) => ToolResult::error(format!("failed to serialize web3 result: {e}")),
        },
        Err(e) => ToolResult::error(e),
    }
}

/// All web3 controller schemas across the swap/bridge/dapp namespaces.
#[cfg(feature = "web3")]
pub fn all_web3_controller_schemas() -> Vec<ControllerSchema> {
    let mut out = swap::schemas::schemas();
    out.extend(bridge::schemas::schemas());
    out.extend(dapp::schemas::schemas());
    out
}

/// All web3 registered controllers across the swap/bridge/dapp namespaces.
#[cfg(feature = "web3")]
pub fn all_web3_registered_controllers() -> Vec<RegisteredController> {
    let mut out = swap::schemas::controllers();
    out.extend(bridge::schemas::controllers());
    out.extend(dapp::schemas::controllers());
    out
}

/// All web3 agent tools. These call the backend per-invocation, so they error
/// gracefully (rather than being hidden) when the user is not signed in.
#[cfg(feature = "web3")]
pub fn all_web3_agent_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(swap::Web3SwapQuoteTool::new()),
        Box::new(swap::Web3SwapExecuteTool::new()),
        Box::new(swap::Web3SwapRoutesTool::new()),
        Box::new(bridge::Web3BridgeQuoteTool::new()),
        Box::new(bridge::Web3BridgeExecuteTool::new()),
        Box::new(dapp::Web3DappCallTool::new()),
        Box::new(dapp::Web3DappExecuteTool::new()),
    ]
}
