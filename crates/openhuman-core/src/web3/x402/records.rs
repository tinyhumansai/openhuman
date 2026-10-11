//! The ledger record a payment starts with, for callers outside the
//! `x402_request` tool (the `http_request` 402 fallback).
//!
//! The tool builds its own records inside the crate; this is the same rule for
//! the host's other writer, so both attribute a payment identically:
//! `session_id` is always the ledger's own session (so the session total counts
//! the payment) and `thread_id` is the chat thread from `APPROVAL_CHAT_CONTEXT`.

use tinywallet_x402::ledger::{self, PaymentRecord, PaymentStatus};
use tinywallet_x402::protocol::X402PaymentResult;
use tinywallet_x402::thread::ThreadScope;

use super::seams::TaskLocalThread;

/// A `Pending` record for `payment`, stamped with the ledger's session and the
/// active chat thread (if any). Later states are written as new lines with the
/// same `id`.
#[allow(clippy::cast_precision_loss)] // atomic USDC amounts stay far below 2^53
pub fn pending_record(payment: &X402PaymentResult) -> PaymentRecord {
    // A ledger that is not initialised has no session to name; the payment
    // would have failed at the budget check before reaching here.
    let session_id = ledger::with_ledger(|l| l.session_id().to_string()).unwrap_or_default();
    PaymentRecord {
        id: uuid::Uuid::new_v4().to_string(),
        url: payment.url.clone(),
        asset: payment.asset.clone(),
        amount_atomic: payment.amount_atomic,
        amount_display: format!("{:.6} USDC", payment.amount_atomic as f64 / 1_000_000.0),
        recipient: payment.recipient.clone(),
        network: payment.network.clone(),
        tx_signature: None,
        status: PaymentStatus::Pending,
        timestamp: chrono::Utc::now(),
        authorization_valid_until: payment.authorization_valid_until,
        solana_last_valid_block_height: payment.solana_last_valid_block_height,
        authorization_expired_at: None,
        authorization_expiry_observed_height: None,
        session_id,
        thread_id: TaskLocalThread.current_thread(),
    }
}

#[cfg(test)]
#[path = "records_tests.rs"]
mod tests;
