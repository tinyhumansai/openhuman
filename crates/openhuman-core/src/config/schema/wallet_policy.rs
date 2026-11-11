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
//! Environment overrides (applied at config load, like every other section):
//! `OPENHUMAN_WALLET_POLICY_AUTOMATION_ENABLED`,
//! `OPENHUMAN_WALLET_POLICY_APPLY_TO_INTERACTIVE` (booleans) and
//! `OPENHUMAN_WALLET_POLICY_ASSETS` (a JSON array of asset caps, replacing the
//! `assets` list).

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
