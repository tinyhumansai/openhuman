//! Controller operations for the spend policy and its ledger:
//! `wallet.policy_get`, `wallet.policy_update` and `wallet.ledger_list`.

use serde::Deserialize;

use crate::config::schema::{WalletAssetCap, WalletPolicyConfig};
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;

use super::ledger::{LedgerRecord, SpendLedger};

/// The largest page `ledger_list` returns.
pub const LEDGER_LIST_MAX: usize = 500;
const LEDGER_LIST_DEFAULT: usize = 50;
const CHAINS: [&str; 4] = ["evm", "btc", "solana", "tron"];

/// A partial policy update; absent fields keep their value. `assets`, when
/// present, replaces the whole list.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WalletPolicyPatch {
    #[serde(default)]
    pub automation_enabled: Option<bool>,
    #[serde(default)]
    pub apply_to_interactive: Option<bool>,
    #[serde(default)]
    pub assets: Option<Vec<WalletAssetCap>>,
}

/// Reject an asset cap the policy could never match, so a typo cannot read as
/// "allowed" or silently disable a cap.
pub fn validate_assets(assets: &[WalletAssetCap]) -> Result<(), String> {
    for (index, asset) in assets.iter().enumerate() {
        if !CHAINS.contains(&asset.chain.to_ascii_lowercase().as_str()) {
            return Err(format!(
                "assets[{index}].chain must be one of {}",
                CHAINS.join(", ")
            ));
        }
        if asset.symbol.trim().is_empty() {
            return Err(format!("assets[{index}].symbol is required"));
        }
    }
    Ok(())
}

/// Apply `patch` to `policy`.
pub fn apply_patch(
    policy: &mut WalletPolicyConfig,
    patch: WalletPolicyPatch,
) -> Result<(), String> {
    if let Some(assets) = &patch.assets {
        validate_assets(assets)?;
    }
    if let Some(value) = patch.automation_enabled {
        policy.automation_enabled = value;
    }
    if let Some(value) = patch.apply_to_interactive {
        policy.apply_to_interactive = value;
    }
    if let Some(assets) = patch.assets {
        policy.assets = assets;
    }
    Ok(())
}

/// The effective policy (config file plus environment overrides).
pub async fn policy_get(ctx: &DispatchCtx) -> Result<Outcome<WalletPolicyConfig>, String> {
    let config = ctx.load_config().await?;
    Ok(Outcome::new(
        config.wallet_policy,
        vec!["wallet policy read".to_string()],
    ))
}

/// Update and persist the policy in `config.toml`. Environment overrides still
/// win at the next load.
pub async fn policy_update(
    ctx: &DispatchCtx,
    patch: WalletPolicyPatch,
) -> Result<Outcome<WalletPolicyConfig>, String> {
    let mut config = ctx.load_config().await?;
    apply_patch(&mut config.wallet_policy, patch)?;
    config.save().await.map_err(|error| {
        crate::config::ops::config_save_error_with_full_context(
            error,
            "failed to save wallet policy",
            true,
        )
    })?;
    tracing::info!(
        automation_enabled = config.wallet_policy.automation_enabled,
        apply_to_interactive = config.wallet_policy.apply_to_interactive,
        assets = config.wallet_policy.assets.len(),
        "[wallet:policy] policy updated"
    );
    Ok(Outcome::new(
        config.wallet_policy,
        vec!["wallet policy updated".to_string()],
    ))
}

/// The newest ledger records, newest first.
pub async fn ledger_list(
    ctx: &DispatchCtx,
    limit: Option<usize>,
) -> Result<Outcome<Vec<LedgerRecord>>, String> {
    let config = ctx.load_config().await?;
    let limit = limit
        .unwrap_or(LEDGER_LIST_DEFAULT)
        .clamp(1, LEDGER_LIST_MAX);
    let records = SpendLedger::open(&config.workspace_dir)
        .and_then(|ledger| ledger.recent(limit))
        .map_err(|error| format!("wallet spend ledger is unavailable: {error:#}"))?;
    Ok(Outcome::new(
        records,
        vec!["wallet ledger listed".to_string()],
    ))
}

#[cfg(test)]
#[path = "policy_ops_tests.rs"]
mod tests;
