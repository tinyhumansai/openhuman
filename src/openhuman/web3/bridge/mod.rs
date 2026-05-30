//! `web3_bridge` — cross-chain bridges via deBridge DLN. Logic lives in the
//! shared [`super::ops`]/[`super::store`]; this module owns the RPC controllers
//! and agent tools.

pub mod schemas;
pub mod tools;

pub use tools::{Web3BridgeExecuteTool, Web3BridgeQuoteTool};
