//! Per-chain wallet executors. Each submodule owns its own key derivation,
//! transaction encoding, signing, and broadcast against a JSON-RPC or REST
//! endpoint. The shared [`super::execution`] module is a thin dispatcher.
//!
//! All execute_* fns are async and return [`ExecutionResult`] on success.
//! All preparation logic (validation, fee estimate, quote storage) lives in
//! `execution.rs`; chain modules only run when a quote is being broadcast.
//!
//! Every chain exposes a small surface:
//! - `execute_*_quote(quote: PreparedTransaction) -> Result<ExecutionResult, String>`
//! - `native_balance(address: &str) -> Result<u128, String>`
//! - `validate_*_address(addr: &str) -> Result<String, String>`
//! - `tx_status` / `tx_receipt` / `lookup_tx` — read-only inspection by hash.
//!
//! EVM and Solana additionally expose crate-internal `sign_and_broadcast_*`
//! primitives (sign+broadcast an externally-built unsigned transaction) that the
//! [`crate::openhuman::web3`] layer builds on.

pub(super) mod btc;
pub(super) mod evm;
pub(super) mod solana;
pub(super) mod tron;
