//! Tron native TRX + TRC20 token transfer. Uses the TronGrid REST API
//! (https://api.trongrid.io) — `wallet/createtransaction`,
//! `wallet/triggersmartcontract`, `wallet/broadcasttransaction`.
//!
//! Derivation: BIP44 m/44'/195'/0'/0/0 → secp256k1 key. Tron addresses are
//! `sha3_256(uncompressed_pubkey[1..])[12..]` prefixed with 0x41, base58check.

use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use ethers_core::utils::keccak256;
use hmac::{Hmac, Mac};
use log::debug;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256, Sha512};

use crate::openhuman::config::rpc as config_rpc;

use super::super::defaults::{explorer_tx_url, rpc_url_for_chain};
use super::super::execution::{
    ExecutionResult, PreparedKind, PreparedStatus, PreparedTransaction, TxLookupInfo,
    TxReceiptInfo, TxState, TxStatusInfo,
};
use super::super::ops::{secret_material, WalletChain};
use super::super::rpc::rest_post_json;

const LOG_PREFIX: &str = "[wallet::tron]";
/// Tron address prefix (mainnet).
const TRON_PREFIX: u8 = 0x41;
/// Fixed TRC20 fee_limit (15 TRX = 15_000_000 SUN). Safe upper bound.
const TRC20_FEE_LIMIT_SUN: u64 = 15_000_000;

pub fn validate_tron_address(addr: &str) -> Result<String, String> {
    let trimmed = addr.trim();
    if trimmed.is_empty() {
        return Err("Tron address is empty".to_string());
    }
    let bytes = bs58::decode(trimmed)
        .with_check(Some(TRON_PREFIX))
        .into_vec()
        .map_err(|e| format!("invalid Tron address '{trimmed}': {e}"))?;
    if bytes.len() != 21 {
        return Err(format!(
            "invalid Tron address '{trimmed}': expected 21 bytes after base58check, got {}",
            bytes.len()
        ));
    }
    Ok(trimmed.to_string())
}

pub fn tron_address_to_hex(addr: &str) -> Result<String, String> {
    let bytes = bs58::decode(addr)
        .with_check(Some(TRON_PREFIX))
        .into_vec()
        .map_err(|e| format!("invalid Tron address '{addr}': {e}"))?;
    Ok(hex::encode(&bytes))
}

pub async fn native_balance(address: &str) -> Result<u128, String> {
    validate_tron_address(address)?;
    let base = rpc_url_for_chain(WalletChain::Tron);
    let url = format!("{}/wallet/getaccount", base.trim_end_matches('/'));
    let body = json!({
        "address": tron_address_to_hex(address)?,
        "visible": false,
    });
    let resp: Value = rest_post_json(&url, &body).await?;
    let balance = resp.get("balance").and_then(Value::as_u64).unwrap_or(0);
    Ok(balance as u128)
}

#[derive(Debug, Deserialize)]
struct CreateTransactionResponse {
    #[serde(rename = "txID")]
    tx_id: String,
    raw_data: Value,
    raw_data_hex: String,
}

#[derive(Debug, Deserialize)]
struct TriggerSmartContractResponse {
    transaction: CreateTransactionResponse,
}

