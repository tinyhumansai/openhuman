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
    compressed_public_key, hex_to_u128, u128_to_hex, ExecutionResult, PreparedKind, PreparedStatus,
    PreparedTransaction, RawBroadcastResult, TxLookupInfo, TxReceiptInfo, TxState, TxStatusInfo,
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
    // Derivation stays here, and so does the key. `tinywallet::key` owns BIP-32
    // for every chain; what changed is that the *signing* no longer happens in
    // this binary either — the transaction is encoded by the loaded wallet
    // module, which hands back a digest for this process to sign. Custody is
    // unchanged: the mnemonic is decrypted from the keyring above, handed over
    // as a `&str` that is not retained, and never crosses the bus.
    let derived = tinywallet::key::derive(
        tinywallet::Chain::Evm,
        mnemonic.as_str(),
        &secret.derivation_path,
    )
    .map_err(|e| e.to_string())?;
    let public_key = compressed_public_key(derived.secret_bytes())?;

    let to = tinywallet::address::evm::validate(to)
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

    let transaction = tinywallet::wire::TransactionSpec::Evm {
        to,
        value_wei: value.to_string(),
        data_hex: tx_data.unwrap_or_default(),
        nonce,
        gas_limit,
        gas_price_wei: gas_price.to_string(),
        chain_id,
    };
    let signed = crate::openhuman::modules::wallet::sign_transaction(
        &config,
        &transaction,
        derived.secret_bytes(),
        &public_key,
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
            tinywallet::address::evm::validate(&quote.to_address).map_err(|e| {
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
                tinywallet::address::evm::validate(token)
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
/// Delegates to the vendored [`tinywallet`] crate, which owns the address
/// format; this wrapper keeps the `Result<_, String>` shape the rest of the
/// domain speaks. Accepts exactly what the previous `ethers-core` based check
/// accepted — prefixed, unprefixed, and any hex case, but not an uppercase
/// `0X` prefix.
pub fn validate_evm_address(addr: &str) -> Result<String, String> {
    let result = tinywallet::address::evm::validate(addr).map_err(|e| e.to_string());
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
mod tests {
    use super::*;
    use crate::openhuman::web3::wallet::execution::TxState;
    use crate::openhuman::web3::wallet::test_support::{setup_wallet_in, TEST_LOCK};
    use axum::{routing::post, Router};
    use serde_json::Value as JsonValue;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::net::TcpListener;

    /// Mock EVM JSON-RPC node. `receipt` is the value returned for
    /// `eth_getTransactionReceipt`; `tx` for `eth_getTransactionByHash`.
    async fn start_evm_mock(
        receipt: JsonValue,
        tx: JsonValue,
    ) -> (
        std::net::SocketAddr,
        Arc<parking_lot::Mutex<Vec<JsonValue>>>,
    ) {
        let calls: Arc<parking_lot::Mutex<Vec<JsonValue>>> =
            Arc::new(parking_lot::Mutex::new(Vec::new()));
        let calls_c = calls.clone();
        let app = Router::new().route(
            "/",
            post(move |axum::Json(payload): axum::Json<JsonValue>| {
                let calls = calls_c.clone();
                let receipt = receipt.clone();
                let tx = tx.clone();
                async move {
                    calls.lock().push(payload.clone());
                    let method = payload
                        .get("method")
                        .and_then(JsonValue::as_str)
                        .unwrap_or_default();
                    let result = match method {
                        "eth_getTransactionReceipt" => receipt,
                        "eth_getTransactionByHash" => tx,
                        "eth_blockNumber" => JsonValue::String("0x12".to_string()),
                        "eth_chainId" => JsonValue::String("0x1".to_string()),
                        "eth_getTransactionCount" => JsonValue::String("0x1".to_string()),
                        "eth_gasPrice" => JsonValue::String("0x3b9aca00".to_string()),
                        "eth_estimateGas" => JsonValue::String("0x5208".to_string()),
                        "eth_sendRawTransaction" => {
                            JsonValue::String(format!("0x{}", "ab".repeat(32)))
                        }
                        _ => JsonValue::Null,
                    };
                    axum::Json(serde_json::json!({"jsonrpc":"2.0","id":1,"result":result}))
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (addr, calls)
    }

    fn set_evm_rpc(addr: std::net::SocketAddr) {
        std::env::set_var("OPENHUMAN_WALLET_RPC_EVM", format!("http://{addr}"));
    }

    #[tokio::test]
    async fn tx_status_confirmed_with_confirmations() {
        let _guard = TEST_LOCK.lock();
        let _env = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let receipt = serde_json::json!({"status": "0x1", "blockNumber": "0x10"});
        let (addr, _calls) = start_evm_mock(receipt, JsonValue::Null).await;
        set_evm_rpc(addr);
        let info = tx_status(EvmNetwork::EthereumMainnet, "0xabc")
            .await
            .unwrap();
        assert_eq!(info.state, TxState::Confirmed);
        assert_eq!(info.block_number, Some(16));
        // tip 0x12 (18) - block 16 + 1 = 3 confirmations.
        assert_eq!(info.confirmations, Some(3));
    }

    #[tokio::test]
    async fn tx_status_pending_when_no_receipt_but_tx_present() {
        let _guard = TEST_LOCK.lock();
        let _env = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (addr, _calls) =
            start_evm_mock(JsonValue::Null, serde_json::json!({"hash": "0xabc"})).await;
        set_evm_rpc(addr);
        let info = tx_status(EvmNetwork::EthereumMainnet, "0xabc")
            .await
            .unwrap();
        assert_eq!(info.state, TxState::Pending);
    }

    #[tokio::test]
    async fn tx_status_not_found_when_receipt_and_tx_null() {
        let _guard = TEST_LOCK.lock();
        let _env = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (addr, _calls) = start_evm_mock(JsonValue::Null, JsonValue::Null).await;
        set_evm_rpc(addr);
        let info = tx_status(EvmNetwork::EthereumMainnet, "0xabc")
            .await
            .unwrap();
        assert_eq!(info.state, TxState::NotFound);
    }

    #[tokio::test]
    async fn tx_receipt_extracts_fee_and_success() {
        let _guard = TEST_LOCK.lock();
        let _env = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let receipt = serde_json::json!({
            "status": "0x1",
            "blockNumber": "0x10",
            "gasUsed": "0x5208",       // 21000
            "effectiveGasPrice": "0x3b9aca00" // 1 gwei
        });
        let (addr, _calls) = start_evm_mock(receipt, JsonValue::Null).await;
        set_evm_rpc(addr);
        let info = tx_receipt(EvmNetwork::EthereumMainnet, "0xabc")
            .await
            .unwrap();
        assert!(info.found);
        assert_eq!(info.success, Some(true));
        assert_eq!(info.gas_used.as_deref(), Some("21000"));
        // 21000 * 1_000_000_000 = 21_000_000_000_000
        assert_eq!(info.fee_raw.as_deref(), Some("21000000000000"));
    }

    #[tokio::test]
    async fn tx_receipt_pending_is_found_when_tx_known() {
        let _guard = TEST_LOCK.lock();
        let _env = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // No receipt yet, but the node knows the tx hash → pending, found=true.
        let (addr, _calls) =
            start_evm_mock(JsonValue::Null, serde_json::json!({"hash": "0xabc"})).await;
        set_evm_rpc(addr);
        let info = tx_receipt(EvmNetwork::EthereumMainnet, "0xabc")
            .await
            .unwrap();
        assert!(info.found);
        assert_eq!(info.success, None);
    }

    #[tokio::test]
    async fn lookup_tx_reports_found_flag() {
        let _guard = TEST_LOCK.lock();
        let _env = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (addr, _calls) =
            start_evm_mock(JsonValue::Null, serde_json::json!({"hash": "0xabc"})).await;
        set_evm_rpc(addr);
        let info = lookup_tx(EvmNetwork::EthereumMainnet, "0xabc")
            .await
            .unwrap();
        assert!(info.found);
    }

    // Drives the real wallet module, so it must be the only such test in its
    // process: tinybus never unloads a module, and the module bus belongs to
    // whichever tokio runtime created it — a second `#[tokio::test]` finds a
    // broker whose tasks died with the first and the call fails with
    // "connection closed". Verified passing in isolation:
    //
    //   cargo test -p openhuman --lib --features "$(bash scripts/ci/product-features.sh)" \
    //     sign_and_broadcast_evm_signs_raw_calldata -- --ignored --test-threads=1
    //
    // Same constraint tinydocs documents for its module-backed tool tests.
    #[ignore = "drives the loaded wallet module; must run alone in its process"]
    #[tokio::test]
    async fn sign_and_broadcast_evm_signs_raw_calldata() {
        let _guard = TEST_LOCK.lock();
        let temp = TempDir::new().unwrap();
        let _workspace_guard = setup_wallet_in(&temp).await.unwrap();
        let (addr, calls) = start_evm_mock(JsonValue::Null, JsonValue::Null).await;
        set_evm_rpc(addr);
        let result = sign_and_broadcast_evm(
            EvmNetwork::EthereumMainnet,
            "0x1111111111111111111111111111111111111111",
            Some("0xabcdef".to_string()),
            "0",
        )
        .await
        .expect("broadcast ok");
        assert_eq!(result.transaction_hash, format!("0x{}", "ab".repeat(32)));
        assert!(result.explorer_url.is_some());
        // The raw tx must have been broadcast.
        let sent = calls
            .lock()
            .iter()
            .any(|c| c.get("method").and_then(|v| v.as_str()) == Some("eth_sendRawTransaction"));
        assert!(sent, "expected eth_sendRawTransaction call");
    }
}
