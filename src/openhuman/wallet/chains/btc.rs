//! Bitcoin P2WPKH signing + broadcast. Uses the `bitcoin` crate plus the
//! Esplora REST API (https://blockstream.info/api) for UTXO discovery and
//! transaction broadcast. Tests can point at any URL via
//! `OPENHUMAN_WALLET_RPC_BTC`.
//!
//! Address derivation uses BIP84 (`m/84'/0'/0'/0/0` mainnet) so any wallet
//! seeded with a standard recovery phrase + this path produces a `bc1q…`
//! native segwit address.

use bitcoin::absolute::LockTime;
use bitcoin::bip32::{DerivationPath, Xpriv};
use bitcoin::hashes::Hash;
use bitcoin::key::{CompressedPublicKey, PrivateKey};
use bitcoin::secp256k1::{Message, Secp256k1};
use bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bitcoin::transaction::Version;
use bitcoin::{
    consensus::encode::serialize_hex, Address, Amount, Network, OutPoint, ScriptBuf, Sequence,
    Transaction, TxIn, TxOut, Witness,
};
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
pub fn validate_btc_address(addr: &str) -> Result<String, String> {
    let trimmed = addr.trim();
    if trimmed.is_empty() {
        return Err("BTC address is empty".to_string());
    }
    Address::from_str(trimmed)
        .map_err(|e| format!("invalid BTC address '{trimmed}': {e}"))?
        .require_network(Network::Bitcoin)
        .map_err(|e| format!("BTC address '{trimmed}' is not mainnet: {e}"))?;
    Ok(trimmed.to_string())
}

/// Sender-side validation — must be P2WPKH because we only know how to
/// derive + sign for native segwit (`bc1q…`). Recipients can be any type.
pub fn validate_btc_sender_address(addr: &str) -> Result<String, String> {
    let trimmed = validate_btc_address(addr)?;
    let parsed = Address::from_str(&trimmed)
        .map_err(|e| format!("invalid BTC sender '{trimmed}': {e}"))?
        .assume_checked();
    if !parsed.script_pubkey().is_p2wpkh() {
        return Err(format!(
            "BTC sender '{trimmed}' is not P2WPKH (only bc1q… native segwit is supported for signing)"
        ));
    }
    Ok(trimmed)
}

use std::str::FromStr;

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

/// Best-effort lookup of an address's spending scriptPubKey by parsing it.
fn script_pubkey_for_addr(addr: &str) -> Result<ScriptBuf, String> {
    let parsed = Address::from_str(addr)
        .map_err(|e| format!("invalid address '{addr}': {e}"))?
        .require_network(Network::Bitcoin)
        .map_err(|e| format!("address '{addr}' is not mainnet: {e}"))?;
    Ok(parsed.script_pubkey())
}

/// Derive a P2WPKH PrivateKey for `derivation_path` from a BIP39 mnemonic.
fn derive_btc_private_key(
    mnemonic: &str,
    derivation_path: &str,
) -> Result<(PrivateKey, CompressedPublicKey), String> {
    use coins_bip39::{English, Mnemonic};
    let mnemonic: Mnemonic<English> = mnemonic
        .trim()
        .parse()
        .map_err(|e| format!("invalid BIP39 mnemonic: {e}"))?;
    let seed = mnemonic
        .to_seed(None)
        .map_err(|e| format!("failed to derive BIP39 seed: {e}"))?;
    let xpriv = Xpriv::new_master(Network::Bitcoin, &seed)
        .map_err(|e| format!("failed to derive BTC master key: {e}"))?;
    let secp = Secp256k1::signing_only();
    let path = DerivationPath::from_str(derivation_path)
        .map_err(|e| format!("invalid BTC derivation path '{derivation_path}': {e}"))?;
    let derived = xpriv
        .derive_priv(&secp, &path)
        .map_err(|e| format!("failed to derive BTC child key: {e}"))?;
    let private_key = PrivateKey::new(derived.private_key, Network::Bitcoin);
    let public_key = CompressedPublicKey::from_private_key(&secp, &private_key)
        .map_err(|e| format!("failed to derive BTC public key: {e}"))?;
    Ok((private_key, public_key))
}

