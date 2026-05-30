//! Solana native SOL + SPL token transfer. We hand-build the wire format so
//! we don't pull in `solana-sdk` (which transitively brings in ~150 crates).
//!
//! Key derivation: SLIP-0010 ed25519 (`m/44'/501'/0'/0'`). Solana mainnet
//! addresses are 32-byte ed25519 public keys, base58-encoded.
//!
//! Wire format references:
//! - https://docs.solana.com/developing/programming-model/transactions
//! - https://docs.solana.com/developing/programming-model/runtime#compact-u16
//! - https://spl.solana.com/token

use base64::engine::{general_purpose::STANDARD as B64, Engine as _};
use curve25519_dalek::edwards::CompressedEdwardsY;
use ed25519_dalek::{Signer, SigningKey, SECRET_KEY_LENGTH};
use hmac::{Hmac, Mac};
use log::debug;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256, Sha512};

use crate::openhuman::config::rpc as config_rpc;

use super::super::defaults::explorer_tx_url;
use super::super::execution::{
    ExecutionResult, PreparedKind, PreparedStatus, PreparedTransaction, RawBroadcastResult,
    TxLookupInfo, TxReceiptInfo, TxState, TxStatusInfo,
};
use super::super::ops::{secret_material, WalletChain};
use super::super::rpc::rpc_call;

const LOG_PREFIX: &str = "[wallet::sol]";

/// System Program ID (all zeros).
const SYSTEM_PROGRAM_ID: [u8; 32] = [0u8; 32];

fn token_program_id() -> [u8; 32] {
    let mut out = [0u8; 32];
    let v = bs58::decode("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
        .into_vec()
        .expect("static base58");
    out.copy_from_slice(&v);
    out
}

fn ata_program_id() -> [u8; 32] {
    let mut out = [0u8; 32];
    let v = bs58::decode("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL")
        .into_vec()
        .expect("static base58");
    out.copy_from_slice(&v);
    out
}

#[derive(Debug, Deserialize)]
struct BlockhashResponse {
    value: BlockhashValue,
}

#[derive(Debug, Deserialize)]
struct BlockhashValue {
    blockhash: String,
}

pub fn validate_solana_address(addr: &str) -> Result<String, String> {
    let trimmed = addr.trim();
    if trimmed.is_empty() {
        return Err("Solana address is empty".to_string());
    }
    let decoded = bs58::decode(trimmed)
        .into_vec()
        .map_err(|e| format!("invalid Solana base58 address '{trimmed}': {e}"))?;
    if decoded.len() != 32 {
        return Err(format!(
            "invalid Solana address '{trimmed}': expected 32 bytes, got {}",
            decoded.len()
        ));
    }
    Ok(trimmed.to_string())
}

pub async fn native_balance(address: &str) -> Result<u128, String> {
    validate_solana_address(address)?;
    #[derive(Deserialize)]
    struct BalanceResult {
        value: u64,
    }
    let result: BalanceResult =
        rpc_call(WalletChain::Solana, "getBalance", json!([address])).await?;
    Ok(result.value as u128)
}

/// SLIP-0010 ed25519 hardened-only derivation. Solana wallets standardize
/// on `m/44'/501'/N'/0'` so we never need to support non-hardened indices.
fn slip10_ed25519_derive(seed: &[u8], path: &[u32]) -> Result<[u8; 32], String> {
    type HmacSha512 = Hmac<Sha512>;
    let mut mac = HmacSha512::new_from_slice(b"ed25519 seed")
        .map_err(|e| format!("HMAC init failed: {e}"))?;
    mac.update(seed);
    let i = mac.finalize().into_bytes();
    let mut key = [0u8; 32];
    let mut chain_code = [0u8; 32];
    key.copy_from_slice(&i[..32]);
    chain_code.copy_from_slice(&i[32..]);
    for index in path {
        let hardened = *index | 0x8000_0000;
        let mut mac = HmacSha512::new_from_slice(&chain_code)
            .map_err(|e| format!("HMAC init failed: {e}"))?;
        mac.update(&[0u8]);
        mac.update(&key);
        mac.update(&hardened.to_be_bytes());
        let i = mac.finalize().into_bytes();
        key.copy_from_slice(&i[..32]);
        chain_code.copy_from_slice(&i[32..]);
    }
    Ok(key)
}

fn parse_path(path: &str) -> Result<Vec<u32>, String> {
    let trimmed = path.trim();
    let mut iter = trimmed.split('/');
    match iter.next() {
        Some("m") => {}
        _ => return Err(format!("Solana path '{path}' must start with 'm'")),
    }
    let mut out = Vec::new();
    for seg in iter {
        let stripped = seg
            .strip_suffix('\'')
            .ok_or_else(|| format!("Solana path '{path}' requires all-hardened segments"))?;
        let v: u32 = stripped
            .parse()
            .map_err(|e| format!("Solana path '{path}' segment '{seg}': {e}"))?;
        out.push(v);
    }
    if out.is_empty() {
        return Err(format!("Solana path '{path}' has no segments"));
    }
    Ok(out)
}

fn derive_solana_keypair(mnemonic: &str, derivation_path: &str) -> Result<SigningKey, String> {
    use coins_bip39::{English, Mnemonic};
    let mnemonic_obj: Mnemonic<English> = mnemonic
        .trim()
        .parse()
        .map_err(|e| format!("invalid BIP39 mnemonic: {e}"))?;
    let seed = mnemonic_obj
        .to_seed(None)
        .map_err(|e| format!("failed to derive BIP39 seed: {e}"))?;
    let path = parse_path(derivation_path)?;
    let secret = slip10_ed25519_derive(&seed, &path)?;
    let bytes: [u8; SECRET_KEY_LENGTH] = secret;
    Ok(SigningKey::from_bytes(&bytes))
}

/// Solana compact-u16 (shortvec) encoding.
fn encode_shortvec(value: u16) -> Vec<u8> {
    let mut out = Vec::new();
    let mut v = value as u32;
    loop {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return out;
        }
        byte |= 0x80;
        out.push(byte);
    }
}

