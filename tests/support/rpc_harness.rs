//! JSON-RPC test harness shared by suites that serve the core router in-process.
//!
//! The pieces every router suite re-declared: serve the router on an ephemeral
//! port, POST a JSON-RPC call with the live bearer, read `/schema`, unwrap
//! `result` / `error`, and write the smallest config the core accepts. The
//! bearer is [`crate::rpc_auth::rpc_token`], never a per-suite constant (see
//! that module for why).
//!
//! Declare it at the root of an aggregated target next to `rpc_auth`; suites
//! `use crate::rpc_harness::{...}`.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::rpc_auth::{ensure_rpc_auth, rpc_token};

/// Serve the full core router on `127.0.0.1:0`; returns its address and task.
pub async fn serve_rpc() -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    ensure_rpc_auth();
    #[cfg(feature = "linked-modules")]
    openhuman_modules_linked::install();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr = listener.local_addr().expect("rpc listener addr");
    let router = openhuman_rpc::server::build_core_http_router(false);
    let join = tokio::spawn(async move { axum::serve(listener, router).await });
    (addr, join)
}

/// `GET /schema` as JSON.
pub async fn schema(rpc_base: &str) -> Value {
    let url = format!("{}/schema", rpc_base.trim_end_matches('/'));
    reqwest::get(&url)
        .await
        .unwrap_or_else(|err| panic!("GET {url}: {err}"))
        .json::<Value>()
        .await
        .expect("schema json")
}

/// POST one JSON-RPC 2.0 call with the live bearer; asserts HTTP 200 and
/// returns the decoded envelope.
pub async fn rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("client");
    let url = format!("{}/rpc", rpc_base.trim_end_matches('/'));
    let response = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {}", rpc_token()))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .unwrap_or_else(|err| panic!("POST {url} {method}: {err}"));
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "HTTP transport should accept {method}"
    );
    response
        .json::<Value>()
        .await
        .unwrap_or_else(|err| panic!("json for {method}: {err}"))
}

/// The envelope's `result`; panics with `context` on a JSON-RPC error.
pub fn ok<'a>(value: &'a Value, context: &str) -> &'a Value {
    if let Some(error) = value.get("error") {
        panic!("{context}: unexpected JSON-RPC error: {error}");
    }
    value
        .get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {value}"))
}

/// The inner `result.result` payload when the controller wraps one, otherwise
/// the `result` itself.
pub fn payload<'a>(value: &'a Value, context: &str) -> &'a Value {
    let result = ok(value, context);
    result.get("result").unwrap_or(result)
}

/// The envelope's `error.message`; panics with `context` when there is none.
pub fn error_message<'a>(value: &'a Value, context: &str) -> &'a str {
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error missing message: {value}"))
}

/// Write the smallest `config.toml` the core accepts (no local AI, no memory
/// provider, no encryption) under `openhuman_dir`, checking it against the
/// schema.
pub fn write_min_config(openhuman_dir: &Path) {
    std::fs::create_dir_all(openhuman_dir).expect("create .openhuman");
    let cfg = r#"api_url = "http://127.0.0.1:9"
default_model = "e2e-model"
default_temperature = 0.2

[secrets]
encrypt = false

[local_ai]
enabled = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0

[memory_tree]
embedding_strict = false
"#;
    let cfg = if cfg!(feature = "linked-modules") {
        format!("{cfg}\n[modules]\nsource = \"linked\"\n")
    } else {
        cfg.to_string()
    };
    std::fs::write(openhuman_dir.join("config.toml"), &cfg).expect("write config.toml");
    let _: openhuman_core::config::Config =
        toml::from_str(&cfg).expect("test config must match schema");
}