/// Select UTXOs to cover `amount_sats + fee_sats`, returning the selected
/// set and the change. Simple greedy: largest-first.
fn select_utxos(
    utxos: &[EsploraUtxo],
    amount_sats: u64,
    fee_sats: u64,
) -> Result<(Vec<EsploraUtxo>, u64), String> {
    let mut sorted = utxos.to_vec();
    sorted.sort_by(|a, b| b.value.cmp(&a.value));
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
    let mnemonic =
        crate::openhuman::encryption::rpc::decrypt_secret(&config, &secret.encrypted_mnemonic)
            .await?
            .value;
    let (private_key, public_key) = derive_btc_private_key(&mnemonic, &secret.derivation_path)?;

    let from_spk = script_pubkey_for_addr(&from_addr)?;
    let to_spk = script_pubkey_for_addr(&to_addr)?;

    // Build inputs.
    let mut tx_inputs = Vec::with_capacity(selected.len());
    for utxo in &selected {
        let txid = bitcoin::Txid::from_str(&utxo.txid)
            .map_err(|e| format!("invalid utxo txid '{}': {e}", utxo.txid))?;
        tx_inputs.push(TxIn {
            previous_output: OutPoint {
                txid,
                vout: utxo.vout,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::default(),
        });
    }
    // Outputs: recipient + (optional) change to self.
    let mut tx_outputs = vec![TxOut {
        value: Amount::from_sat(amount_sats),
        script_pubkey: to_spk,
    }];
    if change_sats > 546 {
        tx_outputs.push(TxOut {
            value: Amount::from_sat(change_sats),
            script_pubkey: from_spk.clone(),
        });
    }

    let mut tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: tx_inputs,
        output: tx_outputs,
    };

    // Sign each input (BIP143 segwit sighash).
    let secp = Secp256k1::signing_only();
    let mut sighash_cache = SighashCache::new(&mut tx);
    let mut witnesses = Vec::with_capacity(selected.len());
    for (idx, utxo) in selected.iter().enumerate() {
        let sighash = sighash_cache
            .p2wpkh_signature_hash(
                idx,
                &from_spk,
                Amount::from_sat(utxo.value),
                EcdsaSighashType::All,
            )
            .map_err(|e| format!("failed to compute BTC sighash: {e}"))?;
        let msg = Message::from_digest(sighash.to_byte_array());
        let sig = secp.sign_ecdsa(&msg, &private_key.inner);
        let mut witness = Witness::new();
        let mut sig_bytes = sig.serialize_der().to_vec();
        sig_bytes.push(EcdsaSighashType::All as u8);
        witness.push(sig_bytes);
        witness.push(public_key.to_bytes());
        witnesses.push(witness);
    }
    for (input, witness) in tx.input.iter_mut().zip(witnesses.into_iter()) {
        input.witness = witness;
    }

    let tx_hex = serialize_hex(&tx);
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
mod tests {
    use super::*;
    use crate::openhuman::wallet::execution::{
        insert_quote_for_test, now_ms, reset_quote_store_for_tests, PreparedKind, PreparedStatus,
        PreparedTransaction,
    };
    use crate::openhuman::wallet::test_support::{sample_btc_address, setup_wallet_in, TEST_LOCK};
    use axum::{
        routing::{get, post},
        Router,
    };
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::net::TcpListener;

    #[test]
    fn validate_btc_address_accepts_known_p2wpkh() {
        // bech32 P2WPKH from BIP173 examples.
        let addr = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
        assert_eq!(validate_btc_address(addr).unwrap(), addr);
    }

    #[test]
    fn validate_btc_address_rejects_testnet() {
        let err =
            validate_btc_address("tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3qccfmv3")
                .unwrap_err();
        assert!(
            err.contains("not mainnet") || err.contains("invalid"),
            "got: {err}"
        );
    }

    #[test]
    fn validate_btc_sender_address_rejects_p2tr() {
        // P2TR (bech32m, bc1p…) is a valid recipient but cannot be a sender —
        // we only know how to sign P2WPKH inputs in this iteration.
        let p2tr = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr";
        // Generic validation must accept it (recipients can be any type).
        assert_eq!(validate_btc_address(p2tr).unwrap(), p2tr);
        // Sender validation must reject it.
        let err = validate_btc_sender_address(p2tr).unwrap_err();
        assert!(err.contains("not P2WPKH"), "got: {err}");
    }

    #[test]
    fn select_utxos_largest_first_returns_change() {
        let utxos = vec![
            EsploraUtxo {
                txid: "a".into(),
                vout: 0,
                value: 5000,
            },
            EsploraUtxo {
                txid: "b".into(),
                vout: 0,
                value: 10_000,
            },
            EsploraUtxo {
                txid: "c".into(),
                vout: 0,
                value: 1_000,
            },
        ];
        let (chosen, change) = select_utxos(&utxos, 6_000, 2_000).unwrap();
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].txid, "b");
        assert_eq!(change, 2_000);
    }

    #[test]
    fn select_utxos_combines_multiple_when_needed() {
        let utxos = vec![
            EsploraUtxo {
                txid: "a".into(),
                vout: 0,
                value: 5000,
            },
            EsploraUtxo {
                txid: "b".into(),
                vout: 0,
                value: 5000,
            },
            EsploraUtxo {
                txid: "c".into(),
                vout: 0,
                value: 5000,
            },
        ];
        let (chosen, change) = select_utxos(&utxos, 11_000, 1_000).unwrap();
        assert_eq!(chosen.len(), 3);
        assert_eq!(change, 3_000);
    }

    #[test]
    fn select_utxos_errors_when_insufficient() {
        let utxos = vec![EsploraUtxo {
            txid: "a".into(),
            vout: 0,
            value: 1_000,
        }];
        let err = select_utxos(&utxos, 5_000, 1_000).unwrap_err();
        assert!(err.contains("insufficient"), "got: {err}");
    }

    #[tokio::test]
    async fn execute_btc_quote_builds_psbt_signs_and_broadcasts() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();

        // Mock state: collect raw tx hex posted to /tx.
        let raw_txs: Arc<parking_lot::Mutex<Vec<String>>> =
            Arc::new(parking_lot::Mutex::new(Vec::new()));
        let raw_txs_clone = raw_txs.clone();
        let from_addr = sample_btc_address().to_string();
        // Real-shaped UTXO; value high enough to cover amount + fee.
        let utxo_txid = "1111111111111111111111111111111111111111111111111111111111111111";
        let utxo_json = json!([
            { "txid": utxo_txid, "vout": 0, "value": 100_000u64 }
        ]);
        let utxo_clone = utxo_json.clone();
        let app = Router::new()
            .route(
                "/address/{addr}/utxo",
                get(move || {
                    let body = utxo_clone.clone();
                    async move { axum::Json(body) }
                }),
            )
            .route(
                "/tx",
                post(move |body: String| {
                    let raw_txs = raw_txs_clone.clone();
                    async move {
                        raw_txs.lock().push(body);
                        // Return a known fake txid the test can assert on.
                        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
                            .to_string()
                    }
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        std::env::set_var("OPENHUMAN_WALLET_RPC_BTC", format!("http://{addr}"));

        let now = now_ms();
        let quote = PreparedTransaction {
            quote_id: "q_btc_native_1".to_string(),
            kind: PreparedKind::NativeTransfer,
            chain: WalletChain::Btc,
            evm_network: None,
            from_address: from_addr.clone(),
            to_address: "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".to_string(),
            asset_symbol: "BTC".to_string(),
            amount_raw: "50000".to_string(),
            amount_formatted: "0.00050000".to_string(),
            receive_symbol: None,
            min_receive_raw: None,
            calldata: None,
            token_address: None,
            estimated_fee_raw: "5000".to_string(),
            status: PreparedStatus::AwaitingConfirmation,
            created_at_ms: now,
            expires_at_ms: now + 60_000,
            notes: vec![],
            owner: None,
        };
        insert_quote_for_test(quote.clone());

        let result = execute_btc_quote(quote).await.expect("btc broadcast ok");
        assert_eq!(result.status, PreparedStatus::Broadcasted);
        assert_eq!(
            result.transaction_hash,
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
        );
        let raw = raw_txs.lock().clone();
        assert_eq!(raw.len(), 1, "exactly one broadcast call");
        let tx_hex = &raw[0];
        assert!(!tx_hex.is_empty(), "tx hex must be non-empty");
        // Witness-segwit transactions include the BIP141 marker+flag (0x0001).
        assert!(
            tx_hex.contains("0001"),
            "expected segwit marker, got: {tx_hex}"
        );
    }

    #[tokio::test]
    async fn execute_btc_quote_rejects_insufficient_utxos() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();

        // Empty UTXO set — must error.
        let app = Router::new().route(
            "/address/{addr}/utxo",
            get(|| async { axum::Json(json!([])) }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        std::env::set_var("OPENHUMAN_WALLET_RPC_BTC", format!("http://{addr}"));

        let now = now_ms();
        let quote = PreparedTransaction {
            quote_id: "q_btc_native_empty".to_string(),
            kind: PreparedKind::NativeTransfer,
            chain: WalletChain::Btc,
            evm_network: None,
            from_address: sample_btc_address().to_string(),
            to_address: "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".to_string(),
            asset_symbol: "BTC".to_string(),
            amount_raw: "50000".to_string(),
            amount_formatted: "0.00050000".to_string(),
            receive_symbol: None,
            min_receive_raw: None,
            calldata: None,
            token_address: None,
            estimated_fee_raw: "5000".to_string(),
            status: PreparedStatus::AwaitingConfirmation,
            created_at_ms: now,
            expires_at_ms: now + 60_000,
            notes: vec![],
            owner: None,
        };
        let err = execute_btc_quote(quote).await.unwrap_err();
        assert!(err.contains("no spendable UTXOs"), "got: {err}");
    }

    #[test]
    fn derive_btc_key_produces_known_p2wpkh_from_test_mnemonic() {
        // BIP84 m/84'/0'/0'/0/0 from "abandon x11 about" → bc1qcr8...
        // The compressed pubkey should serialize to 33 bytes.
        let mnemonic =
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let (pk, pubkey) = derive_btc_private_key(mnemonic, "m/84'/0'/0'/0/0").unwrap();
        assert_eq!(pk.network, bitcoin::NetworkKind::Main);
        assert_eq!(pubkey.to_bytes().len(), 33);
        let secp = Secp256k1::signing_only();
        let addr = Address::p2wpkh(&pubkey, Network::Bitcoin);
        // Known good vector for this mnemonic + path:
        assert_eq!(
            addr.to_string(),
            "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
        );
        let _ = secp; // suppress unused
    }

    #[tokio::test]
    async fn tx_status_confirmed_with_tip_confirmations() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let app = Router::new()
            .route(
                "/tx/{txid}/status",
                get(|| async {
                    axum::Json(json!({"confirmed": true, "block_height": 800_000u64}))
                }),
            )
            .route("/blocks/tip/height", get(|| async { "800002" }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        std::env::set_var("OPENHUMAN_WALLET_RPC_BTC", format!("http://{addr}"));
        let info = tx_status("deadbeef").await.unwrap();
        assert_eq!(
            info.state,
            crate::openhuman::wallet::execution::TxState::Confirmed
        );
        assert_eq!(info.block_number, Some(800_000));
        assert_eq!(info.confirmations, Some(3));
    }

    #[tokio::test]
    async fn lookup_tx_not_found_on_404() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let app = Router::new().route(
            "/tx/{txid}",
            get(|| async { (axum::http::StatusCode::NOT_FOUND, "Transaction not found") }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        std::env::set_var("OPENHUMAN_WALLET_RPC_BTC", format!("http://{addr}"));
        let info = lookup_tx("deadbeef").await.unwrap();
        assert!(!info.found);
    }
}
