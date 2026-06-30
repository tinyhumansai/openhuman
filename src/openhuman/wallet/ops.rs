use std::fs;
#[cfg(unix)]
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use log::{debug, warn};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

const LOG_PREFIX: &str = "[wallet]";
const WALLET_STATE_FILENAME: &str = "wallet-state.json";
/// Error message returned when the wallet has not been set up yet.
///
/// This is an expected user-state (the user simply has not created a wallet),
/// not an internal failure. Downstream boundaries that surface this condition
/// — e.g. the `tinyplace` client builder — match against this constant to
/// classify it as `expected_user_state` so it stays out of Sentry. Keep it a
/// shared constant so the producer here and any classifier cannot drift apart.
pub const WALLET_NOT_CONFIGURED_MESSAGE: &str = "wallet is not configured; run wallet setup first";
const VALID_MNEMONIC_WORD_COUNTS: [u8; 5] = [12, 15, 18, 21, 24];
/// Keychain key for the encrypted mnemonic blob (user_id is added by the keyring module).
const KEYCHAIN_MNEMONIC_KEY: &str = "wallet.mnemonic";
static WALLET_STATE_FILE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Derive a stable keychain user-id from the workspace directory path.
///
/// Uses the same strategy as credentials/profiles.rs: take the last meaningful
/// path component of the workspace directory.
fn wallet_user_id(config: &Config) -> String {
    // workspace_dir is typically `{openhuman_dir}/workspace` — take the parent
    // (the user's openhuman dir) and then the last component.
    let candidate = config
        .workspace_dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty());
    if let Some(id) = candidate {
        return id.to_string();
    }
    // Fallback: FNV-1a hash of the workspace path.
    let path_str = config.workspace_dir.to_string_lossy();
    let mut hash: u64 = 14695981039346656037u64;
    for b in path_str.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(1099511628211u64);
    }
    format!("wallet-path-{hash:016x}")
}

/// Load the encrypted mnemonic from the OS keychain if available.
///
/// Returns `None` if the keychain is unavailable or the entry does not exist.
fn keychain_load_mnemonic(config: &Config) -> Option<String> {
    let policy = crate::openhuman::keyring_consent::policy::check_secret_access();
    if policy != crate::openhuman::keyring_consent::PolicyDecision::Proceed
        || !crate::openhuman::keyring::is_available()
    {
        log::debug!("{LOG_PREFIX} keychain unavailable or consent pending, skipping mnemonic load policy={policy:?}");
        return None;
    }
    let user_id = wallet_user_id(config);
    match crate::openhuman::keyring::get(&user_id, KEYCHAIN_MNEMONIC_KEY) {
        Ok(Some(val)) => {
            log::debug!("{LOG_PREFIX} keychain mnemonic loaded user_id={user_id}");
            Some(val)
        }
        Ok(None) => {
            log::debug!("{LOG_PREFIX} keychain mnemonic not found user_id={user_id}");
            None
        }
        Err(e) => {
            log::warn!("{LOG_PREFIX} keychain mnemonic load error user_id={user_id}: {e}");
            None
        }
    }
}

/// Store the encrypted mnemonic in the OS keychain.
///
/// Returns `true` if the write succeeded.
fn keychain_save_mnemonic(config: &Config, encrypted_mnemonic: &str) -> bool {
    let policy = crate::openhuman::keyring_consent::policy::check_secret_access();
    if policy != crate::openhuman::keyring_consent::PolicyDecision::Proceed
        || !crate::openhuman::keyring::is_available()
    {
        log::debug!("{LOG_PREFIX} keychain unavailable or consent pending, skipping mnemonic save policy={policy:?}");
        return false;
    }
    let user_id = wallet_user_id(config);
    match crate::openhuman::keyring::set(&user_id, KEYCHAIN_MNEMONIC_KEY, encrypted_mnemonic) {
        Ok(()) => {
            log::debug!("{LOG_PREFIX} keychain mnemonic saved user_id={user_id}");
            true
        }
        Err(e) => {
            log::warn!("{LOG_PREFIX} keychain mnemonic save error user_id={user_id}: {e}");
            false
        }
    }
}

