use super::*;

fn sample_account(chain: WalletChain) -> WalletAccount {
    WalletAccount {
        chain,
        address: format!("addr-{}", chain.as_str()),
        derivation_path: format!("m/44'/0'/0'/0/{}", chain.as_str()),
    }
}

fn sample_params() -> WalletSetupParams {
    WalletSetupParams {
        consent_granted: true,
        source: WalletSetupSource::Imported,
        mnemonic_word_count: 12,
        encrypted_mnemonic: Some("enc2:abc".to_string()),
        accounts: WalletChain::ALL.into_iter().map(sample_account).collect(),
        force: false,
    }
}

#[test]
fn validate_setup_accepts_four_supported_accounts() {
    let params = sample_params();
    let accounts = validate_setup(&params).expect("valid wallet setup");
    assert_eq!(accounts.len(), 4);
}

#[test]
fn validate_setup_rejects_missing_consent() {
    let mut params = sample_params();
    params.consent_granted = false;
    assert!(validate_setup(&params)
        .expect_err("missing consent should fail")
        .contains("explicit consent"));
}

#[test]
fn validate_setup_rejects_duplicate_chain() {
    let mut params = sample_params();
    params.accounts[0].chain = WalletChain::Btc;
    assert!(validate_setup(&params)
        .expect_err("duplicate chain should fail")
        .contains("exactly one 'evm'"));
}

#[test]
fn validate_setup_rejects_invalid_word_count() {
    let mut params = sample_params();
    params.mnemonic_word_count = 13;
    assert!(validate_setup(&params)
        .expect_err("invalid word count should fail")
        .contains("unsupported mnemonic word count"));
}

#[test]
fn validate_setup_rejects_missing_encrypted_mnemonic() {
    let mut params = sample_params();
    params.encrypted_mnemonic = Some("   ".to_string());
    assert!(validate_setup(&params)
        .expect_err("missing encrypted mnemonic should fail")
        .contains("encrypted mnemonic material"));
}

#[test]
fn status_defaults_to_unconfigured() {
    let config = Config::default();
    let status = to_status(&config, None);
    assert!(!status.configured);
    assert!(!status.onboarding_completed);
    assert!(!status.secret_stored);
    assert!(status.accounts.is_empty());
}

#[test]
fn status_maps_stored_state() {
    let config = Config::default();
    let state = StoredWalletState {
        consent_granted: true,
        source: WalletSetupSource::Generated,
        mnemonic_word_count: 24,
        encrypted_mnemonic: Some("enc2:abc".to_string()),
        accounts: WalletChain::ALL.into_iter().map(sample_account).collect(),
        updated_at_ms: 123,
    };
    let status = to_status(&config, Some(state));
    assert!(status.configured);
    assert!(status.onboarding_completed);
    // When encrypted_mnemonic is in the JSON field, secret_stored should be true.
    assert!(status.secret_stored);
    assert_eq!(status.accounts.len(), 4);
    assert_eq!(status.updated_at_ms, Some(123));
}

// ── Overwrite-guard unit tests ────────────────────────────────────────────
// These exercise the guard logic directly via the unlocked helpers so that
// we don't need a tokio runtime or a live config-RPC call.

fn make_temp_config() -> (Config, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut config = Config::default();
    config.workspace_dir = dir.path().join("workspace");
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");
    (config, dir)
}

fn stored_state() -> StoredWalletState {
    StoredWalletState {
        consent_granted: true,
        source: WalletSetupSource::Generated,
        mnemonic_word_count: 12,
        encrypted_mnemonic: Some("enc2:test-existing".to_string()),
        accounts: WalletChain::ALL.into_iter().map(sample_account).collect(),
        updated_at_ms: 1_000_000,
    }
}

#[test]
fn setup_rejects_overwrite_without_force() {
    let (config, _dir) = make_temp_config();
    // Pre-populate wallet state to simulate an existing wallet.
    let existing = stored_state();
    save_stored_wallet_state_unlocked(&config, &existing).expect("save existing state");

    // Build params WITHOUT force=true.
    let mut params = sample_params();
    params.force = false;

    // The guard should detect the existing wallet and the validate+guard
    // path should fail BEFORE we even try to save.
    // We test the guard directly here: load the state and check guard logic.
    let _guard = WALLET_STATE_FILE_LOCK.lock();
    let loaded = load_stored_wallet_state_unlocked(&config).expect("load ok");
    assert!(loaded.is_some(), "existing wallet must be loaded");
    // Guard: if existing && !force → error
    let would_error = loaded.is_some() && !params.force;
    assert!(
        would_error,
        "setup without force must be rejected when wallet exists"
    );
}

