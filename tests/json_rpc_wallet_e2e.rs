//! JSON-RPC E2E cases for the wallet namespace.

#[path = "support/rpc_e2e_harness.rs"]
mod harness;
use harness::*;

#[tokio::test]
async fn json_rpc_wallet_setup_round_trips_status() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let encrypted_mnemonic = encrypt_test_mnemonic().await;

    let initial_status = post_json_rpc(&rpc_base, 1005, "openhuman.wallet_status", json!({})).await;
    let initial_body = assert_no_jsonrpc_error(&initial_status, "wallet_status_initial");
    let initial_result = initial_body.get("result").unwrap_or(initial_body);
    assert_eq!(
        initial_result.get("configured").and_then(Value::as_bool),
        Some(false),
        "expected wallet to start unconfigured: {initial_result}"
    );

    let setup = post_json_rpc(
        &rpc_base,
        1006,
        "openhuman.wallet_setup",
        json!({
            "consentGranted": true,
            "source": "generated",
            "mnemonicWordCount": 12,
            "encryptedMnemonic": encrypted_mnemonic,
            "accounts": [
                { "chain": "evm", "address": "0x9858EfFD232B4033E47d90003D41EC34EcaEda94", "derivationPath": "m/44'/60'/0'/0/0" },
                { "chain": "btc", "address": "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu", "derivationPath": "m/84'/0'/0'/0/0" },
                { "chain": "solana", "address": "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk", "derivationPath": "m/44'/501'/0'/0'" },
                { "chain": "tron", "address": "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH", "derivationPath": "m/44'/195'/0'/0/0" }
            ]
        }),
    )
    .await;
    let setup_body = assert_no_jsonrpc_error(&setup, "wallet_setup");
    let setup_result = setup_body.get("result").unwrap_or(setup_body);
    assert_eq!(
        setup_result.get("configured").and_then(Value::as_bool),
        Some(true),
        "expected wallet setup to configure the wallet: {setup_result}"
    );
    assert_eq!(
        setup_result
            .get("accounts")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(4),
        "expected four wallet accounts after setup: {setup_result}"
    );

    let persisted_status =
        post_json_rpc(&rpc_base, 1007, "openhuman.wallet_status", json!({})).await;
    let persisted_body = assert_no_jsonrpc_error(&persisted_status, "wallet_status_persisted");
    let persisted_result = persisted_body.get("result").unwrap_or(persisted_body);
    assert_eq!(
        persisted_result.get("configured").and_then(Value::as_bool),
        Some(true),
        "expected configured wallet status after setup: {persisted_result}"
    );
    assert_eq!(
        persisted_result.get("source").and_then(Value::as_str),
        Some("generated"),
        "expected setup source to persist: {persisted_result}"
    );
    assert_eq!(
        persisted_result
            .get("mnemonicWordCount")
            .and_then(Value::as_u64),
        Some(12),
        "expected mnemonicWordCount to persist: {persisted_result}"
    );
    assert_eq!(
        persisted_result
            .get("consentGranted")
            .and_then(Value::as_bool),
        Some(true),
        "expected consentGranted to persist: {persisted_result}"
    );
    assert_eq!(
        persisted_result
            .get("accounts")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(4),
        "expected persisted wallet accounts to remain intact: {persisted_result}"
    );

    mock_join.abort();
    rpc_join.abort();
}

/// #1396 — wallet execution surface: balances/supported_assets/chain_status
/// read tools, prepare_transfer + execute_prepared write boundary.