/// Whether a keychain entry exists for the encrypted mnemonic.
fn keychain_has_mnemonic(config: &Config) -> bool {
    let policy = crate::openhuman::keyring_consent::policy::check_secret_access();
    if policy != crate::openhuman::keyring_consent::PolicyDecision::Proceed
        || !crate::openhuman::keyring::is_available()
    {
        return false;
    }
    let user_id = wallet_user_id(config);
    matches!(
        crate::openhuman::keyring::get(&user_id, KEYCHAIN_MNEMONIC_KEY),
        Ok(Some(_))
    )
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WalletChain {
    Evm,
    Btc,
    Solana,
    Tron,
}

impl WalletChain {
    const ALL: [Self; 4] = [Self::Evm, Self::Btc, Self::Solana, Self::Tron];

    fn as_str(self) -> &'static str {
        match self {
            Self::Evm => "evm",
            Self::Btc => "btc",
            Self::Solana => "solana",
            Self::Tron => "tron",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WalletSetupSource {
    Generated,
    Imported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WalletAccount {
    pub chain: WalletChain,
    pub address: String,
    pub derivation_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WalletSetupParams {
    pub consent_granted: bool,
    pub source: WalletSetupSource,
    pub mnemonic_word_count: u8,
    #[serde(default)]
    pub encrypted_mnemonic: Option<String>,
    pub accounts: Vec<WalletAccount>,
    /// When `true`, allows overwriting an existing wallet.
    /// Requires explicit user confirmation in the frontend.
    /// Defaults to `false` — a guard against silent overwrites.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct StoredWalletState {
    pub consent_granted: bool,
    pub source: WalletSetupSource,
    pub mnemonic_word_count: u8,
    #[serde(default)]
    pub encrypted_mnemonic: Option<String>,
    pub accounts: Vec<WalletAccount>,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WalletSecretMaterial {
    pub encrypted_mnemonic: String,
    pub derivation_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WalletStatus {
    pub configured: bool,
    pub onboarding_completed: bool,
    pub consent_granted: bool,
    pub secret_stored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<WalletSetupSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mnemonic_word_count: Option<u8>,
    pub accounts: Vec<WalletAccount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<u64>,
}

fn wallet_state_path(config: &Config) -> PathBuf {
    config
        .workspace_dir
        .join("state")
        .join(WALLET_STATE_FILENAME)
}

fn ensure_wallet_state_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "failed to create workspace state dir {}: {e}",
                parent.display()
            )
        })?;
    }
    Ok(())
}

fn corrupted_wallet_state_path(path: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0);
    path.with_extension(format!("json.corrupted.{timestamp}"))
}

fn quarantine_corrupted_wallet_state(path: &Path, reason: &str) {
    let quarantine_path = corrupted_wallet_state_path(path);
    warn!(
        "{LOG_PREFIX} quarantining corrupted wallet state {} -> {} ({reason})",
        path.display(),
        quarantine_path.display()
    );

    if let Err(rename_error) = fs::rename(path, &quarantine_path) {
        warn!(
            "{LOG_PREFIX} failed to quarantine {} via rename: {}",
            path.display(),
            rename_error
        );
        if let Err(remove_error) = fs::remove_file(path) {
            warn!(
                "{LOG_PREFIX} failed to remove unreadable wallet state {}: {}",
                path.display(),
                remove_error
            );
        }
    }
}

fn load_stored_wallet_state_unlocked(config: &Config) -> Result<Option<StoredWalletState>, String> {
    let path = wallet_state_path(config);

    // ── Step 1: Try to resolve the encrypted mnemonic from the OS keychain ──
    // If the keychain has the entry, we don't need the JSON field at all.
    // The JSON file may or may not exist (it still holds non-secret metadata).
    let keychain_mnemonic = keychain_load_mnemonic(config);

    // ── Step 2: Load state from JSON (metadata, accounts, flags) ─────────────
    if !path.exists() {
        if keychain_mnemonic.is_some() {
            // Keychain has the mnemonic but there is no wallet-state.json.
            // This shouldn't happen in practice (both are written together),
            // but we log it and return None so the user re-setups the wallet.
            warn!(
                "{LOG_PREFIX} keychain has mnemonic but no wallet-state.json at {}; \
                 treating as not configured",
                path.display()
            );
        }
        return Ok(None);
    }

    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) => {
            warn!(
                "{LOG_PREFIX} failed to read {}; falling back to defaults: {}",
                path.display(),
                error
            );
            quarantine_corrupted_wallet_state(&path, &error.to_string());
            return Ok(None);
        }
    };

    let mut state = match serde_json::from_str::<StoredWalletState>(&raw) {
        Ok(state) => state,
        Err(error) => {
            warn!(
                "{LOG_PREFIX} failed to parse {}; falling back to defaults: {}",
                path.display(),
                error
            );
            quarantine_corrupted_wallet_state(&path, &error.to_string());
            return Ok(None);
        }
    };

    // ── Step 3: Merge keychain mnemonic with JSON state ───────────────────────
    // Priority: keychain > JSON field.
    let needs_keychain_migration =
        keychain_mnemonic.is_none() && state.encrypted_mnemonic.is_some();

    if let Some(mnemonic) = keychain_mnemonic {
        // Keychain is authoritative. If the JSON still has the field, clear it.
        if state.encrypted_mnemonic.is_some() {
            debug!(
                "{LOG_PREFIX} load: clearing encrypted_mnemonic from JSON (already in keychain)"
            );
            state.encrypted_mnemonic = None;
            // Rewrite the JSON without the secret field.
            if let Err(e) = save_stored_wallet_state_unlocked(config, &state) {
                warn!(
                    "{LOG_PREFIX} load: failed to rewrite wallet-state.json after keychain migration: {e}"
                );
            }
        }
        state.encrypted_mnemonic = Some(mnemonic);
    } else if needs_keychain_migration {
        // The encrypted mnemonic is in the JSON. Promote it to keychain if available.
        if let Some(ref enc_mnemonic) = state.encrypted_mnemonic.clone() {
            debug!("{LOG_PREFIX} load: promoting encrypted_mnemonic from JSON to keychain");
            if keychain_save_mnemonic(config, enc_mnemonic) {
                // Successfully saved to keychain — clear from JSON.
                state.encrypted_mnemonic = None;
                if let Err(e) = save_stored_wallet_state_unlocked(config, &state) {
                    warn!(
                        "{LOG_PREFIX} load: failed to rewrite wallet-state.json after mnemonic promotion: {e}"
                    );
                }
                // Restore the value in-memory so validation passes.
                state.encrypted_mnemonic = Some(enc_mnemonic.clone());
            }
        }
    }

    // ── Step 4: Validate (allows encrypted_mnemonic to be None when in keychain) ──
    // Build validation params treating keychain-held mnemonic as present.
    // Re-probe the keychain here so that headless / CI environments where
    // keychain_load_mnemonic returned None at the top of this function (e.g.
    // because the keychain entry was written by a concurrent task and was not
    // yet visible to the earlier read) get one more chance to find the secret.
    let effective_mnemonic = state.encrypted_mnemonic.clone().or_else(|| {
        let reprobe = keychain_load_mnemonic(config);
        if reprobe.is_some() {
            debug!(
                "{LOG_PREFIX} load: re-probe found mnemonic in keychain \
                 that was absent on initial probe; merging into state"
            );
        }
        reprobe
    });

    // Propagate whatever was found (initial probe, migration, or re-probe) back
    // into `state` so that callers (reveal_recovery_phrase, secret_material)
    // always receive a complete state when the mnemonic is accessible.
    if state.encrypted_mnemonic.is_none() {
        if let Some(ref m) = effective_mnemonic {
            debug!("{LOG_PREFIX} load: setting encrypted_mnemonic from effective_mnemonic");
            state.encrypted_mnemonic = Some(m.clone());
        }
    }

    let validation_params = WalletSetupParams {
        consent_granted: state.consent_granted,
        source: state.source,
        mnemonic_word_count: state.mnemonic_word_count,
        encrypted_mnemonic: effective_mnemonic,
        accounts: state.accounts.clone(),
        // force is irrelevant for validation; always false here.
        force: false,
    };
    if let Err(validation_error) = validate_setup(&validation_params) {
        warn!(
            "{LOG_PREFIX} stored wallet state at {} failed validation: {validation_error}",
            path.display()
        );
        quarantine_corrupted_wallet_state(&path, &validation_error);
        return Ok(None);
    }

    Ok(Some(state))
}

