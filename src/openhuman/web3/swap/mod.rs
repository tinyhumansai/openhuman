//! `web3_swap` — single-chain swaps via deBridge. Cross-chain swaps are
//! redirected to `web3_bridge`. Logic lives in the shared
//! [`super::ops`]/[`super::store`]; this module owns the RPC controllers and
//! agent tools.

pub mod schemas;
pub mod tools;

pub use tools::{Web3SwapExecuteTool, Web3SwapQuoteTool, Web3SwapRoutesTool};
