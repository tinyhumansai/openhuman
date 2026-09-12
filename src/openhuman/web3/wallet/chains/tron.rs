//! Tron native TRX + TRC20 token transfer. Uses the TronGrid REST API
//! (https://api.trongrid.io) — `wallet/createtransaction`,
//! `wallet/triggersmartcontract`, `wallet/broadcasttransaction`.
//!
//! Derivation: BIP44 m/44'/195'/0'/0/0 → secp256k1 key. Tron addresses are
//! `sha3_256(uncompressed_pubkey[1..])[12..]` prefixed with 0x41, base58check.

use log::debug;
use serde::Deserialize;
use serde_json::{json, Value};

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

/// Validate a Tron mainnet base58check address.
///
/// Delegates to the vendored [`tinywallet_bus`] crate, which owns the address
/// format; this wrapper keeps the `Result<_, String>` shape the rest of the
/// domain speaks.
pub fn validate_tron_address(addr: &str) -> Result<String, String> {
    let result = tinywallet_bus::address::tron::validate(addr).map_err(|e| e.to_string());
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
/// Delegates to [`tinywallet_bus`]. Note this now validates the address before
/// converting, where the previous local implementation decoded without a
/// length check — a malformed address that happened to base58check-decode to
/// the wrong length used to produce a short hex string and fail further
/// downstream at the API call.
pub fn tron_address_to_hex(addr: &str) -> Result<String, String> {
    let result = tinywallet_bus::address::tron::to_hex(addr).map_err(|e| e.to_string());
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

/// What the node was asked to build, for verifying what it returned.
///
/// [`tinywallet_bus::wire::TronTransfer`] is that type — it is already on the
/// host/module wire contract, so a second local mirror of it would be one more
/// thing to keep in step for no gain. The one thing it deliberately does not
/// carry is the fee limit, because only the caller knows what it pinned; that
/// rides alongside as [`verify_contract`]'s last argument.
type TronTransferVerification = tinywallet_bus::wire::TronTransfer;

/// Check a node-built Tron transaction, then describe it for the signer.
///
/// The verification itself lives in [`tinywallet_bus::tx::tron::verify_contract`],
/// which parses `raw_data` structurally. The protobuf reader, the contract
/// unwrapping and the per-contract field checks used to be hand-rolled here;
/// they are the same rules for every host, so they moved into the crate. What
/// stays is the part that is OpenHuman's: the fee limit this client pins, and
/// the [`tinywallet_bus::wire::TransactionSpec`] handed to the wallet module.
fn tron_transaction_spec(
    raw_tx: &CreateTransactionResponse,
    expected_to: String,
    transfer: &TronTransferVerification,
) -> Result<tinywallet_bus::wire::TransactionSpec, String> {
    let recomputed_txid = tinywallet_bus::tx::tron::recompute_txid(&raw_tx.raw_data_hex)
        .map_err(|error| format!("invalid Tron raw_data_hex: {error}"))?;

    // The fee limit is ours, not the crate's: it is what this client pinned in
    // the `createtransaction` request, and only a TRC-20 trigger carries one.
    let fee_limit_sun = match transfer {
        TronTransferVerification::Native { .. } => None,
        TronTransferVerification::Trc20 { .. } => Some(TRC20_FEE_LIMIT_SUN),
    };

    tinywallet_bus::tx::tron::verify_contract(
        &raw_tx.raw_data_hex,
        &expected_to,
        &raw_tx.tx_id,
        transfer,
        fee_limit_sun,
    )
    .map_err(|error| format!("Tron node response rejected: {error}"))?;

    Ok(tinywallet_bus::wire::TransactionSpec::Tron {
        raw_data_hex: raw_tx.raw_data_hex.clone(),
        expected_to,
        expected_txid: recomputed_txid,
        // Carried onto the wire so the wallet module re-checks it against the
        // bytes it is about to sign, rather than trusting this side's verdict.
        transfer: transfer.clone(),
    })
}

/// Derive the Tron signing key and its base58check address.
///
/// Test-only, and deliberately on the **root** `tinywallet` crate rather than
/// `tinywallet-bus`: `key` is one of the gates that did not move into the
/// contract crate. The root crate is a dev-dependency here, so this derivation
/// stack is not linked into the shipped binary. Production derives inside the
/// wallet module, via `modules::wallet::derive_account`.
///
/// The root crate owns BIP-32 secp256k1 derivation and the
/// Keccak-then-base58check address construction; the hand-rolled BIP-32 walk
/// and path parser that used to live here moved there wholesale. Custody stays
/// here.
#[cfg(test)]
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
    // Derivation and signing both happen in the loaded wallet module now, so
    // this process never holds the key. The phrase goes over a confidential
    // call, and only to a module that has proved it is an artifact this build
    // pinned — see `modules::wallet::attested_proxy`.
    let signing_secret = tinywallet_bus::wire::SecretMaterial {
        mnemonic,
        derivation_path: secret.derivation_path.clone(),
        chain: tinywallet_bus::Chain::Tron,
    };
    let derived_addr = crate::openhuman::modules::wallet::derive_account(&config, &signing_secret)
        .await
        .map_err(|e| format!("failed to derive the Tron account: {e}"))?
        .address;
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
    let signed = crate::openhuman::modules::wallet::sign_transaction_in_module(
        &config,
        &transaction,
        &signing_secret,
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
#[path = "tron_tests.rs"]
mod tests;
