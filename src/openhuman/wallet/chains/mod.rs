//! Per-chain wallet executors. Each submodule owns its own key derivation,
//! transaction encoding, signing, and broadcast against a JSON-RPC or REST
//! endpoint. The shared [`super::execution`] module is a thin dispatcher.
//!
//! All execute_* fns are async and return [`ExecutionResult`] on success.
//! All preparation logic (validation, fee estimate, quote storage) lives in
//! `execution.rs`; chain modules only run when a quote is being broadcast.
//!
//! Every chain exposes a small surface:
//! - `execute(quote: PreparedTransaction) -> Result<ExecutionResult, String>`
//! - `native_balance(address: &str) -> Result<u128, String>`
//! - `validate_address(addr: &str) -> Result<String, String>`

pub(super) mod btc;
pub(super) mod evm;
pub(super) mod solana;
pub(super) mod tron;
