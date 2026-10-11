//! Wallet setup, status, and secret-material business operations.
//!
//! [`types`] holds the serde domain types shared across this module,
//! [`state`] owns the on-disk / keychain persistence for setup state, and
//! this file exposes the actual RPC-facing operations.

mod state;
mod types;

use types::StoredWalletState;
pub(crate) use types::WalletSecretMaterial;
use {
    tinywallet_web3::crypto::wallet::WalletAccount, tinywallet_web3::crypto::wallet::WalletChain,
    tinywallet_web3::crypto::wallet::WalletStatus,
    tinywallet_web3::quote::WALLET_NOT_CONFIGURED_MESSAGE,
};
pub use {types::RevealRecoveryPhraseResult, types::WalletSetupParams};

use log::debug;

use crate::config::rpc as config_rpc;
use crate::core::Outcome;

use state::{
    keychain_load_mnemonic, load_stored_wallet_state_unlocked, save_stored_wallet_state_unlocked,
    to_status, LOG_PREFIX, WALLET_STATE_FILE_LOCK,
};

const VALID_MNEMONIC_WORD_COUNTS: [u8; 5] = [12, 15, 18, 21, 24];

fn validate_setup(params: &WalletSetupParams) -> Result<Vec<WalletAccount>, String> {
    if !params.consent_granted {
        return Err("wallet setup requires explicit consent".to_string());
    }
    if !VALID_MNEMONIC_WORD_COUNTS.contains(&params.mnemonic_word_count) {
        return Err(format!(
            "unsupported mnemonic word count {}; expected one of {}",
            params.mnemonic_word_count,
            VALID_MNEMONIC_WORD_COUNTS
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if params
        .encrypted_mnemonic
        .as_ref()
        .map(|value| value.trim().is_empty())
        .unwrap_or(true)
    {
        return Err(
            "wallet setup requires encrypted mnemonic material for signing-enabled local wallets"
                .to_string(),
        );
    }

    let mut normalized = Vec::with_capacity(params.accounts.len());
    for account in &params.accounts {
        let address = account.address.trim();
        let derivation_path = account.derivation_path.trim();
        if address.is_empty() {
            return Err(format!(
                "wallet setup account '{}' is missing an address",
                account.chain.as_str()
            ));
        }
        if derivation_path.is_empty() {
            return Err(format!(
                "wallet setup account '{}' is missing a derivation path",
                account.chain.as_str()
            ));
        }
        normalized.push(WalletAccount {
            chain: account.chain,
            address: address.to_string(),
            derivation_path: derivation_path.to_string(),
        });
    }

    for chain in WalletChain::ALL {
        let count = normalized
            .iter()
            .filter(|account| account.chain == chain)
            .count();
        if count != 1 {
            return Err(format!(
                "wallet setup must include exactly one '{}' account",
                chain.as_str()
            ));
        }
    }

    Ok(normalized)
}

pub async fn status() -> Result<Outcome<WalletStatus>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let _guard = WALLET_STATE_FILE_LOCK.lock();
    let status = to_status(&config, load_stored_wallet_state_unlocked(&config)?);

    debug!(
        "{LOG_PREFIX} status configured={} onboarding_completed={} account_count={}",
        status.configured,
        status.onboarding_completed,
        status.accounts.len()
    );

    Ok(Outcome::new(
        status,
        vec!["wallet status fetched".to_string()],
    ))
}

pub async fn setup(params: WalletSetupParams) -> Result<Outcome<WalletStatus>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let accounts = validate_setup(&params)?;
    let encrypted_mnemonic = params
        .encrypted_mnemonic
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "wallet setup requires encrypted mnemonic material for signing-enabled local wallets"
                .to_string()
        })?;

    // ── Idempotency guard: reject overwrite unless caller explicitly set force=true ──
    // Acquire lock before the existence check so no TOCTOU race is possible.
    let _guard = WALLET_STATE_FILE_LOCK.lock();
    if let Some(_existing) = load_stored_wallet_state_unlocked(&config)? {
        if !params.force {
            debug!("{LOG_PREFIX} setup rejected: wallet already configured and force=false");
            return Err("wallet is already configured; pass force=true to overwrite".to_string());
        }
        debug!("{LOG_PREFIX} setup: overwriting existing wallet (force=true)");
    }

    let state = StoredWalletState {
        consent_granted: params.consent_granted,
        source: params.source,
        mnemonic_word_count: params.mnemonic_word_count,
        encrypted_mnemonic: Some(encrypted_mnemonic),
        accounts,
        updated_at_ms: state::current_time_ms(),
    };

    save_stored_wallet_state_unlocked(&config, &state)?;
    let status = to_status(&config, Some(state));

    debug!(
        "{LOG_PREFIX} setup saved source={:?} account_count={} mnemonic_words={} secret_stored={}",
        status.source,
        status.accounts.len(),
        status.mnemonic_word_count.unwrap_or_default(),
        status.secret_stored
    );

    Ok(Outcome::new(status, vec!["wallet setup saved".to_string()]))
}

