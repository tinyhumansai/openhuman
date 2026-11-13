//! Host RPC facade for module-owned x402 payments.
//!
//! TinyWallet owns challenge parsing, chain selection, payment construction,
//! signing, budget reservation, ledger records, and settlement parsing. The
//! host retains generic HTTP execution, explicit user approval, and secret
//! custody; payment material crosses only the confidential module call.

#[cfg(feature = "web3")]
mod fetch;
#[cfg(feature = "web3")]
mod schemas;

#[cfg(feature = "web3")]
pub use schemas::all_controller_schemas as all_x402_controller_schemas;
#[cfg(feature = "web3")]
pub use schemas::all_registered_controllers as all_x402_registered_controllers;

#[cfg(not(feature = "web3"))]
mod stub;
#[cfg(not(feature = "web3"))]
pub use stub::*;