/// Decode a Solana compact-u16 (shortvec). Returns `(value, bytes_consumed)`.
fn decode_shortvec(bytes: &[u8]) -> Result<(u16, usize), String> {
    let mut value: u32 = 0;
    let mut shift = 0u32;
    for (i, byte) in bytes.iter().enumerate() {
        if i >= 3 {
            return Err("shortvec too long".to_string());
        }
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            if value > u16::MAX as u32 {
                return Err("shortvec exceeds u16 range".to_string());
            }
            return Ok((value as u16, i + 1));
        }
        shift += 7;
    }
    Err("shortvec truncated".to_string())
}

#[derive(Debug, Clone)]
struct CompiledInstruction {
    program_id_index: u8,
    accounts: Vec<u8>,
    data: Vec<u8>,
}

fn encode_compiled_instruction(ins: &CompiledInstruction) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(ins.program_id_index);
    out.extend(encode_shortvec(ins.accounts.len() as u16));
    out.extend(&ins.accounts);
    out.extend(encode_shortvec(ins.data.len() as u16));
    out.extend(&ins.data);
    out
}

fn encode_message(
    header: [u8; 3],
    account_keys: &[[u8; 32]],
    recent_blockhash: &[u8; 32],
    instructions: &[CompiledInstruction],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(&header);
    out.extend(encode_shortvec(account_keys.len() as u16));
    for key in account_keys {
        out.extend(key);
    }
    out.extend(recent_blockhash);
    out.extend(encode_shortvec(instructions.len() as u16));
    for ins in instructions {
        out.extend(encode_compiled_instruction(ins));
    }
    out
}

/// Solana `find_program_address` — iterates a bump seed 255..=0, returning
/// the first off-curve PDA. Used to derive Associated Token Accounts.
fn find_program_address(seeds: &[&[u8]], program_id: &[u8; 32]) -> Result<([u8; 32], u8), String> {
    let pda_marker = b"ProgramDerivedAddress";
    for bump in (0u8..=255).rev() {
        let mut hasher = Sha256::new();
        for seed in seeds {
            hasher.update(seed);
        }
        hasher.update([bump]);
        hasher.update(program_id);
        hasher.update(pda_marker);
        let candidate: [u8; 32] = hasher.finalize().into();
        // Off-curve means it cannot be a public key.
        if CompressedEdwardsY(candidate).decompress().is_none() {
            return Ok((candidate, bump));
        }
    }
    Err("no off-curve PDA found".to_string())
}

pub fn associated_token_account(owner: &[u8; 32], mint: &[u8; 32]) -> Result<[u8; 32], String> {
    let token_program = token_program_id();
    let ata_program = ata_program_id();
    let (pda, _bump) =
        find_program_address(&[&owner[..], &token_program[..], &mint[..]], &ata_program)?;
    Ok(pda)
}

