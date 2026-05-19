use super::*;
use crate::openhuman::config::Config;
use crate::openhuman::config::{PolymarketClobCredentials, PolymarketConfig};
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};
use crate::openhuman::wallet::{
    self, WalletAccount, WalletChain, WalletSetupParams, WalletSetupSource,
};
use ethers_signers::{coins_bip39::English, MnemonicBuilder, Signer};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{sleep, Duration};

#[derive(Clone)]
struct MockResponse {
    status: u16,
    body: String,
    delay_ms: u64,
}

impl MockResponse {
    fn json(status: u16, fixture_name: &str) -> Self {
        Self {
            status,
            body: fixture(fixture_name),
            delay_ms: 0,
        }
    }

    fn body(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_string(),
            delay_ms: 0,
        }
    }

    fn with_delay(mut self, delay_ms: u64) -> Self {
        self.delay_ms = delay_ms;
        self
    }
}

#[derive(Clone, Debug, Default)]
struct ObservedRequest {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: String,
}

fn fixture(name: &str) -> String {
    let root = env!("CARGO_MANIFEST_DIR");
    let path = format!("{root}/tests/fixtures/polymarket/{name}.json");
    std::fs::read_to_string(path).expect("fixture must exist")
}

fn test_security() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        ..SecurityPolicy::default()
    })
}

fn test_tool(gamma_base_url: String, clob_base_url: String, timeout_secs: u64) -> PolymarketTool {
    let config = PolymarketConfig {
        enabled: true,
        gamma_base_url,
        clob_base_url,
        timeout_secs,
        ..PolymarketConfig::default()
    };

    PolymarketTool::new(&config, test_security())
}

fn authed_tool(clob_base_url: String, user: &str) -> PolymarketTool {
    let config = PolymarketConfig {
        enabled: true,
        gamma_base_url: clob_base_url.clone(),
        clob_base_url,
        timeout_secs: 15,
        eoa_address: Some(user.to_string()),
        derived_clob_credentials: Some(PolymarketClobCredentials {
            api_key: "test-key".to_string(),
            secret: "dGVzdC1zZWNyZXQ=".to_string(),
            passphrase: "test-passphrase".to_string(),
        }),
        ..PolymarketConfig::default()
    };

    PolymarketTool::new(&config, test_security())
}

fn route(key: &str, responses: Vec<MockResponse>) -> HashMap<String, Vec<MockResponse>> {
    let mut routes = HashMap::new();
    routes.insert(key.to_string(), responses);
    routes
}

async fn start_mock_server(
    routes: HashMap<String, Vec<MockResponse>>,
) -> (String, Arc<AtomicUsize>) {
    let (base, calls, _captured) = start_mock_server_with_capture(routes).await;
    (base, calls)
}

async fn start_mock_server_with_capture(
    routes: HashMap<String, Vec<MockResponse>>,
) -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<ObservedRequest>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));

    let queues: HashMap<String, VecDeque<MockResponse>> = routes
        .into_iter()
        .map(|(path, responses)| (path, responses.into_iter().collect::<VecDeque<_>>()))
        .collect();

    let shared_routes = Arc::new(Mutex::new(queues));
    let shared_calls = Arc::clone(&calls);
    let shared_captured = Arc::clone(&captured);

    tokio::spawn(async move {
        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => return,
            };

            let routes = Arc::clone(&shared_routes);
            let calls = Arc::clone(&shared_calls);
            let captured = Arc::clone(&shared_captured);

            tokio::spawn(async move {
                let mut buf = Vec::with_capacity(32 * 1024);
                let mut chunk = [0_u8; 4096];
                loop {
                    let n = match socket.read(&mut chunk).await {
                        Ok(read) => read,
                        Err(_) => return,
                    };
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if request_is_complete(&buf) {
                        break;
                    }
                }
                if buf.is_empty() {
                    return;
                }

                let request_raw = String::from_utf8_lossy(&buf).to_string();
                let observed = parse_request(&request_raw);
                let target = observed.target.clone();

                {
                    let mut guard = captured.lock().unwrap();
                    guard.push(observed);
                }

                calls.fetch_add(1, Ordering::Relaxed);

                let response = {
                    let mut guard = routes.lock().unwrap();
                    pop_response(&mut guard, &target)
                };

                if response.delay_ms > 0 {
                    sleep(Duration::from_millis(response.delay_ms)).await;
                }

                let reason = reason_phrase(response.status);
                let payload = response.body;
                let wire = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.status,
                    reason,
                    payload.len(),
                    payload
                );

                let _ = socket.write_all(wire.as_bytes()).await;
            });
        }
    });

    (format!("http://127.0.0.1:{}", addr.port()), calls, captured)
}