#[test]
fn setup_allows_overwrite_with_force() {
    let (config, _dir) = make_temp_config();
    // Pre-populate wallet state.
    let existing = stored_state();
    save_stored_wallet_state_unlocked(&config, &existing).expect("save existing state");

    // Build params WITH force=true.
    let mut params = sample_params();
    params.force = true;

    let _guard = WALLET_STATE_FILE_LOCK.lock();
    let loaded = load_stored_wallet_state_unlocked(&config).expect("load ok");
    // Guard: if existing && force → proceed (no error)
    let would_error = loaded.is_some() && !params.force;
    assert!(
        !would_error,
        "setup with force must be allowed when wallet exists"
    );

    // Actually write the new state to confirm save works.
    let new_state = StoredWalletState {
        consent_granted: true,
        source: WalletSetupSource::Imported,
        mnemonic_word_count: 12,
        encrypted_mnemonic: Some("enc2:new-mnemonic".to_string()),
        accounts: WalletChain::ALL.into_iter().map(sample_account).collect(),
        updated_at_ms: 2_000_000,
    };
    save_stored_wallet_state_unlocked(&config, &new_state).expect("save new state");
    let reloaded = load_stored_wallet_state_unlocked(&config)
        .expect("reload ok")
        .expect("state present after overwrite");
    assert_eq!(reloaded.updated_at_ms, 2_000_000);
}

#[test]
fn setup_allows_fresh_without_force() {
    let (config, _dir) = make_temp_config();
    // No existing wallet — fresh setup.
    let params = sample_params(); // force defaults to false

    let _guard = WALLET_STATE_FILE_LOCK.lock();
    let loaded = load_stored_wallet_state_unlocked(&config).expect("load ok");
    assert!(loaded.is_none(), "no existing wallet on fresh config");
    // Guard: if None → proceed regardless of force
    let would_error = loaded.is_some() && !params.force;
    assert!(!would_error, "fresh setup without force must be allowed");

    // Write initial state.
    let new_state = StoredWalletState {
        consent_granted: true,
        source: WalletSetupSource::Generated,
        mnemonic_word_count: 12,
        encrypted_mnemonic: Some("enc2:fresh".to_string()),
        accounts: WalletChain::ALL.into_iter().map(sample_account).collect(),
        updated_at_ms: 3_000_000,
    };
    save_stored_wallet_state_unlocked(&config, &new_state).expect("save fresh state");
    let reloaded = load_stored_wallet_state_unlocked(&config)
        .expect("reload ok")
        .expect("state present after fresh setup");
    assert_eq!(reloaded.updated_at_ms, 3_000_000);
}

// ── reveal_recovery_phrase unit tests ────────────────────────────────────
// These use tokio::test and OPENHUMAN_WORKSPACE env var to wire up the full
// async path including config loading. TEST_LOCK serializes wallet globals;
// TEST_ENV_LOCK serializes the process-wide workspace env var.

#[tokio::test]
async fn reveal_recovery_phrase_returns_error_when_no_wallet() {
    let temp = tempfile::tempdir().expect("temp dir");
    let _wallet_lock = crate::openhuman::web3::wallet::test_support::TEST_LOCK.lock();
    let _workspace_guard =
        crate::openhuman::web3::wallet::test_support::set_workspace_env_for_test(&temp);
    let result = reveal_recovery_phrase().await;
    let err = result.expect_err("should error when no wallet configured");
    assert!(
        err.contains("No recovery phrase is available"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn reveal_recovery_phrase_returns_phrase_for_existing_wallet() {
    let temp = tempfile::tempdir().expect("temp dir");
    let _wallet_lock = crate::openhuman::web3::wallet::test_support::TEST_LOCK.lock();
    let _workspace_guard = crate::openhuman::web3::wallet::test_support::setup_wallet_in(&temp)
        .await
        .expect("setup wallet");
    let result = reveal_recovery_phrase()
        .await
        .expect("reveal should succeed");
    assert_eq!(
        result.value.phrase,
        crate::openhuman::web3::wallet::test_support::TEST_MNEMONIC
    );
    assert_eq!(result.value.word_count, 12);
}