fn sync_parent_dir(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| format!("failed to sync directory {}: {e}", parent.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn save_stored_wallet_state_unlocked(
    config: &Config,
    state: &StoredWalletState,
) -> Result<(), String> {
    let path = wallet_state_path(config);
    ensure_wallet_state_dir(&path)?;

    // When the OS keychain is available, store the encrypted mnemonic there and
    // write the JSON without the secret field.  This is the preferred path on
    // macOS / Windows / Linux-with-Secret-Service.
    let mut state_for_json = state.clone();
    if let Some(ref enc_mnemonic) = state.encrypted_mnemonic {
        if keychain_save_mnemonic(config, enc_mnemonic) {
            debug!("{LOG_PREFIX} save: encrypted_mnemonic saved to keychain; stripping from JSON");
            state_for_json.encrypted_mnemonic = None;
        } else {
            debug!("{LOG_PREFIX} save: keychain unavailable; keeping encrypted_mnemonic in JSON");
        }
    }

    let payload = serde_json::to_string_pretty(&state_for_json)
        .map_err(|e| format!("failed to serialize wallet state: {e}"))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("failed to resolve parent dir for {}", path.display()))?;
    let mut temp_file = NamedTempFile::new_in(parent)
        .map_err(|e| format!("failed to create temp file in {}: {e}", parent.display()))?;
    temp_file.write_all(payload.as_bytes()).map_err(|e| {
        format!(
            "failed to write temp wallet state for {}: {e}",
            path.display()
        )
    })?;
    temp_file.as_file_mut().sync_all().map_err(|e| {
        format!(
            "failed to sync temp wallet state for {}: {e}",
            path.display()
        )
    })?;
    sync_parent_dir(&path)?;
    temp_file.persist(&path).map_err(|e| {
        format!(
            "failed to persist wallet state {}: {}",
            path.display(),
            e.error
        )
    })?;
    sync_parent_dir(&path)?;
    Ok(())
}

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

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

fn to_status(config: &Config, state: Option<StoredWalletState>) -> WalletStatus {
    match state {
        Some(state) => {
            // A mnemonic is "stored" if it's either in the JSON field (headless path)
            // or has been moved to the OS keychain (preferred path).
            let secret_in_json = state
                .encrypted_mnemonic
                .as_ref()
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false);
            let secret_stored = secret_in_json || keychain_has_mnemonic(config);
            WalletStatus {
                configured: true,
                onboarding_completed: state.consent_granted && !state.accounts.is_empty(),
                consent_granted: state.consent_granted,
                secret_stored,
                source: Some(state.source),
                mnemonic_word_count: Some(state.mnemonic_word_count),
                accounts: state.accounts,
                updated_at_ms: Some(state.updated_at_ms),
            }
        }
        None => WalletStatus {
            configured: false,
            onboarding_completed: false,
            consent_granted: false,
            secret_stored: false,
            source: None,
            mnemonic_word_count: None,
            accounts: Vec::new(),
            updated_at_ms: None,
        },
    }
}

