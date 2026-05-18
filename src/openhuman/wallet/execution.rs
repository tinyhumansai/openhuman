//! Wallet execution surface — read tools (balances / supported assets /
//! network defaults / chain status) and write tools (prepare-then-execute)
//! for native sends, token transfers, swaps, and contract calls.
//!
//! Execution is intentionally narrower than the metadata surface:
//! - Every write must be prepared first, then explicitly confirmed.
//! - Secret material stays encrypted at rest in core-owned storage.
//! - Mainnet EVM signing + broadcast are implemented here today.
//! - BTC / Solana / Tron remain read-only / quote-only until their
//!   providers and signing flows are actually wired.

use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_core::types::{Address, Bytes, NameOrAddress, TransactionRequest, U256};
use ethers_signers::{coins_bip39::English, MnemonicBuilder, Signer};
use log::{debug, warn};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

use super::abi::encode_erc20_transfer;
use super::defaults::{
    explorer_tx_url, find_asset, network_defaults as default_networks, rpc_url_for_chain,
    WalletAssetDefinition, WalletNetworkDefaults,
};
use super::ops::{secret_material, status as wallet_status, WalletAccount, WalletChain};
use super::rpc::rpc_call;

const LOG_PREFIX: &str = "[wallet]";
const QUOTE_TTL_MS: u64 = 5 * 60 * 1000;
const QUOTE_STORE_CAP: usize = 64;

