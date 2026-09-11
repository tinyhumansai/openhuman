//! Bitcoin P2WPKH signing + broadcast. Uses the `bitcoin` crate plus the
//! Esplora REST API (https://blockstream.info/api) for UTXO discovery and
//! transaction broadcast. Tests can point at any URL via
//! `OPENHUMAN_WALLET_RPC_BTC`.
//!
//! Address derivation uses BIP84 (`m/84'/0'/0'/0/0` mainnet) so any wallet
//! seeded with a standard recovery phrase + this path produces a `bc1q…`
//! native segwit address.

use log::debug;
use serde::Deserialize;

use crate::openhuman::config::rpc as config_rpc;

use super::super::defaults::{explorer_tx_url, rpc_url_for_chain};
use super::super::execution::{
    ExecutionResult, PreparedKind, PreparedStatus, PreparedTransaction, TxLookupInfo,
    TxReceiptInfo, TxState, TxStatusInfo,
};
use super::super::ops::{secret_material, WalletChain};
use super::super::rpc::{rest_get_json, rest_get_text, rest_post_text};

const LOG_PREFIX: &str = "[wallet::btc]";
/// Hardcoded fee rate (sat/vbyte) used to estimate fees for prepared quotes
/// and to size the change output. Conservative — Bitcoin mempools cap out
/// around 50 sat/vB during congested periods; 20 keeps us in-range without
/// burning sats during quiet times.
const DEFAULT_FEE_RATE_SAT_VB: u64 = 20;
/// Approx vbytes of a 1-input, 2-output P2WPKH tx (in/out + witness).
const TYPICAL_TX_VBYTES: u64 = 141;

#[derive(Debug, Deserialize, Clone)]
pub struct EsploraUtxo {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
}

#[derive(Debug, Deserialize)]
struct EsploraAddressInfo {
    chain_stats: EsploraAddressStats,
    mempool_stats: EsploraAddressStats,
}

#[derive(Debug, Deserialize)]
struct EsploraAddressStats {
    funded_txo_sum: u64,
    spent_txo_sum: u64,
}

pub fn estimated_btc_fee_sats() -> u64 {
    DEFAULT_FEE_RATE_SAT_VB * TYPICAL_TX_VBYTES
}

/// Generic BTC address validation — any well-formed mainnet address is OK.
/// Used for recipients (we don't care what address type they prefer; the
/// `bitcoin` crate's script_pubkey() will encode P2WPKH/P2TR/P2SH correctly).
///
/// Delegates to the vendored [`tinywallet_bus`] crate, which owns the address
/// format itself. Nothing about parsing a Bitcoin address is OpenHuman-
/// specific, so the rules live where any host can reach them; what stays here
/// is the `Result<_, String>` shape the rest of this domain speaks.
pub fn validate_btc_address(addr: &str) -> Result<String, String> {
    let result = tinywallet_bus::address::btc::validate(addr).map_err(|e| e.to_string());
    debug!(
        "{LOG_PREFIX} validate_address role=recipient result={}",
        if result.is_ok() {
            "accepted"
        } else {
            "rejected"
        }
    );
    result
}

/// Sender-side validation — must be P2WPKH because we only know how to
/// derive + sign for native segwit (`bc1q…`). Recipients can be any type.
///
/// See [`validate_btc_address`] for why this delegates. `tinywallet-bus` keeps the
/// two rules as separate functions for the same reason this module does: using
/// the recipient rule for a sender accepts an address that only fails later,
/// at signing time.
pub fn validate_btc_sender_address(addr: &str) -> Result<String, String> {
    let result = tinywallet_bus::address::btc::validate_sender(addr).map_err(|e| e.to_string());
    debug!(
        "{LOG_PREFIX} validate_address role=sender result={}",
        if result.is_ok() {
            "accepted"
        } else {
            "rejected"
        }
    );
    result
}

pub async fn native_balance(address: &str) -> Result<u128, String> {
    validate_btc_address(address)?;
    let base = rpc_url_for_chain(WalletChain::Btc);
    let url = format!("{}/address/{}", base.trim_end_matches('/'), address);
    let info: EsploraAddressInfo = rest_get_json(&url).await?;
    let confirmed = info
        .chain_stats
        .funded_txo_sum
        .saturating_sub(info.chain_stats.spent_txo_sum);
    let pending = info
        .mempool_stats
        .funded_txo_sum
        .saturating_sub(info.mempool_stats.spent_txo_sum);
    Ok((confirmed + pending) as u128)
}

pub async fn fetch_utxos(address: &str) -> Result<Vec<EsploraUtxo>, String> {
    let base = rpc_url_for_chain(WalletChain::Btc);
    let url = format!("{}/address/{}/utxo", base.trim_end_matches('/'), address);
    rest_get_json(&url).await
}

pub async fn broadcast_raw_hex(tx_hex: &str) -> Result<String, String> {
    let base = rpc_url_for_chain(WalletChain::Btc);
    let url = format!("{}/tx", base.trim_end_matches('/'));
    rest_post_text(&url, tx_hex, "text/plain").await
}

