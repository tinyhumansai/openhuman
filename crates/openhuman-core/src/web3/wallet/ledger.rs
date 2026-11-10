//! Persistent ledger of native-wallet executes.
//!
//! Every `wallet_execute_prepared` writes one record **before** the module is
//! asked to sign and broadcast (`pending`) and updates it afterwards
//! (`broadcast` with the signature, or `in_doubt` with the error). A refused
//! execute is recorded as `refused` for the audit trail. The spend policy
//! (`super::policy`) sums these records for its rolling caps, so a crash
//! between the two writes leaves a `pending` record that still counts.
//!
//! Records live in workspace storage on the `tinystoragedrivers` key-value
//! port (`storage::state_kv`, owner `wallet`, namespace `spend_ledger`), the
//! same backend and acting scope as every other host state value, so they
//! survive a restart and follow `[storage] url`. Keys sort by creation time.

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::storage::state_kv::StateKv;

const OWNER: &str = "wallet";
const NAMESPACE: &str = "spend_ledger";

/// Where one execute is in its lifecycle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LedgerStatus {
    /// Recorded; the module has not answered yet. A record left here by a
    /// crash may have been broadcast and is treated like `in_doubt`.
    Pending,
    /// The module returned a transaction signature.
    Broadcast,
    /// The module call failed after the record was written: the transfer may
    /// or may not be on chain. Never retried automatically.
    InDoubt,
    /// Refused by the host before anything was signed.
    Refused,
}

impl LedgerStatus {
    /// Whether the record may have moved funds, and so counts against caps.
    #[must_use]
    pub fn counts_against_caps(self) -> bool {
        !matches!(self, Self::Refused)
    }
}

/// One execute attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LedgerRecord {
    /// Stable record id.
    pub id: String,
    /// The module quote the execute named.
    pub quote_id: String,
    /// When the record was written, epoch milliseconds.
    pub created_at_ms: u64,
    /// When the record last changed, epoch milliseconds.
    pub updated_at_ms: u64,
    /// `interactive` or `automation`.
    pub origin: String,
    /// PII-free turn origin class (`WebChat`, `TrustedAutomation(Cron)`, `rpc`).
    pub origin_class: String,
    /// Whether the spend policy's caps were applied to this execute.
    pub policy_applied: bool,
    /// Chain family (`solana`, `evm`, ...).
    pub chain: String,
    /// EVM network, for EVM transfers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evm_network: Option<String>,
    /// Asset ticker from the quote.
    pub asset_symbol: String,
    /// Token contract or mint, for token transfers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_address: Option<String>,
    /// Recipient address.
    pub to_address: String,
    /// Amount in atomic units, as a decimal string.
    pub amount_raw: String,
    /// Lifecycle status.
    pub status: LedgerStatus,
    /// Transaction hash or signature, once broadcast.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_hash: Option<String>,
    /// Why the execute was refused or failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl LedgerRecord {
    fn key(&self) -> String {
        format!("{:020}-{}", self.created_at_ms, self.id)
    }

    /// The amount as an integer; an unparseable amount reads as the maximum so
    /// it can only make a cap check stricter.
    #[must_use]
    pub fn amount(&self) -> u128 {
        self.amount_raw.trim().parse().unwrap_or(u128::MAX)
    }
}

/// The ledger of one workspace, in the acting storage scope.
#[derive(Clone)]
pub struct SpendLedger {
    kv: StateKv,
}

impl SpendLedger {
    /// Open the ledger for `workspace_dir`.
    ///
    /// # Errors
    ///
    /// When the storage backend cannot be opened.
    pub fn open(workspace_dir: &Path) -> Result<Self> {
        Ok(Self {
            kv: StateKv::open(workspace_dir, OWNER, NAMESPACE)?,
        })
    }

    /// Insert or replace `record`.
    ///
    /// # Errors
    ///
    /// A storage error.
    pub fn put(&self, record: &LedgerRecord) -> Result<()> {
        tracing::debug!(
            record_id = %record.id,
            status = ?record.status,
            "[wallet:ledger] record written"
        );
        self.kv.put(&record.key(), record)
    }

    /// Every record, oldest first.
    ///
    /// # Errors
    ///
    /// A storage error or an unreadable record.
    pub fn records(&self) -> Result<Vec<LedgerRecord>> {
        Ok(self
            .kv
            .list::<LedgerRecord>("")?
            .into_iter()
            .map(|(_, record)| record)
            .collect())
    }

    /// The newest `limit` records, newest first.
    ///
    /// # Errors
    ///
    /// A storage error or an unreadable record.
    pub fn recent(&self, limit: usize) -> Result<Vec<LedgerRecord>> {
        let mut records = self.records()?;
        records.reverse();
        records.truncate(limit);
        Ok(records)
    }

    /// The record of an earlier attempt to execute `quote_id`, if any that may
    /// have moved funds.
    ///
    /// # Errors
    ///
    /// A storage error or an unreadable record.
    pub fn attempt_for_quote(&self, quote_id: &str) -> Result<Option<LedgerRecord>> {
        Ok(self
            .records()?
            .into_iter()
            .rev()
            .find(|record| record.quote_id == quote_id && record.status.counts_against_caps()))
    }
}

#[cfg(test)]
#[path = "ledger_tests.rs"]
mod tests;
