//! `web3_bridge` — cross-chain bridges via deBridge DLN. Logic lives in
//! the compiled TinyWallet module; this module owns the RPC controllers and
//! their schemas. The agent tools call the same module contract.

pub mod schemas;