/// Derive the P2WPKH signing key for `derivation_path` from a BIP-39 mnemonic.
///
/// Test-only, and deliberately on the **root** `tinywallet` crate rather than
/// `tinywallet-bus`: `key` is one of the gates that did not move into the
/// contract crate, because deriving is the module's job. The root crate is a
/// dev-dependency here, so this derivation stack is not linked into the shipped
/// binary. Production derives inside the wallet module, via
/// `modules::wallet::derive_account`.
///
/// Custody stays here: the mnemonic is decrypted from the keyring by this crate
/// and handed over as a `&str` that is not retained.
#[cfg(test)]
fn derive_btc_private_key(
    mnemonic: &str,
    derivation_path: &str,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let derived = tinywallet::key::derive(tinywallet::Chain::Btc, mnemonic, derivation_path)
        .map_err(|e| e.to_string())?;
    let secret = derived.secret_bytes().to_vec();
    // Compressed, because a P2WPKH witness program is defined over the
    // compressed encoding — the uncompressed form yields a valid-looking
    // address for an account holding no funds.
    let public_key = super::super::execution::compressed_public_key(&secret)
        .map_err(|_| "tinywallet returned an unusable BTC key".to_string())?;
    Ok((secret, public_key))
}

/// Select UTXOs to cover `amount_sats + fee_sats`, returning the selected
/// set and the change. Simple greedy: largest-first.
fn select_utxos(
    utxos: &[EsploraUtxo],
    amount_sats: u64,
    fee_sats: u64,
) -> Result<(Vec<EsploraUtxo>, u64), String> {
    let mut sorted = utxos.to_vec();
    sorted.sort_by_key(|item| std::cmp::Reverse(item.value));
    let target = amount_sats
        .checked_add(fee_sats)
        .ok_or_else(|| "amount + fee overflow".to_string())?;
    let mut total: u64 = 0;
    let mut chosen = Vec::new();
    for utxo in sorted {
        total = total
            .checked_add(utxo.value)
            .ok_or_else(|| "utxo sum overflow".to_string())?;
        chosen.push(utxo);
        if total >= target {
            return Ok((chosen, total - target));
        }
    }
    Err(format!(
        "insufficient BTC: have {total} sats, need {target} (amount {amount_sats} + fee {fee_sats})"
    ))
}

pub async fn execute_btc_quote(mut quote: PreparedTransaction) -> Result<ExecutionResult, String> {
    if !matches!(quote.kind, PreparedKind::NativeTransfer) {
        return Err(format!(
            "BTC only supports native transfers; got kind {:?}",
            quote.kind
        ));
    }
    let amount_sats: u64 = quote
        .amount_raw
        .parse()
        .map_err(|e| format!("invalid BTC amount '{}': {e}", quote.amount_raw))?;
    let from_addr = quote.from_address.clone();
    let to_addr = quote.to_address.clone();
    validate_btc_sender_address(&from_addr)?;
    validate_btc_address(&to_addr)?;

    let utxos = fetch_utxos(&from_addr).await?;
    if utxos.is_empty() {
        return Err(format!("no spendable UTXOs for {from_addr}"));
    }
    let fee_sats = estimated_btc_fee_sats();
    let (selected, change_sats) = select_utxos(&utxos, amount_sats, fee_sats)?;

    let secret = secret_material(WalletChain::Btc).await?;
    let config = config_rpc::load_config_with_timeout().await?;
    let mnemonic = crate::openhuman::security::encryption::rpc::decrypt_secret(
        &config,
        &secret.encrypted_mnemonic,
    )
    .await?
    .value;
    // Derivation and signing both happen in the loaded wallet module now, so no
    // private key is reassembled here. The phrase travels over a confidential
    // call to a module that has proved it is an artifact this build pinned —
    // see `modules::wallet::attested_proxy`.
    let signing_secret = tinywallet_bus::wire::SecretMaterial {
        mnemonic,
        derivation_path: secret.derivation_path.clone(),
        chain: tinywallet_bus::Chain::Btc,
    };

    // Selection stays here — this crate knows the fee policy and the UTXO
    // source — but the transaction itself is encoded by the loaded wallet
    // module, which also re-runs the same largest-first selection over the
    // UTXOs it is handed. Passing only the already-selected set keeps the two
    // in agreement: the module's `select_coins` and `select_utxos` above are
    // the same algorithm, down to the 546-sat dust rule, so it reselects
    // exactly what was chosen here.
    let transaction = tinywallet_bus::wire::TransactionSpec::Btc {
        from: from_addr.clone(),
        to: to_addr.clone(),
        amount_sat: amount_sats,
        fee_sat: fee_sats,
        utxos: selected
            .iter()
            .map(|utxo| tinywallet_bus::wire::Utxo {
                txid: utxo.txid.clone(),
                vout: utxo.vout,
                value: utxo.value,
            })
            .collect(),
    };
    // One signature per selected input, all produced inside the module and
    // applied there in input order — see `modules::wallet`.
    let signed = crate::openhuman::modules::wallet::sign_transaction_in_module(
        &config,
        &transaction,
        &signing_secret,
    )
    .await
    .map_err(|e| format!("failed to sign BTC transaction: {e}"))?;

    let tx_hex = signed.raw;
    let txid_hex = broadcast_raw_hex(&tx_hex).await?;
    quote.estimated_fee_raw = fee_sats.to_string();
    quote.status = PreparedStatus::Broadcasted;
    debug!(
        "{LOG_PREFIX} broadcast quote_id={} txid={} amount_sats={} change_sats={}",
        quote.quote_id, txid_hex, amount_sats, change_sats
    );
    let explorer_url = explorer_tx_url(WalletChain::Btc, &txid_hex);
    Ok(ExecutionResult {
        quote_id: quote.quote_id.clone(),
        status: PreparedStatus::Broadcasted,
        chain: WalletChain::Btc,
        evm_network: None,
        transaction_hash: txid_hex,
        explorer_url,
        transaction: quote,
    })
}