#[tokio::test]
#[ignore = "drives the loaded wallet module; run alone with an installed tinywallet artifact"]
async fn json_rpc_wallet_execution_surface_round_trips() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let (wallet_rpc_addr, raw_txs) = start_mock_wallet_evm_rpc().await;
    let _evm_provider_guard = EnvVarGuard::set(
        "OPENHUMAN_WALLET_RPC_EVM",
        format!("http://{wallet_rpc_addr}"),
    );
    // The same mock stands in for every other chain, so the run never reaches a
    // public endpoint: chain_status probes each one.
    let mock_endpoint = format!("http://{wallet_rpc_addr}");
    let _other_provider_guards: Vec<EnvVarGuard> = [
        "OPENHUMAN_WALLET_RPC_BASE",
        "OPENHUMAN_WALLET_RPC_ARBITRUM",
        "OPENHUMAN_WALLET_RPC_OPTIMISM",
        "OPENHUMAN_WALLET_RPC_POLYGON",
        "OPENHUMAN_WALLET_RPC_BSC",
        "OPENHUMAN_WALLET_RPC_SOLANA",
        "OPENHUMAN_WALLET_RPC_TRON",
    ]
    .into_iter()
    .map(|name| EnvVarGuard::set(name, &mock_endpoint))
    .collect();
    let _btc_provider_guard = EnvVarGuard::set("OPENHUMAN_WALLET_RPC_BTC", &mock_endpoint);

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let encrypted_mnemonic = encrypt_test_mnemonic().await;

    // Configure wallet (required precondition for balances / prepare_*).
    let setup = post_json_rpc(
        &rpc_base,
        2001,
        "openhuman.wallet_setup",
        json!({
            "consentGranted": true,
            "source": "imported",
            "mnemonicWordCount": 12,
            "encryptedMnemonic": encrypted_mnemonic,
            "accounts": [
                { "chain": "evm", "address": "0x9858EfFD232B4033E47d90003D41EC34EcaEda94", "derivationPath": "m/44'/60'/0'/0/0" },
                { "chain": "btc", "address": "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu", "derivationPath": "m/84'/0'/0'/0/0" },
                { "chain": "solana", "address": "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk", "derivationPath": "m/44'/501'/0'/0'" },
                { "chain": "tron", "address": "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH", "derivationPath": "m/44'/195'/0'/0/0" }
            ]
        }),
    )
    .await;
    assert_no_jsonrpc_error(&setup, "wallet_setup_for_execution");

    // supported_assets: 4 native assets plus the default EVM token catalog.
    let assets = post_json_rpc(
        &rpc_base,
        2002,
        "openhuman.wallet_supported_assets",
        json!({}),
    )
    .await;
    let body = assert_no_jsonrpc_error(&assets, "wallet_supported_assets");
    let result = body.get("result").unwrap_or(body);
    let list = result.as_array().expect("supported_assets array");
    // Pin the actual expected multi-chain catalog (not just a lower bound) so a
    // regression that silently drops a network is caught.
    for expected_evm in [
        "ethereum_mainnet",
        "base_mainnet",
        "arbitrum_one",
        "optimism_mainnet",
        "polygon_mainnet",
        "bsc_mainnet",
    ] {
        assert!(
            list.iter()
                .any(|a| a.get("evmNetwork").and_then(Value::as_str) == Some(expected_evm)),
            "expected {expected_evm} asset row in catalog: {result}"
        );
    }
    for (chain, symbol) in [("btc", "BTC"), ("solana", "SOL"), ("tron", "TRX")] {
        assert!(
            list.iter()
                .any(|a| a.get("chain").and_then(Value::as_str) == Some(chain)
                    && a.get("symbol").and_then(Value::as_str) == Some(symbol)
                    && a.get("native").and_then(Value::as_bool) == Some(true)),
            "expected native {symbol} on {chain}: {result}"
        );
    }
    assert!(
        list.iter().any(
            |asset| asset.get("symbol").and_then(Value::as_str) == Some("ETH")
                && asset.get("native").and_then(Value::as_bool) == Some(true)
        ),
        "expected native ETH asset in catalog: {result}"
    );
    assert!(
        list.iter().any(
            |asset| asset.get("symbol").and_then(Value::as_str) == Some("USDC")
                && asset.get("native").and_then(Value::as_bool) == Some(false)
        ),
        "expected default USDC token in catalog: {result}"
    );

    // chain_status: every chain is configured and its endpoint answers the
    // tip probe, so the provider row is ready and carries no error.
    let cs = post_json_rpc(&rpc_base, 2003, "openhuman.wallet_chain_status", json!({})).await;
    let body = assert_no_jsonrpc_error(&cs, "wallet_chain_status");
    let result = body.get("result").unwrap_or(body);
    let rows = result.as_array().expect("chain_status array");
    // 6 EVM rows (one per L2 / mainnet, incl. BNB Chain) + 3 non-EVM chains.
    assert_eq!(rows.len(), 9);
    assert!(
        rows.iter()
            .all(|r| r.get("providerStatus").and_then(Value::as_str) == Some("ready")),
        "expected providerStatus=ready for configured chain rows: {result}"
    );
    assert!(
        rows.iter().all(|r| r.get("error").is_none()),
        "a healthy row has no error member: {result}"
    );

    // An endpoint that cannot be reached is reported, not assumed ready: point
    // Bitcoin at a port nothing listens on and read the row again.
    let closed_port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port();
    let _dead_btc_guard = EnvVarGuard::set(
        "OPENHUMAN_WALLET_RPC_BTC",
        format!("http://127.0.0.1:{closed_port}"),
    );
    let cs = post_json_rpc(&rpc_base, 20031, "openhuman.wallet_chain_status", json!({})).await;
    let body = assert_no_jsonrpc_error(&cs, "wallet_chain_status_unreachable");
    let result = body.get("result").unwrap_or(body);
    let rows = result.as_array().expect("chain_status array");
    let btc = rows
        .iter()
        .find(|r| r.get("chain").and_then(Value::as_str) == Some("btc"))
        .expect("btc row");
    assert_eq!(
        btc.get("providerStatus").and_then(Value::as_str),
        Some("missing"),
        "an unreachable endpoint is not ready: {btc}"
    );
    assert_eq!(btc.get("configured").and_then(Value::as_bool), Some(true));
    assert!(
        btc.get("error")
            .and_then(Value::as_str)
            .is_some_and(|e| !e.is_empty()),
        "the row carries the probe failure: {btc}"
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r.get("providerStatus").and_then(Value::as_str) == Some("ready"))
            .count(),
        8,
        "only the dead endpoint changed: {result}"
    );

    // balances: one row per native asset. The EVM account fans out into one
    // row per displayed network (Ethereum, Base, BNB Chain), so 3 EVM rows +
    // BTC + Solana + Tron = 6.
    let balances = post_json_rpc(&rpc_base, 2004, "openhuman.wallet_balances", json!({})).await;
    let body = assert_no_jsonrpc_error(&balances, "wallet_balances");
    let result = body.get("result").unwrap_or(body);
    let rows = result.as_array().expect("balances array");
    assert_eq!(rows.len(), 6);
    assert_eq!(
        rows.iter()
            .filter(|r| r.get("chain").and_then(Value::as_str) == Some("evm"))
            .count(),
        3,
        "expected 3 EVM network rows: {result}"
    );
    // Every row reports a raw integer string. Don't require zero — the
    // BTC/Solana/Tron default REST endpoints may have network access in CI
    // and return non-placeholder values for the deterministic test addresses.
    for r in rows {
        let raw = r.get("raw").and_then(Value::as_str).expect("raw present");
        assert!(
            raw.chars().all(|c| c.is_ascii_digit()),
            "raw must be a decimal string, got: {raw}"
        );
    }

    // prepare_transfer + execute_prepared (happy path).
    let prep = post_json_rpc(
        &rpc_base,
        2005,
        "openhuman.wallet_prepare_transfer",
        json!({
            "chain": "evm",
            "toAddress": "0x000000000000000000000000000000000000dEaD",
            "amountRaw": "1000000000000000",
        }),
    )
    .await;
    let body = assert_no_jsonrpc_error(&prep, "wallet_prepare_transfer");
    let result = body.get("result").unwrap_or(body);
    let quote_id = result
        .get("quoteId")
        .and_then(Value::as_str)
        .expect("quoteId present")
        .to_string();
    assert_eq!(
        result.get("status").and_then(Value::as_str),
        Some("awaiting_confirmation"),
    );
    assert_eq!(
        result.get("kind").and_then(Value::as_str),
        Some("native_transfer"),
    );

    // execute_prepared without confirmed=true must fail.
    let bad = post_json_rpc(
        &rpc_base,
        2006,
        "openhuman.wallet_execute_prepared",
        json!({ "quoteId": quote_id, "confirmed": false }),
    )
    .await;
    assert!(
        bad.get("error").is_some(),
        "expected error for unconfirmed execute: {bad}"
    );

    // Confirmed execute signs and broadcasts the prepared transaction.
    let exec = post_json_rpc(
        &rpc_base,
        2007,
        "openhuman.wallet_execute_prepared",
        json!({ "quoteId": quote_id, "confirmed": true }),
    )
    .await;
    let body = assert_no_jsonrpc_error(&exec, "wallet_execute_prepared");
    let result = body.get("result").unwrap_or(body);
    assert_eq!(
        result.get("status").and_then(Value::as_str),
        Some("broadcasted"),
    );
    assert_eq!(
        result.get("transactionHash").and_then(Value::as_str),
        Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    );
    assert_eq!(
        result
            .get("transaction")
            .and_then(|t| t.get("quoteId"))
            .and_then(Value::as_str),
        Some(quote_id.as_str()),
    );
    let sent_raw_count = match raw_txs.lock() {
        Ok(guard) => guard.len(),
        Err(poisoned) => poisoned.into_inner().len(),
    };
    assert_eq!(sent_raw_count, 1, "expected one raw tx broadcast");

    // A second execute on the same quote must fail (quote consumed).
    let dup = post_json_rpc(
        &rpc_base,
        2008,
        "openhuman.wallet_execute_prepared",
        json!({ "quoteId": quote_id, "confirmed": true }),
    )
    .await;
    assert!(
        dup.get("error").is_some(),
        "expected error re-executing consumed quote: {dup}"
    );

    mock_join.abort();
    rpc_join.abort();
}

