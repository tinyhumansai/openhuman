//! x402 — HTTP 402 payment protocol for machine-payable APIs.
//!
//! Intercepts HTTP 402 responses carrying a `PAYMENT-REQUIRED` header,
//! constructs a Solana SPL token payment (typically USDC), signs it with the
//! wallet's ed25519 key, and retries the request with the payment proof in a
//! `PAYMENT-SIGNATURE` header. The facilitator co-signs as fee payer and
//! broadcasts, so the client never needs SOL for gas.
//!
//! Protocol spec: <https://x402.org> / coinbase/x402 (v2).

//! ## Compile-time gate (`web3` feature)
//!
//! `pub mod x402;` is ALWAYS compiled — it is a facade. The real payment
//! machinery is gated behind the default-ON `web3` Cargo feature (shared with
//! `openhuman::wallet` + `openhuman::web3`). When the feature is off, [`stub`]
//! takes its place and exposes the always-on entry points (`init_ledger`,
//! `all_x402_registered_controllers`, `all_x402_controller_schemas`) with
//! no-op / empty bodies. The `X402RequestTool` and the http_request 402-retry
//! path are `#[cfg(feature = "web3")]` at their call sites, so the rest of the
//! payment surface (`PaymentRecord`, `store`, `SettlementResponse`, …) is not
//! referenced when off and need not be stubbed.

#[cfg(feature = "web3")]
mod ops;
#[cfg(feature = "web3")]
mod schemas;
#[cfg(feature = "web3")]
pub(crate) mod store;
#[cfg(feature = "web3")]
pub mod tools;
#[cfg(feature = "web3")]
mod types;

#[cfg(all(test, feature = "web3"))]
mod x402_tests;

#[cfg(feature = "web3")]
pub use ops::{
    handle_402, handle_402_and_pay, try_paid_request, X402Client, X402Error, X402PaymentResult,
};
#[cfg(feature = "web3")]
pub use schemas::all_controller_schemas as all_x402_controller_schemas;
#[cfg(feature = "web3")]
pub use schemas::all_registered_controllers as all_x402_registered_controllers;
#[cfg(feature = "web3")]
pub use store::{init_global as init_ledger, PaymentRecord, PaymentStatus, SpendingBudget};
#[cfg(feature = "web3")]
pub use types::{
    EvmAuthorization, EvmPaymentProof, PaymentChain, PaymentPayload, PaymentProof, PaymentRequired,
    PaymentRequirements, ResourceInfo, SettlementResponse, SolanaPaymentProof,
};

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `web3` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "web3"))]
mod stub;
#[cfg(not(feature = "web3"))]
pub use stub::*;
