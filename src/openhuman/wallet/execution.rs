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
    Swap,
    ContractCall,
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
pub struct PrepareSwapParams {
    pub chain: WalletChain,
    pub from_symbol: String,
    pub to_symbol: String,
    pub amount_in_raw: String,
    pub slippage_bps: u32,
    pub router_address: String,
    #[serde(default)]
    pub evm_network: Option<EvmNetwork>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareContractCallParams {
    pub chain: WalletChain,
    pub contract_address: String,
    pub calldata: String,
    #[serde(default = "zero_string")]
    pub value_raw: String,
    #[serde(default)]
    pub evm_network: Option<EvmNetwork>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutePreparedParams {
    pub quote_id: String,
    pub confirmed: bool,
}

fn zero_string() -> String {
    "0".to_string()
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
        (WalletChain::Evm, PreparedKind::Swap) => 200_000u128 * 30_000_000_000,
        (WalletChain::Evm, PreparedKind::ContractCall) => 100_000u128 * 30_000_000_000,
        (WalletChain::Btc, _) => 5_000,
        (WalletChain::Solana, PreparedKind::NativeTransfer) => 5_000,
        (WalletChain::Solana, PreparedKind::TokenTransfer) => 5_000,
        (WalletChain::Solana, _) => 5_000,
        (WalletChain::Tron, PreparedKind::NativeTransfer) => 1_000_000,
        (WalletChain::Tron, PreparedKind::TokenTransfer) => 15_000_000,
        (WalletChain::Tron, _) => 1_000_000,
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

pub async fn balances() -> Result<RpcOutcome<Vec<BalanceInfo>>, String> {
    let status = wallet_status().await?.value;
    if !status.configured {
        return Err("wallet is not configured; run wallet setup first".to_string());
    }
    let mut out = Vec::with_capacity(status.accounts.len());
    for account in &status.accounts {
        let asset = super::defaults::asset_catalog(account.chain)
            .into_iter()
            .find(|value| value.native)
            .ok_or_else(|| {
                format!(
                    "native asset metadata missing for '{}'",
                    chain_str(account.chain)
                )
            })?;
        let (raw, provider_status) = match account.chain {
            WalletChain::Evm => {
                match chain_evm::evm_balance(EvmNetwork::EthereumMainnet, &account.address).await {
                    Ok(balance) => (balance.to_string(), ProviderStatus::Ready),
                    Err(error) => {
                        warn!(
                        "{LOG_PREFIX} balances chain=evm address={} falling back to zero: {error}",
                        account.address
                    );
                        ("0".to_string(), ProviderStatus::Missing)
                    }
                }
            }
            WalletChain::Btc => match chain_btc::native_balance(&account.address).await {
                Ok(sats) => (sats.to_string(), ProviderStatus::Ready),
                Err(error) => {
                    warn!(
                        "{LOG_PREFIX} balances chain=btc address={} falling back to zero: {error}",
                        account.address
                    );
                    ("0".to_string(), ProviderStatus::Missing)
                }
            },
            WalletChain::Solana => match chain_sol::native_balance(&account.address).await {
                Ok(lamports) => (lamports.to_string(), ProviderStatus::Ready),
                Err(error) => {
                    warn!(
                        "{LOG_PREFIX} balances chain=solana address={} falling back to zero: {error}",
                        account.address
                    );
                    ("0".to_string(), ProviderStatus::Missing)
                }
            },
            WalletChain::Tron => match chain_tron::native_balance(&account.address).await {
                Ok(sun) => (sun.to_string(), ProviderStatus::Ready),
                Err(error) => {
                    warn!(
                        "{LOG_PREFIX} balances chain=tron address={} falling back to zero: {error}",
                        account.address
                    );
                    ("0".to_string(), ProviderStatus::Missing)
                }
            },
        };
        let raw_u128 = raw.parse::<u128>().unwrap_or(0);
        out.push(BalanceInfo {
            chain: account.chain,
            evm_network: if account.chain == WalletChain::Evm {
                Some(EvmNetwork::EthereumMainnet)
            } else {
                None
            },
            address: account.address.clone(),
            asset_symbol: asset.symbol,
            decimals: asset.decimals,
            raw,
            formatted: format_amount(raw_u128, asset.decimals),
            provider_status,
        });
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

pub async fn prepare_swap(
    params: PrepareSwapParams,
) -> Result<RpcOutcome<PreparedTransaction>, String> {
    if params.from_symbol.trim().is_empty() || params.to_symbol.trim().is_empty() {
        return Err("swap requires non-empty from_symbol and to_symbol".to_string());
    }
    if params.from_symbol.eq_ignore_ascii_case(&params.to_symbol) {
        return Err("swap from_symbol and to_symbol must differ".to_string());
    }
    if params.slippage_bps > 5_000 {
        return Err("slippage_bps too high (cap 5000 = 50%)".to_string());
    }
    let amount = validate_amount(&params.amount_in_raw)?;
    if amount == 0 {
        return Err("swap amount_in_raw must be greater than zero".to_string());
    }
    let router = validate_address(params.chain, &params.router_address)?;
    let account = require_account(params.chain).await?;
    let network = if params.chain == WalletChain::Evm {
        Some(params.evm_network.unwrap_or(EvmNetwork::EthereumMainnet))
    } else {
        None
    };
    let native_decimals = if let Some(net) = network {
        evm_asset_catalog(net)
            .into_iter()
            .find(|value| value.native)
            .map(|value| value.decimals)
            .unwrap_or(18)
    } else {
        super::defaults::asset_catalog(params.chain)
            .into_iter()
            .find(|value| value.native)
            .map(|value| value.decimals)
            .unwrap_or(18)
    };
    let min_out = amount.saturating_mul((10_000 - params.slippage_bps) as u128) / 10_000;
    let now = now_ms();
    let quote = PreparedTransaction {
        quote_id: next_quote_id(),
        kind: PreparedKind::Swap,
        chain: params.chain,
        evm_network: network,
        from_address: account.address.clone(),
        to_address: router,
        asset_symbol: params.from_symbol.clone(),
        amount_raw: amount.to_string(),
        amount_formatted: format_amount(amount, native_decimals),
        receive_symbol: Some(params.to_symbol.clone()),
        min_receive_raw: Some(min_out.to_string()),
        calldata: None,
        token_address: None,
        estimated_fee_raw: estimated_fee_raw(params.chain, PreparedKind::Swap),
        status: PreparedStatus::AwaitingConfirmation,
        created_at_ms: now,
        expires_at_ms: now + QUOTE_TTL_MS,
        notes: vec![format!(
            "Swap {} -> {}, slippage {} bps. Real router quote required before signing.",
            params.from_symbol, params.to_symbol, params.slippage_bps
        )],
        owner: current_owner(),
    };
    debug!(
        "{LOG_PREFIX} prepare_swap chain={} quote_id={} from={} to={} slippage_bps={}",
        chain_str(params.chain),
        quote.quote_id,
        params.from_symbol,
        params.to_symbol,
        params.slippage_bps
    );
    Ok(RpcOutcome::new(
        store_quote(quote),
        vec!["wallet swap prepared".to_string()],
    ))
}

pub async fn prepare_contract_call(
    params: PrepareContractCallParams,
) -> Result<RpcOutcome<PreparedTransaction>, String> {
    if params.chain != WalletChain::Evm {
        return Err(format!(
            "contract calls are currently implemented only for EVM; got '{}'",
            chain_str(params.chain)
        ));
    }
    let contract = validate_address(params.chain, &params.contract_address)?;
    let calldata = validate_calldata(&params.calldata)?;
    let value = validate_amount(&params.value_raw)?;
    let account = require_account(params.chain).await?;
    let network = params.evm_network.unwrap_or(EvmNetwork::EthereumMainnet);
    let native = evm_asset_catalog(network)
        .into_iter()
        .find(|value| value.native)
        .ok_or_else(|| "missing native asset metadata for evm".to_string())?;
    let now = now_ms();
    let quote = PreparedTransaction {
        quote_id: next_quote_id(),
        kind: PreparedKind::ContractCall,
        chain: params.chain,
        evm_network: Some(network),
        from_address: account.address.clone(),
        to_address: contract,
        asset_symbol: native.symbol,
        amount_raw: value.to_string(),
        amount_formatted: format_amount(value, native.decimals),
        receive_symbol: None,
        min_receive_raw: None,
        calldata: Some(calldata),
        token_address: None,
        estimated_fee_raw: estimated_fee_raw(params.chain, PreparedKind::ContractCall),
        status: PreparedStatus::AwaitingConfirmation,
        created_at_ms: now,
        expires_at_ms: now + QUOTE_TTL_MS,
        notes: vec!["Contract call prepared from caller-supplied ABI/calldata.".to_string()],
        owner: current_owner(),
    };
    debug!(
        "{LOG_PREFIX} prepare_contract_call chain={} quote_id={} value={}",
        chain_str(params.chain),
        quote.quote_id,
        quote.amount_raw
    );
    Ok(RpcOutcome::new(
        store_quote(quote),
        vec!["wallet contract call prepared".to_string()],
    ))
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
mod tests {
    use std::net::SocketAddr;
    use std::sync::Arc;

    use axum::{extract::State, routing::post, Json, Router};
    use once_cell::sync::Lazy;
    use serde_json::{json, Value};
    use tempfile::TempDir;
    use tokio::net::TcpListener;

    use super::*;
    use crate::openhuman::config::rpc as config_rpc;
    use crate::openhuman::wallet::ops::{setup, WalletSetupParams, WalletSetupSource};

    pub(crate) static TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    #[derive(Clone)]
    struct MockRpcState {
        estimate_calls: Arc<Mutex<Vec<Value>>>,
        raw_txs: Arc<Mutex<Vec<String>>>,
        chain_id: String,
    }

    fn sample_account(chain: WalletChain) -> super::WalletAccount {
        super::WalletAccount {
            chain,
            address: match chain {
                WalletChain::Evm => "0x9858EfFD232B4033E47d90003D41EC34EcaEda94".to_string(),
                WalletChain::Btc => "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu".to_string(),
                WalletChain::Solana => "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk".to_string(),
                WalletChain::Tron => "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH".to_string(),
            },
            derivation_path: match chain {
                WalletChain::Evm => "m/44'/60'/0'/0/0".to_string(),
                WalletChain::Btc => "m/84'/0'/0'/0/0".to_string(),
                WalletChain::Solana => "m/44'/501'/0'/0'".to_string(),
                WalletChain::Tron => "m/44'/195'/0'/0/0".to_string(),
            },
        }
    }

    async fn mock_rpc(
        State(state): State<MockRpcState>,
        Json(payload): Json<Value>,
    ) -> Json<Value> {
        let method = payload
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = payload
            .get("params")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let result = match method {
            "eth_chainId" => Value::String(state.chain_id.clone()),
            "eth_getTransactionCount" => Value::String("0x7".to_string()),
            "eth_gasPrice" => Value::String("0x3b9aca00".to_string()),
            "eth_estimateGas" => {
                state
                    .estimate_calls
                    .lock()
                    .push(params.first().cloned().unwrap_or(Value::Null));
                Value::String("0x5208".to_string())
            }
            "eth_sendRawTransaction" => {
                if let Some(raw) = params.first().and_then(Value::as_str) {
                    state.raw_txs.lock().push(raw.to_string());
                }
                Value::String(
                    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_string(),
                )
            }
            "eth_getBalance" => Value::String("0xde0b6b3a7640000".to_string()),
            _ => Value::Null,
        };
        Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
    }

    pub(crate) async fn start_mock_rpc_with_chain_id(
        chain_id_hex: &str,
    ) -> Result<(SocketAddr, Arc<Mutex<Vec<Value>>>, Arc<Mutex<Vec<String>>>), String> {
        let estimate_calls = Arc::new(Mutex::new(Vec::new()));
        let raw_txs = Arc::new(Mutex::new(Vec::new()));
        let state = MockRpcState {
            estimate_calls: estimate_calls.clone(),
            raw_txs: raw_txs.clone(),
            chain_id: chain_id_hex.to_string(),
        };
        let app = Router::new().route("/", post(mock_rpc)).with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("failed to bind mock rpc: {e}"))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("failed to read mock rpc addr: {e}"))?;
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Ok((addr, estimate_calls, raw_txs))
    }

    async fn start_mock_rpc(
    ) -> Result<(SocketAddr, Arc<Mutex<Vec<Value>>>, Arc<Mutex<Vec<String>>>), String> {
        start_mock_rpc_with_chain_id("0x1").await
    }

    pub(crate) async fn setup_wallet_in(temp: &TempDir) -> Result<(), String> {
        std::env::set_var("OPENHUMAN_WORKSPACE", temp.path());
        let config = config_rpc::load_config_with_timeout().await?;
        let encrypted = crate::openhuman::encryption::rpc::encrypt_secret(
            &config,
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .await?
        .value;
        setup(WalletSetupParams {
            consent_granted: true,
            source: WalletSetupSource::Imported,
            mnemonic_word_count: 12,
            encrypted_mnemonic: Some(encrypted),
            accounts: [
                WalletChain::Evm,
                WalletChain::Btc,
                WalletChain::Solana,
                WalletChain::Tron,
            ]
            .into_iter()
            .map(sample_account)
            .collect(),
        })
        .await?;
        Ok(())
    }

    #[test]
    fn validates_amount_rejects_empty_and_non_numeric() {
        assert!(validate_amount("").is_err());
        assert!(validate_amount("abc").is_err());
        assert_eq!(validate_amount("42").unwrap(), 42);
    }

    #[test]
    fn validates_calldata_requires_hex() {
        assert!(validate_calldata("deadbeef").is_err());
        assert!(validate_calldata("0xZZ").is_err());
        assert!(validate_calldata("0xabc").is_err());
        assert_eq!(validate_calldata("0xdeadbeef").unwrap(), "0xdeadbeef");
    }

    #[test]
    fn formats_amount_with_decimals() {
        assert_eq!(format_amount(0, 18), "0.000000000000000000");
        assert_eq!(format_amount(1, 8), "0.00000001");
        assert_eq!(format_amount(123_456_789, 8), "1.23456789");
        assert_eq!(format_amount(100, 0), "100");
    }

    #[test]
    fn next_quote_id_is_unique_and_prefixed() {
        let a = next_quote_id();
        let b = next_quote_id();
        assert_ne!(a, b);
        assert!(a.starts_with("q_"));
    }

    #[test]
    fn quote_store_round_trips_and_expires() {
        let _guard = TEST_LOCK.lock();
        reset_quote_store_for_tests();
        let now = now_ms();
        let mut q = PreparedTransaction {
            quote_id: "q_test_1".to_string(),
            kind: PreparedKind::NativeTransfer,
            chain: WalletChain::Evm,
            evm_network: Some(EvmNetwork::EthereumMainnet),
            from_address: "0xfrom".to_string(),
            to_address: "0xto".to_string(),
            asset_symbol: "ETH".to_string(),
            amount_raw: "1".to_string(),
            amount_formatted: "0.000000000000000001".to_string(),
            receive_symbol: None,
            min_receive_raw: None,
            calldata: None,
            token_address: None,
            estimated_fee_raw: "0".to_string(),
            status: PreparedStatus::AwaitingConfirmation,
            created_at_ms: now,
            expires_at_ms: now + 60_000,
            notes: vec![],
            owner: None,
        };
        store_quote(q.clone());
        let taken = take_quote_for("q_test_1", None).expect("quote round-trips");
        assert_eq!(taken.quote_id, "q_test_1");
        assert!(
            take_quote_for("q_test_1", None).is_err(),
            "second take must fail"
        );

        q.quote_id = "q_test_2".to_string();
        q.expires_at_ms = now.saturating_sub(1);
        store_quote(q);
        let err = take_quote_for("q_test_2", None).unwrap_err();
        assert!(err.contains("expired"), "got: {err}");
    }

    #[tokio::test]
    async fn execute_prepared_requires_confirmed_flag() {
        let err = execute_prepared(ExecutePreparedParams {
            quote_id: "missing".to_string(),
            confirmed: false,
        })
        .await
        .unwrap_err();
        assert!(err.contains("confirmed: true"), "got: {err}");
    }

    #[tokio::test]
    async fn supported_assets_lists_default_erc20s_and_l2() {
        let out = supported_assets().await.unwrap();
        assert!(out
            .value
            .iter()
            .any(|asset| asset.symbol == "USDC"
                && asset.evm_network == Some(EvmNetwork::BaseMainnet)));
        assert!(out
            .value
            .iter()
            .any(|asset| asset.symbol == "ETH" && asset.native));
        assert!(out
            .value
            .iter()
            .any(|asset| asset.symbol == "USDT" && asset.chain == WalletChain::Tron));
    }

    #[tokio::test]
    async fn prepare_transfer_rejects_unknown_asset_symbol() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();
        let err = prepare_transfer(PrepareTransferParams {
            chain: WalletChain::Evm,
            to_address: "0x1111111111111111111111111111111111111111".into(),
            amount_raw: "1".into(),
            asset_symbol: Some("NOPE".into()),
            evm_network: None,
        })
        .await
        .unwrap_err();
        assert!(err.contains("unsupported asset_symbol"), "got: {err}");
    }

    #[tokio::test]
    async fn prepare_contract_call_rejects_non_evm_chain() {
        let err = prepare_contract_call(PrepareContractCallParams {
            chain: WalletChain::Btc,
            contract_address: "addr".into(),
            calldata: "0x".into(),
            value_raw: "0".into(),
            evm_network: None,
        })
        .await
        .unwrap_err();
        assert!(err.contains("only for EVM"), "got: {err}");
    }

    #[tokio::test]
    async fn execute_prepared_broadcasts_native_evm_transaction() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();
        let (addr, estimate_calls, raw_txs) = start_mock_rpc().await.unwrap();
        std::env::set_var("OPENHUMAN_WALLET_RPC_EVM", format!("http://{addr}"));

        let prepared = prepare_transfer(PrepareTransferParams {
            chain: WalletChain::Evm,
            to_address: "0x1111111111111111111111111111111111111111".into(),
            amount_raw: "1000".into(),
            asset_symbol: None,
            evm_network: None,
        })
        .await
        .unwrap()
        .value;
        let executed = execute_prepared(ExecutePreparedParams {
            quote_id: prepared.quote_id.clone(),
            confirmed: true,
        })
        .await
        .unwrap()
        .value;

        assert_eq!(executed.status, PreparedStatus::Broadcasted);
        assert!(executed.transaction_hash.starts_with("0xaaaa"));
        assert_eq!(raw_txs.lock().len(), 1);
        let estimate = estimate_calls.lock()[0].clone();
        assert_eq!(
            estimate.get("to").and_then(Value::as_str),
            Some("0x1111111111111111111111111111111111111111")
        );
    }

    #[tokio::test]
    async fn execute_prepared_broadcasts_erc20_transfer_using_default_token_catalog() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();
        let (addr, estimate_calls, raw_txs) = start_mock_rpc().await.unwrap();
        std::env::set_var("OPENHUMAN_WALLET_RPC_EVM", format!("http://{addr}"));

        let prepared = prepare_transfer(PrepareTransferParams {
            chain: WalletChain::Evm,
            to_address: "0x1111111111111111111111111111111111111111".into(),
            amount_raw: "5000000".into(),
            asset_symbol: Some("USDC".into()),
            evm_network: None,
        })
        .await
        .unwrap()
        .value;
        let executed = execute_prepared(ExecutePreparedParams {
            quote_id: prepared.quote_id.clone(),
            confirmed: true,
        })
        .await
        .unwrap()
        .value;

        assert_eq!(executed.status, PreparedStatus::Broadcasted);
        assert_eq!(raw_txs.lock().len(), 1);
        let estimate = estimate_calls.lock()[0].clone();
        assert_eq!(
            estimate.get("to").and_then(Value::as_str),
            Some("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48")
        );
        let data = estimate
            .get("data")
            .and_then(Value::as_str)
            .expect("token transfer calldata");
        assert!(data.starts_with("0xa9059cbb"));
    }

    #[tokio::test]
    async fn execute_prepared_broadcasts_native_evm_on_base_with_chain_id_8453() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();
        // Base uses chain_id 8453 = 0x2105.
        let (addr, _estimate_calls, raw_txs) =
            start_mock_rpc_with_chain_id("0x2105").await.unwrap();
        std::env::set_var("OPENHUMAN_WALLET_RPC_BASE", format!("http://{addr}"));

        let prepared = prepare_transfer(PrepareTransferParams {
            chain: WalletChain::Evm,
            to_address: "0x1111111111111111111111111111111111111111".into(),
            amount_raw: "1000".into(),
            asset_symbol: None,
            evm_network: Some(EvmNetwork::BaseMainnet),
        })
        .await
        .unwrap()
        .value;
        let executed = execute_prepared(ExecutePreparedParams {
            quote_id: prepared.quote_id.clone(),
            confirmed: true,
        })
        .await
        .unwrap()
        .value;
        assert_eq!(executed.status, PreparedStatus::Broadcasted);
        assert_eq!(executed.evm_network, Some(EvmNetwork::BaseMainnet));
        assert_eq!(raw_txs.lock().len(), 1);
    }

    /// Build a Quote-store fixture pinned to a specific owner.
    /// Bypasses prepare_* (which would need full wallet setup + mock RPC) so
    /// the owner-gate behavior can be exercised in isolation.
    fn insert_owned_quote(quote_id: &str, owner: Option<QuoteOwner>) -> PreparedTransaction {
        let now = now_ms();
        let q = PreparedTransaction {
            quote_id: quote_id.to_string(),
            kind: PreparedKind::NativeTransfer,
            chain: WalletChain::Evm,
            evm_network: Some(EvmNetwork::EthereumMainnet),
            from_address: "0xfrom".to_string(),
            to_address: "0xto".to_string(),
            asset_symbol: "ETH".to_string(),
            amount_raw: "1".to_string(),
            amount_formatted: "0.000000000000000001".to_string(),
            receive_symbol: None,
            min_receive_raw: None,
            calldata: None,
            token_address: None,
            estimated_fee_raw: "0".to_string(),
            status: PreparedStatus::AwaitingConfirmation,
            created_at_ms: now,
            expires_at_ms: now + 60_000,
            notes: vec![],
            owner,
        };
        insert_quote_for_test(q)
    }

    fn owner_a() -> QuoteOwner {
        QuoteOwner {
            thread_id: "thread-A".into(),
            client_id: "client-A".into(),
        }
    }

    fn owner_b() -> QuoteOwner {
        QuoteOwner {
            thread_id: "thread-B".into(),
            client_id: "client-B".into(),
        }
    }

    fn chat_ctx_from(owner: &QuoteOwner) -> crate::openhuman::approval::ApprovalChatContext {
        crate::openhuman::approval::ApprovalChatContext {
            thread_id: owner.thread_id.clone(),
            client_id: owner.client_id.clone(),
        }
    }

    /// Cross-thread execute must fail. The quote must remain in the store
    /// (mismatched caller cannot poison it by consuming on Alice's behalf).
    #[tokio::test]
    async fn execute_prepared_rejects_cross_owner_execution() {
        use crate::openhuman::approval::APPROVAL_CHAT_CONTEXT;
        let _guard = TEST_LOCK.lock();
        reset_quote_store_for_tests();
        let q = insert_owned_quote("q_xowner_1", Some(owner_a()));

        // Bob (owner_b) tries to execute Alice's quote. The error string is
        // intentionally identical to a not-found lookup.
        let err = APPROVAL_CHAT_CONTEXT
            .scope(
                chat_ctx_from(&owner_b()),
                execute_prepared(ExecutePreparedParams {
                    quote_id: q.quote_id.clone(),
                    confirmed: true,
                }),
            )
            .await
            .unwrap_err();
        assert_eq!(err, format!("quote '{}' not found", q.quote_id));

        // The quote must still be present and executable by Alice — Bob's
        // failed attempt didn't poison the store.
        let still_present = prepared_quotes_for_test();
        assert!(
            still_present.iter().any(|p| p.quote_id == q.quote_id),
            "quote must survive owner-mismatch attempts"
        );
    }

    /// Same-thread execute must pass the owner gate (it will fail later in
    /// the chain code because there's no mock RPC set up, but the failure
    /// must not be the "not found" oracle — that proves we got past the gate).
    #[tokio::test]
    async fn execute_prepared_allows_same_owner_execution() {
        use crate::openhuman::approval::APPROVAL_CHAT_CONTEXT;
        let _guard = TEST_LOCK.lock();
        reset_quote_store_for_tests();
        let q = insert_owned_quote("q_same_owner_1", Some(owner_a()));

        let result = APPROVAL_CHAT_CONTEXT
            .scope(
                chat_ctx_from(&owner_a()),
                execute_prepared(ExecutePreparedParams {
                    quote_id: q.quote_id.clone(),
                    confirmed: true,
                }),
            )
            .await;
        // Past the owner gate. Chain code may error (no mock RPC, no wallet
        // setup) — but it must NOT be the "not found" shape we use for the
        // owner-mismatch oracle.
        if let Err(err) = &result {
            assert_ne!(
                err,
                &format!("quote '{}' not found", q.quote_id),
                "same-owner path must not return the owner-mismatch oracle"
            );
        }
    }

    /// No-context prepare + no-context execute must work. Keeps CLI / direct
    /// JSON-RPC flows usable.
    #[tokio::test]
    async fn execute_prepared_allows_no_context_flows() {
        let _guard = TEST_LOCK.lock();
        reset_quote_store_for_tests();
        let q = insert_owned_quote("q_no_ctx_1", None);

        // No APPROVAL_CHAT_CONTEXT scope — current_owner() returns None on
        // both prepare and execute, so the gate passes.
        let result = execute_prepared(ExecutePreparedParams {
            quote_id: q.quote_id.clone(),
            confirmed: true,
        })
        .await;
        if let Err(err) = &result {
            assert_ne!(
                err,
                &format!("quote '{}' not found", q.quote_id),
                "no-context path must not return the owner-mismatch oracle"
            );
        }
    }

    /// A quote prepared inside a chat context must NOT be executable from a
    /// caller with no context. Prevents privilege-drop into background /
    /// triage / cron flows that wouldn't surface UI confirmation.
    #[tokio::test]
    async fn execute_prepared_rejects_chat_quote_from_no_context_caller() {
        let _guard = TEST_LOCK.lock();
        reset_quote_store_for_tests();
        let q = insert_owned_quote("q_chat_to_bg_1", Some(owner_a()));

        // No scope around execute → caller_owner = None ≠ Some(owner_a).
        let err = execute_prepared(ExecutePreparedParams {
            quote_id: q.quote_id.clone(),
            confirmed: true,
        })
        .await
        .unwrap_err();
        assert_eq!(err, format!("quote '{}' not found", q.quote_id));
    }

    /// Lock the error-shape invariant: cross-owner reject string MUST be
    /// byte-equal to the not-found string. Regressions here would re-open
    /// the enumeration-oracle gap.
    #[tokio::test]
    async fn execute_prepared_owner_mismatch_error_matches_not_found_shape() {
        use crate::openhuman::approval::APPROVAL_CHAT_CONTEXT;
        let _guard = TEST_LOCK.lock();
        reset_quote_store_for_tests();

        // Real (owner-mismatched) quote.
        let real = insert_owned_quote("q_real_1", Some(owner_a()));
        // Reach the mismatched-owner branch.
        let mismatch_err = APPROVAL_CHAT_CONTEXT
            .scope(
                chat_ctx_from(&owner_b()),
                execute_prepared(ExecutePreparedParams {
                    quote_id: real.quote_id.clone(),
                    confirmed: true,
                }),
            )
            .await
            .unwrap_err();

        // Reach the genuine not-found branch (no quote with this id in store).
        let missing_err = APPROVAL_CHAT_CONTEXT
            .scope(
                chat_ctx_from(&owner_b()),
                execute_prepared(ExecutePreparedParams {
                    quote_id: "q_does_not_exist".into(),
                    confirmed: true,
                }),
            )
            .await
            .unwrap_err();

        // Both branches surface the exact same template, parameterised only
        // by quote_id. No enumeration oracle.
        assert_eq!(mismatch_err, format!("quote '{}' not found", real.quote_id));
        assert_eq!(missing_err, "quote 'q_does_not_exist' not found");
    }

    /// Verify that `prepare_transfer` inside an `APPROVAL_CHAT_CONTEXT` scope
    /// actually stamps `owner` via the task-local — not just via test helpers.
    #[tokio::test]
    async fn prepare_stamps_owner_via_task_local() {
        use crate::openhuman::approval::APPROVAL_CHAT_CONTEXT;
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();

        let expected = owner_a();
        let ctx = chat_ctx_from(&expected);

        let prepared = APPROVAL_CHAT_CONTEXT
            .scope(
                ctx,
                prepare_transfer(PrepareTransferParams {
                    chain: WalletChain::Evm,
                    to_address: "0x1111111111111111111111111111111111111111".into(),
                    amount_raw: "1000".into(),
                    asset_symbol: None,
                    evm_network: Some(EvmNetwork::EthereumMainnet),
                }),
            )
            .await
            .unwrap()
            .value;

        assert_eq!(
            prepared.owner,
            Some(expected),
            "prepare_transfer must stamp owner from APPROVAL_CHAT_CONTEXT"
        );
    }

    #[tokio::test]
    async fn execute_prepared_rejects_evm_chain_id_mismatch() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();
        // Quote says Base; mock reports Ethereum (0x1) — must fail.
        let (addr, _e, _r) = start_mock_rpc_with_chain_id("0x1").await.unwrap();
        std::env::set_var("OPENHUMAN_WALLET_RPC_BASE", format!("http://{addr}"));

        let prepared = prepare_transfer(PrepareTransferParams {
            chain: WalletChain::Evm,
            to_address: "0x1111111111111111111111111111111111111111".into(),
            amount_raw: "1000".into(),
            asset_symbol: None,
            evm_network: Some(EvmNetwork::BaseMainnet),
        })
        .await
        .unwrap()
        .value;
        let err = execute_prepared(ExecutePreparedParams {
            quote_id: prepared.quote_id.clone(),
            confirmed: true,
        })
        .await
        .unwrap_err();
        assert!(err.contains("chain_id mismatch"), "got: {err}");
    }
}
