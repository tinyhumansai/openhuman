//! x402 — HTTP 402 payment protocol for machine-payable APIs.
//!
//! Intercepts HTTP 402 responses carrying a `PAYMENT-REQUIRED` header,
//! constructs a Solana SPL token payment (typically USDC), signs it with the
//! wallet's ed25519 key, and retries the request with the payment proof in a
//! `PAYMENT-SIGNATURE` header. The facilitator co-signs as fee payer and
//! broadcasts, so the client never needs SOL for gas.
//!
//! Protocol spec: <https://x402.org> / coinbase/x402 (v2).

mod ops;
mod schemas;
pub(crate) mod store;
pub mod tools;
mod types;

#[cfg(test)]
mod x402_tests;

pub use ops::{
    handle_402, handle_402_and_pay, try_paid_request, X402Client, X402Error, X402PaymentResult,
};
pub use schemas::all_controller_schemas as all_x402_controller_schemas;
pub use schemas::all_registered_controllers as all_x402_registered_controllers;
pub use store::{init_global as init_ledger, PaymentRecord, PaymentStatus, SpendingBudget};
pub use types::{
    EvmAuthorization, EvmPaymentProof, PaymentChain, PaymentPayload, PaymentProof, PaymentRequired,
    PaymentRequirements, ResourceInfo, SettlementResponse, SolanaPaymentProof,
};