fn b58_to_pubkey(addr: &str) -> Result<[u8; 32], String> {
    let v = bs58::decode(addr)
        .into_vec()
        .map_err(|e| format!("invalid base58 '{addr}': {e}"))?;
    if v.len() != 32 {
        return Err(format!("expected 32-byte pubkey, got {}", v.len()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&v);
    Ok(out)
}

fn pubkey_to_b58(pubkey: &[u8; 32]) -> String {
    bs58::encode(pubkey).into_string()
}

fn build_native_transfer_message(
    from: [u8; 32],
    to: [u8; 32],
    lamports: u64,
    recent_blockhash: [u8; 32],
) -> Vec<u8> {
    // accounts: [from (signer, writable), to (writable), system_program (read-only)]
    let account_keys = vec![from, to, SYSTEM_PROGRAM_ID];
    // header: 1 required sig, 0 readonly signed, 1 readonly unsigned (system program)
    let header = [1u8, 0u8, 1u8];
    let mut data = vec![2u8, 0u8, 0u8, 0u8]; // SystemInstruction::Transfer = 2
    data.extend(&lamports.to_le_bytes());
    let ins = CompiledInstruction {
        program_id_index: 2,
        accounts: vec![0, 1],
        data,
    };
    encode_message(header, &account_keys, &recent_blockhash, &[ins])
}

fn build_spl_transfer_message(
    from_owner: [u8; 32],
    src_ata: [u8; 32],
    dst_ata: [u8; 32],
    amount: u64,
    recent_blockhash: [u8; 32],
) -> Vec<u8> {
    let token_program = token_program_id();
    // accounts:
    //  0: from_owner (signer, writable — to pay fee)
    //  1: src_ata (writable)
    //  2: dst_ata (writable)
    //  3: token_program (readonly, unsigned)
    let account_keys = vec![from_owner, src_ata, dst_ata, token_program];
    let header = [1u8, 0u8, 1u8];
    let mut data = vec![3u8]; // SPL Token instruction: Transfer = 3
    data.extend(&amount.to_le_bytes());
    let ins = CompiledInstruction {
        program_id_index: 3,
        accounts: vec![1, 2, 0], // src, dst, owner(signer)
        data,
    };
    encode_message(header, &account_keys, &recent_blockhash, &[ins])
}

/// Best-effort `getAccountInfo` check — returns `Ok(true)` when the account
/// exists, `Ok(false)` when the RPC reports `value: null`, or propagates the
/// transport error.
async fn account_exists(address_b58: &str) -> Result<bool, String> {
    #[derive(Deserialize)]
    struct AccountInfoResponse {
        value: serde_json::Value,
    }
    let resp: AccountInfoResponse = rpc_call(
        WalletChain::Solana,
        "getAccountInfo",
        json!([address_b58, {"encoding": "base64"}]),
    )
    .await?;
    Ok(!resp.value.is_null())
}

async fn fetch_recent_blockhash() -> Result<[u8; 32], String> {
    let result: BlockhashResponse = rpc_call(
        WalletChain::Solana,
        "getLatestBlockhash",
        json!([{"commitment": "finalized"}]),
    )
    .await?;
    b58_to_pubkey(&result.value.blockhash)
}

async fn broadcast_solana(signed: &[u8]) -> Result<String, String> {
    let b64 = B64.encode(signed);
    let tx_sig: String = rpc_call(
        WalletChain::Solana,
        "sendTransaction",
        json!([b64, {"encoding": "base64", "preflightCommitment": "processed"}]),
    )
    .await?;
    Ok(tx_sig)
}

pub async fn execute_solana_quote(
    mut quote: PreparedTransaction,
) -> Result<ExecutionResult, String> {
    let from_addr = quote.from_address.clone();
    let to_addr = quote.to_address.clone();
    validate_solana_address(&from_addr)?;
    validate_solana_address(&to_addr)?;
    let amount: u64 = quote
        .amount_raw
        .parse()
        .map_err(|e| format!("invalid Solana amount '{}': {e}", quote.amount_raw))?;

    let secret = secret_material(WalletChain::Solana).await?;
    let config = config_rpc::load_config_with_timeout().await?;
    let mnemonic =
        crate::openhuman::encryption::rpc::decrypt_secret(&config, &secret.encrypted_mnemonic)
            .await?
            .value;
    let signing_key = derive_solana_keypair(&mnemonic, &secret.derivation_path)?;
    let from_pk = signing_key.verifying_key().to_bytes();
    let expected_from = b58_to_pubkey(&from_addr)?;
    if from_pk != expected_from {
        return Err(format!(
            "Solana key derivation mismatch: derived {} but expected {}",
            pubkey_to_b58(&from_pk),
            from_addr
        ));
    }

    let recent_blockhash = fetch_recent_blockhash().await?;
    let to_pubkey = b58_to_pubkey(&to_addr)?;

    let message_bytes = match quote.kind {
        PreparedKind::NativeTransfer => {
            build_native_transfer_message(from_pk, to_pubkey, amount, recent_blockhash)
        }
        PreparedKind::TokenTransfer => {
            let mint_addr = quote
                .token_address
                .as_deref()
                .ok_or_else(|| "SPL transfer missing token_address (mint)".to_string())?;
            let mint = b58_to_pubkey(mint_addr)?;
            let src_ata = associated_token_account(&from_pk, &mint)?;
            let dst_ata = associated_token_account(&to_pubkey, &mint)?;
            // Preflight: refuse to send to a non-existent ATA so we don't
            // burn the broadcast on a guaranteed on-chain failure. The
            // caller (or a future PR) can prepend a CreateAssociatedTokenAccount
            // instruction; for now we fail loudly with a clear message.
            if !account_exists(&pubkey_to_b58(&dst_ata)).await? {
                return Err(format!(
                    "SPL preflight: destination Associated Token Account does not exist for mint {} owner {}; create it before transferring",
                    mint_addr,
                    pubkey_to_b58(&to_pubkey)
                ));
            }
            build_spl_transfer_message(from_pk, src_ata, dst_ata, amount, recent_blockhash)
        }
    };

    let signature = signing_key.sign(&message_bytes);
    let sig_bytes = signature.to_bytes();
    let mut wire = Vec::with_capacity(1 + 64 + message_bytes.len());
    wire.extend(encode_shortvec(1));
    wire.extend(&sig_bytes);
    wire.extend(&message_bytes);

    let tx_sig = broadcast_solana(&wire).await?;
    quote.status = PreparedStatus::Broadcasted;
    debug!(
        "{LOG_PREFIX} broadcast quote_id={} sig={} kind={:?}",
        quote.quote_id, tx_sig, quote.kind
    );
    let explorer_url = explorer_tx_url(WalletChain::Solana, &tx_sig);
    Ok(ExecutionResult {
        quote_id: quote.quote_id.clone(),
        status: PreparedStatus::Broadcasted,
        chain: WalletChain::Solana,
        evm_network: None,
        transaction_hash: tx_sig,
        explorer_url,
        transaction: quote,
    })
}

/// Crate-internal primitive: sign an externally-built, hex-encoded
/// `VersionedTransaction` (e.g. a deBridge swap/bridge tx) with the wallet's
/// Solana key and broadcast it. Not exposed as an agent tool or RPC.
///
/// Wire layout (Solana transaction): `shortvec(num_signatures)` followed by
/// `num_signatures * 64` signature slots, then the serialized message. We fill
/// the signature slot at the index whose `account_keys[i]` equals our pubkey,
/// signing the full message bytes (legacy or v0 — the message slice includes
/// the v0 version prefix, which is what Solana signs).
pub(crate) async fn sign_and_broadcast_versioned(
    tx_blob_hex: &str,
) -> Result<RawBroadcastResult, String> {
    let trimmed = tx_blob_hex.trim();
    let normalized = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    let mut wire =
        hex::decode(normalized).map_err(|e| format!("invalid Solana transaction hex blob: {e}"))?;

    let (num_signatures, sig_count_len) = decode_shortvec(&wire)?;
    let sigs_start = sig_count_len;
    let message_start = sigs_start + (num_signatures as usize) * 64;
    if message_start > wire.len() {
        return Err("Solana tx blob truncated before message".to_string());
    }
    let message = &wire[message_start..];
    if message.is_empty() {
        return Err("Solana tx blob has empty message".to_string());
    }

    // Determine message version + header offset.
    let versioned = message[0] & 0x80 != 0;
    let header_off = if versioned { 1 } else { 0 };
    if message.len() < header_off + 3 {
        return Err("Solana message header truncated".to_string());
    }
    let num_required_signatures = message[header_off] as usize;
    if num_required_signatures == 0 {
        return Err("Solana message declares zero required signatures".to_string());
    }
    // Parse account keys (need at least the signer keys to find our index).
    let keys_off = header_off + 3;
    let (account_count, count_len) = decode_shortvec(&message[keys_off..])?;
    let keys_start = keys_off + count_len;
    if account_count as usize > num_required_signatures.max(account_count as usize) {
        // sanity only; continue
    }
    let signer_keys = num_required_signatures.min(account_count as usize);
    if keys_start + signer_keys * 32 > message.len() {
        return Err("Solana account keys region truncated".to_string());
    }

    // Derive our signing key.
    let secret = secret_material(WalletChain::Solana).await?;
    let config = config_rpc::load_config_with_timeout().await?;
    let mnemonic =
        crate::openhuman::encryption::rpc::decrypt_secret(&config, &secret.encrypted_mnemonic)
            .await?
            .value;
    let signing_key = derive_solana_keypair(&mnemonic, &secret.derivation_path)?;
    let our_pubkey = signing_key.verifying_key().to_bytes();

    // Find our signer index.
    let mut our_index: Option<usize> = None;
    for i in 0..signer_keys {
        let off = keys_start + i * 32;
        if message[off..off + 32] == our_pubkey {
            our_index = Some(i);
            break;
        }
    }
    let our_index = our_index.ok_or_else(|| {
        format!(
            "wallet Solana address {} is not a required signer of this transaction",
            pubkey_to_b58(&our_pubkey)
        )
    })?;
    if our_index >= num_signatures as usize {
        return Err("Solana signer index exceeds signature slot count".to_string());
    }

    // Sign the message bytes and write into our signature slot.
    let signature = signing_key.sign(message);
    let sig_bytes = signature.to_bytes();
    let slot_off = sigs_start + our_index * 64;
    wire[slot_off..slot_off + 64].copy_from_slice(&sig_bytes);

    let tx_sig = broadcast_solana(&wire).await?;
    debug!("{LOG_PREFIX} sign_and_broadcast_versioned sig={tx_sig}");
    Ok(RawBroadcastResult {
        transaction_hash: tx_sig.clone(),
        explorer_url: explorer_tx_url(WalletChain::Solana, &tx_sig),
        // Solana fees are dynamic (base + priority) and only known once the tx
        // is confirmed — leave unset rather than misreporting a free transfer.
        fee_raw: None,
    })
}

/// `getSignatureStatuses` → normalized status.
pub async fn tx_status(hash: &str) -> Result<TxStatusInfo, String> {
    #[derive(Deserialize)]
    struct StatusResp {
        value: Vec<Option<SigStatus>>,
    }
    #[derive(Deserialize)]
    struct SigStatus {
        slot: u64,
        confirmations: Option<u64>,
        err: Option<serde_json::Value>,
    }
    let resp: StatusResp = rpc_call(
        WalletChain::Solana,
        "getSignatureStatuses",
        json!([[hash], {"searchTransactionHistory": true}]),
    )
    .await?;
    let entry = resp.value.into_iter().next().flatten();
    let (state, confirmations, block_number) = match entry {
        None => (TxState::NotFound, None, None),
        Some(status) => {
            let state = if status.err.is_some() {
                TxState::Failed
            } else if status.confirmations.is_none() {
                // null confirmations means "finalized / rooted".
                TxState::Confirmed
            } else {
                TxState::Pending
            };
            (state, status.confirmations, Some(status.slot))
        }
    };
    Ok(TxStatusInfo {
        chain: WalletChain::Solana,
        evm_network: None,
        hash: hash.to_string(),
        state,
        confirmations,
        block_number,
    })
}

/// `getTransaction` → normalized receipt with raw passthrough.
pub async fn tx_receipt(hash: &str) -> Result<TxReceiptInfo, String> {
    let tx: serde_json::Value = rpc_call(
        WalletChain::Solana,
        "getTransaction",
        json!([hash, {"maxSupportedTransactionVersion": 0, "encoding": "json"}]),
    )
    .await?;
    if tx.is_null() {
        return Ok(TxReceiptInfo {
            chain: WalletChain::Solana,
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
    let meta = tx.get("meta");
    let success = meta.map(|m| m.get("err").map(|e| e.is_null()).unwrap_or(true));
    let fee_raw = meta
        .and_then(|m| m.get("fee"))
        .and_then(|v| v.as_u64())
        .map(|f| f.to_string());
    let block_number = tx.get("slot").and_then(|v| v.as_u64());
    Ok(TxReceiptInfo {
        chain: WalletChain::Solana,
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

/// `getTransaction` → raw transaction passthrough.
pub async fn lookup_tx(hash: &str) -> Result<TxLookupInfo, String> {
    let tx: serde_json::Value = rpc_call(
        WalletChain::Solana,
        "getTransaction",
        json!([hash, {"maxSupportedTransactionVersion": 0, "encoding": "json"}]),
    )
    .await?;
    Ok(TxLookupInfo {
        chain: WalletChain::Solana,
        evm_network: None,
        hash: hash.to_string(),
        found: !tx.is_null(),
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
    use crate::openhuman::wallet::test_support::{
        sample_solana_address, setup_wallet_in, TEST_LOCK,
    };
    use axum::{routing::post, Router};
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::net::TcpListener;

    #[test]
    fn shortvec_encodes_small_and_large_values() {
        assert_eq!(encode_shortvec(0), vec![0]);
        assert_eq!(encode_shortvec(1), vec![1]);
        assert_eq!(encode_shortvec(127), vec![127]);
        assert_eq!(encode_shortvec(128), vec![0x80, 1]);
        assert_eq!(encode_shortvec(16_383), vec![0xff, 0x7f]);
        assert_eq!(encode_shortvec(16_384), vec![0x80, 0x80, 1]);
    }

    #[test]
    fn validate_solana_address_accepts_known_32_byte_pubkey() {
        let addr = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
        assert_eq!(validate_solana_address(addr).unwrap(), addr);
    }

    #[test]
    fn validate_solana_address_rejects_wrong_length() {
        // "tooShort" decodes to ~6 bytes, not 32.
        let err = validate_solana_address("tooShort").unwrap_err();
        assert!(err.contains("32 bytes"), "got: {err}");
    }

    #[test]
    fn parse_path_requires_hardened_segments() {
        assert!(parse_path("m/44'/501'/0'/0'").is_ok());
        assert!(parse_path("m/44/501/0/0").is_err());
        assert!(parse_path("m").is_err());
    }

    #[test]
    fn derive_solana_keypair_produces_known_address_for_test_mnemonic() {
        // SLIP-0010 ed25519 hardened derivation at m/44'/501'/0'/0' from the
        // standard "abandon × 11 about" mnemonic. Deterministic output —
        // pinned here so a regression in HMAC-SHA512 path traversal or seed
        // derivation flips this test before it ships.
        let mnemonic =
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let signing = derive_solana_keypair(mnemonic, "m/44'/501'/0'/0'").unwrap();
        let pk = signing.verifying_key().to_bytes();
        let addr = pubkey_to_b58(&pk);
        assert_eq!(addr, "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk");
        validate_solana_address(&addr).expect("derived addr is 32-byte base58");
    }

    #[test]
    fn native_transfer_message_round_trips_basic_structure() {
        let from = [1u8; 32];
        let to = [2u8; 32];
        let bh = [3u8; 32];
        let msg = build_native_transfer_message(from, to, 1_000_000, bh);
        // header is first 3 bytes.
        assert_eq!(&msg[..3], &[1u8, 0u8, 1u8]);
        // shortvec(3) = [3], then 3 keys = 96 bytes.
        assert_eq!(msg[3], 3);
        assert_eq!(&msg[4..36], &from);
        assert_eq!(&msg[36..68], &to);
        assert_eq!(&msg[68..100], &SYSTEM_PROGRAM_ID);
        // blockhash next
        assert_eq!(&msg[100..132], &bh);
        // shortvec(1) instructions = [1]
        assert_eq!(msg[132], 1);
        // program_id_index = 2 (system program)
        assert_eq!(msg[133], 2);
        // shortvec(2) accounts = [2]
        assert_eq!(msg[134], 2);
        assert_eq!(msg[135], 0); // from
        assert_eq!(msg[136], 1); // to
                                 // shortvec(12) data length = [12]
        assert_eq!(msg[137], 12);
        // Transfer discriminator + 8 LE amount bytes
        assert_eq!(&msg[138..142], &[2u8, 0u8, 0u8, 0u8]);
        let amt = u64::from_le_bytes(msg[142..150].try_into().unwrap());
        assert_eq!(amt, 1_000_000);
    }

    async fn start_solana_mock(
        sig: &'static str,
    ) -> (
        std::net::SocketAddr,
        Arc<parking_lot::Mutex<Vec<serde_json::Value>>>,
    ) {
        let calls: Arc<parking_lot::Mutex<Vec<serde_json::Value>>> =
            Arc::new(parking_lot::Mutex::new(Vec::new()));
        let calls_clone = calls.clone();
        let app = Router::new().route(
            "/",
            post(move |axum::Json(payload): axum::Json<serde_json::Value>| {
                let calls = calls_clone.clone();
                async move {
                    calls.lock().push(payload.clone());
                    let method = payload
                        .get("method")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    let result = match method {
                        "getLatestBlockhash" => json!({
                            "context": {"slot": 0},
                            "value": {
                                "blockhash": "GHtXQBsoZHVnNFa9YevAzFr17DJjgHXk3ycTKD5xD3Zi",
                                "lastValidBlockHeight": 0u64
                            }
                        }),
                        "getBalance" => json!({
                            "context": {"slot": 0},
                            "value": 1_000_000u64
                        }),
                        "getAccountInfo" => json!({
                            "context": {"slot": 0},
                            "value": {
                                "lamports": 2_039_280u64,
                                "owner": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
                                "data": ["", "base64"],
                                "executable": false,
                                "rentEpoch": 0u64
                            }
                        }),
                        "sendTransaction" => serde_json::Value::String(sig.to_string()),
                        _ => serde_json::Value::Null,
                    };
                    axum::Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
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

    #[tokio::test]
    async fn execute_solana_quote_signs_and_broadcasts_native_transfer() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();

        let fake_sig = "5xS9pXmqVz8R1nuRZTfsdsAxBdBFmtnAtuYbCsmK5DYzGn5vR4VqWGmiR5McLnYx8oFqLdo62q4qiUZpQyR4Hkn3";
        let (addr, calls) = start_solana_mock(fake_sig).await;
        std::env::set_var("OPENHUMAN_WALLET_RPC_SOLANA", format!("http://{addr}"));

        let now = now_ms();
        let quote = PreparedTransaction {
            quote_id: "q_sol_native_1".to_string(),
            kind: PreparedKind::NativeTransfer,
            chain: WalletChain::Solana,
            evm_network: None,
            from_address: sample_solana_address().to_string(),
            to_address: "Vote111111111111111111111111111111111111111".to_string(),
            asset_symbol: "SOL".to_string(),
            amount_raw: "1000".to_string(),
            amount_formatted: "0.000001000".to_string(),
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

        let result = execute_solana_quote(quote)
            .await
            .expect("solana broadcast ok");
        assert_eq!(result.status, PreparedStatus::Broadcasted);
        assert_eq!(result.transaction_hash, fake_sig);
        // Two RPC calls: getLatestBlockhash + sendTransaction.
        let recorded = calls.lock().clone();
        assert_eq!(recorded.len(), 2);
        assert_eq!(
            recorded[0].get("method").and_then(|v| v.as_str()),
            Some("getLatestBlockhash")
        );
        assert_eq!(
            recorded[1].get("method").and_then(|v| v.as_str()),
            Some("sendTransaction")
        );
    }

    #[tokio::test]
    async fn execute_solana_quote_signs_and_broadcasts_spl_transfer() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();

        let fake_sig = "5xS9pXmqVz8R1nuRZTfsdsAxBdBFmtnAtuYbCsmK5DYzGn5vR4VqWGmiR5McLnYx8oFqLdo62q4qiUZpQyR4Hkn3";
        let (addr, calls) = start_solana_mock(fake_sig).await;
        std::env::set_var("OPENHUMAN_WALLET_RPC_SOLANA", format!("http://{addr}"));

        let now = now_ms();
        let quote = PreparedTransaction {
            quote_id: "q_sol_spl_1".to_string(),
            kind: PreparedKind::TokenTransfer,
            chain: WalletChain::Solana,
            evm_network: None,
            from_address: sample_solana_address().to_string(),
            to_address: "Vote111111111111111111111111111111111111111".to_string(),
            asset_symbol: "USDC".to_string(),
            amount_raw: "1000000".to_string(),
            amount_formatted: "1.000000".to_string(),
            receive_symbol: None,
            min_receive_raw: None,
            calldata: None,
            token_address: Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string()),
            estimated_fee_raw: "5000".to_string(),
            status: PreparedStatus::AwaitingConfirmation,
            created_at_ms: now,
            expires_at_ms: now + 60_000,
            notes: vec![],
            owner: None,
        };
        insert_quote_for_test(quote.clone());

        let result = execute_solana_quote(quote).await.expect("spl broadcast ok");
        assert_eq!(result.status, PreparedStatus::Broadcasted);
        let recorded = calls.lock().clone();
        // SPL preflight calls getAccountInfo somewhere in the request set, plus
        // getLatestBlockhash + sendTransaction.
        assert_eq!(recorded.len(), 3);
        assert!(
            recorded
                .iter()
                .any(|c| c.get("method").and_then(|v| v.as_str()) == Some("getAccountInfo")),
            "SPL preflight must call getAccountInfo"
        );
        // The sendTransaction param[0] is base64-encoded signed tx; pull the
        // base64 string and decode it to confirm it carries the SPL token
        // program ID in its account_keys.
        // sendTransaction is the last call after getAccountInfo + getLatestBlockhash.
        let send_call = recorded
            .iter()
            .rev()
            .find(|c| c.get("method").and_then(|v| v.as_str()) == Some("sendTransaction"))
            .expect("sendTransaction call recorded");
        let params = send_call.get("params").and_then(|v| v.as_array()).unwrap();
        let tx_b64 = params[0].as_str().unwrap();
        let raw = B64.decode(tx_b64).expect("valid base64");
        // shortvec(1) signature + 64-byte sig + message
        assert_eq!(raw[0], 1, "exactly one signature");
        let message = &raw[1 + 64..];
        // header (3) + shortvec(4) + 4*32 keys: token program must be one of them.
        let token_program = token_program_id();
        assert!(
            message.windows(32).any(|w| w == token_program),
            "expected token program in account_keys"
        );
    }

    #[tokio::test]
    async fn execute_solana_quote_refuses_spl_when_destination_ata_missing() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();

        // Custom mock that returns null for getAccountInfo — simulates an ATA
        // that was never created on-chain.
        let app = Router::new().route(
            "/",
            post(
                |axum::Json(payload): axum::Json<serde_json::Value>| async move {
                    let method = payload
                        .get("method")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    let result = match method {
                        "getAccountInfo" => json!({"context": {"slot": 0}, "value": null}),
                        "getLatestBlockhash" => json!({
                            "context": {"slot": 0},
                            "value": {
                                "blockhash": "GHtXQBsoZHVnNFa9YevAzFr17DJjgHXk3ycTKD5xD3Zi",
                                "lastValidBlockHeight": 0u64
                            }
                        }),
                        _ => serde_json::Value::Null,
                    };
                    axum::Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
                },
            ),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        std::env::set_var("OPENHUMAN_WALLET_RPC_SOLANA", format!("http://{addr}"));

        let now = now_ms();
        let quote = PreparedTransaction {
            quote_id: "q_sol_spl_missing_ata".to_string(),
            kind: PreparedKind::TokenTransfer,
            chain: WalletChain::Solana,
            evm_network: None,
            from_address: sample_solana_address().to_string(),
            to_address: "Vote111111111111111111111111111111111111111".to_string(),
            asset_symbol: "USDC".to_string(),
            amount_raw: "1000000".to_string(),
            amount_formatted: "1.000000".to_string(),
            receive_symbol: None,
            min_receive_raw: None,
            calldata: None,
            token_address: Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string()),
            estimated_fee_raw: "5000".to_string(),
            status: PreparedStatus::AwaitingConfirmation,
            created_at_ms: now,
            expires_at_ms: now + 60_000,
            notes: vec![],
            owner: None,
        };
        insert_quote_for_test(quote.clone());

        let err = execute_solana_quote(quote).await.unwrap_err();
        assert!(
            err.contains("SPL preflight")
                && err.contains("Associated Token Account does not exist"),
            "got: {err}"
        );
    }

    #[test]
    fn associated_token_account_derives_off_curve_pda_for_usdc_mint() {
        // find_program_address must produce an off-curve point (else it
        // would be a valid pubkey, which violates the ATA program's
        // contract). We verify two invariants:
        //  - derivation is deterministic for fixed (owner, mint)
        //  - result is off-curve (CompressedEdwardsY::decompress is None)
        let owner = b58_to_pubkey("HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk").unwrap();
        let mint = b58_to_pubkey("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        let ata_a = associated_token_account(&owner, &mint).unwrap();
        let ata_b = associated_token_account(&owner, &mint).unwrap();
        assert_eq!(ata_a, ata_b, "ATA derivation must be deterministic");
        assert!(
            CompressedEdwardsY(ata_a).decompress().is_none(),
            "ATA must be off-curve"
        );
    }

    #[test]
    fn spl_transfer_message_uses_token_program_and_correct_accounts() {
        let from = [1u8; 32];
        let src = [2u8; 32];
        let dst = [3u8; 32];
        let bh = [4u8; 32];
        let msg = build_spl_transfer_message(from, src, dst, 42, bh);
        // 4 account keys: from, src, dst, token_program
        assert_eq!(msg[3], 4);
        let token_program = token_program_id();
        let key3 = &msg[4 + 96..4 + 128];
        assert_eq!(key3, &token_program);
    }

    #[test]
    fn decode_shortvec_round_trips_encode() {
        for v in [0u16, 1, 127, 128, 16_383, 16_384, 65_535] {
            let enc = encode_shortvec(v);
            let (decoded, len) = decode_shortvec(&enc).unwrap();
            assert_eq!(decoded, v, "value {v} round-trips");
            assert_eq!(len, enc.len(), "consumed length matches for {v}");
        }
    }

    /// Build a minimal legacy VersionedTransaction wire with `signer` as the
    /// sole required signer and an empty signature slot.
    fn build_unsigned_legacy(signer: &[u8; 32]) -> Vec<u8> {
        let mut message = Vec::new();
        message.extend([1u8, 0u8, 0u8]); // header: 1 required sig
        message.extend(encode_shortvec(1)); // 1 account key
        message.extend(signer);
        message.extend([0u8; 32]); // recent blockhash
        message.extend(encode_shortvec(0)); // 0 instructions
        let mut wire = Vec::new();
        wire.extend(encode_shortvec(1)); // 1 signature slot
        wire.extend([0u8; 64]); // empty sig
        wire.extend(&message);
        wire
    }

    #[tokio::test]
    async fn sign_and_broadcast_versioned_fills_signature_and_broadcasts() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();

        let fake_sig = "5xS9pXmqVz8R1nuRZTfsdsAxBdBFmtnAtuYbCsmK5DYzGn5vR4VqWGmiR5McLnYx8oFqLdo62q4qiUZpQyR4Hkn3";
        let (addr, calls) = start_solana_mock(fake_sig).await;
        std::env::set_var("OPENHUMAN_WALLET_RPC_SOLANA", format!("http://{addr}"));

        let signer = b58_to_pubkey(sample_solana_address()).unwrap();
        let wire = build_unsigned_legacy(&signer);
        let result = sign_and_broadcast_versioned(&hex::encode(&wire))
            .await
            .expect("sign+broadcast ok");
        assert_eq!(result.transaction_hash, fake_sig);

        // The broadcast tx must carry a non-zero signature in slot 0.
        let send = calls
            .lock()
            .iter()
            .rev()
            .find(|c| c.get("method").and_then(|v| v.as_str()) == Some("sendTransaction"))
            .cloned()
            .expect("sendTransaction recorded");
        let b64 = send.get("params").and_then(|p| p.as_array()).unwrap()[0]
            .as_str()
            .unwrap()
            .to_string();
        let raw = B64.decode(b64).unwrap();
        // shortvec(1) + 64-byte sig; the sig must not be all zeros now.
        assert_eq!(raw[0], 1);
        assert!(raw[1..1 + 64].iter().any(|b| *b != 0), "signature filled");
    }

    #[tokio::test]
    async fn sign_and_broadcast_versioned_rejects_non_signer() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_quote_store_for_tests();
        let temp = TempDir::new().unwrap();
        setup_wallet_in(&temp).await.unwrap();

        // A signer pubkey that is NOT our wallet — sign must refuse.
        let other = [7u8; 32];
        let wire = build_unsigned_legacy(&other);
        let err = sign_and_broadcast_versioned(&hex::encode(&wire))
            .await
            .unwrap_err();
        assert!(err.contains("not a required signer"), "got: {err}");
    }

    #[tokio::test]
    async fn tx_status_reads_signature_status() {
        let _guard = TEST_LOCK.lock();
        let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let app = Router::new().route(
            "/",
            post(|axum::Json(_p): axum::Json<serde_json::Value>| async move {
                axum::Json(json!({
                    "jsonrpc": "2.0", "id": 1,
                    "result": {"context": {"slot": 0}, "value": [
                        {"slot": 123u64, "confirmations": null, "err": null}
                    ]}
                }))
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        std::env::set_var("OPENHUMAN_WALLET_RPC_SOLANA", format!("http://{addr}"));
        let info = tx_status("somesig").await.unwrap();
        assert_eq!(
            info.state,
            crate::openhuman::wallet::execution::TxState::Confirmed
        );
        assert_eq!(info.block_number, Some(123));
    }
}
