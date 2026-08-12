//! Tron native TRX + TRC20 token transfer. Uses the TronGrid REST API
//! (https://api.trongrid.io) — `wallet/createtransaction`,
//! `wallet/triggersmartcontract`, `wallet/broadcasttransaction`.
//!
//! Derivation: BIP44 m/44'/195'/0'/0/0 → secp256k1 key. Tron addresses are
//! `sha3_256(uncompressed_pubkey[1..])[12..]` prefixed with 0x41, base58check.

use log::debug;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::openhuman::config::rpc as config_rpc;

use super::super::defaults::{explorer_tx_url, rpc_url_for_chain};
use super::super::execution::{
    compressed_public_key, ExecutionResult, PreparedKind, PreparedStatus, PreparedTransaction,
    TxLookupInfo, TxReceiptInfo, TxState, TxStatusInfo,
};
use super::super::ops::{secret_material, WalletChain};
use super::super::rpc::rest_post_json;

const LOG_PREFIX: &str = "[wallet::tron]";
/// Tron address prefix (mainnet).
const TRON_PREFIX: u8 = 0x41;
/// Fixed TRC20 fee_limit (15 TRX = 15_000_000 SUN). Safe upper bound.
const TRC20_FEE_LIMIT_SUN: u64 = 15_000_000;

/// Validate a Tron mainnet base58check address.
///
/// Delegates to the vendored [`tinywallet`] crate, which owns the address
/// format; this wrapper keeps the `Result<_, String>` shape the rest of the
/// domain speaks.
pub fn validate_tron_address(addr: &str) -> Result<String, String> {
    let result = tinywallet::address::tron::validate(addr).map_err(|e| e.to_string());
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

/// Convert a base58check Tron address into the 42-hex-digit form the TronGrid
/// API expects, version prefix included.
///
/// Delegates to [`tinywallet`]. Note this now validates the address before
/// converting, where the previous local implementation decoded without a
/// length check — a malformed address that happened to base58check-decode to
/// the wrong length used to produce a short hex string and fail further
/// downstream at the API call.
pub fn tron_address_to_hex(addr: &str) -> Result<String, String> {
    let result = tinywallet::address::tron::to_hex(addr).map_err(|e| e.to_string());
    debug!(
        "{LOG_PREFIX} address_to_hex result={}",
        if result.is_ok() {
            "accepted"
        } else {
            "rejected"
        }
    );
    result
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

#[derive(Debug)]
enum TronTransferVerification {
    Native { amount_sun: u64 },
    Trc20 { parameter_hex: String },
}

#[derive(Debug)]
enum ProtoValue<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Other,
}

#[derive(Debug)]
struct ProtoField<'a> {
    number: u64,
    value: ProtoValue<'a>,
}

fn tron_transaction_spec(
    raw_tx: &CreateTransactionResponse,
    expected_to: String,
    transfer: &TronTransferVerification,
) -> Result<tinywallet::wire::TransactionSpec, String> {
    let recomputed_txid = recompute_tron_txid(&raw_tx.raw_data_hex)?;
    if !recomputed_txid.eq_ignore_ascii_case(raw_tx.tx_id.trim()) {
        return Err("Tron node txID does not match sha256(raw_data)".to_string());
    }

    let raw = hex::decode(raw_tx.raw_data_hex.trim())
        .map_err(|error| format!("invalid Tron raw_data_hex: {error}"))?;
    let expected_recipient = hex::decode(tron_address_to_hex(&expected_to)?)
        .map_err(|error| format!("invalid Tron recipient encoding: {error}"))?;
    let raw_fields = parse_proto_fields(&raw)?;
    let contract = parse_single_tron_contract(&raw_fields)?;
    match transfer {
        TronTransferVerification::Native { amount_sun } => {
            if contract.kind != 1 || !contract.type_url.ends_with(".TransferContract") {
                return Err("Tron node transaction is not a native transfer".to_string());
            }
            let payload = parse_proto_fields(contract.payload)?;
            let recipient = one_bytes(&payload, 2, "TransferContract.to_address")?;
            let amount = one_varint(&payload, 3, "TransferContract.amount")?;
            if recipient != expected_recipient {
                return Err(
                    "Tron node transaction does not pay the requested recipient".to_string()
                );
            }
            if amount != *amount_sun {
                return Err("Tron node transaction has a different native amount".to_string());
            }
        }
        TronTransferVerification::Trc20 { parameter_hex } => {
            if contract.kind != 31 || !contract.type_url.ends_with(".TriggerSmartContract") {
                return Err("Tron node transaction is not a smart-contract trigger".to_string());
            }
            let payload = parse_proto_fields(contract.payload)?;
            let recipient = one_bytes(&payload, 2, "TriggerSmartContract.contract_address")?;
            if recipient != expected_recipient {
                return Err("Tron node transaction targets a different contract".to_string());
            }
            let call_value =
                optional_varint(&payload, 3, "TriggerSmartContract.call_value")?.unwrap_or(0);
            if call_value != 0 {
                return Err("Tron node transaction has non-zero TRC20 call_value".to_string());
            }
            if let Some(fee_limit) = optional_varint(&raw_fields, 18, "Transaction.raw.fee_limit")?
            {
                if fee_limit != TRC20_FEE_LIMIT_SUN {
                    return Err("Tron node transaction has a different fee_limit".to_string());
                }
            }
            let parameter = hex::decode(parameter_hex)
                .map_err(|error| format!("invalid TRC20 parameter: {error}"))?;
            let mut expected_data = hex::decode("a9059cbb").expect("fixed selector is valid hex");
            expected_data.extend(parameter);
            let data = one_bytes(&payload, 4, "TriggerSmartContract.data")?;
            if data != expected_data {
                return Err("Tron node transaction has different TRC20 transfer data".to_string());
            }
        }
    }

    Ok(tinywallet::wire::TransactionSpec::Tron {
        raw_data_hex: raw_tx.raw_data_hex.clone(),
        expected_to,
        expected_txid: recomputed_txid,
    })
}

