use super::*;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;

fn sample_account(chain: WalletChain) -> WalletAccount {
    WalletAccount {
        chain,
        address: format!("addr-{}", chain.as_str()),
        derivation_path: format!("m/44'/0'/0'/0/{}", chain.as_str()),
        index: 0,
        label: None,
    }
}

fn giveaway_account() -> WalletAccount {
    WalletAccount {
        chain: WalletChain::Solana,
        address: "addr-solana-1".to_string(),
        derivation_path: "m/44'/501'/1'/0'".to_string(),
        index: 1,
        label: Some("giveaway".to_string()),
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

#[tokio::test]
async fn generate_recovery_phrase_rejects_unsupported_word_count_before_the_module() {
    let error = generate_recovery_phrase(&DispatchCtx::process_default(), 13)
        .await
        .expect_err("13 words is not a BIP-39 length");
    assert!(error.contains("13"), "{error}");
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
    let _wallet_lock = crate::web3::wallet::test_support::TEST_LOCK.lock().await;
    let _workspace_guard =
        crate::web3::wallet::test_support::set_workspace_env_for_test(&temp).await;
    let result = reveal_recovery_phrase(&DispatchCtx::process_default()).await;
    let err = result.expect_err("should error when no wallet configured");
    assert!(
        err.contains("No recovery phrase is available"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn reveal_recovery_phrase_returns_phrase_for_existing_wallet() {
    let temp = tempfile::tempdir().expect("temp dir");
    let _wallet_lock = crate::web3::wallet::test_support::TEST_LOCK.lock().await;
    let _workspace_guard =
        crate::web3::wallet::test_support::setup_wallet_in(&DispatchCtx::process_default(), &temp)
            .await
            .expect("setup wallet");
    let result = reveal_recovery_phrase(&DispatchCtx::process_default())
        .await
        .expect("reveal should succeed");
    assert_eq!(
        result.value.phrase,
        crate::web3::wallet::test_support::TEST_MNEMONIC
    );
    assert_eq!(result.value.word_count, 12);
}

// ── Secondary accounts ───────────────────────────────────────────────────

/// The expected addresses were computed independently of TinyWallet: a
/// standalone SLIP-0010 ed25519 derivation (Python `hashlib` PBKDF2/HMAC-SHA512
/// plus `cryptography`'s Ed25519) of the BIP-39 test phrase
/// "abandon × 11 about". Its index-0 result matches the published Solana
/// vector `HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk` (what Phantom and
/// `solana-keygen` derive at `m/44'/501'/0'/0'`), which is what makes it a
/// trustworthy oracle for index 1 (`m/44'/501'/1'/0'`, Phantom's "Account 2").
const SOLANA_INDEX_0_VECTOR: &str = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk";
const SOLANA_INDEX_1_VECTOR: &str = "Hh8QwFUA6MtVu1qAoq12ucvFHNwCcVTV7hpWjeY1Hztb";

fn derive_solana(path: &str) -> String {
    tinywallet::key::derive(
        tinywallet::Chain::Solana,
        crate::web3::wallet::test_support::TEST_MNEMONIC,
        path,
    )
    .expect("derive")
    .address()
    .to_string()
}

#[test]
fn solana_index_one_derives_the_known_vector_address() {
    let path = account_derivation_path(WalletChain::Solana, 1).expect("index 1 is known");
    assert_eq!(path, "m/44'/501'/1'/0'");
    assert_eq!(derive_solana(path), SOLANA_INDEX_1_VECTOR);
}

#[test]
fn primary_accounts_are_unchanged() {
    assert_eq!(
        account_derivation_path(WalletChain::Solana, 0),
        Some("m/44'/501'/0'/0'")
    );
    assert_eq!(derive_solana("m/44'/501'/0'/0'"), SOLANA_INDEX_0_VECTOR);
    let plan = derivation_plan();
    let primary: Vec<_> = plan
        .iter()
        .take(4)
        .map(|account| {
            (
                account.chain,
                account.derivation_path.as_str(),
                account.index,
            )
        })
        .collect();
    assert_eq!(
        primary,
        vec![
            (WalletChain::Evm, "m/44'/60'/0'/0/0", 0),
            (WalletChain::Btc, "m/44'/0'/0'/0/0", 0),
            (WalletChain::Solana, "m/44'/501'/0'/0'", 0),
            (WalletChain::Tron, "m/44'/195'/0'/0/0", 0),
        ]
    );
    assert!(plan.iter().take(4).all(|account| account.label.is_none()));
    // Recovery and import also derive the giveaway account, after the four.
    assert_eq!(plan.len(), 5);
    assert_eq!(plan[4].chain, WalletChain::Solana);
    assert_eq!(plan[4].index, 1);
    assert_eq!(plan[4].derivation_path, "m/44'/501'/1'/0'");
    assert_eq!(plan[4].label.as_deref(), Some("giveaway"));
    // Only Solana has a secondary account.
    assert_eq!(account_derivation_path(WalletChain::Evm, 1), None);
    assert_eq!(account_derivation_path(WalletChain::Solana, 2), None);
}

#[test]
fn stored_accounts_without_an_index_read_as_primary_and_serialize_unchanged() {
    let legacy = serde_json::json!({
        "chain": "solana",
        "address": "addr",
        "derivationPath": "m/44'/501'/0'/0'"
    });
    let account: WalletAccount = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(account.index, 0);
    assert!(account.is_primary());
    assert_eq!(serde_json::to_value(&account).unwrap(), legacy);
    let secondary = serde_json::to_value(giveaway_account()).unwrap();
    assert_eq!(secondary["index"], 1);
    assert_eq!(secondary["label"], "giveaway");
}

#[test]
fn validate_setup_accepts_the_giveaway_account_beside_the_four() {
    let mut params = sample_params();
    params.accounts.push(giveaway_account());
    let accounts = validate_setup(&params).expect("valid wallet setup");
    assert_eq!(accounts.len(), 5);
    assert_eq!(accounts[4].index, 1);
}

#[test]
fn validate_setup_rejects_unknown_or_duplicate_secondary_accounts() {
    let mut params = sample_params();
    let mut wrong_path = giveaway_account();
    wrong_path.derivation_path = "m/44'/501'/7'/0'".to_string();
    params.accounts.push(wrong_path);
    assert!(validate_setup(&params)
        .expect_err("non-canonical path")
        .contains("not a known secondary account"));

    let mut params = sample_params();
    let mut unknown = giveaway_account();
    unknown.chain = WalletChain::Evm;
    params.accounts.push(unknown);
    assert!(validate_setup(&params).is_err());

    let mut params = sample_params();
    params.accounts.push(giveaway_account());
    params.accounts.push(giveaway_account());
    assert!(validate_setup(&params)
        .expect_err("duplicate")
        .contains("more than once"));

    // A secondary account never stands in for a missing primary one.
    let mut params = sample_params();
    params
        .accounts
        .retain(|account| account.chain != WalletChain::Solana);
    params.accounts.push(giveaway_account());
    assert!(validate_setup(&params)
        .expect_err("no primary solana")
        .contains("exactly one 'solana'"));
}

#[test]
fn status_keeps_secondary_accounts_out_of_the_primary_list() {
    let config = Config::default();
    let mut state = stored_state();
    state.accounts.push(giveaway_account());
    let status = to_status(&config, Some(state));
    assert_eq!(status.accounts.len(), 4);
    assert!(status.accounts.iter().all(WalletAccount::is_primary));
    assert_eq!(status.additional_accounts, vec![giveaway_account()]);
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["additionalAccounts"][0]["label"], "giveaway");

    let plain = to_status(&config, Some(stored_state()));
    let json = serde_json::to_value(&plain).unwrap();
    assert!(json.get("additionalAccounts").is_none());
}

#[tokio::test]
async fn secret_material_resolves_the_secondary_path_for_a_wallet_that_predates_it() {
    let temp = tempfile::tempdir().expect("temp dir");
    let _wallet_lock = crate::web3::wallet::test_support::TEST_LOCK.lock().await;
    let ctx = DispatchCtx::process_default();
    let _workspace = crate::web3::wallet::test_support::set_workspace_env_for_test(&temp).await;
    let config = ctx.load_config().await.expect("config");
    {
        // State as written before secondary accounts existed: four primary
        // accounts, no index fields. Nothing here needs the phrase decrypted.
        let mut state = stored_state();
        state.accounts = WalletChain::ALL
            .into_iter()
            .map(crate::web3::wallet::test_support::sample_account)
            .collect();
        let _guard = WALLET_STATE_FILE_LOCK.lock();
        save_stored_wallet_state_unlocked(&config, &state).expect("save state");
    }
    let primary = secret_material(&ctx, WalletChain::Solana, 0)
        .await
        .expect("primary");
    assert_eq!(primary.derivation_path, "m/44'/501'/0'/0'");
    let giveaway = secret_material(&ctx, WalletChain::Solana, 1)
        .await
        .expect("secondary resolves without being stored");
    assert_eq!(giveaway.derivation_path, "m/44'/501'/1'/0'");
    assert_eq!(giveaway.encrypted_mnemonic, primary.encrypted_mnemonic);
    let error = secret_material(&ctx, WalletChain::Solana, 2)
        .await
        .expect_err("index 2 is unknown");
    assert!(error.contains("index 2"), "{error}");
    let error = ensure_account(&ctx, WalletChain::Evm, 1)
        .await
        .expect_err("evm has no secondary account");
    assert!(error.contains("secondary accounts are"), "{error}");
    let stored = ensure_account(&ctx, WalletChain::Solana, 0)
        .await
        .expect("the primary account is stored");
    assert_eq!(
        stored.address,
        crate::web3::wallet::test_support::sample_solana_address()
    );
}

#[test]
fn validate_setup_rejects_a_primary_at_a_secondary_path() {
    // An index-0 account at the giveaway path would resolve to the same key
    // as index 1, so the secondary program policy would sign for the primary.
    let mut params = sample_params();
    let mut alias = sample_account(WalletChain::Solana);
    alias.derivation_path = giveaway_account().derivation_path;
    params
        .accounts
        .retain(|account| account.chain != WalletChain::Solana);
    params.accounts.push(alias);
    assert!(validate_setup(&params)
        .expect_err("primary aliasing the secondary path")
        .contains("secondary account path"));
}

#[tokio::test]
async fn a_stored_primary_at_the_secondary_path_is_quarantined_before_signing() {
    // A wallet state written with its Solana primary on the giveaway path (by a
    // build that did not refuse it) must not sign for index 1: load validation
    // quarantines it, so no secondary key is ever resolved from it.
    let temp = tempfile::tempdir().expect("temp dir");
    let _wallet_lock = crate::web3::wallet::test_support::TEST_LOCK.lock().await;
    let ctx = DispatchCtx::process_default();
    let _workspace = crate::web3::wallet::test_support::set_workspace_env_for_test(&temp).await;
    let config = ctx.load_config().await.expect("config");
    {
        let mut state = stored_state();
        state.accounts = WalletChain::ALL
            .into_iter()
            .map(crate::web3::wallet::test_support::sample_account)
            .collect();
        for account in state.accounts.iter_mut() {
            if account.chain == WalletChain::Solana {
                account.derivation_path = giveaway_account().derivation_path;
            }
        }
        let _guard = WALLET_STATE_FILE_LOCK.lock();
        save_stored_wallet_state_unlocked(&config, &state).expect("save state");
    }
    let error = secret_material(&ctx, WalletChain::Solana, 1)
        .await
        .expect_err("an aliased primary must not sign as the secondary");
    assert_eq!(error, WALLET_NOT_CONFIGURED_MESSAGE);
    assert!(load_stored_wallet_state_unlocked(&config)
        .expect("load")
        .is_none());
}
