use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use log::{debug, warn};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::rpc::RpcOutcome;

use super::chains::{btc as chain_btc, evm as chain_evm, solana as chain_sol, tron as chain_tron};
use super::defaults::{
    evm_asset_catalog, explorer_tx_url, find_asset_for_network,
    network_defaults as default_networks, rpc_url_for_chain, EvmNetwork, WalletAssetDefinition,
    WalletNetworkDefaults,
};
use super::ops::{
    status as wallet_status, WalletAccount, WalletChain, WALLET_NOT_CONFIGURED_MESSAGE,
};

const LOG_PREFIX: &str = "[wallet]";
const QUOTE_TTL_MS: u64 = 5 * 60 * 1000;
const QUOTE_STORE_CAP: usize = 64;

static QUOTE_STORE: Lazy<Mutex<Vec<PreparedTransaction>>> = Lazy::new(|| Mutex::new(Vec::new()));
static QUOTE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Return the compressed SEC1 public key for a secp256k1 secret.
///
/// Test-only. Production never holds a secp256k1 secret: the wallet module
/// derives the key and reports the public half through `DeriveAccount`.
#[cfg(test)]
pub(super) fn compressed_public_key(secret: &[u8]) -> Result<Vec<u8>, String> {
    let key = k256::ecdsa::SigningKey::from_slice(secret)
        .map_err(|_| "derived key is not a valid secp256k1 scalar".to_string())?;
    Ok(key
        .verifying_key()
        .to_encoded_point(true)
        .as_bytes()
        .to_vec())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainStatus {
    pub chain: WalletChain,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_network: Option<EvmNetwork>,
    pub configured: bool,
    pub provider_status: ProviderStatus,
    pub rpc_url: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Ready,
    Missing,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedAsset {
    pub chain: WalletChain,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_network: Option<EvmNetwork>,
    pub symbol: String,
    pub name: String,
    pub native: bool,
    pub decimals: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract_address: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceInfo {
    pub chain: WalletChain,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_network: Option<EvmNetwork>,
    pub address: String,
    pub asset_symbol: String,
    pub decimals: u8,
    pub raw: String,
    pub formatted: String,
    pub provider_status: ProviderStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreparedKind {
    NativeTransfer,
    TokenTransfer,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreparedStatus {
    AwaitingConfirmation,
    Broadcasted,
    Consumed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedTransaction {
    pub quote_id: String,
    pub kind: PreparedKind,
    pub chain: WalletChain,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_network: Option<EvmNetwork>,
    pub from_address: String,
    pub to_address: String,
    pub asset_symbol: String,
    pub amount_raw: String,
    pub amount_formatted: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receive_symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_receive_raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calldata: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_address: Option<String>,
    pub estimated_fee_raw: String,
    pub status: PreparedStatus,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub notes: Vec<String>,
    /// Chat-thread owner stamped at prepare time. Present when the quote
    /// was prepared from inside an interactive chat turn (web channel sets
    /// `APPROVAL_CHAT_CONTEXT`); `None` for CLI / direct-RPC / background
    /// callers. Internal gate data — never serialized over the wire.
    #[serde(skip_serializing)]
    pub(crate) owner: Option<QuoteOwner>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResult {
    pub quote_id: String,
    pub status: PreparedStatus,
    pub chain: WalletChain,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_network: Option<EvmNetwork>,
    pub transaction_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explorer_url: Option<String>,
    pub transaction: PreparedTransaction,
}

/// Result of a low-level "sign this unsigned transaction and broadcast it"
/// primitive. Unlike [`ExecutionResult`], this carries no `PreparedTransaction`
/// — it is the minimal output the `web3` layer needs after handing the wallet
/// an externally-built (e.g. deBridge) unsigned transaction to sign+broadcast.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawBroadcastResult {
    pub transaction_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explorer_url: Option<String>,
    /// Simulated fee in the chain's smallest unit. `None` when the fee is not
    /// known at broadcast time (e.g. Solana's dynamic base+priority fee, which
    /// must be read back from the confirmed transaction).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_raw: Option<String>,
}

/// Normalized lifecycle state of a broadcast transaction.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TxState {
    /// Seen by the node but not yet included in a block.
    Pending,
    /// Included in a block and succeeded.
    Confirmed,
    /// Included in a block but reverted/failed.
    Failed,
    /// The node has no record of this hash.
    NotFound,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxStatusInfo {
    pub chain: WalletChain,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_network: Option<EvmNetwork>,
    pub hash: String,
    pub state: TxState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmations: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_number: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxReceiptInfo {
    pub chain: WalletChain,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_network: Option<EvmNetwork>,
    pub hash: String,
    pub found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_number: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_used: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_raw: Option<String>,
    /// Raw provider receipt payload, passed through unchanged.
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxLookupInfo {
    pub chain: WalletChain,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evm_network: Option<EvmNetwork>,
    pub hash: String,
    pub found: bool,
    /// Raw provider transaction payload, passed through unchanged.
    pub raw: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareTransferParams {
    pub chain: WalletChain,
    pub to_address: String,
    pub amount_raw: String,
    #[serde(default)]
    pub asset_symbol: Option<String>,
    #[serde(default)]
    pub evm_network: Option<EvmNetwork>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutePreparedParams {
    pub quote_id: String,
    pub confirmed: bool,
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn next_quote_id() -> String {
    let n = QUOTE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("q_{}_{}", now_ms(), n)
}

/// Identity of the chat thread that prepared a quote.
///
/// The wallet executes prepare/execute as a two-step flow keyed by `quote_id`.
/// `quote_id`s are visible in the shared chat broadcast (the prepared-tx
/// summary that gets sent back into the channel), so a co-channel caller can
/// read another caller's `quote_id` and try to drive its execute from their
/// own (now per-sender-isolated, post-#2331) agent session. Binding the
/// quote to the originating chat thread closes that gap: execute is only
/// allowed when the caller's `current_owner()` equals the prepare-time owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QuoteOwner {
    pub(crate) thread_id: String,
    pub(crate) client_id: String,
}

/// Read the per-turn chat context that scopes the agent tool loop.
///
/// Returns `Some(owner)` when called from inside an interactive chat turn
/// (the web channel installs `APPROVAL_CHAT_CONTEXT` around `run_chat_task`).
/// Returns `None` for non-chat callers (CLI, direct JSON-RPC, background
/// triage / cron / sub-agents) — these keep the pre-binding behavior and
/// remain executable without an owner gate, since they have no shared
/// channel from which a `quote_id` could leak.
///
// SAFETY: relies on the inline `.await` chain in
// `web_chat::run_chat_task`. `tokio::task_local!` propagates
// across `.await` but **not** across `tokio::spawn`. If the chat path ever
// detaches the tool loop onto a freshly-spawned task without wrapping it in
// `APPROVAL_CHAT_CONTEXT.scope(...)`, this helper will silently start
// returning `None` and the owner gate will become a no-op. Keep the
// prepare/execute calls inline within the scope.
pub(crate) fn current_owner() -> Option<QuoteOwner> {
    crate::openhuman::security::approval::APPROVAL_CHAT_CONTEXT
        .try_with(|ctx| QuoteOwner {
            thread_id: ctx.thread_id.clone(),
            client_id: ctx.client_id.clone(),
        })
        .ok()
}

/// Resolve the derived EVM account address, erroring if the wallet is not
/// configured. Used by the `web3` signing primitives that operate on the
/// single shared EVM address.
pub(crate) async fn require_evm_account() -> Result<String, String> {
    Ok(require_account(WalletChain::Evm).await?.address)
}

async fn require_account(chain: WalletChain) -> Result<WalletAccount, String> {
    let status = wallet_status().await?.value;
    if !status.configured {
        return Err(WALLET_NOT_CONFIGURED_MESSAGE.to_string());
    }
    status
        .accounts
        .into_iter()
        .find(|account| account.chain == chain)
        .ok_or_else(|| format!("no wallet account derived for chain '{}'", chain_str(chain)))
}

pub(crate) fn chain_str(chain: WalletChain) -> &'static str {
    match chain {
        WalletChain::Evm => "evm",
        WalletChain::Btc => "btc",
        WalletChain::Solana => "solana",
        WalletChain::Tron => "tron",
    }
}

pub(crate) fn validate_amount(raw: &str) -> Result<u128, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("amount is empty".to_string());
    }
    trimmed
        .parse::<u128>()
        .map_err(|_| format!("amount '{trimmed}' is not a valid non-negative integer"))
}

/// Validate `addr` for `chain`, returning it trimmed.
///
/// Every arm delegates to the vendored [`tinywallet_bus`] crate, which owns the
/// four address formats. The dispatch stays here rather than calling
/// `tinywallet_bus::address::validate` directly because [`WalletChain`] is
/// OpenHuman's enum, and mapping it onto `tinywallet_bus::Chain` here keeps that
/// translation in one place.
///
/// For Bitcoin this is the **recipient** rule — any well-formed mainnet
/// address. Sender addresses go through `chain_btc::validate_btc_sender_address`,
/// which additionally requires P2WPKH; the distinction has no equivalent on
/// the other three chains, so it cannot be expressed through this entry point.
fn validate_address(chain: WalletChain, addr: &str) -> Result<String, String> {
    let tw_chain = match chain {
        WalletChain::Evm => tinywallet_bus::Chain::Evm,
        WalletChain::Btc => tinywallet_bus::Chain::Btc,
        WalletChain::Solana => tinywallet_bus::Chain::Solana,
        WalletChain::Tron => tinywallet_bus::Chain::Tron,
    };
    debug!("{LOG_PREFIX} validate_address chain={chain:?} role=recipient dispatch=tinywallet_bus");
    let result = tinywallet_bus::address::validate(tw_chain, addr).map_err(|e| e.to_string());
    debug!(
        "{LOG_PREFIX} validate_address chain={chain:?} role=recipient result={}",
        if result.is_ok() {
            "accepted"
        } else {
            "rejected"
        }
    );
    result
}

pub(crate) fn validate_calldata(data: &str) -> Result<String, String> {
    let trimmed = data.trim();
    if !trimmed.starts_with("0x") {
        return Err("calldata must be 0x-prefixed hex".to_string());
    }
    let body = &trimmed[2..];
    if !body.len().is_multiple_of(2) {
        return Err("calldata hex must be byte-aligned".to_string());
    }
    if !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("calldata contains non-hex characters".to_string());
    }
    Ok(trimmed.to_string())
}

pub(crate) fn format_amount(raw: u128, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_string();
    }
    let s = raw.to_string();
    let d = decimals as usize;
    if s.len() <= d {
        format!("0.{:0>width$}", s, width = d)
    } else {
        let split = s.len() - d;
        format!("{}.{}", &s[..split], &s[split..])
    }
}

fn estimated_fee_raw(chain: WalletChain, kind: PreparedKind) -> String {
    let base = match (chain, kind) {
        (WalletChain::Evm, PreparedKind::NativeTransfer) => 21_000u128 * 30_000_000_000,
        (WalletChain::Evm, PreparedKind::TokenTransfer) => 65_000u128 * 30_000_000_000,
        (WalletChain::Btc, _) => 5_000,
        (WalletChain::Solana, _) => 5_000,
        (WalletChain::Tron, PreparedKind::NativeTransfer) => 1_000_000,
        (WalletChain::Tron, PreparedKind::TokenTransfer) => 15_000_000,
    };
    base.to_string()
}

fn asset_to_supported(asset: WalletAssetDefinition) -> SupportedAsset {
    SupportedAsset {
        chain: asset.chain,
        evm_network: asset.evm_network,
        symbol: asset.symbol,
        name: asset.name,
        native: asset.native,
        decimals: asset.decimals,
        contract_address: asset.contract_address,
    }
}

fn store_quote(quote: PreparedTransaction) -> PreparedTransaction {
    let mut store = QUOTE_STORE.lock();
    let cutoff = now_ms();
    store.retain(|q| q.expires_at_ms > cutoff && q.status != PreparedStatus::Consumed);
    if store.len() >= QUOTE_STORE_CAP {
        store.remove(0);
    }
    store.push(quote.clone());
    quote
}

fn get_quote(quote_id: &str) -> Result<PreparedTransaction, String> {
    let store = QUOTE_STORE.lock();
    let now = now_ms();
    let quote = store
        .iter()
        .find(|q| q.quote_id == quote_id)
        .cloned()
        .ok_or_else(|| format!("quote '{quote_id}' not found"))?;
    if quote.status == PreparedStatus::Consumed {
        return Err(format!("quote '{quote_id}' already executed"));
    }
    if quote.expires_at_ms <= now {
        return Err(format!("quote '{quote_id}' expired"));
    }
    Ok(quote)
}

/// Remove a quote from the store and return it to the caller, if and only if
/// the caller's chat-thread owner matches the prepare-time owner.
///
/// On owner mismatch this returns the **exact same** "quote '…' not found"
/// error shape that a missing-row lookup would, so cross-thread callers
/// cannot distinguish "wrong owner" from "no such quote" — i.e. no
/// enumeration oracle for leaked `quote_id`s.
///
/// Callers with no chat context (`caller_owner == None`, e.g. CLI / direct
/// JSON-RPC / background turns) can only execute quotes that were also
/// prepared with no chat context. This intentionally prevents privilege-drop
/// where a background flow could pick up an interactive user's quote.
fn take_quote_for(
    quote_id: &str,
    caller_owner: Option<QuoteOwner>,
) -> Result<PreparedTransaction, String> {
    let not_found = || format!("quote '{quote_id}' not found");
    let mut store = QUOTE_STORE.lock();
    let now = now_ms();
    let pos = store
        .iter()
        .position(|q| q.quote_id == quote_id)
        .ok_or_else(not_found)?;
    // Owner check happens before status / expiry checks so the error shape on
    // mismatch can be byte-equal to the not-found path. Removing the quote
    // only happens *after* this check passes — a mismatched caller cannot
    // poison the store by consuming someone else's quote.
    if store[pos].owner != caller_owner {
        debug!(
            "{LOG_PREFIX} take_quote_for quote_id={} owner_mismatch (caller_has_ctx={})",
            quote_id,
            caller_owner.is_some()
        );
        return Err(not_found());
    }
    let quote = store.remove(pos);
    if quote.status == PreparedStatus::Consumed {
        return Err(format!("quote '{quote_id}' already executed"));
    }
    if quote.expires_at_ms <= now {
        return Err(format!("quote '{quote_id}' expired"));
    }
    Ok(quote)
}

pub fn prepared_quotes_for_test() -> Vec<PreparedTransaction> {
    let now = now_ms();
    QUOTE_STORE
        .lock()
        .iter()
        .filter(|q| q.expires_at_ms > now && q.status != PreparedStatus::Consumed)
        .cloned()
        .collect()
}

#[cfg(test)]
pub(crate) fn reset_quote_store_for_tests() {
    QUOTE_STORE.lock().clear();
}

#[cfg(test)]
pub(crate) fn insert_quote_for_test(quote: PreparedTransaction) -> PreparedTransaction {
    store_quote(quote)
}

/// Parse an `0x`-prefixed hex quantity, as every EVM JSON-RPC result encodes
/// integers.
///
/// `u128` rather than a 256-bit type. Nothing this wallet reads from a node —
/// a nonce, a gas price, a gas limit, a wei balance — approaches 2^128, which
/// is about 3.4e20 ETH, and carrying `ethers-core` for a bignum that is never
/// exercised past 128 bits is the trade this port exists to stop making. A
/// value that genuinely did overflow is reported rather than truncated.
///
/// # Errors
///
/// A message naming the offending value if it is not hex, or does not fit.
pub fn hex_to_u128(hex_value: &str) -> Result<u128, String> {
    let trimmed = hex_value.trim();
    let normalized = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    u128::from_str_radix(normalized, 16)
        .map_err(|e| format!("invalid hex quantity '{hex_value}': {e}"))
}

/// Render an integer the way an EVM JSON-RPC parameter expects it.
#[must_use]
pub fn u128_to_hex(value: u128) -> String {
    format!("0x{value:x}")
}

pub fn hex_to_bytes(value: &str) -> Result<Vec<u8>, String> {
    let trimmed = value.trim();
    let normalized = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    hex::decode(normalized).map_err(|e| format!("invalid hex bytes '{value}': {e}"))
}

pub async fn network_defaults() -> Result<RpcOutcome<Vec<WalletNetworkDefaults>>, String> {
    let rows = default_networks();
    debug!("{LOG_PREFIX} network_defaults count={}", rows.len());
    Ok(RpcOutcome::new(
        rows,
        vec!["wallet network defaults listed".to_string()],
    ))
}

pub async fn supported_assets() -> Result<RpcOutcome<Vec<SupportedAsset>>, String> {
    let mut assets: Vec<SupportedAsset> = Vec::new();
    for network in EvmNetwork::ALL {
        for asset in evm_asset_catalog(network) {
            assets.push(asset_to_supported(asset));
        }
    }
    for chain in [WalletChain::Btc, WalletChain::Solana, WalletChain::Tron] {
        for asset in super::defaults::asset_catalog(chain) {
            assets.push(asset_to_supported(asset));
        }
    }
    debug!("{LOG_PREFIX} supported_assets count={}", assets.len());
    Ok(RpcOutcome::new(
        assets,
        vec!["wallet supported_assets listed".to_string()],
    ))
}

pub async fn chain_status() -> Result<RpcOutcome<Vec<ChainStatus>>, String> {
    let status = wallet_status().await?.value;
    let mut rows = Vec::new();
    for network in EvmNetwork::ALL {
        let has_account = status
            .accounts
            .iter()
            .any(|account| account.chain == WalletChain::Evm);
        rows.push(ChainStatus {
            chain: WalletChain::Evm,
            evm_network: Some(network),
            configured: has_account,
            provider_status: if has_account {
                ProviderStatus::Ready
            } else {
                ProviderStatus::Missing
            },
            rpc_url: network.rpc_url(),
        });
    }
    for chain in [WalletChain::Btc, WalletChain::Solana, WalletChain::Tron] {
        let has_account = status.accounts.iter().any(|account| account.chain == chain);
        rows.push(ChainStatus {
            chain,
            evm_network: None,
            configured: has_account,
            provider_status: if has_account {
                ProviderStatus::Ready
            } else {
                ProviderStatus::Missing
            },
            rpc_url: rpc_url_for_chain(chain),
        });
    }
    debug!("{LOG_PREFIX} chain_status reported chains={}", rows.len());
    Ok(RpcOutcome::new(
        rows,
        vec!["wallet chain_status listed".to_string()],
    ))
}

/// EVM networks surfaced as their own native-balance rows. The single derived
/// EVM account address is shared across all of them, so `balances()` reads the
/// native asset (ETH / ETH / BNB) on each network independently.
pub const EVM_BALANCE_NETWORKS: [EvmNetwork; 3] = [
    EvmNetwork::EthereumMainnet,
    EvmNetwork::BaseMainnet,
    EvmNetwork::BscMainnet,
];

/// Build a single native-balance row, reading the live on-chain balance and
/// falling back to a zero/`Missing` row when the provider is unreachable.
fn balance_row(
    chain: WalletChain,
    evm_network: Option<EvmNetwork>,
    address: &str,
    asset: WalletAssetDefinition,
    raw: String,
    provider_status: ProviderStatus,
) -> BalanceInfo {
    let raw_u128 = raw.parse::<u128>().unwrap_or(0);
    BalanceInfo {
        chain,
        evm_network,
        address: address.to_string(),
        asset_symbol: asset.symbol,
        decimals: asset.decimals,
        formatted: format_amount(raw_u128, asset.decimals),
        raw,
        provider_status,
    }
}

fn native_asset_for(chain: WalletChain) -> Result<WalletAssetDefinition, String> {
    super::defaults::asset_catalog(chain)
        .into_iter()
        .find(|value| value.native)
        .ok_or_else(|| format!("native asset metadata missing for '{}'", chain_str(chain)))
}

fn evm_native_asset(network: EvmNetwork) -> Result<WalletAssetDefinition, String> {
    evm_asset_catalog(network)
        .into_iter()
        .find(|value| value.native)
        .ok_or_else(|| {
            format!(
                "native asset metadata missing for evm network '{}'",
                network.as_str()
            )
        })
}