fn derive_tron_keypair(
    mnemonic: &str,
    derivation_path: &str,
) -> Result<(SecretKey, String), String> {
    use coins_bip39::{English, Mnemonic};
    let mnemonic_obj: Mnemonic<English> = mnemonic
        .trim()
        .parse()
        .map_err(|e| format!("invalid BIP39 mnemonic: {e}"))?;
    let seed = mnemonic_obj
        .to_seed(None)
        .map_err(|e| format!("failed to derive BIP39 seed: {e}"))?;

    // BIP32 derivation (secp256k1) — same algorithm as BTC, distinct from Solana.
    type HmacSha512 = Hmac<Sha512>;
    let mut mac = HmacSha512::new_from_slice(b"Bitcoin seed")
        .map_err(|e| format!("HMAC init failed: {e}"))?;
    mac.update(&seed);
    let i = mac.finalize().into_bytes();
    let mut key = [0u8; 32];
    let mut chain_code = [0u8; 32];
    key.copy_from_slice(&i[..32]);
    chain_code.copy_from_slice(&i[32..]);

    let segments = parse_bip32_path(derivation_path)?;
    let secp = Secp256k1::signing_only();
    for (idx, hardened) in segments {
        let mut mac = HmacSha512::new_from_slice(&chain_code)
            .map_err(|e| format!("HMAC init failed: {e}"))?;
        if hardened {
            mac.update(&[0u8]);
            mac.update(&key);
        } else {
            let sk = SecretKey::from_slice(&key).map_err(|e| format!("bad sk: {e}"))?;
            let pk = sk.public_key(&secp);
            mac.update(&pk.serialize());
        }
        let composite = idx | if hardened { 0x8000_0000 } else { 0 };
        mac.update(&composite.to_be_bytes());
        let i = mac.finalize().into_bytes();
        let mut il = [0u8; 32];
        il.copy_from_slice(&i[..32]);
        // child = (parent + IL) mod n
        let mut child_sk = SecretKey::from_slice(&il).map_err(|e| format!("bad il: {e}"))?;
        let parent = bitcoin::secp256k1::Scalar::from_be_bytes(key)
            .map_err(|e| format!("scalar from key: {e}"))?;
        child_sk = child_sk
            .add_tweak(&parent)
            .map_err(|e| format!("child tweak failed: {e}"))?;
        key = child_sk.secret_bytes();
        chain_code.copy_from_slice(&i[32..]);
    }

    let sk = SecretKey::from_slice(&key).map_err(|e| format!("bad final sk: {e}"))?;
    let secp_all = Secp256k1::new();
    let pk = sk.public_key(&secp_all);
    let uncompressed = pk.serialize_uncompressed();
    // Drop the 0x04 prefix, then keccak256 the 64-byte payload.
    let hash = keccak256(&uncompressed[1..]);
    // Tron address = TRON_PREFIX || hash[12..]
    let mut addr_bytes = [0u8; 21];
    addr_bytes[0] = TRON_PREFIX;
    addr_bytes[1..].copy_from_slice(&hash[12..]);
    let address = bs58::encode(&addr_bytes).with_check().into_string();
    Ok((sk, address))
}

fn parse_bip32_path(path: &str) -> Result<Vec<(u32, bool)>, String> {
    let trimmed = path.trim();
    let mut iter = trimmed.split('/');
    match iter.next() {
        Some("m") => {}
        _ => return Err(format!("BIP32 path '{path}' must start with 'm'")),
    }
    let mut out = Vec::new();
    for seg in iter {
        let (idx_str, hardened) = if let Some(stripped) = seg.strip_suffix('\'') {
            (stripped, true)
        } else {
            (seg, false)
        };
        let v: u32 = idx_str
            .parse()
            .map_err(|e| format!("BIP32 path '{path}' segment '{seg}': {e}"))?;
        out.push((v, hardened));
    }
    Ok(out)
}

fn pad_left_32(bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; 32];
    if bytes.len() <= 32 {
        out[32 - bytes.len()..].copy_from_slice(bytes);
    } else {
        out.copy_from_slice(&bytes[bytes.len() - 32..]);
    }
    out
}

fn encode_trc20_transfer_param(to_hex: &str, amount: u128) -> Result<String, String> {
    // For TRC20 triggerSmartContract `parameter` field: hex-encoded ABI args
    // (no 4-byte selector — TronGrid prepends it from `function_selector`).
    // arg0: address (left-padded to 32 bytes, drop the 0x41 prefix → keep
    // last 20 bytes of the hex address).
    let addr_bytes = hex::decode(to_hex).map_err(|e| format!("invalid hex addr: {e}"))?;
    if addr_bytes.len() != 21 {
        return Err(format!(
            "expected 21-byte Tron address, got {}",
            addr_bytes.len()
        ));
    }
    let mut param = vec![0u8; 32];
    param[12..].copy_from_slice(&addr_bytes[1..]); // skip the 0x41 prefix
    let amount_bytes = amount.to_be_bytes();
    param.extend(pad_left_32(&amount_bytes[..]));
    Ok(hex::encode(param))
}

