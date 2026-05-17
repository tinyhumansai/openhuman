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
const VALID_MNEMONIC_WORD_COUNTS: [u8; 5] = [12, 15, 18, 21, 24];
static WALLET_STATE_FILE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

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
    if !path.exists() {
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

    let state = match serde_json::from_str::<StoredWalletState>(&raw) {
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

    let validation_params = WalletSetupParams {
        consent_granted: state.consent_granted,
        source: state.source,
        mnemonic_word_count: state.mnemonic_word_count,
        encrypted_mnemonic: state.encrypted_mnemonic.clone(),
        accounts: state.accounts.clone(),
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
    let payload = serde_json::to_string_pretty(state)
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

fn to_status(state: Option<StoredWalletState>) -> WalletStatus {
    match state {
        Some(state) => WalletStatus {
            configured: true,
            onboarding_completed: state.consent_granted && !state.accounts.is_empty(),
            consent_granted: state.consent_granted,
            secret_stored: state
                .encrypted_mnemonic
                .as_ref()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false),
            source: Some(state.source),
            mnemonic_word_count: Some(state.mnemonic_word_count),
            accounts: state.accounts,
            updated_at_ms: Some(state.updated_at_ms),
        },
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
    let status = to_status(load_stored_wallet_state_unlocked(&config)?);

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
    let state = StoredWalletState {
        consent_granted: params.consent_granted,
        source: params.source,
        mnemonic_word_count: params.mnemonic_word_count,
        encrypted_mnemonic: Some(encrypted_mnemonic),
        accounts,
        updated_at_ms: current_time_ms(),
    };

    let _guard = WALLET_STATE_FILE_LOCK.lock();
    save_stored_wallet_state_unlocked(&config, &state)?;
    let status = to_status(Some(state));

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
            return Err("wallet is not configured; run wallet setup first".to_string());
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
        let status = to_status(None);
        assert!(!status.configured);
        assert!(!status.onboarding_completed);
        assert!(!status.secret_stored);
        assert!(status.accounts.is_empty());
    }

    #[test]
    fn status_maps_stored_state() {
        let state = StoredWalletState {
            consent_granted: true,
            source: WalletSetupSource::Generated,
            mnemonic_word_count: 24,
            encrypted_mnemonic: Some("enc2:abc".to_string()),
            accounts: WalletChain::ALL.into_iter().map(sample_account).collect(),
            updated_at_ms: 123,
        };
        let status = to_status(Some(state));
        assert!(status.configured);
        assert!(status.onboarding_completed);
        assert!(status.secret_stored);
        assert_eq!(status.accounts.len(), 4);
        assert_eq!(status.updated_at_ms, Some(123));
    }
}
