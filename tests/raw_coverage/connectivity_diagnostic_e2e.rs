//! JSON-RPC coverage for the connectivity diagnostics surface.

use crate::env_guard::EnvVarGuard;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::OnceLock;

use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_rpc::server::build_core_http_router;

const TEST_RPC_TOKEN: &str = "connectivity-raw-coverage-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();
static ENV_LOCK: &OnceLock<tokio::sync::Mutex<()>> = &crate::SHARED_ENV_LOCK;

struct TestHarness {
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    rpc_join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

async fn env_lock_async() -> tokio::sync::MutexGuard<'static, ()> {
    let mutex = ENV_LOCK.get_or_init(|| tokio::sync::Mutex::new(()));
    mutex.lock().await
}

fn ensure_rpc_auth() {
    AUTH_INIT.get_or_init(|| {
        std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN);
        let token_dir = std::env::temp_dir().join("openhuman-connectivity-raw-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
}

/// The bearer this process actually validates.
///
/// `core::auth::RPC_TOKEN` is a process-global `OnceLock` and `init_rpc_token`
/// returns early once it is set — deliberately, so a second call cannot 401 live
/// clients. Since `tests/raw_coverage/` is one aggregated binary, only the first
/// suite to reach `ensure_rpc_auth` pins its own `TEST_RPC_TOKEN`; every other
/// suite sending its literal gets a 401 and trips its own `assert_eq!` (#6112).
/// Ask the auth module what it settled on instead of assuming we won the race.
fn rpc_bearer() -> &'static str {
    ensure_rpc_auth();
    openhuman_core::core::auth::get_rpc_token()
        .expect("ensure_rpc_auth initialises the token subsystem on the line above")
}

async fn serve_rpc() -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    ensure_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr = listener.local_addr().expect("rpc listener addr");
    let router = build_core_http_router(false);
    let join = tokio::spawn(async move { axum::serve(listener, router).await });
    (addr, join)
}

fn write_min_config(openhuman_dir: &Path) {
    std::fs::create_dir_all(openhuman_dir).expect("create .openhuman");
    std::fs::write(
        openhuman_dir.join("config.toml"),
        r#"api_url = "http://127.0.0.1:9"
default_model = "e2e-model"

[secrets]
encrypt = false

[local_ai]
enabled = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0
"#,
    )
    .expect("write config.toml");
}

async fn setup() -> TestHarness {
    let tmp = tempdir().expect("tempdir");
    let openhuman_dir = tmp.path().join(".openhuman");
    write_min_config(&openhuman_dir);
    let guards = vec![
        EnvVarGuard::set_to_path("OPENHUMAN_HOME", &openhuman_dir),
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", tmp.path()),
        EnvVarGuard::set("OPENHUMAN_API_URL", "http://127.0.0.1:9"),
        EnvVarGuard::set("OPENHUMAN_SECRETS_ENCRYPT", "false"),
        EnvVarGuard::unset("OPENHUMAN_CORE_RPC_URL"),
        EnvVarGuard::unset("OPENHUMAN_CORE_PORT"),
    ];
    let (addr, rpc_join) = serve_rpc().await;
    TestHarness {
        _tmp: tmp,
        _guards: guards,
        rpc_base: format!("http://{addr}/rpc"),
        rpc_join,
    }
}

async fn rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("client");
    let response = client
        .post(rpc_base)
        .bearer_auth(rpc_bearer())
        .json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .expect("send rpc");
    assert_eq!(response.status(), StatusCode::OK, "rpc status for {method}");
    response.json().await.expect("rpc json")
}

fn payload<'a>(value: &'a Value, context: &str) -> &'a Value {
    value
        .get("result")
        .and_then(|r| r.get("payload").or_else(|| r.get("result")))
        .unwrap_or_else(|| panic!("{context} should include result.payload: {value}"))
}

#[tokio::test]
async fn connectivity_diag_rpc_reports_live_listener_port_and_process() {
    let _lock = env_lock_async().await;
    let harness = setup().await;
    let rpc_port = harness
        .rpc_base
        .parse::<url::Url>()
        .expect("rpc url")
        .port()
        .expect("rpc port");
    let _core_port = EnvVarGuard::set("OPENHUMAN_CORE_PORT", rpc_port.to_string());

    let diag_result = rpc(
        &harness.rpc_base,
        91_001,
        "openhuman.connectivity_diag",
        json!({}),
    )
    .await;
    let diag_payload = payload(&diag_result, "connectivity_diag")
        .get("diag")
        .unwrap_or_else(|| panic!("diag payload missing: {diag_result}"));

    assert_eq!(diag_payload["listen_port"], json!(rpc_port));
    assert_eq!(diag_payload["listen_port_in_use"], json!(true));
    assert!(
        diag_payload["socket_state"] == json!("uninitialized")
            || diag_payload["socket_state"] == json!("disconnected"),
        "unexpected socket state: {diag_payload}"
    );
    assert_eq!(
        diag_payload["sidecar_pid"],
        json!(u64::from(std::process::id()))
    );

    harness.rpc_join.abort();
}
