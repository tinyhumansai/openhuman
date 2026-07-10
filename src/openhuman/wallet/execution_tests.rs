use std::net::SocketAddr;
use std::sync::Arc;

use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::net::TcpListener;

use super::*;
use crate::openhuman::wallet::test_support::{setup_wallet_in, TEST_LOCK};

#[derive(Clone)]
struct MockRpcState {
    estimate_calls: Arc<Mutex<Vec<Value>>>,
    raw_txs: Arc<Mutex<Vec<String>>>,
    chain_id: String,
}

async fn mock_rpc(State(state): State<MockRpcState>, Json(payload): Json<Value>) -> Json<Value> {
    let method = payload
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = payload
        .get("params")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let result = match method {
        "eth_chainId" => Value::String(state.chain_id.clone()),
        "eth_getTransactionCount" => Value::String("0x7".to_string()),
        "eth_gasPrice" => Value::String("0x3b9aca00".to_string()),
        "eth_estimateGas" => {
            state
                .estimate_calls
                .lock()
                .push(params.first().cloned().unwrap_or(Value::Null));
            Value::String("0x5208".to_string())
        }
        "eth_sendRawTransaction" => {
            if let Some(raw) = params.first().and_then(Value::as_str) {
                state.raw_txs.lock().push(raw.to_string());
            }
            Value::String(
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            )
        }
        "eth_getBalance" => Value::String("0xde0b6b3a7640000".to_string()),
        _ => Value::Null,
    };
    Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
}

pub(crate) async fn start_mock_rpc_with_chain_id(
    chain_id_hex: &str,
) -> Result<(SocketAddr, Arc<Mutex<Vec<Value>>>, Arc<Mutex<Vec<String>>>), String> {
    let estimate_calls = Arc::new(Mutex::new(Vec::new()));
    let raw_txs = Arc::new(Mutex::new(Vec::new()));
    let state = MockRpcState {
        estimate_calls: estimate_calls.clone(),
        raw_txs: raw_txs.clone(),
        chain_id: chain_id_hex.to_string(),
    };
    let app = Router::new().route("/", post(mock_rpc)).with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("failed to bind mock rpc: {e}"))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("failed to read mock rpc addr: {e}"))?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Ok((addr, estimate_calls, raw_txs))
}

async fn start_mock_rpc(
) -> Result<(SocketAddr, Arc<Mutex<Vec<Value>>>, Arc<Mutex<Vec<String>>>), String> {
    start_mock_rpc_with_chain_id("0x1").await
}

#[test]
fn validates_amount_rejects_empty_and_non_numeric() {
    assert!(validate_amount("").is_err());
    assert!(validate_amount("abc").is_err());
    assert_eq!(validate_amount("42").unwrap(), 42);
}

#[test]
fn validates_calldata_requires_hex() {
    assert!(validate_calldata("deadbeef").is_err());
    assert!(validate_calldata("0xZZ").is_err());
    assert!(validate_calldata("0xabc").is_err());
    assert_eq!(validate_calldata("0xdeadbeef").unwrap(), "0xdeadbeef");
}

#[test]
fn formats_amount_with_decimals() {
    assert_eq!(format_amount(0, 18), "0.000000000000000000");
    assert_eq!(format_amount(1, 8), "0.00000001");
    assert_eq!(format_amount(123_456_789, 8), "1.23456789");
    assert_eq!(format_amount(100, 0), "100");
}

#[test]
fn next_quote_id_is_unique_and_prefixed() {
    let a = next_quote_id();
    let b = next_quote_id();
    assert_ne!(a, b);
    assert!(a.starts_with("q_"));
}