fn request_is_complete(buf: &[u8]) -> bool {
    let raw = String::from_utf8_lossy(buf);
    let Some((head, body)) = raw.split_once("\r\n\r\n") else {
        return false;
    };

    let content_length = head
        .lines()
        .find_map(|line| {
            let (k, v) = line.split_once(':')?;
            if k.trim().eq_ignore_ascii_case("content-length") {
                v.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);

    body.as_bytes().len() >= content_length
}

fn parse_request(raw: &str) -> ObservedRequest {
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
    let mut lines = head.lines();
    let first_line = lines.next().unwrap_or_default();

    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or("/").to_string();

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    ObservedRequest {
        method,
        target,
        headers,
        body: body.to_string(),
    }
}

fn pop_response(
    routes: &mut HashMap<String, VecDeque<MockResponse>>,
    target: &str,
) -> MockResponse {
    if let Some(response) = pop_from_queue(routes.get_mut(target)) {
        return response;
    }

    let path_only = target.split('?').next().unwrap_or(target);
    if let Some(response) = pop_from_queue(routes.get_mut(path_only)) {
        return response;
    }

    MockResponse {
        status: 404,
        body: r#"{"error":"not found"}"#.to_string(),
        delay_ms: 0,
    }
}

fn pop_from_queue(queue: Option<&mut VecDeque<MockResponse>>) -> Option<MockResponse> {
    let queue = queue?;
    if queue.len() <= 1 {
        return queue.front().cloned();
    }
    queue.pop_front()
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Error",
    }
}

fn parse_tool_output(result: &ToolResult) -> Value {
    serde_json::from_str::<Value>(&result.output()).expect("tool output should be valid json")
}

fn header<'a>(request: &'a ObservedRequest, key: &str) -> Option<&'a str> {
    request
        .headers
        .get(&key.to_ascii_lowercase())
        .map(|value| value.as_str())
}

struct EnvVarGuard {
    key: &'static str,
    prev: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &std::path::Path) -> Self {
        let prev = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, path);
        }
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

fn test_wallet_accounts(evm_address: &str, evm_derivation_path: &str) -> Vec<WalletAccount> {
    vec![
        WalletAccount {
            chain: WalletChain::Evm,
            address: evm_address.to_string(),
            derivation_path: evm_derivation_path.to_string(),
        },
        WalletAccount {
            chain: WalletChain::Btc,
            address: "btc-test-address".to_string(),
            derivation_path: "m/84'/0'/0'/0/0".to_string(),
        },
        WalletAccount {
            chain: WalletChain::Solana,
            address: "solana-test-address".to_string(),
            derivation_path: "m/44'/501'/0'/0'".to_string(),
        },
        WalletAccount {
            chain: WalletChain::Tron,
            address: "tron-test-address".to_string(),
            derivation_path: "m/44'/195'/0'/0/0".to_string(),
        },
    ]
}

