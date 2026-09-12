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
    out.extend(tinywallet_bus::tx::proto::encode_varint(number << 3));
    out.extend(tinywallet_bus::tx::proto::encode_varint(value));
}

fn push_bytes_field(out: &mut Vec<u8>, number: u64, value: &[u8]) {
    out.extend(tinywallet_bus::tx::proto::encode_varint((number << 3) | 2));
    out.extend(tinywallet_bus::tx::proto::encode_varint(value.len() as u64));
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
                    let txid = tinywallet_bus::tx::tron::recompute_txid(&raw).unwrap();
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
                    let txid = tinywallet_bus::tx::tron::recompute_txid(&raw).unwrap();
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
    let native_txid = tinywallet_bus::tx::tron::recompute_txid(&native_raw_hex).unwrap();
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
        tinywallet_bus::wire::TransactionSpec::Tron {
            raw_data_hex: native_raw_hex,
            expected_to: recipient.to_string(),
            expected_txid: native_txid,
            // Carried through to the module, which re-verifies it against
            // the bytes rather than trusting this side's check.
            transfer: TronTransferVerification::Native {
                amount_sun: 1_000_000,
            },
        }
    );

    let parameter = "01".repeat(64);
    let token_raw = trc20_raw(&contract_hex, &parameter);
    let token_txid = tinywallet_bus::tx::tron::recompute_txid(&token_raw).unwrap();
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
        tinywallet_bus::wire::TransactionSpec::Tron {
            raw_data_hex: token_raw,
            expected_to: contract.to_string(),
            expected_txid: token_txid,
            transfer: TronTransferVerification::Trc20 {
                parameter_hex: parameter.clone(),
            },
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
            tx_id: tinywallet_bus::tx::tron::recompute_txid(&raw_data_hex).unwrap(),
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
    decoy.extend(tinywallet_bus::tx::proto::encode_varint(1_000_000));
    push_bytes_field(&mut spoofed_raw, 10, &decoy);
    let spoofed_raw = hex::encode(spoofed_raw);
    let spoofed_tx = CreateTransactionResponse {
        tx_id: tinywallet_bus::tx::tron::recompute_txid(&spoofed_raw).unwrap(),
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
                let txid = tinywallet_bus::tx::tron::recompute_txid(&raw).unwrap();
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
