
/// Decrypt and return the stored recovery phrase for the current wallet.
///
/// This is a read-only operation — it never writes to disk or the keychain.
/// The plaintext phrase is returned only in the RPC response and must be kept
/// in transient React state on the frontend; it must never be logged or persisted.
pub async fn reveal_recovery_phrase() -> Result<RpcOutcome<RevealRecoveryPhraseResult>, String> {
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

    let phrase = crate::openhuman::security::credentials::ops::decrypt_secret(&config, &ciphertext)
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

    Ok(RpcOutcome::new(
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