async fn configure_wallet_for_place_order_test(
    workspace_root: &std::path::Path,
) -> Result<String, String> {
    let mut config = Config::default();
    config.config_path = workspace_root.join("config.toml");
    config.workspace_dir = workspace_root.join("workspace");
    config
        .save()
        .await
        .map_err(|e| format!("failed to save test config: {e}"))?;

    let mnemonic = "test test test test test test test test test test test junk";
    let evm_derivation_path = "m/44'/60'/0'/0/0";
    let evm_wallet = MnemonicBuilder::<English>::default()
        .phrase(mnemonic)
        .derivation_path(evm_derivation_path)
        .map_err(|e| format!("failed to set derivation path: {e}"))?
        .build()
        .map_err(|e| format!("failed to derive EVM wallet: {e}"))?;
    let evm_address = format!("{:#x}", evm_wallet.address());

    let encrypted_mnemonic = crate::openhuman::encryption::rpc::encrypt_secret(&config, mnemonic)
        .await
        .map_err(|e| format!("failed to encrypt mnemonic: {e}"))?
        .value;

    wallet::setup(WalletSetupParams {
        consent_granted: true,
        source: WalletSetupSource::Imported,
        mnemonic_word_count: 12,
        encrypted_mnemonic: Some(encrypted_mnemonic),
        accounts: test_wallet_accounts(&evm_address, evm_derivation_path),
    })
    .await
    .map_err(|e| format!("failed to configure wallet state: {e}"))?;

    Ok(evm_address)
}

