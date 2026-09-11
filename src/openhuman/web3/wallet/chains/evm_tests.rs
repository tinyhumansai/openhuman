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
                    "eth_sendRawTransaction" => JsonValue::String(format!("0x{}", "ab".repeat(32))),
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