fn encode_protobuf_varint(mut value: u64) -> Vec<u8> {
    let mut encoded = Vec::new();
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        encoded.push(byte);
        if value == 0 {
            return encoded;
        }
    }
}

fn recompute_tron_txid(raw_data_hex: &str) -> Result<String, String> {
    let raw = hex::decode(raw_data_hex.trim())
        .map_err(|error| format!("invalid Tron raw_data_hex: {error}"))?;
    Ok(hex::encode(Sha256::digest(raw)))
}

struct ParsedTronContract<'a> {
    kind: u64,
    type_url: &'a str,
    payload: &'a [u8],
}

fn parse_single_tron_contract<'a>(
    raw_fields: &[ProtoField<'a>],
) -> Result<ParsedTronContract<'a>, String> {
    let contract_bytes = one_bytes(raw_fields, 11, "Transaction.raw.contract")?;
    let contract_fields = parse_proto_fields(contract_bytes)?;
    let kind = one_varint(&contract_fields, 1, "Transaction.Contract.type")?;
    let any_bytes = one_bytes(&contract_fields, 2, "Transaction.Contract.parameter")?;
    let any_fields = parse_proto_fields(any_bytes)?;
    let type_url = std::str::from_utf8(one_bytes(&any_fields, 1, "Any.type_url")?)
        .map_err(|_| "Tron contract type_url is not UTF-8".to_string())?;
    let payload = one_bytes(&any_fields, 2, "Any.value")?;
    Ok(ParsedTronContract {
        kind,
        type_url,
        payload,
    })
}

fn one_bytes<'a>(fields: &[ProtoField<'a>], number: u64, name: &str) -> Result<&'a [u8], String> {
    let mut matches = fields.iter().filter(|field| field.number == number);
    let Some(field) = matches.next() else {
        return Err(format!("Tron protobuf is missing {name}"));
    };
    if matches.next().is_some() {
        return Err(format!("Tron protobuf repeats singular field {name}"));
    }
    match field.value {
        ProtoValue::Bytes(value) => Ok(value),
        _ => Err(format!(
            "Tron protobuf field {name} has the wrong wire type"
        )),
    }
}