/// Wallet tx-read surface (tx_status / tx_receipt / lookup_tx) plus the web3
/// surface gates (routes/quote require auth; same-chain bridge + unsignable
/// chain are rejected before any network call).

#[tokio::test]
async fn json_rpc_wallet_tx_reads_and_web3_gates_round_trip() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let (wallet_rpc_addr, _raw_txs) = start_mock_wallet_evm_rpc().await;
    let _evm_provider_guard = EnvVarGuard::set(
        "OPENHUMAN_WALLET_RPC_EVM",
        format!("http://{wallet_rpc_addr}"),
    );

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // tx_status against the mock EVM node → confirmed with derived confirmations.
    let status = post_json_rpc(
        &rpc_base,
        2101,
        "openhuman.wallet_tx_status",
        json!({ "chain": "evm", "hash": "0xdeadbeef" }),
    )
    .await;
    let body = assert_no_jsonrpc_error(&status, "wallet_tx_status");
    let result = body.get("result").unwrap_or(body);
    assert_eq!(
        result.get("state").and_then(Value::as_str),
        Some("confirmed")
    );

    // tx_receipt extracts gasUsed + computed fee.
    let receipt = post_json_rpc(
        &rpc_base,
        2102,
        "openhuman.wallet_tx_receipt",
        json!({ "chain": "evm", "hash": "0xdeadbeef" }),
    )
    .await;
    let body = assert_no_jsonrpc_error(&receipt, "wallet_tx_receipt");
    let result = body.get("result").unwrap_or(body);
    assert_eq!(result.get("success").and_then(Value::as_bool), Some(true));
    assert_eq!(result.get("gasUsed").and_then(Value::as_str), Some("21000"));

    // lookup_tx reports found.
    let lookup = post_json_rpc(
        &rpc_base,
        2103,
        "openhuman.wallet_lookup_tx",
        json!({ "chain": "evm", "hash": "0xdeadbeef" }),
    )
    .await;
    let body = assert_no_jsonrpc_error(&lookup, "wallet_lookup_tx");
    let result = body.get("result").unwrap_or(body);
    assert_eq!(result.get("found").and_then(Value::as_bool), Some(true));

    // web3_bridge rejects same-chain requests. This gate runs *before* any
    // auth / backend call, so no session setup is needed — assert the
    // gate-specific message (not just any error) to rule out auth false-positives.
    let same_chain = post_json_rpc(
        &rpc_base,
        2104,
        "openhuman.web3_bridge_quote",
        json!({
            "srcChainId": 1, "srcChainTokenIn": "0x0", "srcChainTokenInAmount": "1",
            "dstChainId": 1, "dstChainTokenOut": "0x1"
        }),
    )
    .await;
    let same_chain_msg = same_chain
        .get("error")
        .and_then(|e| e.get("message").or_else(|| e.get("data")))
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        same_chain_msg.contains("different source and destination"),
        "expected same-chain bridge gate rejection, got: {same_chain}"
    );

    // web3_swap rejects a chain id the wallet can't sign for — also a pre-auth gate.
    let unsignable = post_json_rpc(
        &rpc_base,
        2105,
        "openhuman.web3_swap_quote",
        json!({
            "chainId": 999999, "tokenIn": "0x0", "tokenInAmount": "1", "tokenOut": "0x1"
        }),
    )
    .await;
    let unsignable_msg = unsignable
        .get("error")
        .and_then(|e| e.get("message").or_else(|| e.get("data")))
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        unsignable_msg.contains("not signable"),
        "expected unsignable-chain swap gate rejection, got: {unsignable}"
    );

    mock_join.abort();
    rpc_join.abort();
}

