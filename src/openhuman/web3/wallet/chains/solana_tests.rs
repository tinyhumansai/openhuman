use super::*;
use crate::openhuman::web3::wallet::execution::{
    insert_quote_for_test, now_ms, reset_quote_store_for_tests, PreparedKind, PreparedStatus,
    PreparedTransaction,
};
use crate::openhuman::web3::wallet::test_support::{
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
fn unhardened_paths_are_rejected() {
    // Path parsing lives in the root `tinywallet` crate, so this exercises
    // the rule through the derivation entry point rather than a private
    // helper.
    const MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon \
                            abandon abandon abandon abandon abandon about";
    assert!(derive_solana_keypair(MNEMONIC, "m/44'/501'/0'/0'").is_ok());
    // Non-hardened segments are underivable on ed25519, not merely
    // unsupported — silently hardening them would yield a different account.
    assert!(derive_solana_keypair(MNEMONIC, "m/44/501/0/0").is_err());
    assert!(derive_solana_keypair(MNEMONIC, "m").is_err());
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
    reset_quote_store_for_tests();
    let temp = TempDir::new().unwrap();
    let _workspace_guard = setup_wallet_in(&temp).await.unwrap();

    let fake_sig =
        "5xS9pXmqVz8R1nuRZTfsdsAxBdBFmtnAtuYbCsmK5DYzGn5vR4VqWGmiR5McLnYx8oFqLdo62q4qiUZpQyR4Hkn3";
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
    reset_quote_store_for_tests();
    let temp = TempDir::new().unwrap();
    let _workspace_guard = setup_wallet_in(&temp).await.unwrap();

    let fake_sig =
        "5xS9pXmqVz8R1nuRZTfsdsAxBdBFmtnAtuYbCsmK5DYzGn5vR4VqWGmiR5McLnYx8oFqLdo62q4qiUZpQyR4Hkn3";
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
    reset_quote_store_for_tests();
    let temp = TempDir::new().unwrap();
    let _workspace_guard = setup_wallet_in(&temp).await.unwrap();

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
        err.contains("SPL preflight") && err.contains("Associated Token Account does not exist"),
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
    reset_quote_store_for_tests();
    let temp = TempDir::new().unwrap();
    let _workspace_guard = setup_wallet_in(&temp).await.unwrap();

    let fake_sig =
        "5xS9pXmqVz8R1nuRZTfsdsAxBdBFmtnAtuYbCsmK5DYzGn5vR4VqWGmiR5McLnYx8oFqLdo62q4qiUZpQyR4Hkn3";
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
    reset_quote_store_for_tests();
    let temp = TempDir::new().unwrap();
    let _workspace_guard = setup_wallet_in(&temp).await.unwrap();

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
        crate::openhuman::web3::wallet::execution::TxState::Confirmed
    );
    assert_eq!(info.block_number, Some(123));
}