#[test]
fn quote_store_round_trips_and_expires() {
    let _guard = TEST_LOCK.lock();
    reset_quote_store_for_tests();
    let now = now_ms();
    let mut q = PreparedTransaction {
        quote_id: "q_test_1".to_string(),
        kind: PreparedKind::NativeTransfer,
        chain: WalletChain::Evm,
        evm_network: Some(EvmNetwork::EthereumMainnet),
        from_address: "0xfrom".to_string(),
        to_address: "0xto".to_string(),
        asset_symbol: "ETH".to_string(),
        amount_raw: "1".to_string(),
        amount_formatted: "0.000000000000000001".to_string(),
        receive_symbol: None,
        min_receive_raw: None,
        calldata: None,
        token_address: None,
        estimated_fee_raw: "0".to_string(),
        status: PreparedStatus::AwaitingConfirmation,
        created_at_ms: now,
        expires_at_ms: now + 60_000,
        notes: vec![],
        owner: None,
    };
    store_quote(q.clone());
    let taken = take_quote_for("q_test_1", None).expect("quote round-trips");
    assert_eq!(taken.quote_id, "q_test_1");
    assert!(
        take_quote_for("q_test_1", None).is_err(),
        "second take must fail"
    );

    q.quote_id = "q_test_2".to_string();
    q.expires_at_ms = now.saturating_sub(1);
    store_quote(q);
    let err = take_quote_for("q_test_2", None).unwrap_err();
    assert!(err.contains("expired"), "got: {err}");
}

#[tokio::test]
async fn execute_prepared_requires_confirmed_flag() {
    let err = execute_prepared(ExecutePreparedParams {
        quote_id: "missing".to_string(),
        confirmed: false,
    })
    .await
    .unwrap_err();
    assert!(err.contains("confirmed: true"), "got: {err}");
}

#[tokio::test]
async fn supported_assets_lists_default_erc20s_and_l2() {
    let out = supported_assets().await.unwrap();
    assert!(out
        .value
        .iter()
        .any(|asset| asset.symbol == "USDC" && asset.evm_network == Some(EvmNetwork::BaseMainnet)));
    assert!(out
        .value
        .iter()
        .any(|asset| asset.symbol == "ETH" && asset.native));
    assert!(out
        .value
        .iter()
        .any(|asset| asset.symbol == "USDT" && asset.chain == WalletChain::Tron));
}

#[tokio::test]
async fn prepare_transfer_rejects_unknown_asset_symbol() {
    let _guard = TEST_LOCK.lock();
    reset_quote_store_for_tests();
    let temp = TempDir::new().unwrap();
    let _workspace_guard = setup_wallet_in(&temp).await.unwrap();
    let err = prepare_transfer(PrepareTransferParams {
        chain: WalletChain::Evm,
        to_address: "0x1111111111111111111111111111111111111111".into(),
        amount_raw: "1".into(),
        asset_symbol: Some("NOPE".into()),
        evm_network: None,
    })
    .await
    .unwrap_err();
    assert!(err.contains("unsupported asset_symbol"), "got: {err}");
}

