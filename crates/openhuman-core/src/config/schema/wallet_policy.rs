//! Native-wallet spend policy (`[wallet_policy]`).
//!
//! Caps what a prepared native or token transfer may move when OpenHuman
//! executes it without a person confirming that exact transfer. The policy is
//! read by `web3::wallet::policy` at every `wallet_execute_prepared`, and the
//! ledger it checks against lives in workspace storage (`web3::wallet::ledger`).
//!
//! The default keeps today's behaviour for people: interactive executes (the
//! wallet UI, a WebChat turn the user approved) are not capped unless
//! `apply_to_interactive` is set, and trusted automation (cron, background
//! jobs, approval-free workflows) cannot execute a transfer at all until
//! `automation_enabled` is set **and** the asset is listed in `assets`.
//!
//! Amounts are atomic units of the asset (lamports for SOL, 10^-6 for USDC,
//! wei for ETH). There is no price oracle: a dollar cap is written as the
//! atomic amount it corresponds to. Fees are not counted against a cap.
//!
//! Windows are rolling, measured from the execute time: `daily_max_raw` is the
//! last 24 hours, `recipient_weekly_max_raw` is the last 7 days for one
//! recipient address.
//!
//! ```toml
//! [wallet_policy]
//! automation_enabled = true
//!
//! [[wallet_policy.assets]]
//! chain = "solana"
//! symbol = "USDC"
//! per_tx_max_raw = 50000000            # 50 USDC
//! daily_max_raw = 100000000            # 100 USDC per rolling 24h
//! recipient_weekly_max_raw = 50000000  # 50 USDC per recipient per rolling 7d
//! ```
//!
//! ## Per-account program policy
//!
//! `assets` above governs the primary account of each chain, which signs the
//! module's transfers. A secondary account (Solana index 1, `m/44'/501'/1'/0'`)
//! and any account used to sign a host-built transaction through
//! `wallet.prepare_program` / `wallet.execute_prepared_program` is governed
//! only by its own section, keyed `<chain>_<index>`. An account with no
//! section is refused for every program execute; the section never affects
//! `assets` or the primary account's transfers.
//!
//! ```toml
//! [wallet_policy.accounts.solana_1]
//! allowed_programs = [
//!   "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",   # Token-2022
//!   "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",  # Associated Token Account
//!   "<distributor program id>",                     # the program being called
//! ]
//!
//! [[wallet_policy.accounts.solana_1.mints]]
//! mint = "<mint address>"
//! per_tx_max_raw = 1000000000   # atomic units of the mint
//! daily_max_raw = 2000000000    # rolling 24h
//! ```
//!
//! Environment overrides (applied at config load, like every other section):
//! `OPENHUMAN_WALLET_POLICY_AUTOMATION_ENABLED`,
//! `OPENHUMAN_WALLET_POLICY_APPLY_TO_INTERACTIVE` (booleans) and
//! `OPENHUMAN_WALLET_POLICY_ASSETS` (a JSON array of asset caps, replacing the
//! `assets` list).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The `[wallet_policy]` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WalletPolicyConfig {
    /// Allow trusted automation (cron jobs, internal background jobs and
    /// workflows that do not require approval) to execute prepared transfers
    /// within the caps. Off by default: automation cannot move funds.
    pub automation_enabled: bool,

    /// Also apply the allowlist and caps to interactive executes (the wallet
    /// UI and approved WebChat turns). Off by default, which keeps the
    /// interactive flow exactly as it was.
    pub apply_to_interactive: bool,

    /// The assets a capped execute may move, each with its caps. An asset that
    /// is not listed is refused for every capped execute.
    pub assets: Vec<WalletAssetCap>,

    /// Per-account policy for host-built program transactions, keyed
    /// `<chain>_<index>` (`solana_1`). An account with no entry cannot sign a
    /// program transaction. Independent of `assets`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, WalletAccountPolicy>,
}

impl WalletPolicyConfig {
    /// The key of `chain`'s account `index` in [`Self::accounts`].
    #[must_use]
    pub fn account_key(chain: &str, index: u32) -> String {
        format!("{}_{index}", chain.to_ascii_lowercase())
    }

    /// The program policy of `chain`'s account `index`, if it has one.
    #[must_use]
    pub fn account_policy(&self, chain: &str, index: u32) -> Option<&WalletAccountPolicy> {
        self.accounts.get(&Self::account_key(chain, index))
    }
}

/// What one account may sign through `wallet.execute_prepared_program`.
///
/// Every instruction of the transaction must invoke a listed program, and the
/// token debits the caller declares (and simulation confirms) must be of a
/// listed mint within its caps.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WalletAccountPolicy {
    /// Program ids (base58) the transaction's instructions may invoke.
    pub allowed_programs: Vec<String>,
    /// The mints the account may move, each with its caps.
    pub mints: Vec<WalletMintCap>,
}

/// Caps for one SPL mint. Both caps are required: a program account has no
/// uncapped mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WalletMintCap {
    /// Mint address (base58).
    pub mint: String,
    /// Largest total of this mint one transaction may move, atomic units.
    pub per_tx_max_raw: u64,
    /// Largest total over the last 24 hours, atomic units.
    pub daily_max_raw: u64,
}

/// The caps for one asset. A cap that is absent does not limit that dimension;
/// listing the asset is what allows it at all.
///
/// Unknown fields are rejected, so a misspelled cap (`daily_max_raww`) fails
/// to parse instead of silently leaving that dimension uncapped.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WalletAssetCap {
    /// Chain family: `evm`, `btc`, `solana` or `tron`.
    pub chain: String,
    /// Asset ticker as the wallet catalog names it (`SOL`, `USDC`, `ETH`).
    /// Compared case-insensitively.
    pub symbol: String,
    /// Optional EVM network (`base_mainnet`, ...). Absent matches every EVM
    /// network, and their spend is counted together.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_network: Option<String>,
    /// Optional contract or mint address. When set, a quote for the same
    /// symbol with another token address is refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_address: Option<String>,
    /// Largest single transfer, atomic units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_tx_max_raw: Option<u64>,
    /// Largest total over the last 24 hours, atomic units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_max_raw: Option<u64>,
    /// Largest total to one recipient over the last 7 days, atomic units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient_weekly_max_raw: Option<u64>,
}

#[cfg(test)]
#[path = "wallet_policy_tests.rs"]
mod tests;