async fn create_native_transaction(
    owner_hex: &str,
    to_hex: &str,
    amount_sun: u64,
) -> Result<CreateTransactionResponse, String> {
    let base = rpc_url_for_chain(WalletChain::Tron);
    let url = format!("{}/wallet/createtransaction", base.trim_end_matches('/'));
    let body = json!({
        "owner_address": owner_hex,
        "to_address": to_hex,
        "amount": amount_sun,
        "visible": false,
    });
    rest_post_json(&url, &body).await
}

async fn trigger_trc20_transfer(
    owner_hex: &str,
    contract_hex: &str,
    parameter_hex: &str,
) -> Result<CreateTransactionResponse, String> {
    let base = rpc_url_for_chain(WalletChain::Tron);
    let url = format!("{}/wallet/triggersmartcontract", base.trim_end_matches('/'));
    let body = json!({
        "owner_address": owner_hex,
        "contract_address": contract_hex,
        "function_selector": "transfer(address,uint256)",
        "parameter": parameter_hex,
        "fee_limit": TRC20_FEE_LIMIT_SUN,
        "call_value": 0,
        "visible": false,
    });
    let resp: TriggerSmartContractResponse = rest_post_json(&url, &body).await?;
    Ok(resp.transaction)
}

async fn broadcast_signed(tx_json: Value) -> Result<Value, String> {
    let base = rpc_url_for_chain(WalletChain::Tron);
    let url = format!("{}/wallet/broadcasttransaction", base.trim_end_matches('/'));
    rest_post_json(&url, &tx_json).await
}

pub async fn execute_tron_quote(mut quote: PreparedTransaction) -> Result<ExecutionResult, String> {
    validate_tron_address(&quote.from_address)?;
    validate_tron_address(&quote.to_address)?;
    let amount: u128 = quote
        .amount_raw
        .parse()
        .map_err(|e| format!("invalid Tron amount '{}': {e}", quote.amount_raw))?;

    let owner_hex = tron_address_to_hex(&quote.from_address)?;
    let to_hex = tron_address_to_hex(&quote.to_address)?;

    let secret = secret_material(WalletChain::Tron).await?;
    let config = config_rpc::load_config_with_timeout().await?;
    let mnemonic =
        crate::openhuman::encryption::rpc::decrypt_secret(&config, &secret.encrypted_mnemonic)
            .await?
            .value;
    let (sk, derived_addr) = derive_tron_keypair(&mnemonic, &secret.derivation_path)?;
    if derived_addr != quote.from_address {
        return Err(format!(
            "Tron key derivation mismatch: derived {derived_addr} but expected {}",
            quote.from_address
        ));
    }

    let raw_tx = match quote.kind {
        PreparedKind::NativeTransfer => {
            let amount_sun: u64 = amount
                .try_into()
                .map_err(|_| format!("Tron amount {amount} exceeds u64"))?;
            create_native_transaction(&owner_hex, &to_hex, amount_sun).await?
        }
        PreparedKind::TokenTransfer => {
            let contract = quote
                .token_address
                .as_deref()
                .ok_or_else(|| "TRC20 transfer missing token_address".to_string())?;
            validate_tron_address(contract)?;
            let contract_hex = tron_address_to_hex(contract)?;
            let parameter = encode_trc20_transfer_param(&to_hex, amount)?;
            trigger_trc20_transfer(&owner_hex, &contract_hex, &parameter).await?
        }
    };

    // Tron signs sha256(raw_data_hex bytes).
    let raw_bytes = hex::decode(&raw_tx.raw_data_hex)
        .map_err(|e| format!("invalid raw_data_hex from Tron: {e}"))?;
    let mut hasher = Sha256::new();
    hasher.update(&raw_bytes);
    let hash: [u8; 32] = hasher.finalize().into();

    let secp = Secp256k1::new();
    let msg = Message::from_digest(hash);
    let sig = secp.sign_ecdsa_recoverable(&msg, &sk);
    let (rec_id, compact) = sig.serialize_compact();
    let mut sig_bytes = [0u8; 65];
    sig_bytes[..64].copy_from_slice(&compact);
    sig_bytes[64] = rec_id.to_i32() as u8;
    let sig_hex = hex::encode(sig_bytes);

    let mut tx_with_sig = serde_json::to_value(&serde_json::json!({
        "txID": raw_tx.tx_id,
        "raw_data": raw_tx.raw_data,
        "raw_data_hex": raw_tx.raw_data_hex,
        "signature": [sig_hex],
    }))
    .map_err(|e| format!("failed to build Tron signed tx: {e}"))?;
    // visible: false flag for broadcast
    tx_with_sig
        .as_object_mut()
        .expect("object")
        .insert("visible".to_string(), Value::Bool(false));

    let response = broadcast_signed(tx_with_sig).await?;
    let ok = response
        .get("result")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !ok {
        let code = response.get("code").and_then(Value::as_str).unwrap_or("");
        let msg = response
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Err(format!(
            "Tron broadcast rejected: code={code} message={msg}"
        ));
    }
    let txid = response
        .get("txid")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| raw_tx.tx_id.clone());

    quote.status = PreparedStatus::Broadcasted;
    debug!(
        "{LOG_PREFIX} broadcast quote_id={} txid={} kind={:?}",
        quote.quote_id, txid, quote.kind
    );
    let explorer_url = explorer_tx_url(WalletChain::Tron, &txid);
    Ok(ExecutionResult {
        quote_id: quote.quote_id.clone(),
        status: PreparedStatus::Broadcasted,
        chain: WalletChain::Tron,
        evm_network: None,
        transaction_hash: txid,
        explorer_url,
        transaction: quote,
    })
}