// ---------------------------------------------------------------------------
// Wallet host seam E2E coverage for chain selection and prepared execution.
// ---------------------------------------------------------------------------

const E2E_TEST_MNEMONIC_ADDRS_EVM: &str = "0x9858EfFD232B4033E47d90003D41EC34EcaEda94";
const E2E_TEST_MNEMONIC_ADDRS_BTC: &str = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu";
const E2E_TEST_MNEMONIC_ADDRS_SOL: &str = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk";
const E2E_TEST_MNEMONIC_ADDRS_TRON: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";

fn wallet_setup_accounts_value() -> Value {
    json!([
        { "chain": "evm", "address": E2E_TEST_MNEMONIC_ADDRS_EVM, "derivationPath": "m/44'/60'/0'/0/0" },
        { "chain": "btc", "address": E2E_TEST_MNEMONIC_ADDRS_BTC, "derivationPath": "m/84'/0'/0'/0/0" },
        { "chain": "solana", "address": E2E_TEST_MNEMONIC_ADDRS_SOL, "derivationPath": "m/44'/501'/0'/0'" },
        { "chain": "tron", "address": E2E_TEST_MNEMONIC_ADDRS_TRON, "derivationPath": "m/44'/195'/0'/0/0" }
    ])
}

#[derive(Clone)]
struct MockBaseRpcState {
    raw_txs: Arc<Mutex<Vec<String>>>,
    chain_id_hex: String,
}

