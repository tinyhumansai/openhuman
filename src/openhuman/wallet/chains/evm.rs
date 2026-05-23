//! EVM signing + broadcast (Ethereum mainnet + L2s: Base, Arbitrum,
//! Optimism, Polygon). Single key derivation path (`m/44'/60'/...`) — every
//! EVM network shares the same address.
//!
//! Network selection comes from `PreparedTransaction.evm_network`. If it's
//! `None` (legacy quotes or callers that didn't specify), Ethereum mainnet
//! is assumed.

use std::str::FromStr;

use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_core::types::{Address, Bytes, NameOrAddress, TransactionRequest, U256};
use ethers_signers::{coins_bip39::English, MnemonicBuilder, Signer};
use log::debug;
use serde_json::json;

use crate::openhuman::config::rpc as config_rpc;

use super::super::abi::encode_erc20_transfer;
use super::super::defaults::{
    explorer_tx_url_for_evm_network, rpc_url_for_evm_network, EvmNetwork,
};
use super::super::execution::{
    hex_to_bytes, hex_to_u256, u256_to_hex, ExecutionResult, PreparedKind, PreparedStatus,
    PreparedTransaction,
};
use super::super::ops::{secret_material, WalletChain};
use super::super::rpc::{evm_rpc_call, rpc_call_to};

const LOG_PREFIX: &str = "[wallet::evm]";

pub async fn evm_balance(network: EvmNetwork, address: &str) -> Result<U256, String> {
    let raw: String = evm_rpc_call(network, "eth_getBalance", json!([address, "latest"])).await?;
    hex_to_u256(&raw)
}

pub async fn execute_evm_quote(mut quote: PreparedTransaction) -> Result<ExecutionResult, String> {
    let network = quote.evm_network.unwrap_or(EvmNetwork::EthereumMainnet);
    let rpc_url = rpc_url_for_evm_network(network);
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

    let chain_id_hex: String = rpc_call_to(&rpc_url, "eth_chainId", json!([])).await?;
    // Use "pending" so already-submitted-but-not-mined txs don't cause a
    // nonce collision when two confirmations land back-to-back.
    let nonce_hex: String = rpc_call_to(
        &rpc_url,
        "eth_getTransactionCount",
        json!([quote.from_address, "pending"]),
    )
    .await?;
    let gas_price_hex: String = rpc_call_to(&rpc_url, "eth_gasPrice", json!([])).await?;
    let mut estimate_tx = json!({
        "from": quote.from_address,
        "to": format!("{tx_to:#x}"),
        "value": u256_to_hex(tx_value),
    });
    if let Some(data_hex) = tx_data.as_deref() {
        estimate_tx["data"] = json!(data_hex);
    }
    let gas_hex: String = rpc_call_to(&rpc_url, "eth_estimateGas", json!([estimate_tx])).await?;
    let chain_id = hex_to_u256(&chain_id_hex)?.as_u64();
    if chain_id != network.chain_id() {
        return Err(format!(
            "EVM RPC chain_id mismatch: rpc reported {} but network {} expects {}",
            chain_id,
            network.as_str(),
            network.chain_id()
        ));
    }
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
    let tx_hash: String = rpc_call_to(&rpc_url, "eth_sendRawTransaction", json!([raw_tx])).await?;
    quote.estimated_fee_raw = gas_price.checked_mul(gas).unwrap_or_default().to_string();
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

pub fn validate_evm_address(addr: &str) -> Result<String, String> {
    let trimmed = addr.trim();
    if trimmed.is_empty() {
        return Err("address is empty".to_string());
    }
    Address::from_str(trimmed).map_err(|e| format!("invalid EVM address '{trimmed}': {e}"))?;
    Ok(trimmed.to_string())
}