async fn tron_post(path: &str, body: Value) -> Result<Value, String> {
    let base = rpc_url_for_chain(WalletChain::Tron);
    let url = format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    rest_post_json(&url, &body).await
}

/// TronGrid `/wallet/gettransactioninfobyid` → normalized status.
pub async fn tx_status(hash: &str) -> Result<TxStatusInfo, String> {
    let info = tron_post("wallet/gettransactioninfobyid", json!({ "value": hash })).await?;
    let block_number = info.get("blockNumber").and_then(Value::as_u64);
    let (state, block_number) = match block_number {
        None => {
            // The info endpoint only has a row once the tx is mined. A freshly
            // broadcast tx is still pending — disambiguate via gettransactionbyid.
            let tx = tron_post("wallet/gettransactionbyid", json!({ "value": hash })).await?;
            let seen = tx.get("txID").is_some() || tx.get("raw_data").is_some();
            (
                if seen {
                    TxState::Pending
                } else {
                    TxState::NotFound
                },
                None,
            )
        }
        Some(bn) => {
            // `receipt.result` carries SUCCESS / REVERT / FAILED for contract txs;
            // a bare TRX transfer omits it but is successful once mined.
            let result = info
                .get("receipt")
                .and_then(|r| r.get("result"))
                .and_then(Value::as_str);
            let state = match result {
                Some("SUCCESS") | None => TxState::Confirmed,
                Some(_) => TxState::Failed,
            };
            (state, Some(bn))
        }
    };
    Ok(TxStatusInfo {
        chain: WalletChain::Tron,
        evm_network: None,
        hash: hash.to_string(),
        state,
        confirmations: None,
        block_number,
    })
}

/// TronGrid `/wallet/gettransactioninfobyid` → normalized receipt.
pub async fn tx_receipt(hash: &str) -> Result<TxReceiptInfo, String> {
    let info = tron_post("wallet/gettransactioninfobyid", json!({ "value": hash })).await?;
    let block_number = info.get("blockNumber").and_then(Value::as_u64);
    if block_number.is_none() {
        return Ok(TxReceiptInfo {
            chain: WalletChain::Tron,
            evm_network: None,
            hash: hash.to_string(),
            found: false,
            success: None,
            block_number: None,
            gas_used: None,
            fee_raw: None,
            raw: serde_json::Value::Null,
        });
    }
    let result = info
        .get("receipt")
        .and_then(|r| r.get("result"))
        .and_then(Value::as_str);
    let success = Some(matches!(result, Some("SUCCESS") | None));
    let fee_raw = info
        .get("fee")
        .and_then(Value::as_u64)
        .map(|f| f.to_string());
    let gas_used = info
        .get("receipt")
        .and_then(|r| r.get("energy_usage_total"))
        .and_then(Value::as_u64)
        .map(|g| g.to_string());
    Ok(TxReceiptInfo {
        chain: WalletChain::Tron,
        evm_network: None,
        hash: hash.to_string(),
        found: true,
        success,
        block_number,
        gas_used,
        fee_raw,
        raw: info,
    })
}