#[tokio::test]
async fn list_markets_happy_path() {
    let (gamma_base, _) = start_mock_server(route(
        "/markets?limit=2&offset=0&active=true",
        vec![MockResponse::json(200, "markets_list")],
    ))
    .await;

    let tool = test_tool(gamma_base.clone(), gamma_base, 15);
    let result = tool
        .execute(json!({
            "action": "list_markets",
            "limit": 2,
            "offset": 0,
            "active": true
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "list_markets");
    assert!(output["data"].is_array());
    assert_eq!(output["data"][0]["slug"], "will-eth-hit-10k");
}

#[tokio::test]
async fn get_market_by_id_happy_path() {
    let (gamma_base, _) = start_mock_server(route(
        "/markets/12345",
        vec![MockResponse::json(200, "market_by_id")],
    ))
    .await;

    let tool = test_tool(gamma_base.clone(), gamma_base, 15);
    let result = tool
        .execute(json!({
            "action": "get_market",
            "market_id": "12345"
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "get_market");
    assert_eq!(output["lookup"], "market_id");
    assert_eq!(output["data"]["id"], "12345");
}

#[tokio::test]
async fn get_market_by_slug_happy_path() {
    let (gamma_base, _) = start_mock_server(route(
        "/markets?slug=will-eth-hit-10k",
        vec![MockResponse::json(200, "market_by_slug")],
    ))
    .await;

    let tool = test_tool(gamma_base.clone(), gamma_base, 15);
    let result = tool
        .execute(json!({
            "action": "get_market",
            "slug": "will-eth-hit-10k"
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["lookup"], "slug");
    assert_eq!(output["data"]["id"], "12345");
    assert_eq!(output["data"]["slug"], "will-eth-hit-10k");
}

#[tokio::test]
async fn list_events_happy_path() {
    let (gamma_base, _) = start_mock_server(route(
        "/events?limit=2",
        vec![MockResponse::json(200, "events_list")],
    ))
    .await;

    let tool = test_tool(gamma_base.clone(), gamma_base, 15);
    let result = tool
        .execute(json!({
            "action": "list_events",
            "limit": 2
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "list_events");
    assert!(output["data"].is_array());
    assert_eq!(output["data"][0]["id"], "event-1");
}

#[tokio::test]
async fn get_orderbook_happy_path() {
    let (clob_base, _) = start_mock_server(route(
        "/book?token_id=1001",
        vec![MockResponse::json(200, "orderbook")],
    ))
    .await;

    let tool = test_tool(clob_base.clone(), clob_base, 15);
    let result = tool
        .execute(json!({
            "action": "get_orderbook",
            "token_id": "1001"
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "get_orderbook");
    assert_eq!(output["data"]["token_id"], "1001");
}

#[tokio::test]
async fn get_price_happy_path() {
    let (clob_base, _) = start_mock_server(route(
        "/price?token_id=1001&side=buy",
        vec![MockResponse::json(200, "price")],
    ))
    .await;

    let tool = test_tool(clob_base.clone(), clob_base, 15);
    let result = tool
        .execute(json!({
            "action": "get_price",
            "token_id": "1001",
            "side": "buy"
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "get_price");
    assert_eq!(output["data"]["price"], "0.47");
}

#[tokio::test]
async fn get_positions_happy_path_signs_l2_headers() {
    let user = "0x1111111111111111111111111111111111111111";
    let (clob_base, _calls, captured) = start_mock_server_with_capture(route(
        "/data/positions?user=0x1111111111111111111111111111111111111111",
        vec![MockResponse::body(
            200,
            r#"{"positions":[{"token_id":"1001","size":"5"}]}"#,
        )],
    ))
    .await;

    let tool = authed_tool(clob_base, user);
    let result = tool
        .execute(json!({
            "action": "get_positions",
            "user": user
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "get_positions");
    assert_eq!(output["user"], user.to_ascii_lowercase());

    let requests = captured.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.target,
        "/data/positions?user=0x1111111111111111111111111111111111111111"
    );
    assert_eq!(header(request, "poly_api_key"), Some("test-key"));
    assert_eq!(header(request, "poly_passphrase"), Some("test-passphrase"));
    assert_eq!(header(request, "poly_nonce"), Some("0"));
    assert_eq!(header(request, "poly_address"), Some(user));
    assert!(header(request, "poly_signature")
        .map(|sig| !sig.trim().is_empty())
        .unwrap_or(false));
}

#[tokio::test]
async fn get_balance_happy_path_defaults_to_usdce() {
    let user = "0x2222222222222222222222222222222222222222";
    let (clob_base, _calls, captured) = start_mock_server_with_capture(route(
        "/data/balance?user=0x2222222222222222222222222222222222222222&token=usdce",
        vec![MockResponse::body(200, r#"{"balance":"4200000"}"#)],
    ))
    .await;

    let tool = authed_tool(clob_base, user);
    let result = tool
        .execute(json!({
            "action": "get_balance",
            "user": user
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "get_balance");
    assert_eq!(output["data"]["balance"], "4200000");

    let requests = captured.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.target,
        "/data/balance?user=0x2222222222222222222222222222222222222222&token=usdce"
    );
    assert_eq!(header(request, "poly_api_key"), Some("test-key"));
}

#[tokio::test]
async fn get_open_orders_happy_path_authenticated() {
    let user = "0x3333333333333333333333333333333333333333";
    let (clob_base, _calls, captured) = start_mock_server_with_capture(route(
        "/orders?user=0x3333333333333333333333333333333333333333",
        vec![MockResponse::body(200, r#"{"data":[{"id":"ord-1"}]}"#)],
    ))
    .await;

    let tool = authed_tool(clob_base, user);
    let result = tool
        .execute(json!({
            "action": "get_open_orders",
            "user": user
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "get_open_orders");
    assert_eq!(output["data"]["data"][0]["id"], "ord-1");

    let requests = captured.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.target,
        "/orders?user=0x3333333333333333333333333333333333333333"
    );
    assert_eq!(header(request, "poly_api_key"), Some("test-key"));
}

#[tokio::test]
async fn get_usdc_allowance_happy_path_uses_polygon_eth_call() {
    let user = "0x4444444444444444444444444444444444444444";
    let (rpc_base, _calls, captured) = start_mock_server_with_capture(route(
        "/",
        vec![MockResponse::body(
            200,
            r#"{"jsonrpc":"2.0","id":1,"result":"0x00000000000000000000000000000000000000000000000000000000000f4240"}"#,
        )],
    ))
    .await;

    let config = PolymarketConfig {
        enabled: true,
        gamma_base_url: rpc_base.clone(),
        clob_base_url: rpc_base.clone(),
        polygon_rpc_url: rpc_base,
        timeout_secs: 15,
        ..PolymarketConfig::default()
    };
    let tool = PolymarketTool::new(&config, test_security());

    let result = tool
        .execute(json!({
            "action": "get_usdc_allowance",
            "user": user
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "get_usdc_allowance");
    assert_eq!(output["allowance"], "1000000");

    let requests = captured.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/");
    assert!(request.body.contains("\"method\":\"eth_call\""));
    assert!(request.body.contains("dd62ed3e"));
    // Strict JSON-RPC providers (Alchemy/Infura) reject calls without an
    // explicit Content-Type header — this is the regression guard for
    // graycyrus comment 3265708296.
    assert_eq!(
        header(request, "content-type"),
        Some("application/json"),
        "Polygon RPC eth_call must declare application/json Content-Type"
    );
}

#[tokio::test]
async fn place_order_requires_approval_and_does_not_issue_http() {
    let (clob_base, calls) = start_mock_server(route(
        "/order",
        vec![MockResponse::body(200, r#"{"ok":true}"#)],
    ))
    .await;

    let tool = test_tool(clob_base.clone(), clob_base, 15);
    let result = tool
        .execute(json!({
            "action": "place_order",
            "user": "0x1111111111111111111111111111111111111111",
            "side": "BUY",
            "token_id": "1001",
            "price": 0.5,
            "size": 10
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("requires explicit user approval"));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn place_order_blocked_by_readonly_security_policy_before_approval_check() {
    let (clob_base, calls) = start_mock_server(route(
        "/order",
        vec![MockResponse::body(200, r#"{"ok":true}"#)],
    ))
    .await;

    let readonly_security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let config = PolymarketConfig {
        enabled: true,
        gamma_base_url: clob_base.clone(),
        clob_base_url: clob_base,
        timeout_secs: 15,
        ..PolymarketConfig::default()
    };
    let tool = PolymarketTool::new(&config, readonly_security);

    let result = tool
        .execute(json!({
            "action": "place_order",
            "approved": true,
            "user": "0x1111111111111111111111111111111111111111",
            "side": "BUY",
            "token_id": "1001",
            "price": 0.5,
            "size": 10
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(
        result.output().contains("read-only mode"),
        "expected security-policy block, got: {}",
        result.output()
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn place_order_happy_path_posts_signed_order() {
    use crate::openhuman::config::TEST_ENV_LOCK;

    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", tmp.path());

    let user = configure_wallet_for_place_order_test(tmp.path())
        .await
        .expect("wallet setup");

    let mut routes = HashMap::new();
    routes.insert(
        format!("/nonce?user={user}"),
        vec![MockResponse::body(200, r#"{"nonce": 42}"#)],
    );
    routes.insert(
        "/order".to_string(),
        vec![MockResponse::body(
            200,
            r#"{"success":true,"orderID":"ord-test-1"}"#,
        )],
    );

    let (clob_base, _calls, captured) = start_mock_server_with_capture(routes).await;
    let config = PolymarketConfig {
        enabled: true,
        gamma_base_url: clob_base.clone(),
        clob_base_url: clob_base,
        timeout_secs: 15,
        eoa_address: Some(user.clone()),
        derived_clob_credentials: Some(PolymarketClobCredentials {
            api_key: "test-key".to_string(),
            secret: "dGVzdC1zZWNyZXQ=".to_string(),
            passphrase: "test-passphrase".to_string(),
        }),
        ..PolymarketConfig::default()
    };
    let tool = PolymarketTool::new(&config, test_security());

    let result = tool
        .execute(json!({
            "action": "place_order",
            "user": user.clone(),
            "side": "BUY",
            "token_id": "1001",
            "price": 0.5,
            "size": 10.0,
            "time_in_force": "GTC",
            "approved": true
        }))
        .await
        .unwrap();

    assert!(
        !result.is_error,
        "expected success, got: {}",
        result.output()
    );
    let output = parse_tool_output(&result);
    assert_eq!(output["action"], "place_order");
    assert_eq!(output["time_in_force"], "GTC");
    assert_eq!(output["data"]["success"], true);
    assert_eq!(output["data"]["orderID"], "ord-test-1");

    let requests = captured.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);

    let nonce_request = &requests[0];
    assert_eq!(nonce_request.method, "GET");
    assert_eq!(
        nonce_request.target,
        format!(
            "/nonce?user={}",
            output["user"].as_str().expect("user should be a string")
        )
    );
    assert_eq!(header(nonce_request, "poly_api_key"), Some("test-key"));
    assert_eq!(
        header(nonce_request, "poly_passphrase"),
        Some("test-passphrase")
    );
    assert_eq!(
        header(nonce_request, "poly_address"),
        output["user"].as_str()
    );

    let order_request = &requests[1];
    assert_eq!(order_request.method, "POST");
    assert_eq!(order_request.target, "/order");
    assert_eq!(header(order_request, "poly_api_key"), Some("test-key"));
    assert_eq!(
        header(order_request, "poly_passphrase"),
        Some("test-passphrase")
    );
    assert_eq!(
        header(order_request, "content-type"),
        Some("application/json")
    );
    assert!(header(order_request, "poly_signature")
        .map(|sig| !sig.is_empty())
        .unwrap_or(false));

    let posted: Value =
        serde_json::from_str(&order_request.body).expect("post body should be valid JSON");
    assert_eq!(posted["owner"], output["user"]);
    assert_eq!(posted["orderType"], "limit");
    assert_eq!(posted["timeInForce"], "GTC");
    assert_eq!(posted["order"]["maker"], output["user"]);
    assert_eq!(posted["order"]["signer"], output["user"]);
    assert_eq!(posted["order"]["tokenId"], "1001");
    assert_eq!(posted["order"]["makerAmount"], "5000000");
    assert_eq!(posted["order"]["takerAmount"], "10000000");
    assert_eq!(posted["order"]["nonce"], "42");
    assert_eq!(posted["order"]["side"], "BUY");
    assert_eq!(posted["order"]["signatureType"], 0);
    assert!(posted["order"]["signature"]
        .as_str()
        .map(|sig| sig.starts_with("0x") && sig.len() > 10)
        .unwrap_or(false));
}

#[tokio::test]
async fn cancel_order_requires_approval_and_does_not_issue_http() {
    let (clob_base, calls) = start_mock_server(route(
        "/order/ord-1",
        vec![MockResponse::body(200, r#"{"ok":true}"#)],
    ))
    .await;

    let tool = test_tool(clob_base.clone(), clob_base, 15);
    let result = tool
        .execute(json!({
            "action": "cancel_order",
            "order_id": "ord-1",
            "user": "0x1111111111111111111111111111111111111111"
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("requires explicit user approval"));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn client_error_4xx_returns_error_not_retried() {
    let (clob_base, calls) = start_mock_server(route(
        "/book?token_id=bad-token",
        vec![MockResponse::json(400, "error_client")],
    ))
    .await;

    let tool = test_tool(clob_base.clone(), clob_base, 15);
    let result = tool
        .execute(json!({
            "action": "get_orderbook",
            "token_id": "bad-token"
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("client error 400"));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn server_error_5xx_returns_transient_error() {
    let (clob_base, calls) = start_mock_server(route(
        "/price?token_id=1001&side=sell",
        vec![
            MockResponse::json(500, "error_server"),
            MockResponse::json(500, "error_server"),
            MockResponse::json(500, "error_server"),
        ],
    ))
    .await;

    let tool = test_tool(clob_base.clone(), clob_base, 15);
    let result = tool
        .execute(json!({
            "action": "get_price",
            "token_id": "1001",
            "side": "sell"
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("transient server error 500"));
    assert_eq!(calls.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn timeout_returns_deadline_error() {
    let (gamma_base, _) = start_mock_server(route(
        "/markets?limit=1",
        vec![MockResponse::json(200, "markets_list").with_delay(1_500)],
    ))
    .await;

    let tool = test_tool(gamma_base.clone(), gamma_base, 1);
    let result = tool
        .execute(json!({
            "action": "list_markets",
            "limit": 1
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("timed out"));
}

#[test]
fn parameters_schema_deserializes_for_all_actions() {
    let config = PolymarketConfig::default();
    let tool = PolymarketTool::new(&config, test_security());

    let schema = tool.parameters_schema();
    let actions = schema["properties"]["action"]["enum"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    for expected in [
        "list_markets",
        "get_market",
        "list_events",
        "get_orderbook",
        "get_price",
        "get_positions",
        "get_balance",
        "get_open_orders",
        "get_usdc_allowance",
        "place_order",
        "cancel_order",
    ] {
        assert!(
            actions.contains(&json!(expected)),
            "missing action {expected}"
        );
    }

    let samples = vec![
        json!({"action": "list_markets", "limit": 1}),
        json!({"action": "get_market", "market_id": "123"}),
        json!({"action": "list_events", "limit": 1}),
        json!({"action": "get_orderbook", "token_id": "1001"}),
        json!({"action": "get_price", "token_id": "1001", "side": "buy"}),
        json!({"action": "get_positions", "user": "0x1111111111111111111111111111111111111111"}),
        json!({"action": "get_balance", "user": "0x1111111111111111111111111111111111111111"}),
        json!({"action": "get_open_orders", "user": "0x1111111111111111111111111111111111111111"}),
        json!({"action": "get_usdc_allowance", "user": "0x1111111111111111111111111111111111111111"}),
        json!({"action": "place_order", "side": "BUY", "token_id": "1001", "price": 0.5, "size": 1.0, "approved": true}),
        json!({"action": "cancel_order", "order_id": "ord-1", "approved": true}),
    ];

    for sample in samples {
        let parsed: PolymarketRequest = serde_json::from_value(sample).unwrap();
        assert!(matches!(
            parsed,
            PolymarketRequest::ListMarkets { .. }
                | PolymarketRequest::GetMarket { .. }
                | PolymarketRequest::ListEvents { .. }
                | PolymarketRequest::GetOrderbook { .. }
                | PolymarketRequest::GetPrice { .. }
                | PolymarketRequest::GetPositions { .. }
                | PolymarketRequest::GetBalance { .. }
                | PolymarketRequest::GetOpenOrders { .. }
                | PolymarketRequest::GetUsdcAllowance { .. }
                | PolymarketRequest::PlaceOrder { .. }
                | PolymarketRequest::CancelOrder { .. }
        ));
    }
}

#[test]
fn signed_request_path_includes_query_pairs() {
    let signed = signed_request_path(
        "/data/positions",
        &[("user".to_string(), "0xabc".to_string())],
    );
    assert_eq!(signed, "/data/positions?user=0xabc");
}

#[test]
fn parse_order_nonce_accepts_number_and_string_payloads() {
    assert_eq!(parse_order_nonce(&json!({"nonce": 7})).unwrap(), 7);
    assert_eq!(parse_order_nonce(&json!({"nonce": "8"})).unwrap(), 8);
    assert_eq!(
        parse_order_nonce(&json!({"data": {"nonce": 9}})).unwrap(),
        9
    );
}

#[tokio::test]
async fn get_market_without_market_id_or_slug_errors() {
    let (gamma_base, calls) =
        start_mock_server(route("/markets", vec![MockResponse::body(200, r#"[]"#)])).await;

    let tool = test_tool(gamma_base.clone(), gamma_base, 15);
    let result = tool
        .execute(json!({ "action": "get_market" }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(
        result
            .output()
            .contains("requires either 'market_id' or 'slug'"),
        "got: {}",
        result.output()
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn ensure_https_accepts_https_url() {
    assert!(ensure_https("https://clob.polymarket.com").is_ok());
}

#[test]
fn ensure_https_accepts_loopback_http_for_mock() {
    assert!(ensure_https("http://127.0.0.1:8901").is_ok());
    assert!(ensure_https("http://localhost:8901/order").is_ok());
    assert!(ensure_https("http://[::1]:8901").is_ok());
}

#[test]
fn ensure_https_rejects_remote_http_url() {
    let err = ensure_https("http://clob.polymarket.com")
        .unwrap_err()
        .to_string();
    assert!(err.contains("non-HTTPS"), "unexpected error message: {err}");
}
