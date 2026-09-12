//! EVM signing + broadcast (Ethereum mainnet + L2s: Base, Arbitrum,
//! Optimism, Polygon). Single key derivation path (`m/44'/60'/...`) — every
//! EVM network shares the same address.
//!
//! Network selection comes from `PreparedTransaction.evm_network`. If it's
//! `None` (legacy quotes or callers that didn't specify), Ethereum mainnet
//! is assumed.

use log::debug;
use serde_json::json;

use crate::openhuman::config::rpc as config_rpc;

use super::super::abi::encode_erc20_transfer;
use super::super::defaults::{
    explorer_tx_url_for_evm_network, rpc_url_for_evm_network, EvmNetwork,
};
use super::super::execution::{
    hex_to_u128, u128_to_hex, ExecutionResult, PreparedKind, PreparedStatus, PreparedTransaction,
    RawBroadcastResult, TxLookupInfo, TxReceiptInfo, TxState, TxStatusInfo,
};
use super::super::ops::{secret_material, WalletChain};
use super::super::rpc::{evm_rpc_call, rpc_call_to};

const LOG_PREFIX: &str = "[wallet::evm]";

pub async fn evm_balance(network: EvmNetwork, address: &str) -> Result<u128, String> {
    let raw: String = evm_rpc_call(network, "eth_getBalance", json!([address, "latest"])).await?;
    hex_to_u128(&raw)
}

/// Sign an EVM transaction `(to, value, data)` from the wallet's encrypted
/// recovery phrase and broadcast it on `network`. Shared core behind both
/// [`execute_evm_quote`] (native + token transfers) and
/// [`sign_and_broadcast_evm`] (raw / dapp / swap calldata from the web3 layer).
///
/// Returns `(tx_hash, fee_raw)` where `fee_raw` is the simulated `gas * gasPrice`.
async fn sign_and_broadcast(
    network: EvmNetwork,
    from_address: &str,
    to: &str,
    value_raw: &str,
    tx_data: Option<String>,
) -> Result<(String, u128), String> {
    let rpc_url = rpc_url_for_evm_network(network);
    let secret = secret_material(WalletChain::Evm).await?;
    let config = config_rpc::load_config_with_timeout().await?;
    let mnemonic = crate::openhuman::security::encryption::rpc::decrypt_secret(
        &config,
        &secret.encrypted_mnemonic,
    )
    .await?
    .value;
    // Neither derivation nor signing happens in this binary any more. The
    // phrase decrypted above is handed to the loaded wallet module over a
    // confidential call, which derives, encodes, signs and assembles; this
    // process never holds a private key for the transaction it is sending.
    //
    // The module is only sent the phrase once it has proved it is an artifact
    // this build pinned — see `modules::wallet::attested_proxy`.
    let signing_secret = tinywallet_bus::wire::SecretMaterial {
        mnemonic,
        derivation_path: secret.derivation_path.clone(),
        chain: tinywallet_bus::Chain::Evm,
    };

    let to = tinywallet_bus::address::evm::validate(to)
        .map_err(|e| format!("invalid EVM target address '{to}': {e}"))?;

    let chain_id_hex: String = rpc_call_to(&rpc_url, "eth_chainId", json!([])).await?;
    // Use "pending" so already-submitted-but-not-mined txs don't cause a
    // nonce collision when two confirmations land back-to-back.
    let nonce_hex: String = rpc_call_to(
        &rpc_url,
        "eth_getTransactionCount",
        json!([from_address, "pending"]),
    )
    .await?;
    let gas_price_hex: String = rpc_call_to(&rpc_url, "eth_gasPrice", json!([])).await?;
    let value = value_raw
        .trim()
        .parse::<u128>()
        .map_err(|e| format!("invalid native value '{value_raw}': {e}"))?;
    let mut estimate_tx = json!({
        "from": from_address,
        "to": to,
        "value": u128_to_hex(value),
    });
    if let Some(data_hex) = tx_data.as_deref() {
        estimate_tx["data"] = json!(data_hex);
    }
    let gas_hex: String = rpc_call_to(&rpc_url, "eth_estimateGas", json!([estimate_tx])).await?;
    let chain_id = u64::try_from(hex_to_u128(&chain_id_hex)?)
        .map_err(|_| format!("EVM RPC reported an implausible chain_id '{chain_id_hex}'"))?;
    if chain_id != network.chain_id() {
        return Err(format!(
            "EVM RPC chain_id mismatch: rpc reported {} but network {} expects {}",
            chain_id,
            network.as_str(),
            network.chain_id()
        ));
    }
    let nonce = u64::try_from(hex_to_u128(&nonce_hex)?)
        .map_err(|_| format!("EVM RPC reported an implausible nonce '{nonce_hex}'"))?;
    let gas_price = hex_to_u128(&gas_price_hex)?;
    let gas_limit = u64::try_from(hex_to_u128(&gas_hex)?)
        .map_err(|_| format!("EVM RPC reported an implausible gas limit '{gas_hex}'"))?;

    let transaction = tinywallet_bus::wire::TransactionSpec::Evm {
        to,
        value_wei: value.to_string(),
        data_hex: tx_data.unwrap_or_default(),
        nonce,
        gas_limit,
        gas_price_wei: gas_price.to_string(),
        chain_id,
    };
    let signed = crate::openhuman::modules::wallet::sign_transaction_in_module(
        &config,
        &transaction,
        &signing_secret,
    )
    .await
    .map_err(|e| format!("failed to sign EVM transaction: {e}"))?;

    let tx_hash: String =
        rpc_call_to(&rpc_url, "eth_sendRawTransaction", json!([signed.raw])).await?;
    let fee = gas_price
        .checked_mul(u128::from(gas_limit))
        .unwrap_or_default();
    debug!(
        "{LOG_PREFIX} sign_and_broadcast network={} tx_hash={}",
        network.as_str(),
        tx_hash
    );
    Ok((tx_hash, fee))
}

