//! In-memory store for prepared web3 quotes plus the shared confirm→execute
//! path. Mirrors the wallet quote store's security model: each quote is bound
//! to the chat thread that prepared it (so a `quoteId` leaked into a shared
//! channel can't be hijacked from another agent session), TTL'd, and capped.
//!
//! On execute the stored [`UnsignedTx`] is routed to the matching crate-internal
//! wallet signer — EVM `to`/`data`/`value` or a Solana `VersionedTransaction`
//! hex blob — which signs from the wallet's encrypted recovery phrase and
//! broadcasts. The wallet remains the only place private keys are touched.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;

use crate::openhuman::wallet;

use super::types::{ExecuteQuoteParams, UnsignedTx, Web3QuoteKind};

const LOG_PREFIX: &str = "[web3]";
const QUOTE_TTL_MS: u64 = 5 * 60 * 1000;
const QUOTE_STORE_CAP: usize = 64;

static QUOTE_STORE: Lazy<Mutex<Vec<StoredQuote>>> = Lazy::new(|| Mutex::new(Vec::new()));
static QUOTE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Identity of the chat thread that prepared a quote (see wallet quote-owner).
#[derive(Debug, Clone, PartialEq, Eq)]
struct QuoteOwner {
    thread_id: String,
    client_id: String,
}

fn current_owner() -> Option<QuoteOwner> {
    crate::openhuman::approval::APPROVAL_CHAT_CONTEXT
        .try_with(|ctx| QuoteOwner {
            thread_id: ctx.thread_id.clone(),
            client_id: ctx.client_id.clone(),
        })
        .ok()
}

#[derive(Clone)]
pub struct StoredQuote {
    pub quote_id: String,
    pub kind: Web3QuoteKind,
    pub unsigned: UnsignedTx,
    /// Human/agent-facing summary returned at prepare time and echoed on execute.
    pub summary: Value,
    owner: Option<QuoteOwner>,
    expires_at_ms: u64,
}

/// Result returned to the agent / RPC caller after broadcasting.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Web3ExecutionResult {
    pub quote_id: String,
    pub kind: Web3QuoteKind,
    pub transaction_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explorer_url: Option<String>,
    /// Simulated fee in the chain's smallest unit; `None` when not known at
    /// broadcast time (e.g. Solana's dynamic fee).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_raw: Option<String>,
}

/// The prepared-quote envelope returned to the caller.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Web3Quote {
    pub quote_id: String,
    pub kind: Web3QuoteKind,
    pub expires_at_ms: u64,
    /// Backend (deBridge) quote/estimation passthrough, or a dapp-call summary.
    pub quote: Value,
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn next_quote_id() -> String {
    let n = QUOTE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("w3_{}_{}", now_ms(), n)
}

/// Store a prepared quote and return the caller-facing envelope.
pub fn store_quote(kind: Web3QuoteKind, unsigned: UnsignedTx, summary: Value) -> Web3Quote {
    let now = now_ms();
    let expires_at_ms = now + QUOTE_TTL_MS;
    let quote = StoredQuote {
        quote_id: next_quote_id(),
        kind,
        unsigned,
        summary: summary.clone(),
        owner: current_owner(),
        expires_at_ms,
    };
    let envelope = Web3Quote {
        quote_id: quote.quote_id.clone(),
        kind,
        expires_at_ms,
        quote: summary,
    };
    let mut store = QUOTE_STORE.lock();
    store.retain(|q| q.expires_at_ms > now);
    if store.len() >= QUOTE_STORE_CAP {
        store.remove(0);
    }
    store.push(quote);
    envelope
}

/// Remove the quote iff the caller's chat-thread owner matches. Returns the
/// byte-identical "not found" error on owner mismatch (no enumeration oracle).
fn take_quote_for(quote_id: &str, caller: Option<QuoteOwner>) -> Result<StoredQuote, String> {
    let not_found = || format!("quote '{quote_id}' not found");
    let mut store = QUOTE_STORE.lock();
    let now = now_ms();
    let pos = store
        .iter()
        .position(|q| q.quote_id == quote_id)
        .ok_or_else(not_found)?;
    if store[pos].owner != caller {
        return Err(not_found());
    }
    let quote = store.remove(pos);
    if quote.expires_at_ms <= now {
        return Err(format!("quote '{quote_id}' expired"));
    }
    Ok(quote)
}

#[cfg(test)]
pub(crate) fn reset_store_for_tests() {
    QUOTE_STORE.lock().clear();
}

#[cfg(test)]
pub(crate) fn stored_quote_count() -> usize {
    QUOTE_STORE.lock().len()
}

/// Confirm and execute a prepared quote: sign+broadcast the stored unsigned
/// transaction via the wallet, restoring the quote (refreshed TTL) on failure
/// so it stays retryable.
pub async fn execute_quote(
    params: ExecuteQuoteParams,
) -> Result<crate::rpc::RpcOutcome<Web3ExecutionResult>, String> {
    if !params.confirmed {
        return Err("execute requires `confirmed: true`".to_string());
    }
    let caller = current_owner();
    let quote = take_quote_for(&params.quote_id, caller)?;
    let kind = quote.kind;
    let restorable = quote.clone();

    let result = match &quote.unsigned {
        UnsignedTx::Evm {
            network,
            to,
            data,
            value,
        } => wallet::sign_and_broadcast_evm(*network, to, data.clone(), value).await,
        UnsignedTx::Solana { tx_blob_hex } => wallet::sign_and_broadcast_solana(tx_blob_hex).await,
    };

    let broadcast = match result {
        Ok(b) => b,
        Err(error) => {
            // Restore with a refreshed TTL so the caller can retry.
            let mut refreshed = restorable;
            refreshed.expires_at_ms = now_ms() + QUOTE_TTL_MS;
            let mut store = QUOTE_STORE.lock();
            if store.len() >= QUOTE_STORE_CAP {
                store.remove(0);
            }
            store.push(refreshed);
            tracing::warn!(
                "{LOG_PREFIX} execute quote_id={} kind={:?} failed (restored): {error}",
                params.quote_id,
                kind
            );
            return Err(error);
        }
    };

    Ok(crate::rpc::RpcOutcome::new(
        Web3ExecutionResult {
            quote_id: params.quote_id,
            kind,
            transaction_hash: broadcast.transaction_hash,
            explorer_url: broadcast.explorer_url,
            fee_raw: broadcast.fee_raw,
        },
        vec!["web3 transaction broadcast".to_string()],
    ))
}