fn one_varint(fields: &[ProtoField<'_>], number: u64, name: &str) -> Result<u64, String> {
    optional_varint(fields, number, name)?.ok_or_else(|| format!("Tron protobuf is missing {name}"))
}

fn optional_varint(
    fields: &[ProtoField<'_>],
    number: u64,
    name: &str,
) -> Result<Option<u64>, String> {
    let mut matches = fields.iter().filter(|field| field.number == number);
    let Some(field) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Err(format!("Tron protobuf repeats singular field {name}"));
    }
    match field.value {
        ProtoValue::Varint(value) => Ok(Some(value)),
        _ => Err(format!(
            "Tron protobuf field {name} has the wrong wire type"
        )),
    }
}

fn parse_proto_fields(mut input: &[u8]) -> Result<Vec<ProtoField<'_>>, String> {
    let mut fields = Vec::new();
    while !input.is_empty() {
        let key = take_varint(&mut input)?;
        let number = key >> 3;
        if number == 0 {
            return Err("Tron protobuf contains field zero".to_string());
        }
        let value = match key & 0x07 {
            0 => ProtoValue::Varint(take_varint(&mut input)?),
            1 => {
                take_exact(&mut input, 8)?;
                ProtoValue::Other
            }
            2 => {
                let length = usize::try_from(take_varint(&mut input)?)
                    .map_err(|_| "Tron protobuf field length is too large".to_string())?;
                ProtoValue::Bytes(take_exact(&mut input, length)?)
            }
            5 => {
                take_exact(&mut input, 4)?;
                ProtoValue::Other
            }
            wire => return Err(format!("unsupported Tron protobuf wire type {wire}")),
        };
        fields.push(ProtoField { number, value });
    }
    Ok(fields)
}

fn take_varint(input: &mut &[u8]) -> Result<u64, String> {
    let mut value = 0u64;
    for shift in (0..=63).step_by(7) {
        let (&byte, rest) = input
            .split_first()
            .ok_or_else(|| "truncated Tron protobuf varint".to_string())?;
        *input = rest;
        let part = u64::from(byte & 0x7f);
        if shift == 63 && part > 1 {
            return Err("Tron protobuf varint overflows u64".to_string());
        }
        value |= part << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("Tron protobuf varint is too long".to_string())
}

fn take_exact<'a>(input: &mut &'a [u8], length: usize) -> Result<&'a [u8], String> {
    if input.len() < length {
        return Err("truncated Tron protobuf field".to_string());
    }
    let (value, rest) = input.split_at(length);
    *input = rest;
    Ok(value)
}