#[tokio::test]
async fn balances_fans_evm_account_into_eth_base_bsc_rows() {
    let _guard = TEST_LOCK.lock();
    let temp = TempDir::new().unwrap();
    let _workspace_guard = setup_wallet_in(&temp).await.unwrap();
    // Point all three displayed EVM networks at a mock returning 1e18 wei.
    let (addr, _estimate_calls, _raw_txs) = start_mock_rpc().await.unwrap();
    for var in [
        "OPENHUMAN_WALLET_RPC_EVM",
        "OPENHUMAN_WALLET_RPC_BASE",
        "OPENHUMAN_WALLET_RPC_BSC",
    ] {
        std::env::set_var(var, format!("http://{addr}"));
    }

    let rows = balances().await.unwrap().value;

    // One row per displayed EVM network plus BTC / Solana / Tron.
    let evm_rows: Vec<_> = rows
        .iter()
        .filter(|row| row.chain == WalletChain::Evm)
        .collect();
    assert_eq!(evm_rows.len(), 3, "expected 3 EVM rows, got {evm_rows:?}");
    let networks: Vec<EvmNetwork> = evm_rows.iter().filter_map(|row| row.evm_network).collect();
    assert!(networks.contains(&EvmNetwork::EthereumMainnet));
    assert!(networks.contains(&EvmNetwork::BaseMainnet));
    assert!(networks.contains(&EvmNetwork::BscMainnet));
    // Native symbols differ by network: ETH on Ethereum/Base, BNB on BNB Chain.
    let bnb = evm_rows
        .iter()
        .find(|row| row.evm_network == Some(EvmNetwork::BscMainnet))
        .expect("bsc row present");
    assert_eq!(bnb.asset_symbol, "BNB");
    // Mock RPC returns 1e18 wei for eth_getBalance on every network.
    assert_eq!(bnb.raw, "1000000000000000000");
    assert!(matches!(bnb.provider_status, ProviderStatus::Ready));
    // The non-EVM chains each still produce exactly one row.
    for chain in [WalletChain::Btc, WalletChain::Solana, WalletChain::Tron] {
        assert_eq!(
            rows.iter().filter(|row| row.chain == chain).count(),
            1,
            "expected one row for {chain:?}"
        );
    }

    for var in [
        "OPENHUMAN_WALLET_RPC_EVM",
        "OPENHUMAN_WALLET_RPC_BASE",
        "OPENHUMAN_WALLET_RPC_BSC",
    ] {
        std::env::remove_var(var);
    }
}

#[tokio::test]
async fn tx_status_rejects_empty_hash() {
    let err = tx_status(WalletChain::Evm, None, "   ").await.unwrap_err();
    assert!(err.contains("tx hash is empty"), "got: {err}");
}

#[tokio::test]
async fn execute_prepared_broadcasts_native_evm_transaction() {
    let _guard = TEST_LOCK.lock();
    reset_quote_store_for_tests();
    let temp = TempDir::new().unwrap();
    let _workspace_guard = setup_wallet_in(&temp).await.unwrap();
    let (addr, estimate_calls, raw_txs) = start_mock_rpc().await.unwrap();
    std::env::set_var("OPENHUMAN_WALLET_RPC_EVM", format!("http://{addr}"));

    let prepared = prepare_transfer(PrepareTransferParams {
        chain: WalletChain::Evm,
        to_address: "0x1111111111111111111111111111111111111111".into(),
        amount_raw: "1000".into(),
        asset_symbol: None,
        evm_network: None,
    })
    .await
    .unwrap()
    .value;
    let executed = execute_prepared(ExecutePreparedParams {
        quote_id: prepared.quote_id.clone(),
        confirmed: true,
    })
    .await
    .unwrap()
    .value;

    assert_eq!(executed.status, PreparedStatus::Broadcasted);
    assert!(executed.transaction_hash.starts_with("0xaaaa"));
    assert_eq!(raw_txs.lock().len(), 1);
    let estimate = estimate_calls.lock()[0].clone();
    assert_eq!(
        estimate.get("to").and_then(Value::as_str),
        Some("0x1111111111111111111111111111111111111111")
    );
}

#[tokio::test]
async fn execute_prepared_broadcasts_erc20_transfer_using_default_token_catalog() {
    let _guard = TEST_LOCK.lock();
    reset_quote_store_for_tests();
    let temp = TempDir::new().unwrap();
    let _workspace_guard = setup_wallet_in(&temp).await.unwrap();
    let (addr, estimate_calls, raw_txs) = start_mock_rpc().await.unwrap();
    std::env::set_var("OPENHUMAN_WALLET_RPC_EVM", format!("http://{addr}"));

    let prepared = prepare_transfer(PrepareTransferParams {
        chain: WalletChain::Evm,
        to_address: "0x1111111111111111111111111111111111111111".into(),
        amount_raw: "5000000".into(),
        asset_symbol: Some("USDC".into()),
        evm_network: None,
    })
    .await
    .unwrap()
    .value;
    let executed = execute_prepared(ExecutePreparedParams {
        quote_id: prepared.quote_id.clone(),
        confirmed: true,
    })
    .await
    .unwrap()
    .value;

    assert_eq!(executed.status, PreparedStatus::Broadcasted);
    assert_eq!(raw_txs.lock().len(), 1);
    let estimate = estimate_calls.lock()[0].clone();
    assert_eq!(
        estimate.get("to").and_then(Value::as_str),
        Some("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48")
    );
    let data = estimate
        .get("data")
        .and_then(Value::as_str)
        .expect("token transfer calldata");
    assert!(data.starts_with("0xa9059cbb"));
}

