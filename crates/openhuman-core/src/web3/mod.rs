//! High-level web3 surface built on top of the [`crate::web3::wallet`]
//! signing primitives. Focuses on EVM/Solana dapp interactions: swaps, bridges,
//! and generic contract calls.
//!
//! The swap/bridge/dapp logic (quote preparation, the confirm-then-execute quote
//! store, the agent tools) lives in `tinywallet_web3::crypto::service`. What
//! stays here is the host adapter: the RPC controllers and their schemas (the
//! namespace strings are wire contracts), the [`client`] over the backend's
//! deBridge proxy (`/agent-integrations/crypto/*`), and [`seams`], which holds
//! the host implementations of the crate's seams and the process-wide service.
//! Three sub-modules expose distinct RPC namespaces:
//! - [`swap`] (`web3_swap`) — single-chain swaps (cross-chain → `web3_bridge`).
//! - [`bridge`] (`web3_bridge`) — cross-chain DLN bridges.
//! - [`dapp`] (`web3_dapp`) — generic EVM contract calls.
//!
//! ## Compile-time gate (`web3` feature)
//!
//! `pub mod web3;` is ALWAYS compiled — it is a facade. The real swap/bridge/
//! dapp implementation is gated behind the default-ON `web3` Cargo feature
//! (shared with `openhuman::web3::wallet` + `openhuman::web3::x402`). When the feature is
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
pub mod seams;
#[cfg(feature = "web3")]
pub mod swap;

// Ungated family members: `wallet` and `x402` are facades in their own right —
// each keeps its own `stub.rs` and gates its real submodules on the same
// default-ON `web3` feature. Always-compiled callers resolve through those
// stubs (`tools/impl/network/host.rs` -> `x402`), so these
// declarations must NOT carry a `#[cfg]`.
pub mod wallet;
pub mod x402;

#[cfg(feature = "web3")]
use crate::core::all::RegisteredController;
#[cfg(feature = "web3")]
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
#[cfg(feature = "web3")]
use tinytools::Tool;

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
    use tinywallet_web3::tools::web3::{
        Web3BridgeExecuteTool, Web3BridgeQuoteTool, Web3DappCallTool, Web3DappExecuteTool,
        Web3SwapExecuteTool, Web3SwapQuoteTool, Web3SwapRoutesTool,
    };

    let service = seams::service();
    vec![
        Box::new(Web3SwapQuoteTool::new(service.clone())),
        Box::new(Web3SwapExecuteTool::new(service.clone())),
        Box::new(Web3SwapRoutesTool::new(service.clone())),
        Box::new(Web3BridgeQuoteTool::new(service.clone())),
        Box::new(Web3BridgeExecuteTool::new(service.clone())),
        Box::new(Web3DappCallTool::new(service.clone())),
        Box::new(Web3DappExecuteTool::new(service)),
    ]
}