/// Derive the Tron signing key and its base58check address.
///
/// Delegates to the vendored [`tinywallet`] crate, which owns BIP-32
/// secp256k1 derivation and the Keccak-then-base58check address construction.
/// The hand-rolled BIP-32 walk and path parser that used to live here moved
/// there wholesale. Custody stays here.
fn derive_tron_keypair(mnemonic: &str, derivation_path: &str) -> Result<(Vec<u8>, String), String> {
    let derived = tinywallet::key::derive(tinywallet::Chain::Tron, mnemonic, derivation_path)
        .map_err(|e| e.to_string())?;
    Ok((
        derived.secret_bytes().to_vec(),
        derived.address().to_string(),
    ))
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
    let mnemonic = crate::openhuman::security::encryption::rpc::decrypt_secret(
        &config,
        &secret.encrypted_mnemonic,
    )
    .await?
    .value;
    let (sk, derived_addr) = derive_tron_keypair(&mnemonic, &secret.derivation_path)?;
    if derived_addr != quote.from_address {
        return Err(format!(
            "Tron key derivation mismatch: derived {derived_addr} but expected {}",
            quote.from_address
        ));
    }

    // Which address the *transaction* pays, which is not always the address the
    // user is paying. A native transfer pays the recipient; a TRC20 transfer
    // pays the token contract and carries the recipient inside the call
    // parameter, left-padded to 32 bytes and so without the `41` prefix that
    // appears in `raw_data` for a native transfer. Verifying a TRC20 against
    // the user's recipient would therefore never match.
    let (verified_recipient, transfer, raw_tx) = match quote.kind {
        PreparedKind::NativeTransfer => {
            let amount_sun: u64 = amount
                .try_into()
                .map_err(|_| format!("Tron amount {amount} exceeds u64"))?;
            (
                quote.to_address.clone(),
                TronTransferVerification::Native { amount_sun },
                create_native_transaction(&owner_hex, &to_hex, amount_sun).await?,
            )
        }
        PreparedKind::TokenTransfer => {
            let contract = quote
                .token_address
                .as_deref()
                .ok_or_else(|| "TRC20 transfer missing token_address".to_string())?;
            validate_tron_address(contract)?;
            let contract_hex = tron_address_to_hex(contract)?;
            let parameter = encode_trc20_transfer_param(&to_hex, amount)?;
            (
                contract.to_string(),
                TronTransferVerification::Trc20 {
                    parameter_hex: parameter.clone(),
                },
                trigger_trc20_transfer(&owner_hex, &contract_hex, &parameter).await?,
            )
        }
    };

    // The node builds the transaction, so verify every requested field here
    // before the module hands back a digest to sign. The module independently
    // rechecks the locally recomputed txid and recipient; the host additionally
    // binds the native amount or full TRC20 parameter.
    let public_key = compressed_public_key(&sk)?;
    let transfer_kind = match &transfer {
        TronTransferVerification::Native { .. } => "native",
        TronTransferVerification::Trc20 { .. } => "trc20",
    };
    let transaction = match tron_transaction_spec(&raw_tx, verified_recipient, &transfer) {
        Ok(transaction) => {
            debug!(
                "{LOG_PREFIX} validation=accepted quote_id={} txid={} kind={transfer_kind}",
                quote.quote_id, raw_tx.tx_id
            );
            transaction
        }
        Err(error) => {
            debug!(
                "{LOG_PREFIX} validation=rejected quote_id={} txid={} kind={transfer_kind} reason={error}",
                quote.quote_id, raw_tx.tx_id
            );
            return Err(error);
        }
    };
    let signed = crate::openhuman::modules::wallet::sign_transaction(
        &config,
        &transaction,
        &sk,
        &public_key,
    )
    .await
    .map_err(|e| format!("failed to sign Tron transaction: {e}"))?;
    let sig_hex = signed.raw;

    let mut tx_with_sig = serde_json::to_value(serde_json::json!({
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
    use crate::openhuman::web3::wallet::execution::{
        insert_quote_for_test, now_ms, reset_quote_store_for_tests, PreparedKind, PreparedStatus,
        PreparedTransaction,
    };
    use crate::openhuman::web3::wallet::test_support::{
        sample_tron_address, setup_wallet_in, TEST_LOCK,
    };
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

    fn push_varint_field(out: &mut Vec<u8>, number: u64, value: u64) {
        out.extend(encode_protobuf_varint(number << 3));
        out.extend(encode_protobuf_varint(value));
    }

    fn push_bytes_field(out: &mut Vec<u8>, number: u64, value: &[u8]) {
        out.extend(encode_protobuf_varint((number << 3) | 2));
        out.extend(encode_protobuf_varint(value.len() as u64));
        out.extend(value);
    }

    fn tron_raw_contract(kind: u64, type_name: &str, payload: &[u8]) -> String {
        let mut any = Vec::new();
        push_bytes_field(
            &mut any,
            1,
            format!("type.googleapis.com/protocol.{type_name}").as_bytes(),
        );
        push_bytes_field(&mut any, 2, payload);

        let mut contract = Vec::new();
        push_varint_field(&mut contract, 1, kind);
        push_bytes_field(&mut contract, 2, &any);

        let mut raw = Vec::new();
        push_bytes_field(&mut raw, 11, &contract);
        hex::encode(raw)
    }

    fn native_raw(recipient_hex: &str, amount: u64) -> String {
        let mut payload = Vec::new();
        push_bytes_field(&mut payload, 2, &hex::decode(recipient_hex).unwrap());
        push_varint_field(&mut payload, 3, amount);
        tron_raw_contract(1, "TransferContract", &payload)
    }

    fn trc20_raw_with_values(
        contract_hex: &str,
        parameter_hex: &str,
        call_value: Option<u64>,
        fee_limit: Option<u64>,
    ) -> String {
        let mut payload = Vec::new();
        push_bytes_field(&mut payload, 2, &hex::decode(contract_hex).unwrap());
        if let Some(call_value) = call_value {
            push_varint_field(&mut payload, 3, call_value);
        }
        let mut data = hex::decode("a9059cbb").unwrap();
        data.extend(hex::decode(parameter_hex).unwrap());
        push_bytes_field(&mut payload, 4, &data);
        let mut raw = hex::decode(tron_raw_contract(31, "TriggerSmartContract", &payload)).unwrap();
        if let Some(fee_limit) = fee_limit {
            push_varint_field(&mut raw, 18, fee_limit);
        }
        hex::encode(raw)
    }

    fn trc20_raw(contract_hex: &str, parameter_hex: &str) -> String {
        trc20_raw_with_values(
            contract_hex,
            parameter_hex,
            Some(0),
            Some(TRC20_FEE_LIMIT_SUN),
        )
    }

    async fn start_tron_mock(record: TronMockRecord) -> std::net::SocketAddr {
        let create = record.create_calls.clone();
        let trigger = record.trigger_calls.clone();
        let broadcast = record.broadcast_calls.clone();
        let app = Router::new()
            .route(
                "/wallet/createtransaction",
                post(move |axum::Json(payload): axum::Json<Value>| {
                    let create = create.clone();
                    async move {
                        let recipient = payload["to_address"].as_str().unwrap();
                        let amount = payload["amount"].as_u64().unwrap();
                        let raw = native_raw(recipient, amount);
                        let txid = recompute_tron_txid(&raw).unwrap();
                        create.lock().push(payload);
                        axum::Json(json!({
                            "txID": txid,
                            "raw_data": {"contract": []},
                            "raw_data_hex": raw,
                        }))
                    }
                }),
            )
            .route(
                "/wallet/triggersmartcontract",
                post(move |axum::Json(payload): axum::Json<Value>| {
                    let trigger = trigger.clone();
                    async move {
                        let contract = payload["contract_address"].as_str().unwrap();
                        let parameter = payload["parameter"].as_str().unwrap();
                        let raw = trc20_raw(contract, parameter);
                        let txid = recompute_tron_txid(&raw).unwrap();
                        trigger.lock().push(payload);
                        axum::Json(json!({
                            "transaction": {
                                "txID": txid,
                                "raw_data": {"contract": []},
                                "raw_data_hex": raw,
                            }
                        }))
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

    #[test]
    fn tron_specs_bind_native_and_trc20_verification_fields() {
        let recipient = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
        let contract = "TLyqzVGLV1srkB7dToTAEqgDSfPtXRJZYH";
        let recipient_hex = tron_address_to_hex(recipient).unwrap();
        let contract_hex = tron_address_to_hex(contract).unwrap();

        let native_raw_hex = native_raw(&recipient_hex, 1_000_000);
        let native_txid = recompute_tron_txid(&native_raw_hex).unwrap();
        let native_tx = CreateTransactionResponse {
            tx_id: native_txid.clone(),
            raw_data: json!({}),
            raw_data_hex: native_raw_hex.clone(),
        };
        let native = tron_transaction_spec(
            &native_tx,
            recipient.to_string(),
            &TronTransferVerification::Native {
                amount_sun: 1_000_000,
            },
        )
        .unwrap();
        assert_eq!(
            native,
            tinywallet::wire::TransactionSpec::Tron {
                raw_data_hex: native_raw_hex,
                expected_to: recipient.to_string(),
                expected_txid: native_txid,
            }
        );

        let parameter = "01".repeat(64);
        let token_raw = trc20_raw(&contract_hex, &parameter);
        let token_txid = recompute_tron_txid(&token_raw).unwrap();
        let token_tx = CreateTransactionResponse {
            tx_id: token_txid.clone(),
            raw_data: json!({}),
            raw_data_hex: token_raw.clone(),
        };
        let token = tron_transaction_spec(
            &token_tx,
            contract.to_string(),
            &TronTransferVerification::Trc20 {
                parameter_hex: parameter.clone(),
            },
        )
        .unwrap();
        assert_eq!(
            token,
            tinywallet::wire::TransactionSpec::Tron {
                raw_data_hex: token_raw,
                expected_to: contract.to_string(),
                expected_txid: token_txid,
            }
        );
        assert_ne!(contract, recipient);

        assert!(tron_transaction_spec(
            &native_tx,
            recipient.to_string(),
            &TronTransferVerification::Native { amount_sun: 2 },
        )
        .unwrap_err()
        .contains("different native amount"));
        assert!(tron_transaction_spec(
            &token_tx,
            contract.to_string(),
            &TronTransferVerification::Trc20 {
                parameter_hex: "02".repeat(64),
            },
        )
        .unwrap_err()
        .contains("different TRC20 transfer data"));

        for (raw_data_hex, expected_error) in [
            (
                trc20_raw_with_values(
                    &contract_hex,
                    &parameter,
                    Some(1),
                    Some(TRC20_FEE_LIMIT_SUN),
                ),
                "non-zero TRC20 call_value",
            ),
            (
                trc20_raw_with_values(
                    &contract_hex,
                    &parameter,
                    Some(0),
                    Some(TRC20_FEE_LIMIT_SUN + 1),
                ),
                "different fee_limit",
            ),
        ] {
            let altered_tx = CreateTransactionResponse {
                tx_id: recompute_tron_txid(&raw_data_hex).unwrap(),
                raw_data: json!({}),
                raw_data_hex,
            };
            assert!(tron_transaction_spec(
                &altered_tx,
                contract.to_string(),
                &TronTransferVerification::Trc20 {
                    parameter_hex: parameter.clone(),
                },
            )
            .unwrap_err()
            .contains(expected_error));
        }

        // A matching value hidden in an unrelated raw-data field must not
        // satisfy validation when the selected contract pays something else.
        let mut spoofed_raw = hex::decode(native_raw(&contract_hex, 2)).unwrap();
        let mut decoy = hex::decode(&recipient_hex).unwrap();
        decoy.extend(encode_protobuf_varint(1_000_000));
        push_bytes_field(&mut spoofed_raw, 10, &decoy);
        let spoofed_raw = hex::encode(spoofed_raw);
        let spoofed_tx = CreateTransactionResponse {
            tx_id: recompute_tron_txid(&spoofed_raw).unwrap(),
            raw_data: json!({}),
            raw_data_hex: spoofed_raw,
        };
        assert!(tron_transaction_spec(
            &spoofed_tx,
            recipient.to_string(),
            &TronTransferVerification::Native {
                amount_sun: 1_000_000,
            },
        )
        .unwrap_err()
        .contains("requested recipient"));
    }

    // Drives the real wallet module, so it must be the only such test in its
    // process: tinybus never unloads a module, and the module bus belongs to
    // whichever tokio runtime created it — a second `#[tokio::test]` finds a
    // broker whose tasks died with the first and the call fails with
    // "connection closed". Verified passing in isolation:
    //
    //   cargo test -p openhuman --lib --features "$(bash scripts/ci/product-features.sh)" \
    //     execute_tron_quote_signs_and_broadcasts_native_transfer -- --ignored --test-threads=1
    //
    // Same constraint tinydocs documents for its module-backed tool tests.
    #[ignore = "drives the loaded wallet module; must run alone in its process"]
    #[tokio::test]
    async fn execute_tron_quote_signs_and_broadcasts_native_transfer() {
        let _guard = TEST_LOCK.lock();
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        let _workspace_guard = setup_wallet_in(&temp).await.unwrap();

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

    // Drives the real wallet module, so it must be the only such test in its
    // process: tinybus never unloads a module, and the module bus belongs to
    // whichever tokio runtime created it — a second `#[tokio::test]` finds a
    // broker whose tasks died with the first and the call fails with
    // "connection closed". Verified passing in isolation:
    //
    //   cargo test -p openhuman --lib --features "$(bash scripts/ci/product-features.sh)" \
    //     execute_tron_quote_signs_and_broadcasts_trc20_transfer -- --ignored --test-threads=1
    //
    // Same constraint tinydocs documents for its module-backed tool tests.
    #[ignore = "drives the loaded wallet module; must run alone in its process"]
    #[tokio::test]
    async fn execute_tron_quote_signs_and_broadcasts_trc20_transfer() {
        let _guard = TEST_LOCK.lock();
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        let _workspace_guard = setup_wallet_in(&temp).await.unwrap();

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

    // Drives the real wallet module, so it must be the only such test in its
    // process: tinybus never unloads a module, and the module bus belongs to
    // whichever tokio runtime created it — a second `#[tokio::test]` finds a
    // broker whose tasks died with the first and the call fails with
    // "connection closed". Verified passing in isolation:
    //
    //   cargo test -p openhuman --lib --features "$(bash scripts/ci/product-features.sh)" \
    //     execute_tron_quote_surfaces_node_rejection -- --ignored --test-threads=1
    //
    // Same constraint tinydocs documents for its module-backed tool tests.
    #[ignore = "drives the loaded wallet module; must run alone in its process"]
    #[tokio::test]
    async fn execute_tron_quote_surfaces_node_rejection() {
        let _guard = TEST_LOCK.lock();
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        let _workspace_guard = setup_wallet_in(&temp).await.unwrap();

        // Custom mock returning result=false on broadcast.
        let app = Router::new()
            .route(
                "/wallet/createtransaction",
                post(|axum::Json(payload): axum::Json<Value>| async move {
                    let recipient = payload["to_address"].as_str().unwrap();
                    let amount = payload["amount"].as_u64().unwrap();
                    let raw = native_raw(recipient, amount);
                    let txid = recompute_tron_txid(&raw).unwrap();
                    axum::Json(json!({
                        "txID": txid,
                        "raw_data": {"contract": []},
                        "raw_data_hex": raw,
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
    fn tron_address_to_hex_rejects_a_wrong_length_decoded_address() {
        // A valid Base58Check encoding with the Tron prefix but a 20-byte
        // decoded payload must not be accepted as a 21-byte Tron address.
        let short = bs58::encode([TRON_PREFIX; 20]).with_check().into_string();
        assert!(tron_address_to_hex(&short).is_err());
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