#[tokio::test]
async fn execute_prepared_broadcasts_native_evm_on_base_with_chain_id_8453() {
    let _guard = TEST_LOCK.lock();
    reset_quote_store_for_tests();
    let temp = TempDir::new().unwrap();
    let _workspace_guard = setup_wallet_in(&temp).await.unwrap();
    // Base uses chain_id 8453 = 0x2105.
    let (addr, _estimate_calls, raw_txs) = start_mock_rpc_with_chain_id("0x2105").await.unwrap();
    std::env::set_var("OPENHUMAN_WALLET_RPC_BASE", format!("http://{addr}"));

    let prepared = prepare_transfer(PrepareTransferParams {
        chain: WalletChain::Evm,
        to_address: "0x1111111111111111111111111111111111111111".into(),
        amount_raw: "1000".into(),
        asset_symbol: None,
        evm_network: Some(EvmNetwork::BaseMainnet),
    })
    .await
    .unwrap()
    .value;
    let executed = execute_prepared(ExecutePreparedParams {
        quote_id: prepared.quote_id.clone(),
        confirmed: true,
    })
    .await
    .unwrap()
    .value;
    assert_eq!(executed.status, PreparedStatus::Broadcasted);
    assert_eq!(executed.evm_network, Some(EvmNetwork::BaseMainnet));
    assert_eq!(raw_txs.lock().len(), 1);
}

/// Build a Quote-store fixture pinned to a specific owner.
/// Bypasses prepare_* (which would need full wallet setup + mock RPC) so
/// the owner-gate behavior can be exercised in isolation.
fn insert_owned_quote(quote_id: &str, owner: Option<QuoteOwner>) -> PreparedTransaction {
    let now = now_ms();
    let q = PreparedTransaction {
        quote_id: quote_id.to_string(),
        kind: PreparedKind::NativeTransfer,
        chain: WalletChain::Evm,
        evm_network: Some(EvmNetwork::EthereumMainnet),
        from_address: "0xfrom".to_string(),
        to_address: "0xto".to_string(),
        asset_symbol: "ETH".to_string(),
        amount_raw: "1".to_string(),
        amount_formatted: "0.000000000000000001".to_string(),
        receive_symbol: None,
        min_receive_raw: None,
        calldata: None,
        token_address: None,
        estimated_fee_raw: "0".to_string(),
        status: PreparedStatus::AwaitingConfirmation,
        created_at_ms: now,
        expires_at_ms: now + 60_000,
        notes: vec![],
        owner,
    };
    insert_quote_for_test(q)
}

fn owner_a() -> QuoteOwner {
    QuoteOwner {
        thread_id: "thread-A".into(),
        client_id: "client-A".into(),
    }
}

fn owner_b() -> QuoteOwner {
    QuoteOwner {
        thread_id: "thread-B".into(),
        client_id: "client-B".into(),
    }
}

fn chat_ctx_from(owner: &QuoteOwner) -> crate::openhuman::approval::ApprovalChatContext {
    crate::openhuman::approval::ApprovalChatContext {
        thread_id: owner.thread_id.clone(),
        client_id: owner.client_id.clone(),
    }
}