async fn mock_evm_chain_rpc(
    State(state): State<MockBaseRpcState>,
    Json(payload): Json<Value>,
) -> Json<Value> {
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
        "eth_chainId" => Value::String(state.chain_id_hex.clone()),
        "eth_getTransactionCount" => Value::String("0x7".to_string()),
        "eth_gasPrice" => Value::String("0x3b9aca00".to_string()),
        "eth_estimateGas" => Value::String("0x5208".to_string()),
        "eth_sendRawTransaction" => {
            if let Some(raw) = params.first().and_then(Value::as_str) {
                match state.raw_txs.lock() {
                    Ok(mut guard) => guard.push(raw.to_string()),
                    Err(p) => p.into_inner().push(raw.to_string()),
                }
            }
            Value::String(
                "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            )
        }
        _ => Value::Null,
    };
    Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
}

async fn start_mock_evm_with_chain_id(chain_id_hex: &str) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
    let raw_txs = Arc::new(Mutex::new(Vec::new()));
    let state = MockBaseRpcState {
        raw_txs: raw_txs.clone(),
        chain_id_hex: chain_id_hex.to_string(),
    };
    let app = Router::new()
        .route("/", post(mock_evm_chain_rpc))
        .with_state(state);
    let (addr, _join) = serve_on_ephemeral(app).await;
    (addr, raw_txs)
}

async fn wallet_setup_via_rpc(rpc_base: &str, encrypted_mnemonic: &str) {
    let setup = post_json_rpc(
        rpc_base,
        9001,
        "openhuman.wallet_setup",
        json!({
            "consentGranted": true,
            "source": "imported",
            "mnemonicWordCount": 12,
            "encryptedMnemonic": encrypted_mnemonic,
            "accounts": wallet_setup_accounts_value(),
        }),
    )
    .await;
    assert_no_jsonrpc_error(&setup, "wallet_setup_for_chain_e2e");
}

