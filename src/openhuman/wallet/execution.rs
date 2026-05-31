//! Wallet execution surface — read tools (balances / supported assets /
//! network defaults / chain status) and write tools (prepare-then-execute)
//! for native sends, token transfers, swaps, and contract calls.
//!
//! Execution is intentionally narrower than the metadata surface:
//! - Every write must be prepared first, then explicitly confirmed.
//! - Secret material stays encrypted at rest in core-owned storage.
//! - EVM (Ethereum + Base/Arbitrum/Optimism/Polygon L2s), Bitcoin (P2WPKH),
//!   Solana (native + SPL), and Tron (native + TRC20) all sign and broadcast.
//!   Swap broadcast is still quote-only on every chain.

use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ethers_core::types::{Address, U256};
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
use super::ops::{status as wallet_status, WalletAccount, WalletChain};

const LOG_PREFIX: &str = "[wallet]";
const QUOTE_TTL_MS: u64 = 5 * 60 * 1000;
const QUOTE_STORE_CAP: usize = 64;

static QUOTE_STORE: Lazy<Mutex<Vec<PreparedTransaction>>> = Lazy::new(|| Mutex::new(Vec::new()));
static QUOTE_COUNTER: AtomicU64 = AtomicU64::new(1);

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
// `channels/providers/web.rs::run_chat_task`. `tokio::task_local!` propagates
// across `.await` but **not** across `tokio::spawn`. If the chat path ever
// detaches the tool loop onto a freshly-spawned task without wrapping it in
// `APPROVAL_CHAT_CONTEXT.scope(...)`, this helper will silently start
// returning `None` and the owner gate will become a no-op. Keep the
// prepare/execute calls inline within the scope.
pub(crate) fn current_owner() -> Option<QuoteOwner> {
    crate::openhuman::approval::APPROVAL_CHAT_CONTEXT
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
        return Err("wallet is not configured; run wallet setup first".to_string());
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

fn validate_address(chain: WalletChain, addr: &str) -> Result<String, String> {
    let trimmed = addr.trim();
    if trimmed.is_empty() {
        return Err("address is empty".to_string());
    }
    match chain {
        WalletChain::Evm => {
            Address::from_str(trimmed)
                .map_err(|e| format!("invalid EVM address '{trimmed}': {e}"))?;
            Ok(trimmed.to_string())
        }
        WalletChain::Btc => chain_btc::validate_btc_address(trimmed),
        WalletChain::Solana => chain_sol::validate_solana_address(trimmed),
        WalletChain::Tron => chain_tron::validate_tron_address(trimmed),
    }
}

pub(crate) fn validate_calldata(data: &str) -> Result<String, String> {
    let trimmed = data.trim();
    if !trimmed.starts_with("0x") {
        return Err("calldata must be 0x-prefixed hex".to_string());
    }
    let body = &trimmed[2..];
    if body.len() % 2 != 0 {
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

pub fn hex_to_u256(hex_value: &str) -> Result<U256, String> {
    let trimmed = hex_value.trim();
    let normalized = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    U256::from_str_radix(normalized, 16)
        .map_err(|e| format!("invalid hex quantity '{hex_value}': {e}"))
}

pub fn u256_to_hex(value: U256) -> String {
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

pub async fn balances() -> Result<RpcOutcome<Vec<BalanceInfo>>, String> {
    let status = wallet_status().await?.value;
    if !status.configured {
        return Err("wallet is not configured; run wallet setup first".to_string());
    }
    let mut out = Vec::with_capacity(status.accounts.len() + EVM_BALANCE_NETWORKS.len());
    for account in &status.accounts {
        match account.chain {
            // The EVM account fans out into one native-balance row per displayed
            // network (Ethereum, Base, BNB Chain), all sharing the same address.
            WalletChain::Evm => {
                for network in EVM_BALANCE_NETWORKS {
                    let asset = evm_native_asset(network)?;
                    let (raw, provider_status) = match chain_evm::evm_balance(
                        network,
                        &account.address,
                    )
                    .await
                    {
                        Ok(balance) => (balance.to_string(), ProviderStatus::Ready),
                        Err(error) => {
                            warn!(
                                    "{LOG_PREFIX} balances chain=evm network={} address={} falling back to zero: {error}",
                                    network.as_str(),
                                    account.address
                                );
                            ("0".to_string(), ProviderStatus::Missing)
                        }
                    };
                    out.push(balance_row(
                        WalletChain::Evm,
                        Some(network),
                        &account.address,
                        asset,
                        raw,
                        provider_status,
                    ));
                }
            }
            WalletChain::Btc => {
                let (raw, provider_status) = match chain_btc::native_balance(&account.address).await
                {
                    Ok(sats) => (sats.to_string(), ProviderStatus::Ready),
                    Err(error) => {
                        warn!(
                            "{LOG_PREFIX} balances chain=btc address={} falling back to zero: {error}",
                            account.address
                        );
                        ("0".to_string(), ProviderStatus::Missing)
                    }
                };
                out.push(balance_row(
                    WalletChain::Btc,
                    None,
                    &account.address,
                    native_asset_for(WalletChain::Btc)?,
                    raw,
                    provider_status,
                ));
            }
            WalletChain::Solana => {
                let (raw, provider_status) = match chain_sol::native_balance(&account.address).await
                {
                    Ok(lamports) => (lamports.to_string(), ProviderStatus::Ready),
                    Err(error) => {
                        warn!(
                                "{LOG_PREFIX} balances chain=solana address={} falling back to zero: {error}",
                                account.address
                            );
                        ("0".to_string(), ProviderStatus::Missing)
                    }
                };
                out.push(balance_row(
                    WalletChain::Solana,
                    None,
                    &account.address,
                    native_asset_for(WalletChain::Solana)?,
                    raw,
                    provider_status,
                ));
            }
            WalletChain::Tron => {
                let (raw, provider_status) = match chain_tron::native_balance(&account.address)
                    .await
                {
                    Ok(sun) => (sun.to_string(), ProviderStatus::Ready),
                    Err(error) => {
                        warn!(
                            "{LOG_PREFIX} balances chain=tron address={} falling back to zero: {error}",
                            account.address
                        );
                        ("0".to_string(), ProviderStatus::Missing)
                    }
                };
                out.push(balance_row(
                    WalletChain::Tron,
                    None,
                    &account.address,
                    native_asset_for(WalletChain::Tron)?,
                    raw,
                    provider_status,
                ));
            }
        }
    }
    debug!("{LOG_PREFIX} balances returned rows={}", out.len());
    Ok(RpcOutcome::new(
        out,
        vec!["wallet balances listed".to_string()],
    ))
}

pub async fn prepare_transfer(
    params: PrepareTransferParams,
) -> Result<RpcOutcome<PreparedTransaction>, String> {
    let to = validate_address(params.chain, &params.to_address)?;
    let amount = validate_amount(&params.amount_raw)?;
    if amount == 0 {
        return Err("transfer amount must be greater than zero".to_string());
    }
    let network = if params.chain == WalletChain::Evm {
        Some(params.evm_network.unwrap_or(EvmNetwork::EthereumMainnet))
    } else {
        None
    };
    let account = require_account(params.chain).await?;
    let asset = match params.asset_symbol.as_deref().map(str::trim) {
        None | Some("") => {
            // native asset for the chain (or chosen EVM network).
            let catalog = if let Some(net) = network {
                evm_asset_catalog(net)
            } else {
                super::defaults::asset_catalog(params.chain)
            };
            catalog
                .into_iter()
                .find(|value| value.native)
                .ok_or_else(|| {
                    format!(
                        "native asset metadata missing for '{}'",
                        chain_str(params.chain)
                    )
                })?
        }
        Some(symbol) => find_asset_for_network(params.chain, network, symbol).ok_or_else(|| {
            format!(
                "unsupported asset_symbol '{symbol}' for chain '{}'",
                chain_str(params.chain)
            )
        })?,
    };
    let kind = if asset.native {
        PreparedKind::NativeTransfer
    } else {
        PreparedKind::TokenTransfer
    };
    // BTC has no native token concept; reject TokenTransfer on btc.
    if matches!(params.chain, WalletChain::Btc) && !asset.native {
        return Err("token transfers are not supported on Bitcoin".to_string());
    }
    let now = now_ms();
    let label = if let Some(net) = network {
        format!("{} ({})", chain_str(params.chain), net.network_label())
    } else {
        chain_str(params.chain).to_string()
    };
    let quote = PreparedTransaction {
        quote_id: next_quote_id(),
        kind,
        chain: params.chain,
        evm_network: network,
        from_address: account.address.clone(),
        to_address: to,
        asset_symbol: asset.symbol.clone(),
        amount_raw: amount.to_string(),
        amount_formatted: format_amount(amount, asset.decimals),
        receive_symbol: None,
        min_receive_raw: None,
        calldata: None,
        token_address: asset.contract_address.clone(),
        estimated_fee_raw: estimated_fee_raw(params.chain, kind),
        status: PreparedStatus::AwaitingConfirmation,
        created_at_ms: now,
        expires_at_ms: now + QUOTE_TTL_MS,
        notes: vec![format!(
            "Prepared {} transfer on {} using default network settings.",
            asset.symbol, label
        )],
        owner: current_owner(),
    };
    debug!(
        "{LOG_PREFIX} prepare_transfer chain={} kind={:?} quote_id={} amount={} asset={}",
        chain_str(params.chain),
        kind,
        quote.quote_id,
        quote.amount_raw,
        quote.asset_symbol
    );
    Ok(RpcOutcome::new(
        store_quote(quote),
        vec!["wallet transfer prepared".to_string()],
    ))
}

/// Resolve the EVM network for a tx read, defaulting to Ethereum mainnet.
fn read_network(chain: WalletChain, evm_network: Option<EvmNetwork>) -> Option<EvmNetwork> {
    if chain == WalletChain::Evm {
        Some(evm_network.unwrap_or(EvmNetwork::EthereumMainnet))
    } else {
        None
    }
}

/// Check the on-chain lifecycle state of a previously broadcast transaction.
pub async fn tx_status(
    chain: WalletChain,
    evm_network: Option<EvmNetwork>,
    hash: &str,
) -> Result<RpcOutcome<TxStatusInfo>, String> {
    let hash = hash.trim();
    if hash.is_empty() {
        return Err("tx hash is empty".to_string());
    }
    let info = match chain {
        WalletChain::Evm => {
            chain_evm::tx_status(read_network(chain, evm_network).unwrap(), hash).await?
        }
        WalletChain::Btc => chain_btc::tx_status(hash).await?,
        WalletChain::Solana => chain_sol::tx_status(hash).await?,
        WalletChain::Tron => chain_tron::tx_status(hash).await?,
    };
    debug!(
        "{LOG_PREFIX} tx_status chain={} hash={} state={:?}",
        chain_str(chain),
        hash,
        info.state
    );
    Ok(RpcOutcome::new(
        info,
        vec!["wallet tx status fetched".to_string()],
    ))
}

/// Fetch the receipt of a broadcast transaction (success flag, fee, block).
pub async fn tx_receipt(
    chain: WalletChain,
    evm_network: Option<EvmNetwork>,
    hash: &str,
) -> Result<RpcOutcome<TxReceiptInfo>, String> {
    let hash = hash.trim();
    if hash.is_empty() {
        return Err("tx hash is empty".to_string());
    }
    let info = match chain {
        WalletChain::Evm => {
            chain_evm::tx_receipt(read_network(chain, evm_network).unwrap(), hash).await?
        }
        WalletChain::Btc => chain_btc::tx_receipt(hash).await?,
        WalletChain::Solana => chain_sol::tx_receipt(hash).await?,
        WalletChain::Tron => chain_tron::tx_receipt(hash).await?,
    };
    debug!(
        "{LOG_PREFIX} tx_receipt chain={} hash={} found={}",
        chain_str(chain),
        hash,
        info.found
    );
    Ok(RpcOutcome::new(
        info,
        vec!["wallet tx receipt fetched".to_string()],
    ))
}

/// Look up the raw transaction payload by hash.
pub async fn lookup_tx(
    chain: WalletChain,
    evm_network: Option<EvmNetwork>,
    hash: &str,
) -> Result<RpcOutcome<TxLookupInfo>, String> {
    let hash = hash.trim();
    if hash.is_empty() {
        return Err("tx hash is empty".to_string());
    }
    let info = match chain {
        WalletChain::Evm => {
            chain_evm::lookup_tx(read_network(chain, evm_network).unwrap(), hash).await?
        }
        WalletChain::Btc => chain_btc::lookup_tx(hash).await?,
        WalletChain::Solana => chain_sol::lookup_tx(hash).await?,
        WalletChain::Tron => chain_tron::lookup_tx(hash).await?,
    };
    debug!(
        "{LOG_PREFIX} lookup_tx chain={} hash={} found={}",
        chain_str(chain),
        hash,
        info.found
    );
    Ok(RpcOutcome::new(
        info,
        vec!["wallet tx looked up".to_string()],
    ))
}

/// Crate-internal: sign+broadcast an externally-built unsigned EVM transaction
/// (deBridge swap/bridge or generic dapp calldata). See [`chain_evm::sign_and_broadcast_evm`].
pub(crate) async fn sign_and_broadcast_evm(
    network: EvmNetwork,
    to: &str,
    data_hex: Option<String>,
    value_raw: &str,
) -> Result<RawBroadcastResult, String> {
    chain_evm::sign_and_broadcast_evm(network, to, data_hex, value_raw).await
}

/// Crate-internal: sign+broadcast an externally-built hex `VersionedTransaction`
/// (deBridge Solana swap/bridge). See [`chain_sol::sign_and_broadcast_versioned`].
pub(crate) async fn sign_and_broadcast_solana(
    tx_blob_hex: &str,
) -> Result<RawBroadcastResult, String> {
    chain_sol::sign_and_broadcast_versioned(tx_blob_hex).await
}

pub async fn execute_prepared(
    params: ExecutePreparedParams,
) -> Result<RpcOutcome<ExecutionResult>, String> {
    if !params.confirmed {
        return Err("execute_prepared requires `confirmed: true`".to_string());
    }
    // Bind execute to the chat-thread that prepared the quote.
    // `current_owner()` returns the caller's `APPROVAL_CHAT_CONTEXT` (or `None`
    // for non-chat callers). `take_quote_for` enforces equality with the
    // prepare-time owner and returns the same "not found" error on mismatch
    // — leaked `quote_id`s in a shared channel cannot be hijacked from a
    // different agent session.
    let caller = current_owner();
    // Atomically remove the quote *before* broadcasting so two concurrent
    // confirmations can't both pass get_quote() and double-submit. If signing
    // or broadcast fails we restore the quote to keep it retryable.
    let quote = take_quote_for(&params.quote_id, caller)?;
    let chain = quote.chain;
    let restorable = quote.clone();
    let result = match chain {
        WalletChain::Evm => chain_evm::execute_evm_quote(quote).await,
        WalletChain::Btc => chain_btc::execute_btc_quote(quote).await,
        WalletChain::Solana => chain_sol::execute_solana_quote(quote).await,
        WalletChain::Tron => chain_tron::execute_tron_quote(quote).await,
    };
    let result = match result {
        Ok(value) => value,
        Err(error) => {
            // Restore the quote so the caller can fix the cause and retry.
            // Refresh the TTL window so a slow chain call (network timeouts
            // can chew through the original 5-min budget) doesn't hand back
            // an immediately-expired quote.
            let mut refreshed = restorable;
            let now = now_ms();
            refreshed.expires_at_ms = now + QUOTE_TTL_MS;
            store_quote(refreshed);
            warn!(
                "{LOG_PREFIX} execute chain={} quote_id={} failed (quote restored, ttl refreshed): {error}",
                chain_str(chain),
                params.quote_id
            );
            return Err(error);
        }
    };
    let explorer_fallback = explorer_tx_url(chain, &result.transaction_hash);
    let mut final_result = result;
    if final_result.explorer_url.is_none() {
        final_result.explorer_url = explorer_fallback;
    }
    Ok(RpcOutcome::new(
        final_result,
        vec!["wallet transaction broadcast".to_string()],
    ))
}

#[cfg(test)]
#[path = "execution_tests.rs"]
mod tests;