/// TronGrid `/wallet/gettransactionbyid` → raw transaction passthrough.
pub async fn lookup_tx(hash: &str) -> Result<TxLookupInfo, String> {
    let tx = tron_post("wallet/gettransactionbyid", json!({ "value": hash })).await?;
    // TronGrid returns `{}` for an unknown id.
    let found = tx.get("txID").is_some() || tx.get("raw_data").is_some();
    Ok(TxLookupInfo {
        chain: WalletChain::Tron,
        evm_network: None,
        hash: hash.to_string(),
        found,
        raw: tx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::wallet::execution::{
        insert_quote_for_test, now_ms, reset_quote_store_for_tests, PreparedKind, PreparedStatus,
        PreparedTransaction,
    };
    use crate::openhuman::wallet::test_support::{sample_tron_address, setup_wallet_in, TEST_LOCK};
    use axum::{routing::post, Router};
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::net::TcpListener;

    #[derive(Clone, Default)]
    struct TronMockRecord {
        create_calls: Arc<parking_lot::Mutex<Vec<Value>>>,
        trigger_calls: Arc<parking_lot::Mutex<Vec<Value>>>,
        broadcast_calls: Arc<parking_lot::Mutex<Vec<Value>>>,
    }

    async fn start_tron_mock(record: TronMockRecord) -> std::net::SocketAddr {
        let create = record.create_calls.clone();
        let trigger = record.trigger_calls.clone();
        let broadcast = record.broadcast_calls.clone();
        // Fixed raw_data_hex shape — minimal but valid hex so sha256 + sign work.
        let canned_tx = json!({
            "txID": "ab".repeat(32),
            "raw_data": {"contract": []},
            "raw_data_hex": "0a02ab1d2208deadbeef00deadbe40c89efd8a82325802",
        });
        let canned_tx_create = canned_tx.clone();
        let canned_tx_trigger = canned_tx.clone();
        let app = Router::new()
            .route(
                "/wallet/createtransaction",
                post(move |axum::Json(payload): axum::Json<Value>| {
                    let create = create.clone();
                    let canned = canned_tx_create.clone();
                    async move {
                        create.lock().push(payload);
                        axum::Json(canned)
                    }
                }),
            )
            .route(
                "/wallet/triggersmartcontract",
                post(move |axum::Json(payload): axum::Json<Value>| {
                    let trigger = trigger.clone();
                    let canned = canned_tx_trigger.clone();
                    async move {
                        trigger.lock().push(payload);
                        axum::Json(json!({ "transaction": canned }))
                    }
                }),
            )
            .route(
                "/wallet/broadcasttransaction",
                post(move |axum::Json(payload): axum::Json<Value>| {
                    let broadcast = broadcast.clone();
                    async move {
                        broadcast.lock().push(payload);
                        axum::Json(json!({
                            "result": true,
                            "txid": "ab".repeat(32),
                        }))
                    }
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        addr
    }

    #[tokio::test]
    async fn execute_tron_quote_signs_and_broadcasts_native_transfer() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();

        let record = TronMockRecord::default();
        let addr = start_tron_mock(record.clone()).await;
        std::env::set_var("OPENHUMAN_WALLET_RPC_TRON", format!("http://{addr}"));

        let now = now_ms();
        let quote = PreparedTransaction {
            quote_id: "q_tron_native_1".to_string(),
            kind: PreparedKind::NativeTransfer,
            chain: WalletChain::Tron,
            evm_network: None,
            from_address: sample_tron_address().to_string(),
            to_address: "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t".to_string(),
            asset_symbol: "TRX".to_string(),
            amount_raw: "1000000".to_string(),
            amount_formatted: "1.000000".to_string(),
            receive_symbol: None,
            min_receive_raw: None,
            calldata: None,
            token_address: None,
            estimated_fee_raw: "1000000".to_string(),
            status: PreparedStatus::AwaitingConfirmation,
            created_at_ms: now,
            expires_at_ms: now + 60_000,
            notes: vec![],
            owner: None,
        };
        insert_quote_for_test(quote.clone());

        let result = execute_tron_quote(quote).await.expect("tron broadcast ok");
        assert_eq!(result.status, PreparedStatus::Broadcasted);
        assert_eq!(result.transaction_hash, "ab".repeat(32));
        assert_eq!(record.create_calls.lock().len(), 1);
        assert_eq!(record.trigger_calls.lock().len(), 0);
        assert_eq!(record.broadcast_calls.lock().len(), 1);
        // Signed broadcast carries a 65-byte signature (hex = 130 chars).
        let payload = record.broadcast_calls.lock()[0].clone();
        let sig = payload
            .get("signature")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(sig.len(), 130, "expected 65-byte signature, got: {sig}");
    }

    #[tokio::test]
    async fn execute_tron_quote_signs_and_broadcasts_trc20_transfer() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();

        let record = TronMockRecord::default();
        let addr = start_tron_mock(record.clone()).await;
        std::env::set_var("OPENHUMAN_WALLET_RPC_TRON", format!("http://{addr}"));

        let now = now_ms();
        let quote = PreparedTransaction {
            quote_id: "q_tron_trc20_1".to_string(),
            kind: PreparedKind::TokenTransfer,
            chain: WalletChain::Tron,
            evm_network: None,
            from_address: sample_tron_address().to_string(),
            to_address: "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t".to_string(),
            asset_symbol: "USDT".to_string(),
            amount_raw: "5000000".to_string(),
            amount_formatted: "5.000000".to_string(),
            receive_symbol: None,
            min_receive_raw: None,
            calldata: None,
            token_address: Some("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t".to_string()),
            estimated_fee_raw: "15000000".to_string(),
            status: PreparedStatus::AwaitingConfirmation,
            created_at_ms: now,
            expires_at_ms: now + 60_000,
            notes: vec![],
            owner: None,
        };
        insert_quote_for_test(quote.clone());

        let result = execute_tron_quote(quote).await.expect("trc20 broadcast ok");
        assert_eq!(result.status, PreparedStatus::Broadcasted);
        assert_eq!(record.create_calls.lock().len(), 0);
        assert_eq!(record.trigger_calls.lock().len(), 1);
        assert_eq!(record.broadcast_calls.lock().len(), 1);
        // The triggersmartcontract payload must carry the ABI parameter and
        // selector for transfer(address,uint256).
        let trigger = record.trigger_calls.lock()[0].clone();
        assert_eq!(
            trigger.get("function_selector").and_then(|v| v.as_str()),
            Some("transfer(address,uint256)")
        );
        let param = trigger.get("parameter").and_then(|v| v.as_str()).unwrap();
        assert_eq!(param.len(), 128, "64-byte ABI args, hex-encoded");
    }

    #[tokio::test]
    async fn execute_tron_quote_surfaces_node_rejection() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();

        // Custom mock returning result=false on broadcast.
        let app = Router::new()
            .route(
                "/wallet/createtransaction",
                post(|| async {
                    axum::Json(json!({
                        "txID": "cd".repeat(32),
                        "raw_data": {"contract": []},
                        "raw_data_hex": "0a02ab1d2208deadbeef00deadbe40c89efd8a82325802",
                    }))
                }),
            )
            .route(
                "/wallet/broadcasttransaction",
                post(|| async {
                    axum::Json(json!({
                        "result": false,
                        "code": "BANDWIDTH_ERROR",
                        "message": "not enough bandwidth",
                    }))
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        std::env::set_var("OPENHUMAN_WALLET_RPC_TRON", format!("http://{addr}"));

        let now = now_ms();
        let quote = PreparedTransaction {
            quote_id: "q_tron_reject_1".to_string(),
            kind: PreparedKind::NativeTransfer,
            chain: WalletChain::Tron,
            evm_network: None,
            from_address: sample_tron_address().to_string(),
            to_address: "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t".to_string(),
            asset_symbol: "TRX".to_string(),
            amount_raw: "1000000".to_string(),
            amount_formatted: "1.000000".to_string(),
            receive_symbol: None,
            min_receive_raw: None,
            calldata: None,
            token_address: None,
            estimated_fee_raw: "1000000".to_string(),
            status: PreparedStatus::AwaitingConfirmation,
            created_at_ms: now,
            expires_at_ms: now + 60_000,
            notes: vec![],
            owner: None,
        };
        let err = execute_tron_quote(quote).await.unwrap_err();
        assert!(err.contains("BANDWIDTH_ERROR"), "got: {err}");
    }

    #[test]
    fn validate_tron_address_accepts_known_address() {
        // USDT TRC20 contract address — real mainnet, valid base58check.
        let addr = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
        assert_eq!(validate_tron_address(addr).unwrap(), addr);
    }

    #[test]
    fn validate_tron_address_rejects_btc_format() {
        let err = validate_tron_address("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").unwrap_err();
        assert!(err.contains("invalid"), "got: {err}");
    }

    #[test]
    fn tron_address_to_hex_roundtrips_prefix_byte() {
        let addr = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
        let h = tron_address_to_hex(addr).unwrap();
        assert!(h.starts_with("41"), "expected 0x41 prefix, got: {h}");
        assert_eq!(h.len(), 42); // 21 bytes * 2 hex chars
    }

    #[test]
    fn derive_tron_address_for_known_test_mnemonic() {
        // BIP44 m/44'/195'/0'/0/0 from the standard "abandon × 11 about" mnemonic.
        // Deterministic output of our SLIP-44 / secp256k1 / keccak256 / base58check
        // pipeline — pinning here so regressions in any of those primitives are caught.
        let mnemonic =
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let (_sk, addr) = derive_tron_keypair(mnemonic, "m/44'/195'/0'/0/0").unwrap();
        assert_eq!(addr, "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH");
        // Address must be a valid base58check 0x41 mainnet address.
        validate_tron_address(&addr).expect("derived addr passes validation");
    }

    #[test]
    fn encode_trc20_transfer_param_pads_addr_and_amount() {
        let to_hex = tron_address_to_hex("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t").unwrap();
        let param = encode_trc20_transfer_param(&to_hex, 12345).unwrap();
        // 64 bytes hex = 32 bytes addr param + 32 bytes amount param = 128 hex chars.
        assert_eq!(param.len(), 128);
        // First 12 bytes = 24 hex chars zero-padded.
        assert!(
            param.starts_with("000000000000000000000000"),
            "expected 12-byte zero padding, got: {param}"
        );
        // Amount 12345 = 0x3039 → last 8 hex chars should be "00003039".
        assert!(param.ends_with("00003039"), "got: {param}");
    }

    #[test]
    fn pad_left_32_zero_pads_short_input() {
        let p = pad_left_32(&[1, 2, 3]);
        assert_eq!(p.len(), 32);
        assert_eq!(&p[..29], &[0u8; 29]);
        assert_eq!(&p[29..], &[1, 2, 3]);
    }

    #[tokio::test]
    async fn tx_status_confirmed_from_info() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let app = Router::new().route(
            "/wallet/gettransactioninfobyid",
            post(|| async {
                axum::Json(json!({
                    "id": "ab".repeat(32),
                    "blockNumber": 555u64,
                    "receipt": {"result": "SUCCESS"},
                    "fee": 1100u64
                }))
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        std::env::set_var("OPENHUMAN_WALLET_RPC_TRON", format!("http://{addr}"));
        let info = tx_status("ab").await.unwrap();
        assert_eq!(info.state, TxState::Confirmed);
        assert_eq!(info.block_number, Some(555));
        let receipt = tx_receipt("ab").await.unwrap();
        assert!(receipt.found);
        assert_eq!(receipt.success, Some(true));
        assert_eq!(receipt.fee_raw.as_deref(), Some("1100"));
    }

    #[tokio::test]
    async fn tx_status_not_found_on_empty_info() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let app = Router::new()
            .route(
                "/wallet/gettransactioninfobyid",
                post(|| async { axum::Json(json!({})) }),
            )
            .route(
                "/wallet/gettransactionbyid",
                post(|| async { axum::Json(json!({})) }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        std::env::set_var("OPENHUMAN_WALLET_RPC_TRON", format!("http://{addr}"));
        let info = tx_status("missing").await.unwrap();
        assert_eq!(info.state, TxState::NotFound);
    }
}