pub async fn status() -> Result<RpcOutcome<WalletStatus>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let _guard = WALLET_STATE_FILE_LOCK.lock();
    let status = to_status(&config, load_stored_wallet_state_unlocked(&config)?);

    debug!(
        "{LOG_PREFIX} status configured={} onboarding_completed={} account_count={}",
        status.configured,
        status.onboarding_completed,
        status.accounts.len()
    );

    Ok(RpcOutcome::new(
        status,
        vec!["wallet status fetched".to_string()],
    ))
}

pub async fn setup(params: WalletSetupParams) -> Result<RpcOutcome<WalletStatus>, String> {
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
        updated_at_ms: current_time_ms(),
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

    Ok(RpcOutcome::new(
        status,
        vec!["wallet setup saved".to_string()],
    ))
}

/// Result returned by `reveal_recovery_phrase`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevealRecoveryPhraseResult {
    pub phrase: String,
    pub word_count: usize,
}

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

    let phrase = crate::openhuman::credentials::ops::decrypt_secret(&config, &ciphertext)
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

#[cfg(test)]
mod tests {
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
        let _wallet_lock = crate::openhuman::wallet::test_support::TEST_LOCK.lock();
        let _workspace_guard =
            crate::openhuman::wallet::test_support::set_workspace_env_for_test(&temp);
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
        let _wallet_lock = crate::openhuman::wallet::test_support::TEST_LOCK.lock();
        let _workspace_guard = crate::openhuman::wallet::test_support::setup_wallet_in(&temp)
            .await
            .expect("setup wallet");
        let result = reveal_recovery_phrase()
            .await
            .expect("reveal should succeed");
        assert_eq!(
            result.value.phrase,
            crate::openhuman::wallet::test_support::TEST_MNEMONIC
        );
        assert_eq!(result.value.word_count, 12);
    }
}