pub async fn execute_evm_quote(mut quote: PreparedTransaction) -> Result<ExecutionResult, String> {
    let network = quote.evm_network.unwrap_or(EvmNetwork::EthereumMainnet);
    let (tx_to, tx_value, tx_data) = match quote.kind {
        // A native transfer pays the recipient directly and carries no data.
        PreparedKind::NativeTransfer => (
            tinywallet_bus::address::evm::validate(&quote.to_address).map_err(|e| {
                format!("invalid EVM recipient address '{}': {e}", quote.to_address)
            })?,
            quote.amount_raw.clone(),
            None,
        ),
        // A token transfer pays the *contract* zero and puts the recipient and
        // the amount in the calldata instead.
        PreparedKind::TokenTransfer => {
            let token = quote
                .token_address
                .as_deref()
                .ok_or_else(|| "prepared token transfer is missing token_address".to_string())?;
            let calldata = encode_erc20_transfer(&quote.to_address, &quote.amount_raw)?;
            (
                tinywallet_bus::address::evm::validate(token)
                    .map_err(|e| format!("invalid ERC20 token contract address '{token}': {e}"))?,
                "0".to_string(),
                Some(calldata),
            )
        }
    };

    let (tx_hash, fee) =
        sign_and_broadcast(network, &quote.from_address, &tx_to, &tx_value, tx_data).await?;
    quote.estimated_fee_raw = fee.to_string();
    quote.status = PreparedStatus::Broadcasted;
    debug!(
        "{LOG_PREFIX} execute_prepared quote_id={} network={} tx_hash={}",
        quote.quote_id,
        network.as_str(),
        tx_hash
    );
    Ok(ExecutionResult {
        quote_id: quote.quote_id.clone(),
        status: PreparedStatus::Broadcasted,
        chain: WalletChain::Evm,
        evm_network: Some(network),
        transaction_hash: tx_hash.clone(),
        explorer_url: explorer_tx_url_for_evm_network(network, &tx_hash),
        transaction: quote,
    })
}

/// Crate-internal primitive: sign an externally-built unsigned EVM transaction
/// (`to` / `data` / `value`) with the wallet's key and broadcast it. Used by
/// the `web3` layer for deBridge swap/bridge transactions and generic dapp
/// contract calls. Not exposed as an agent tool or RPC controller.
pub(crate) async fn sign_and_broadcast_evm(
    network: EvmNetwork,
    to: &str,
    data_hex: Option<String>,
    value_raw: &str,
) -> Result<RawBroadcastResult, String> {
    let account = super::super::execution::require_evm_account().await?;
    let data = data_hex
        .map(|d| super::super::execution::validate_calldata(&d))
        .transpose()?;
    // `to` and `value_raw` are validated inside `sign_and_broadcast`, which is
    // the only place that needs them parsed.
    let (tx_hash, fee) = sign_and_broadcast(network, &account, to, value_raw, data).await?;
    Ok(RawBroadcastResult {
        transaction_hash: tx_hash.clone(),
        explorer_url: explorer_tx_url_for_evm_network(network, &tx_hash),
        fee_raw: Some(fee.to_string()),
    })
}

