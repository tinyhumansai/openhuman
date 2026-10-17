//! `web3_swap` — single-chain swaps via deBridge. Cross-chain swaps are
//! redirected to `web3_bridge`. Quote and execution logic live in the compiled
//! TinyWallet module; this module owns the RPC controllers and their schemas.
//! Agent tools use that same bus API.

pub mod schemas;