/// Decrypt and return the stored recovery phrase for the current wallet.
///
/// This is a read-only operation — it never writes to disk or the keychain.
/// The plaintext phrase is returned only in the RPC response and must be kept
/// in transient React state on the frontend; it must never be logged or persisted.
pub async fn reveal_recovery_phrase() -> Result<Outcome<RevealRecoveryPhraseResult>, String> {
    debug!("{LOG_PREFIX} reveal_recovery_phrase ENTRY");

    let config = config_rpc::load_config_with_timeout().await.map_err(|e| {
        log::warn!("{LOG_PREFIX} reveal_recovery_phrase config load failed: {e}");
        e
    })?;

    // Acquire the lock to load state, then drop it before any await point.
    // parking_lot::MutexGuard is not Send, so it must not be held across awaits.
    let ciphertext = {
        let _guard = WALLET_STATE_FILE_LOCK.lock();
        debug!("{LOG_PREFIX} reveal_recovery_phrase state lock acquired");

        let state = match load_stored_wallet_state_unlocked(&config)? {
            Some(s) => s,
            None => {
                debug!("{LOG_PREFIX} reveal_recovery_phrase no wallet state found");
                return Err(
                    "No recovery phrase is available to reveal. Set up or unlock your wallet first."
                        .to_string(),
                );
            }
        };

        // Primary path: mnemonic is in the state returned by load (either from
        // the JSON field or merged in from the OS keychain by
        // load_stored_wallet_state_unlocked).  Fallback: probe the keychain
        // directly in case the mnemonic is stored there but was not merged into
        // `state` (e.g. headless / CI keychain that was transiently unavailable
        // during the initial probe inside load_stored_wallet_state_unlocked, or
        // any environment where the mnemonic lives only in the keychain).
        let enc_mnemonic_opt = state
            .encrypted_mnemonic
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                debug!(
                    "{LOG_PREFIX} reveal_recovery_phrase: mnemonic absent from state, \
                     falling back to direct keychain probe"
                );
                keychain_load_mnemonic(&config)
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            });

        enc_mnemonic_opt.ok_or_else(|| {
            debug!("{LOG_PREFIX} reveal_recovery_phrase encrypted mnemonic missing from state");
            "No recovery phrase is available to reveal. Set up or unlock your wallet first."
                .to_string()
        })?
        // _guard dropped here — before the decrypt await below
    };

    debug!("{LOG_PREFIX} reveal_recovery_phrase decrypting mnemonic");

    let phrase = crate::security::credentials::ops::decrypt_secret(&config, &ciphertext)
        .await
        .map_err(|e| {
            log::warn!("{LOG_PREFIX} reveal_recovery_phrase decrypt failed: {e}");
            format!("Failed to decrypt recovery phrase: {e}")
        })?
        .value;

    let word_count = phrase.split_whitespace().count();

    debug!(
        "{LOG_PREFIX} reveal_recovery_phrase OK word_count={}",
        word_count
    );

    Ok(Outcome::new(
        RevealRecoveryPhraseResult { phrase, word_count },
        vec!["recovery phrase revealed".to_string()],
    ))
}

pub(crate) async fn secret_material(chain: WalletChain) -> Result<WalletSecretMaterial, String> {
    debug!(
        "{LOG_PREFIX} secret_material loading config chain={}",
        chain.as_str()
    );
    let config = config_rpc::load_config_with_timeout().await?;
    debug!(
        "{LOG_PREFIX} secret_material acquiring state lock chain={}",
        chain.as_str()
    );
    let _guard = WALLET_STATE_FILE_LOCK.lock();
    let state = match load_stored_wallet_state_unlocked(&config)? {
        Some(state) => state,
        None => {
            debug!(
                "{LOG_PREFIX} secret_material missing wallet state chain={}",
                chain.as_str()
            );
            return Err(WALLET_NOT_CONFIGURED_MESSAGE.to_string());
        }
    };
    let encrypted_mnemonic = state
        .encrypted_mnemonic
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            debug!(
                "{LOG_PREFIX} secret_material missing encrypted mnemonic chain={}",
                chain.as_str()
            );
            "wallet secret material is missing; re-import the recovery phrase to enable signing"
                .to_string()
        })?;
    let derivation_path = state
        .accounts
        .iter()
        .find(|account| account.chain == chain)
        .map(|account| account.derivation_path.clone())
        .ok_or_else(|| {
            debug!(
                "{LOG_PREFIX} secret_material missing account chain={}",
                chain.as_str()
            );
            format!("no wallet account derived for chain '{}'", chain.as_str())
        })?;
    debug!(
        "{LOG_PREFIX} secret_material loaded chain={} derivation_path={}",
        chain.as_str(),
        derivation_path
    );
    Ok(WalletSecretMaterial {
        encrypted_mnemonic,
        derivation_path,
    })
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
