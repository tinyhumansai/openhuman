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
#[cfg(test)]
use ed25519_dalek::{SigningKey, SECRET_KEY_LENGTH};
use log::debug;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

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

/// Validate a Solana address (a base58 ed25519 public key).
///
/// Delegates to the vendored [`tinywallet_bus`] crate, which owns the address
/// format; this wrapper keeps the `Result<_, String>` shape the rest of the
/// domain speaks.
pub fn validate_solana_address(addr: &str) -> Result<String, String> {
    let result = tinywallet_bus::address::solana::validate(addr).map_err(|e| e.to_string());
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

/// Derive the Solana signing key for `derivation_path` from a BIP-39 mnemonic.
///
/// Test-only, and deliberately on the **root** `tinywallet` crate rather than
/// `tinywallet-bus`: `key` is one of the gates that did not move into the
/// contract crate. The root crate is a dev-dependency here, so this derivation
/// stack is not linked into the shipped binary. Production derives inside the
/// wallet module, via `modules::wallet::derive_account`.
///
/// The root crate owns SLIP-0010 ed25519 derivation; the hand-rolled HMAC walk
/// and path parser that used to live here moved there wholesale — nothing about
/// "derive an ed25519 key at a hardened path" is OpenHuman-specific. Custody
/// stays here: the mnemonic arrives already decrypted from the keyring and
/// `tinywallet` never sees a stored secret.
///
/// One behavioural note: `tinywallet` reports a non-hardened Solana path as
/// its own error variant rather than folding it into a generic parse failure,
/// because such a path is derivable-looking but underivable on ed25519 — and
/// silently hardening it would return a different account than the path names.
/// Test-only: production derives inside the wallet module.
#[cfg(test)]
fn derive_solana_keypair(mnemonic: &str, derivation_path: &str) -> Result<SigningKey, String> {
    let derived = tinywallet::key::derive(tinywallet::Chain::Solana, mnemonic, derivation_path)
        .map_err(|e| e.to_string())?;
    let bytes: [u8; SECRET_KEY_LENGTH] = derived
        .secret_bytes()
        .try_into()
        .map_err(|_| "tinywallet returned an unexpected Solana key length".to_string())?;
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

/// The wallet's Solana account, and the phrase to sign with.
///
/// Derivation and signing both happen in the loaded wallet module; this process
/// holds the phrase only long enough to hand it over on a confidential call,
/// and never assembles a private key. The module is sent it only after proving
/// it is an artifact this build pinned — see `modules::wallet::attested_proxy`.
///
/// # The `cfg(test)` branch
///
/// Under `cfg(test)` this derives locally instead of calling the module, and so
/// does [`solana_sign`]. A unit test has no loaded module, and the coverage
/// these tests carry is the RPC choreography and wire format around signing —
/// how many calls go out, in what order, and what bytes get broadcast — none of
/// which is about *who* holds the key.
///
/// What that deliberately does not cover is the module wiring itself. That is
/// covered where it can be honest: tinywallet's loader E2E signs through a real
/// `dlopen`'d module over a real broker, and `modules::wallet`'s own tests pin
/// the attestation guard. The local branch cannot exist in a shipped binary.
async fn solana_signer(
    config: &crate::openhuman::config::Config,
) -> Result<(tinywallet_bus::wire::SecretMaterial, [u8; 32]), String> {
    let secret = secret_material(WalletChain::Solana).await?;
    let mnemonic = crate::openhuman::security::encryption::rpc::decrypt_secret(
        config,
        &secret.encrypted_mnemonic,
    )
    .await?
    .value;
    let signing_secret = tinywallet_bus::wire::SecretMaterial {
        mnemonic,
        derivation_path: secret.derivation_path.clone(),
        chain: tinywallet_bus::Chain::Solana,
    };
    #[cfg(test)]
    {
        let _ = config;
        let derived =
            derive_solana_keypair(&signing_secret.mnemonic, &signing_secret.derivation_path)?;
        return Ok((signing_secret, derived.verifying_key().to_bytes()));
    }

    #[cfg(not(test))]
    {
        let account = crate::openhuman::modules::wallet::derive_account(config, &signing_secret)
            .await
            .map_err(|e| format!("failed to derive the Solana account: {e}"))?;
        let pubkey = b58_to_pubkey(&account.address)?;
        Ok((signing_secret, pubkey))
    }
}

/// Sign `message` with the wallet key, inside the module.
async fn solana_sign(
    config: &crate::openhuman::config::Config,
    signing_secret: &tinywallet_bus::wire::SecretMaterial,
    message: &[u8],
) -> Result<[u8; 64], String> {
    #[cfg(test)]
    {
        use ed25519_dalek::Signer as _;
        let _ = config;
        let key = derive_solana_keypair(&signing_secret.mnemonic, &signing_secret.derivation_path)?;
        return Ok(key.sign(message).to_bytes());
    }

    #[cfg(not(test))]
    let signature = crate::openhuman::modules::wallet::sign_message(
        config,
        signing_secret,
        message,
        tinywallet_bus::wire::Scheme::Ed25519,
    )
    .await
    .map_err(|e| format!("failed to sign the Solana message: {e}"))?;
    #[cfg(not(test))]
    {
        let tinywallet_bus::wire::Signature::Ed25519 { signature_hex } = signature else {
            return Err("the wallet module returned a non-ed25519 Solana signature".to_string());
        };
        let bytes = hex_to_bytes(&signature_hex)?;
        <[u8; 64]>::try_from(bytes.as_slice())
            .map_err(|_| "the wallet module returned a malformed Solana signature".to_string())
    }
}

/// Decode lowercase hex.
fn hex_to_bytes(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("odd-length hex from the wallet module".to_string());
    }
    (0..value.len() / 2)
        .map(|i| {
            u8::from_str_radix(&value[i * 2..i * 2 + 2], 16)
                .map_err(|e| format!("invalid hex from the wallet module: {e}"))
        })
        .collect()
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

    let config = config_rpc::load_config_with_timeout().await?;
    let (signing_secret, from_pk) = solana_signer(&config).await?;
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

    let sig_bytes = solana_sign(&config, &signing_secret, &message_bytes).await?;
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
    let config = config_rpc::load_config_with_timeout().await?;
    let (signing_secret, our_pubkey) = solana_signer(&config).await?;

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
    let sig_bytes = solana_sign(&config, &signing_secret, message).await?;
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
#[path = "solana_tests.rs"]
mod tests;