static QUOTE_STORE: Lazy<Mutex<Vec<PreparedTransaction>>> = Lazy::new(|| Mutex::new(Vec::new()));
static QUOTE_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainStatus {
    pub chain: WalletChain,
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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResult {
    pub quote_id: String,
    pub status: PreparedStatus,
    pub chain: WalletChain,
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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareContractCallParams {
    pub chain: WalletChain,
    pub contract_address: String,
    pub calldata: String,
    #[serde(default = "zero_string")]
    pub value_raw: String,
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn next_quote_id() -> String {
    let n = QUOTE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("q_{}_{}", now_ms(), n)
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

fn chain_str(chain: WalletChain) -> &'static str {
    match chain {
        WalletChain::Evm => "evm",
        WalletChain::Btc => "btc",
        WalletChain::Solana => "solana",
        WalletChain::Tron => "tron",
    }
}

fn validate_amount(raw: &str) -> Result<u128, String> {
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
    if matches!(chain, WalletChain::Evm) {
        Address::from_str(trimmed).map_err(|e| format!("invalid EVM address '{trimmed}': {e}"))?;
    }
    Ok(trimmed.to_string())
}

fn validate_calldata(data: &str) -> Result<String, String> {
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

fn format_amount(raw: u128, decimals: u8) -> String {
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
        (WalletChain::Solana, _) => 5_000,
        (WalletChain::Tron, _) => 1_000_000,
    };
    base.to_string()
}

fn asset_to_supported(asset: WalletAssetDefinition) -> SupportedAsset {
    SupportedAsset {
        chain: asset.chain,
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

fn take_quote(quote_id: &str) -> Result<PreparedTransaction, String> {
    let mut store = QUOTE_STORE.lock();
    let now = now_ms();
    let pos = store
        .iter()
        .position(|q| q.quote_id == quote_id)
        .ok_or_else(|| format!("quote '{quote_id}' not found"))?;
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
fn reset_quote_store_for_tests() {
    QUOTE_STORE.lock().clear();
}

fn hex_to_u256(hex_value: &str) -> Result<U256, String> {
    let trimmed = hex_value.trim();
    let normalized = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    U256::from_str_radix(normalized, 16)
        .map_err(|e| format!("invalid hex quantity '{hex_value}': {e}"))
}

fn u256_to_hex(value: U256) -> String {
    format!("0x{value:x}")
}

fn hex_to_bytes(value: &str) -> Result<Vec<u8>, String> {
    let trimmed = value.trim();
    let normalized = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    hex::decode(normalized).map_err(|e| format!("invalid hex bytes '{value}': {e}"))
}

async fn evm_balance(address: &str) -> Result<U256, String> {
    let raw: String = rpc_call(
        WalletChain::Evm,
        "eth_getBalance",
        json!([address, "latest"]),
    )
    .await?;
    hex_to_u256(&raw)
}

async fn evm_tx_context(
    from_address: &str,
    to_address: &str,
    value: U256,
    data: Option<String>,
) -> Result<(u64, U256, U256), String> {
    let chain_id_hex: String = rpc_call(WalletChain::Evm, "eth_chainId", json!([])).await?;
    let nonce_hex: String = rpc_call(
        WalletChain::Evm,
        "eth_getTransactionCount",
        json!([from_address, "latest"]),
    )
    .await?;
    let gas_price_hex: String = rpc_call(WalletChain::Evm, "eth_gasPrice", json!([])).await?;
    let mut tx = json!({
        "from": from_address,
        "to": to_address,
        "value": u256_to_hex(value),
    });
    if let Some(data_hex) = data.as_deref() {
        tx["data"] = json!(data_hex);
    }
    let gas_hex: String = rpc_call(WalletChain::Evm, "eth_estimateGas", json!([tx])).await?;
    Ok((
        hex_to_u256(&chain_id_hex)?.as_u64(),
        hex_to_u256(&nonce_hex)?,
        hex_to_u256(&gas_price_hex)? * hex_to_u256(&gas_hex)?,
    ))
}

async fn execute_evm_quote(mut quote: PreparedTransaction) -> Result<ExecutionResult, String> {
    let secret = secret_material(WalletChain::Evm).await?;
    let config = config_rpc::load_config_with_timeout().await?;
    let mnemonic =
        crate::openhuman::encryption::rpc::decrypt_secret(&config, &secret.encrypted_mnemonic)
            .await?
            .value;
    let signer = MnemonicBuilder::<English>::default()
        .phrase(mnemonic.as_str())
        .derivation_path(&secret.derivation_path)
        .map_err(|e| {
            format!(
                "invalid EVM derivation path '{}': {e}",
                secret.derivation_path
            )
        })?
        .build()
        .map_err(|e| format!("failed to derive EVM signer from wallet secret: {e}"))?;
    let from = Address::from_str(&quote.from_address).map_err(|e| {
        format!(
            "invalid stored EVM sender address '{}': {e}",
            quote.from_address
        )
    })?;
    let (tx_to, tx_value, tx_data) = match quote.kind {
        PreparedKind::NativeTransfer => (
            Address::from_str(&quote.to_address).map_err(|e| {
                format!("invalid EVM recipient address '{}': {e}", quote.to_address)
            })?,
            U256::from_dec_str(&quote.amount_raw).map_err(|e| {
                format!("invalid prepared native value '{}': {e}", quote.amount_raw)
            })?,
            None,
        ),
        PreparedKind::TokenTransfer => {
            let token = quote
                .token_address
                .as_deref()
                .ok_or_else(|| "prepared token transfer is missing token_address".to_string())?;
            let calldata = encode_erc20_transfer(&quote.to_address, &quote.amount_raw)?;
            (
                Address::from_str(token)
                    .map_err(|e| format!("invalid ERC20 token contract address '{token}': {e}"))?,
                U256::zero(),
                Some(calldata),
            )
        }
        PreparedKind::ContractCall => (
            Address::from_str(&quote.to_address)
                .map_err(|e| format!("invalid contract target '{}': {e}", quote.to_address))?,
            U256::from_dec_str(&quote.amount_raw).map_err(|e| {
                format!(
                    "invalid prepared contract value '{}': {e}",
                    quote.amount_raw
                )
            })?,
            quote.calldata.clone(),
        ),
        PreparedKind::Swap => {
            return Err(
                "swap broadcast is not implemented yet; keep it in quote-only mode".to_string(),
            );
        }
    };

    let chain_id_hex: String = rpc_call(WalletChain::Evm, "eth_chainId", json!([])).await?;
    let nonce_hex: String = rpc_call(
        WalletChain::Evm,
        "eth_getTransactionCount",
        json!([quote.from_address, "latest"]),
    )
    .await?;
    let gas_price_hex: String = rpc_call(WalletChain::Evm, "eth_gasPrice", json!([])).await?;
    let mut estimate_tx = json!({
        "from": quote.from_address,
        "to": format!("{tx_to:#x}"),
        "value": u256_to_hex(tx_value),
    });
    if let Some(data_hex) = tx_data.as_deref() {
        estimate_tx["data"] = json!(data_hex);
    }
    let gas_hex: String =
        rpc_call(WalletChain::Evm, "eth_estimateGas", json!([estimate_tx])).await?;
    let chain_id = hex_to_u256(&chain_id_hex)?.as_u64();
    let nonce = hex_to_u256(&nonce_hex)?;
    let gas_price = hex_to_u256(&gas_price_hex)?;
    let gas = hex_to_u256(&gas_hex)?;

    let tx_data_bytes = tx_data
        .clone()
        .map(|value| hex_to_bytes(&value).map(Bytes::from))
        .transpose()?;
    let mut request = TransactionRequest::new()
        .from(from)
        .to(NameOrAddress::Address(tx_to))
        .value(tx_value)
        .nonce(nonce)
        .gas(gas)
        .gas_price(gas_price)
        .chain_id(chain_id);
    if let Some(data) = tx_data_bytes {
        request = request.data(data);
    }
    let tx: TypedTransaction = request.into();
    let signature = signer
        .with_chain_id(chain_id)
        .sign_transaction(&tx)
        .await
        .map_err(|e| format!("failed to sign EVM transaction: {e}"))?;
    let raw_bytes = tx.rlp_signed(&signature);
    let raw_tx = format!("0x{}", hex::encode(raw_bytes));
    let tx_hash: String =
        rpc_call(WalletChain::Evm, "eth_sendRawTransaction", json!([raw_tx])).await?;
    quote.estimated_fee_raw = gas_price.checked_mul(gas).unwrap_or_default().to_string();
    quote.status = PreparedStatus::Broadcasted;
    debug!(
        "{LOG_PREFIX} execute_prepared quote_id={} chain=evm tx_hash={} rpc={}",
        quote.quote_id,
        tx_hash,
        rpc_url_for_chain(WalletChain::Evm)
    );
    Ok(ExecutionResult {
        quote_id: quote.quote_id.clone(),
        status: PreparedStatus::Broadcasted,
        chain: WalletChain::Evm,
        transaction_hash: tx_hash.clone(),
        explorer_url: explorer_tx_url(WalletChain::Evm, &tx_hash),
        transaction: quote,
    })
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
    let assets = [
        WalletChain::Evm,
        WalletChain::Btc,
        WalletChain::Solana,
        WalletChain::Tron,
    ]
    .into_iter()
    .flat_map(super::defaults::asset_catalog)
    .map(asset_to_supported)
    .collect::<Vec<_>>();
    debug!("{LOG_PREFIX} supported_assets count={}", assets.len());
    Ok(RpcOutcome::new(
        assets,
        vec!["wallet supported_assets listed".to_string()],
    ))
}

pub async fn chain_status() -> Result<RpcOutcome<Vec<ChainStatus>>, String> {
    let status = wallet_status().await?.value;
    let rows = [
        WalletChain::Evm,
        WalletChain::Btc,
        WalletChain::Solana,
        WalletChain::Tron,
    ]
    .into_iter()
    .map(|chain| {
        let has_account = status.accounts.iter().any(|account| account.chain == chain);
        ChainStatus {
            chain,
            configured: has_account,
            provider_status: if has_account {
                ProviderStatus::Ready
            } else {
                ProviderStatus::Missing
            },
            rpc_url: rpc_url_for_chain(chain),
        }
    })
    .collect::<Vec<_>>();
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
        let (raw, provider_status) = if account.chain == WalletChain::Evm {
            match evm_balance(&account.address).await {
                Ok(balance) => (balance.to_string(), ProviderStatus::Ready),
                Err(error) => {
                    warn!(
                        "{LOG_PREFIX} balances chain=evm address={} falling back to zero placeholder: {}",
                        account.address, error
                    );
                    ("0".to_string(), ProviderStatus::Missing)
                }
            }
        } else {
            warn!(
                "{LOG_PREFIX} balances chain={} uses placeholder until native provider support lands",
                chain_str(account.chain)
            );
            ("0".to_string(), ProviderStatus::Ready)
        };
        let raw_u128 = raw.parse::<u128>().unwrap_or(0);
        out.push(BalanceInfo {
            chain: account.chain,
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
    let account = require_account(params.chain).await?;
    let asset = match params.asset_symbol.as_deref().map(str::trim) {
        None | Some("") => super::defaults::asset_catalog(params.chain)
            .into_iter()
            .find(|value| value.native)
            .ok_or_else(|| {
                format!(
                    "native asset metadata missing for '{}'",
                    chain_str(params.chain)
                )
            })?,
        Some(symbol) => find_asset(params.chain, symbol).ok_or_else(|| {
            format!(
                "unsupported asset_symbol '{symbol}' for chain '{}'",
                chain_str(params.chain)
            )
        })?,
    };
    if !asset.native && params.chain != WalletChain::Evm {
        return Err(format!(
            "token transfers are currently implemented only for EVM; got chain '{}'",
            chain_str(params.chain)
        ));
    }
    let kind = if asset.native {
        PreparedKind::NativeTransfer
    } else {
        PreparedKind::TokenTransfer
    };
    let now = now_ms();
    let quote = PreparedTransaction {
        quote_id: next_quote_id(),
        kind,
        chain: params.chain,
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
            asset.symbol,
            chain_str(params.chain)
        )],
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
    let native_decimals = super::defaults::asset_catalog(params.chain)
        .into_iter()
        .find(|value| value.native)
        .map(|value| value.decimals)
        .unwrap_or(18);
    let min_out = amount.saturating_mul((10_000 - params.slippage_bps) as u128) / 10_000;
    let now = now_ms();
    let quote = PreparedTransaction {
        quote_id: next_quote_id(),
        kind: PreparedKind::Swap,
        chain: params.chain,
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
    let native = super::defaults::asset_catalog(params.chain)
        .into_iter()
        .find(|value| value.native)
        .ok_or_else(|| "missing native asset metadata for evm".to_string())?;
    let now = now_ms();
    let quote = PreparedTransaction {
        quote_id: next_quote_id(),
        kind: PreparedKind::ContractCall,
        chain: params.chain,
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
    let quote = get_quote(&params.quote_id)?;
    let result = match quote.chain {
        WalletChain::Evm => execute_evm_quote(quote).await?,
        other => {
            return Err(format!(
                "on-chain execution is not implemented yet for chain '{}'; prepare-only mode remains active",
                chain_str(other)
            ));
        }
    };
    let _ = take_quote(&params.quote_id)?;
    Ok(RpcOutcome::new(
        result,
        vec!["wallet transaction broadcast".to_string()],
    ))
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::Arc;

    use axum::{extract::State, routing::post, Json, Router};
    use once_cell::sync::Lazy;
    use serde_json::Value;
    use tempfile::TempDir;
    use tokio::net::TcpListener;

    use super::*;
    use crate::openhuman::wallet::ops::{setup, WalletSetupParams, WalletSetupSource};

    static TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    #[derive(Clone)]
    struct MockRpcState {
        estimate_calls: Arc<Mutex<Vec<Value>>>,
        raw_txs: Arc<Mutex<Vec<String>>>,
    }

    fn sample_account(chain: WalletChain) -> super::WalletAccount {
        super::WalletAccount {
            chain,
            address: match chain {
                WalletChain::Evm => "0x9858EfFD232B4033E47d90003D41EC34EcaEda94".to_string(),
                WalletChain::Btc => "btc".to_string(),
                WalletChain::Solana => "sol".to_string(),
                WalletChain::Tron => "tron".to_string(),
            },
            derivation_path: match chain {
                WalletChain::Evm => "m/44'/60'/0'/0/0".to_string(),
                WalletChain::Btc => "m/44'/0'/0'/0/0".to_string(),
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
            "eth_chainId" => Value::String("0x1".to_string()),
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

    async fn start_mock_rpc(
    ) -> Result<(SocketAddr, Arc<Mutex<Vec<Value>>>, Arc<Mutex<Vec<String>>>), String> {
        let estimate_calls = Arc::new(Mutex::new(Vec::new()));
        let raw_txs = Arc::new(Mutex::new(Vec::new()));
        let state = MockRpcState {
            estimate_calls: estimate_calls.clone(),
            raw_txs: raw_txs.clone(),
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

    async fn setup_wallet(temp: &TempDir) -> Result<(), String> {
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
        reset_quote_store_for_tests();
        let now = now_ms();
        let mut q = PreparedTransaction {
            quote_id: "q_test_1".to_string(),
            kind: PreparedKind::NativeTransfer,
            chain: WalletChain::Evm,
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
        };
        store_quote(q.clone());
        let taken = take_quote("q_test_1").expect("quote round-trips");
        assert_eq!(taken.quote_id, "q_test_1");
        assert!(take_quote("q_test_1").is_err(), "second take must fail");

        q.quote_id = "q_test_2".to_string();
        q.expires_at_ms = now.saturating_sub(1);
        store_quote(q);
        let err = take_quote("q_test_2").unwrap_err();
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
    async fn execute_prepared_keeps_quote_when_chain_is_not_supported_for_broadcast() {
        let _guard = TEST_LOCK.lock();
        reset_quote_store_for_tests();
        let now = now_ms();
        let quote = PreparedTransaction {
            quote_id: "q_retry".to_string(),
            kind: PreparedKind::NativeTransfer,
            chain: WalletChain::Btc,
            from_address: "btc-from".to_string(),
            to_address: "btc-to".to_string(),
            asset_symbol: "BTC".to_string(),
            amount_raw: "1".to_string(),
            amount_formatted: "0.00000001".to_string(),
            receive_symbol: None,
            min_receive_raw: None,
            calldata: None,
            token_address: None,
            estimated_fee_raw: "5000".to_string(),
            status: PreparedStatus::AwaitingConfirmation,
            created_at_ms: now,
            expires_at_ms: now + 60_000,
            notes: vec![],
        };
        store_quote(quote);

        let err = execute_prepared(ExecutePreparedParams {
            quote_id: "q_retry".to_string(),
            confirmed: true,
        })
        .await
        .unwrap_err();

        assert!(err.contains("not implemented yet"), "got: {err}");
        assert!(
            get_quote("q_retry").is_ok(),
            "quote should remain retryable"
        );
    }

    #[tokio::test]
    async fn supported_assets_lists_default_erc20s() {
        let out = supported_assets().await.unwrap();
        assert!(out.value.iter().any(|asset| asset.symbol == "USDC"));
        assert!(out
            .value
            .iter()
            .any(|asset| asset.symbol == "ETH" && asset.native));
    }

    #[tokio::test]
    async fn prepare_transfer_rejects_unknown_asset_symbol() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet(&temp).await.unwrap();
        let err = prepare_transfer(PrepareTransferParams {
            chain: WalletChain::Evm,
            to_address: "0x1111111111111111111111111111111111111111".into(),
            amount_raw: "1".into(),
            asset_symbol: Some("NOPE".into()),
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
        setup_wallet(&temp).await.unwrap();
        let (addr, estimate_calls, raw_txs) = start_mock_rpc().await.unwrap();
        std::env::set_var("OPENHUMAN_WALLET_RPC_EVM", format!("http://{addr}"));

        let prepared = prepare_transfer(PrepareTransferParams {
            chain: WalletChain::Evm,
            to_address: "0x1111111111111111111111111111111111111111".into(),
            amount_raw: "1000".into(),
            asset_symbol: None,
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
        setup_wallet(&temp).await.unwrap();
        let (addr, estimate_calls, raw_txs) = start_mock_rpc().await.unwrap();
        std::env::set_var("OPENHUMAN_WALLET_RPC_EVM", format!("http://{addr}"));

        let prepared = prepare_transfer(PrepareTransferParams {
            chain: WalletChain::Evm,
            to_address: "0x1111111111111111111111111111111111111111".into(),
            amount_raw: "5000000".into(),
            asset_symbol: Some("USDC".into()),
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
}
