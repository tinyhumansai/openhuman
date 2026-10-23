//! Wallet network helpers — JSON-RPC POST + REST GET/POST primitives.
//!
//! JSON-RPC is used for EVM and Solana. REST is used for BTC (Esplora) and
//! Tron (TronGrid). Both honor an `OPENHUMAN_WALLET_RPC_<CHAIN>` env override
//! so tests can point everything at an axum mock. The wallet engine reaches
//! these through [`super::transport::OpenHumanTransport`].

use std::time::Duration;

use once_cell::sync::Lazy;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

const LOG_PREFIX: &str = "[wallet::rpc]";

/// Process-wide shared client so reqwest's connection pool stays hot across
/// repeated wallet RPC/REST calls. Building a new Client per call rebuilds
/// the TLS connector each time and tears down connection pooling — the
/// recommended pattern per reqwest 0.12 docs is to reuse a single Client.
static SHARED_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("wallet RPC client builder must succeed with default settings")
});

fn client() -> reqwest::Client {
    SHARED_CLIENT.clone()
}

/// Distinguishes an endpoint being unreachable/broken (retry the next fallback)
/// from a healthy endpoint returning an authoritative JSON-RPC error (do NOT
/// fall back — e.g. a real insufficient-funds result must surface as-is).
enum RpcCallError {
    Transport(String),
    Rpc(String),
}

pub async fn rpc_call_to<T: DeserializeOwned>(
    url: &str,
    method: &str,
    params: Value,
) -> Result<T, String> {
    rpc_call_to_inner(url, method, params)
        .await
        .map_err(|e| match e {
            RpcCallError::Transport(message) | RpcCallError::Rpc(message) => message,
        })
}

/// JSON-RPC POST returning a typed error so callers can distinguish endpoint
/// unreachability (retryable on a fallback) from an authoritative JSON-RPC error.
async fn rpc_call_to_inner<T: DeserializeOwned>(
    url: &str,
    method: &str,
    params: Value,
) -> Result<T, RpcCallError> {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    log::debug!(
        "{LOG_PREFIX} jsonrpc method={} url={}",
        method,
        crate::security::redact::endpoint_origin(url)
    );
    let client = client();
    let response = client.post(url).json(&payload).send().await.map_err(|e| {
        RpcCallError::Transport(format!("wallet RPC transport failed for {method}: {e}"))
    })?;
    let status = response.status();
    let raw_body = response.text().await.map_err(|e| {
        RpcCallError::Transport(format!("wallet RPC read body failed for {method}: {e}"))
    })?;
    log::debug!(
        "{LOG_PREFIX} jsonrpc method={method} status={status} body_len={}",
        raw_body.len()
    );
    if !status.is_success() {
        return Err(RpcCallError::Transport(format!(
            "wallet RPC HTTP failure for {method}: status={status} body={raw_body}"
        )));
    }
    let body: Value = serde_json::from_str(&raw_body).map_err(|e| {
        RpcCallError::Transport(format!(
            "wallet RPC decode failed for {method}: {e}; body={raw_body}"
        ))
    })?;
    if let Some(error) = body.get("error") {
        return Err(RpcCallError::Rpc(format!(
            "wallet RPC error for {method}: {error}"
        )));
    }
    let result = body
        .get("result")
        .cloned()
        .ok_or_else(|| RpcCallError::Rpc(format!("wallet RPC missing result for {method}")))?;
    serde_json::from_value(result)
        .map_err(|e| RpcCallError::Rpc(format!("wallet RPC invalid result for {method}: {e}")))
}

/// Plain REST GET returning JSON or text.
pub async fn rest_get_text(url: &str) -> Result<String, String> {
    log::debug!(
        "{LOG_PREFIX} rest_get url={}",
        crate::security::redact::endpoint_origin(url)
    );
    let response = client()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("wallet REST GET transport failed: {e}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("wallet REST GET read body failed: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "wallet REST GET HTTP failure: status={status} body={body}"
        ));
    }
    Ok(body)
}

/// Plain REST POST with a raw text body (e.g. Esplora /tx accepts hex).
pub async fn rest_post_text(url: &str, body: &str, content_type: &str) -> Result<String, String> {
    log::debug!(
        "{LOG_PREFIX} rest_post url={} body_len={}",
        crate::security::redact::endpoint_origin(url),
        body.len()
    );
    let response = client()
        .post(url)
        .header("content-type", content_type)
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| format!("wallet REST POST transport failed: {e}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| format!("wallet REST POST read body failed: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "wallet REST POST HTTP failure: status={status} body={text}"
        ));
    }
    Ok(text)
}