/// EVM L2 selection: Base mainnet (chain_id 8453 = 0x2105). Verifies
/// `evmNetwork: base_mainnet` routes signing + broadcast to the Base RPC
/// override and *not* the Ethereum default.

#[tokio::test]
#[ignore = "drives the loaded wallet module; run alone with an installed tinywallet artifact"]
async fn json_rpc_wallet_evm_base_network_prepare_execute_round_trips() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _evm_guard = EnvVarGuard::unset("OPENHUMAN_WALLET_RPC_EVM");
    let _base_guard = EnvVarGuard::unset("OPENHUMAN_WALLET_RPC_BASE");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let encrypted_mnemonic = encrypt_test_mnemonic().await;

    // Mock Base RPC at 0x2105 = 8453.
    let (base_rpc_addr, base_raw_txs) = start_mock_evm_with_chain_id("0x2105").await;
    std::env::set_var(
        "OPENHUMAN_WALLET_RPC_BASE",
        format!("http://{base_rpc_addr}"),
    );

    wallet_setup_via_rpc(&rpc_base, &encrypted_mnemonic).await;

    let prep = post_json_rpc(
        &rpc_base,
        9101,
        "openhuman.wallet_prepare_transfer",
        json!({
            "chain": "evm",
            "evmNetwork": "base_mainnet",
            "toAddress": "0x1111111111111111111111111111111111111111",
            "amountRaw": "1000",
        }),
    )
    .await;
    let prep_body = assert_no_jsonrpc_error(&prep, "wallet_prepare_transfer_base");
    let prep_result = prep_body.get("result").unwrap_or(prep_body);
    assert_eq!(
        prep_result.get("evmNetwork").and_then(Value::as_str),
        Some("base_mainnet"),
    );
    let quote_id = prep_result
        .get("quoteId")
        .and_then(Value::as_str)
        .expect("quoteId")
        .to_string();

    let exec = post_json_rpc(
        &rpc_base,
        9102,
        "openhuman.wallet_execute_prepared",
        json!({"quoteId": quote_id, "confirmed": true}),
    )
    .await;
    let exec_body = assert_no_jsonrpc_error(&exec, "wallet_execute_prepared_base");
    let exec_result = exec_body.get("result").unwrap_or(exec_body);
    assert_eq!(
        exec_result.get("status").and_then(Value::as_str),
        Some("broadcasted"),
    );
    assert_eq!(
        exec_result.get("evmNetwork").and_then(Value::as_str),
        Some("base_mainnet"),
    );
    let raw_count = match base_raw_txs.lock() {
        Ok(g) => g.len(),
        Err(p) => p.into_inner().len(),
    };
    assert_eq!(raw_count, 1, "expected one raw tx broadcast on Base RPC");

    mock_join.abort();
    rpc_join.abort();
}

/// Wallet network_defaults returns non-empty network rows with chain identifiers.

#[tokio::test]
async fn json_rpc_wallet_network_defaults_returns_network_rows() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let resp = post_json_rpc(
        &rpc_base,
        9601,
        "openhuman.wallet_network_defaults",
        json!({}),
    )
    .await;
    let body = assert_no_jsonrpc_error(&resp, "wallet_network_defaults");
    let result = body.get("result").unwrap_or(body);
    let rows = result.as_array().expect("array");
    assert!(
        !rows.is_empty(),
        "wallet defaults must include at least one network"
    );
    for row in rows {
        assert!(
            row.is_object(),
            "each network default must be an object: {row}"
        );
        assert!(
            row.get("chain")
                .and_then(Value::as_str)
                .is_some_and(|chain| !chain.is_empty()),
            "each network default must expose a chain identifier: {row}"
        );
        if row.get("evmNetwork").is_some() {
            assert!(
                row.get("chainId").and_then(Value::as_u64).is_some(),
                "EVM rows must expose a numeric chain id: {row}"
            );
        }
    }

    mock_join.abort();
    rpc_join.abort();
}