/// `eth_getTransactionReceipt` + `eth_blockNumber` → normalized status.
pub async fn tx_status(network: EvmNetwork, hash: &str) -> Result<TxStatusInfo, String> {
    let rpc_url = rpc_url_for_evm_network(network);
    let receipt: serde_json::Value =
        rpc_call_to(&rpc_url, "eth_getTransactionReceipt", json!([hash])).await?;
    if receipt.is_null() {
        // No receipt yet — distinguish pending (tx known) from not-found.
        let tx: serde_json::Value =
            rpc_call_to(&rpc_url, "eth_getTransactionByHash", json!([hash])).await?;
        let state = if tx.is_null() {
            TxState::NotFound
        } else {
            TxState::Pending
        };
        return Ok(TxStatusInfo {
            chain: WalletChain::Evm,
            evm_network: Some(network),
            hash: hash.to_string(),
            state,
            confirmations: None,
            block_number: None,
        });
    }
    let status_ok = receipt
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| hex_to_u128(s).map(|v| v != 0).unwrap_or(true))
        .unwrap_or(true);
    let block_number = receipt
        .get("blockNumber")
        .and_then(|v| v.as_str())
        .and_then(|s| hex_to_u128(s).ok())
        .map(|v| u64::try_from(v).unwrap_or(u64::MAX));
    let confirmations = match block_number {
        Some(bn) => {
            let head_hex: String = rpc_call_to(&rpc_url, "eth_blockNumber", json!([])).await?;
            hex_to_u128(&head_hex).ok().map(|head| {
                u64::try_from(head)
                    .unwrap_or(u64::MAX)
                    .saturating_sub(bn)
                    .saturating_add(1)
            })
        }
        None => None,
    };
    Ok(TxStatusInfo {
        chain: WalletChain::Evm,
        evm_network: Some(network),
        hash: hash.to_string(),
        state: if status_ok {
            TxState::Confirmed
        } else {
            TxState::Failed
        },
        confirmations,
        block_number,
    })
}

/// `eth_getTransactionReceipt` → normalized receipt with raw passthrough.
pub async fn tx_receipt(network: EvmNetwork, hash: &str) -> Result<TxReceiptInfo, String> {
    let rpc_url = rpc_url_for_evm_network(network);
    let receipt: serde_json::Value =
        rpc_call_to(&rpc_url, "eth_getTransactionReceipt", json!([hash])).await?;
    if receipt.is_null() {
        // No receipt yet — a freshly broadcast tx is still "found" if the node
        // knows the tx hash; only report not-found when both calls are null.
        let tx: serde_json::Value =
            rpc_call_to(&rpc_url, "eth_getTransactionByHash", json!([hash])).await?;
        return Ok(TxReceiptInfo {
            chain: WalletChain::Evm,
            evm_network: Some(network),
            hash: hash.to_string(),
            found: !tx.is_null(),
            success: None,
            block_number: None,
            gas_used: None,
            fee_raw: None,
            raw: serde_json::Value::Null,
        });
    }
    let success = receipt
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| hex_to_u128(s).map(|v| v != 0).unwrap_or(true));
    let block_number = receipt
        .get("blockNumber")
        .and_then(|v| v.as_str())
        .and_then(|s| hex_to_u128(s).ok())
        .map(|v| u64::try_from(v).unwrap_or(u64::MAX));
    let gas_used = receipt
        .get("gasUsed")
        .and_then(|v| v.as_str())
        .and_then(|s| hex_to_u128(s).ok());
    let effective_gas_price = receipt
        .get("effectiveGasPrice")
        .and_then(|v| v.as_str())
        .and_then(|s| hex_to_u128(s).ok());
    let fee_raw = match (gas_used, effective_gas_price) {
        (Some(g), Some(p)) => g.checked_mul(p).map(|f| f.to_string()),
        _ => None,
    };
    Ok(TxReceiptInfo {
        chain: WalletChain::Evm,
        evm_network: Some(network),
        hash: hash.to_string(),
        found: true,
        success,
        block_number,
        gas_used: gas_used.map(|g| g.to_string()),
        fee_raw,
        raw: receipt,
    })
}

/// `eth_getTransactionByHash` → raw transaction passthrough.
pub async fn lookup_tx(network: EvmNetwork, hash: &str) -> Result<TxLookupInfo, String> {
    let rpc_url = rpc_url_for_evm_network(network);
    let tx: serde_json::Value =
        rpc_call_to(&rpc_url, "eth_getTransactionByHash", json!([hash])).await?;
    Ok(TxLookupInfo {
        chain: WalletChain::Evm,
        evm_network: Some(network),
        hash: hash.to_string(),
        found: !tx.is_null(),
        raw: tx,
    })
}

/// Validate an EVM address (20 bytes, 40 hex digits, optional `0x`).
///
/// Delegates to the vendored [`tinywallet_bus`] crate, which owns the address
/// format; this wrapper keeps the `Result<_, String>` shape the rest of the
/// domain speaks. Accepts exactly what the previous `ethers-core` based check
/// accepted — prefixed, unprefixed, and any hex case, but not an uppercase
/// `0X` prefix.
pub fn validate_evm_address(addr: &str) -> Result<String, String> {
    let result = tinywallet_bus::address::evm::validate(addr).map_err(|e| e.to_string());
    debug!(
        "{LOG_PREFIX} validate_address result={}",
        if result.is_ok() {
            "accepted"
        } else {
            "rejected"
        }
    );
    result
}

#[cfg(test)]
#[path = "evm_tests.rs"]
mod tests;