/// Esplora `/tx/:txid/status` → normalized status. Confirmations are derived
/// from the chain tip (`/blocks/tip/height`) when the tx is confirmed.
pub async fn tx_status(hash: &str) -> Result<TxStatusInfo, String> {
    let base = rpc_url_for_chain(WalletChain::Btc);
    let base = base.trim_end_matches('/');
    let status: serde_json::Value = match rest_get_json(&format!("{base}/tx/{hash}/status")).await {
        Ok(v) => v,
        // Esplora returns 404 for unknown txids; surface as NotFound.
        Err(e) if e.contains("status=404") => {
            return Ok(TxStatusInfo {
                chain: WalletChain::Btc,
                evm_network: None,
                hash: hash.to_string(),
                state: TxState::NotFound,
                confirmations: None,
                block_number: None,
            });
        }
        Err(e) => return Err(e),
    };
    let confirmed = status
        .get("confirmed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !confirmed {
        return Ok(TxStatusInfo {
            chain: WalletChain::Btc,
            evm_network: None,
            hash: hash.to_string(),
            state: TxState::Pending,
            confirmations: Some(0),
            block_number: None,
        });
    }
    let block_number = status
        .get("block_height")
        .and_then(serde_json::Value::as_u64);
    let confirmations = match block_number {
        Some(bn) => {
            let tip = rest_get_text(&format!("{base}/blocks/tip/height"))
                .await
                .ok();
            tip.and_then(|t| t.trim().parse::<u64>().ok())
                .map(|tip| tip.saturating_sub(bn).saturating_add(1))
        }
        None => None,
    };
    Ok(TxStatusInfo {
        chain: WalletChain::Btc,
        evm_network: None,
        hash: hash.to_string(),
        state: TxState::Confirmed,
        confirmations,
        block_number,
    })
}

/// Esplora `/tx/:txid` → normalized receipt (fee + confirmed height).
pub async fn tx_receipt(hash: &str) -> Result<TxReceiptInfo, String> {
    let base = rpc_url_for_chain(WalletChain::Btc);
    let base = base.trim_end_matches('/');
    let tx: serde_json::Value = match rest_get_json(&format!("{base}/tx/{hash}")).await {
        Ok(v) => v,
        Err(e) if e.contains("status=404") => {
            return Ok(TxReceiptInfo {
                chain: WalletChain::Btc,
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
        Err(e) => return Err(e),
    };
    let confirmed = tx
        .get("status")
        .and_then(|s| s.get("confirmed"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let block_number = tx
        .get("status")
        .and_then(|s| s.get("block_height"))
        .and_then(serde_json::Value::as_u64);
    let fee_raw = tx
        .get("fee")
        .and_then(serde_json::Value::as_u64)
        .map(|f| f.to_string());
    // Leave `success` unset until the tx is confirmed — an unconfirmed mempool
    // tx is pending (see tx_status), not a failure.
    let success = if confirmed { Some(true) } else { None };
    Ok(TxReceiptInfo {
        chain: WalletChain::Btc,
        evm_network: None,
        hash: hash.to_string(),
        found: true,
        success,
        block_number,
        gas_used: None,
        fee_raw,
        raw: tx,
    })
}

/// Esplora `/tx/:txid` → raw transaction passthrough.
pub async fn lookup_tx(hash: &str) -> Result<TxLookupInfo, String> {
    let base = rpc_url_for_chain(WalletChain::Btc);
    let base = base.trim_end_matches('/');
    match rest_get_json::<serde_json::Value>(&format!("{base}/tx/{hash}")).await {
        Ok(tx) => Ok(TxLookupInfo {
            chain: WalletChain::Btc,
            evm_network: None,
            hash: hash.to_string(),
            found: true,
            raw: tx,
        }),
        Err(e) if e.contains("status=404") => Ok(TxLookupInfo {
            chain: WalletChain::Btc,
            evm_network: None,
            hash: hash.to_string(),
            found: false,
            raw: serde_json::Value::Null,
        }),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
#[path = "btc_tests.rs"]
mod tests;