/// Cross-thread execute must fail. The quote must remain in the store
/// (mismatched caller cannot poison it by consuming on Alice's behalf).
#[tokio::test]
async fn execute_prepared_rejects_cross_owner_execution() {
    use crate::openhuman::approval::APPROVAL_CHAT_CONTEXT;
    let _guard = TEST_LOCK.lock();
    reset_quote_store_for_tests();
    let q = insert_owned_quote("q_xowner_1", Some(owner_a()));

    // Bob (owner_b) tries to execute Alice's quote. The error string is
    // intentionally identical to a not-found lookup.
    let err = APPROVAL_CHAT_CONTEXT
        .scope(
            chat_ctx_from(&owner_b()),
            execute_prepared(ExecutePreparedParams {
                quote_id: q.quote_id.clone(),
                confirmed: true,
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(err, format!("quote '{}' not found", q.quote_id));

    // The quote must still be present and executable by Alice — Bob's
    // failed attempt didn't poison the store.
    let still_present = prepared_quotes_for_test();
    assert!(
        still_present.iter().any(|p| p.quote_id == q.quote_id),
        "quote must survive owner-mismatch attempts"
    );
}

/// Same-thread execute must pass the owner gate (it will fail later in
/// the chain code because there's no mock RPC set up, but the failure
/// must not be the "not found" oracle — that proves we got past the gate).
#[tokio::test]
async fn execute_prepared_allows_same_owner_execution() {
    use crate::openhuman::approval::APPROVAL_CHAT_CONTEXT;
    let _guard = TEST_LOCK.lock();
    reset_quote_store_for_tests();
    let q = insert_owned_quote("q_same_owner_1", Some(owner_a()));

    let result = APPROVAL_CHAT_CONTEXT
        .scope(
            chat_ctx_from(&owner_a()),
            execute_prepared(ExecutePreparedParams {
                quote_id: q.quote_id.clone(),
                confirmed: true,
            }),
        )
        .await;
    // Past the owner gate. Chain code may error (no mock RPC, no wallet
    // setup) — but it must NOT be the "not found" shape we use for the
    // owner-mismatch oracle.
    if let Err(err) = &result {
        assert_ne!(
            err,
            &format!("quote '{}' not found", q.quote_id),
            "same-owner path must not return the owner-mismatch oracle"
        );
    }
}

/// No-context prepare + no-context execute must work. Keeps CLI / direct
/// JSON-RPC flows usable.
#[tokio::test]
async fn execute_prepared_allows_no_context_flows() {
    let _guard = TEST_LOCK.lock();
    reset_quote_store_for_tests();
    let q = insert_owned_quote("q_no_ctx_1", None);

    // No APPROVAL_CHAT_CONTEXT scope — current_owner() returns None on
    // both prepare and execute, so the gate passes.
    let result = execute_prepared(ExecutePreparedParams {
        quote_id: q.quote_id.clone(),
        confirmed: true,
    })
    .await;
    if let Err(err) = &result {
        assert_ne!(
            err,
            &format!("quote '{}' not found", q.quote_id),
            "no-context path must not return the owner-mismatch oracle"
        );
    }
}

/// A quote prepared inside a chat context must NOT be executable from a
/// caller with no context. Prevents privilege-drop into background /
/// triage / cron flows that wouldn't surface UI confirmation.
#[tokio::test]
async fn execute_prepared_rejects_chat_quote_from_no_context_caller() {
    let _guard = TEST_LOCK.lock();
    reset_quote_store_for_tests();
    let q = insert_owned_quote("q_chat_to_bg_1", Some(owner_a()));

    // No scope around execute → caller_owner = None ≠ Some(owner_a).
    let err = execute_prepared(ExecutePreparedParams {
        quote_id: q.quote_id.clone(),
        confirmed: true,
    })
    .await
    .unwrap_err();
    assert_eq!(err, format!("quote '{}' not found", q.quote_id));
}

/// Lock the error-shape invariant: cross-owner reject string MUST be
/// byte-equal to the not-found string. Regressions here would re-open
/// the enumeration-oracle gap.
#[tokio::test]
async fn execute_prepared_owner_mismatch_error_matches_not_found_shape() {
    use crate::openhuman::approval::APPROVAL_CHAT_CONTEXT;
    let _guard = TEST_LOCK.lock();
    reset_quote_store_for_tests();

    // Real (owner-mismatched) quote.
    let real = insert_owned_quote("q_real_1", Some(owner_a()));
    // Reach the mismatched-owner branch.
    let mismatch_err = APPROVAL_CHAT_CONTEXT
        .scope(
            chat_ctx_from(&owner_b()),
            execute_prepared(ExecutePreparedParams {
                quote_id: real.quote_id.clone(),
                confirmed: true,
            }),
        )
        .await
        .unwrap_err();

    // Reach the genuine not-found branch (no quote with this id in store).
    let missing_err = APPROVAL_CHAT_CONTEXT
        .scope(
            chat_ctx_from(&owner_b()),
            execute_prepared(ExecutePreparedParams {
                quote_id: "q_does_not_exist".into(),
                confirmed: true,
            }),
        )
        .await
        .unwrap_err();

    // Both branches surface the exact same template, parameterised only
    // by quote_id. No enumeration oracle.
    assert_eq!(mismatch_err, format!("quote '{}' not found", real.quote_id));
    assert_eq!(missing_err, "quote 'q_does_not_exist' not found");
}

/// Verify that `prepare_transfer` inside an `APPROVAL_CHAT_CONTEXT` scope
/// actually stamps `owner` via the task-local — not just via test helpers.
#[tokio::test]
async fn prepare_stamps_owner_via_task_local() {
    use crate::openhuman::approval::APPROVAL_CHAT_CONTEXT;
    let _guard = TEST_LOCK.lock();
    reset_quote_store_for_tests();
    let temp = TempDir::new().unwrap();
    let _workspace_guard = setup_wallet_in(&temp).await.unwrap();

    let expected = owner_a();
    let ctx = chat_ctx_from(&expected);

    let prepared = APPROVAL_CHAT_CONTEXT
        .scope(
            ctx,
            prepare_transfer(PrepareTransferParams {
                chain: WalletChain::Evm,
                to_address: "0x1111111111111111111111111111111111111111".into(),
                amount_raw: "1000".into(),
                asset_symbol: None,
                evm_network: Some(EvmNetwork::EthereumMainnet),
            }),
        )
        .await
        .unwrap()
        .value;

    assert_eq!(
        prepared.owner,
        Some(expected),
        "prepare_transfer must stamp owner from APPROVAL_CHAT_CONTEXT"
    );
}

#[tokio::test]
async fn execute_prepared_rejects_evm_chain_id_mismatch() {
    let _guard = TEST_LOCK.lock();
    reset_quote_store_for_tests();
    let temp = TempDir::new().unwrap();
    let _workspace_guard = setup_wallet_in(&temp).await.unwrap();
    // Quote says Base; mock reports Ethereum (0x1) — must fail.
    let (addr, _e, _r) = start_mock_rpc_with_chain_id("0x1").await.unwrap();
    std::env::set_var("OPENHUMAN_WALLET_RPC_BASE", format!("http://{addr}"));

    let prepared = prepare_transfer(PrepareTransferParams {
        chain: WalletChain::Evm,
        to_address: "0x1111111111111111111111111111111111111111".into(),
        amount_raw: "1000".into(),
        asset_symbol: None,
        evm_network: Some(EvmNetwork::BaseMainnet),
    })
    .await
    .unwrap()
    .value;
    let err = execute_prepared(ExecutePreparedParams {
        quote_id: prepared.quote_id.clone(),
        confirmed: true,
    })
    .await
    .unwrap_err();
    assert!(err.contains("chain_id mismatch"), "got: {err}");
}
