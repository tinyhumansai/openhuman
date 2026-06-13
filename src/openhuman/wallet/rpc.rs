//! Wallet network helpers — JSON-RPC POST + REST GET/POST primitives.
//!
//! JSON-RPC is used for EVM and Solana. REST is used for BTC (Esplora) and
//! Tron (TronGrid). Both honor an `OPENHUMAN_WALLET_RPC_<CHAIN>` env override
//! so tests can point everything at an axum mock.

use std::time::Duration;

use once_cell::sync::Lazy;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use super::defaults::{rpc_url_for_chain, rpc_url_for_evm_network, EvmNetwork};
use super::ops::WalletChain;

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

fn redact_rpc_url(raw: &str) -> String {
    match reqwest::Url::parse(raw) {
        Ok(url) => match url.host_str() {
            Some(host) => format!("{}://{}", url.scheme(), host),
            None => format!("{}://<unknown-host>", url.scheme()),
        },
        Err(_) => "<invalid-url>".to_string(),
    }
}

fn client() -> reqwest::Client {
    SHARED_CLIENT.clone()
}

/// JSON-RPC POST against a chain's default/override endpoint.
pub async fn rpc_call<T: DeserializeOwned>(
    chain: WalletChain,
    method: &str,
    params: Value,
) -> Result<T, String> {
    rpc_call_to(&rpc_url_for_chain(chain), method, params).await
}

/// JSON-RPC POST against a specific EVM network's RPC URL.
pub async fn evm_rpc_call<T: DeserializeOwned>(
    network: EvmNetwork,
    method: &str,
    params: Value,
) -> Result<T, String> {
    rpc_call_to(&rpc_url_for_evm_network(network), method, params).await
}

pub async fn rpc_call_to<T: DeserializeOwned>(
    url: &str,
    method: &str,
    params: Value,
) -> Result<T, String> {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    log::debug!(
        "{LOG_PREFIX} jsonrpc method={} url={}",
        method,
        redact_rpc_url(url)
    );
    let client = client();
    let response = client
        .post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("wallet RPC transport failed for {method}: {e}"))?;
    let status = response.status();
    let raw_body = response
        .text()
        .await
        .map_err(|e| format!("wallet RPC read body failed for {method}: {e}"))?;
    log::debug!(
        "{LOG_PREFIX} jsonrpc method={method} status={status} body_len={}",
        raw_body.len()
    );
    if !status.is_success() {
        return Err(format!(
            "wallet RPC HTTP failure for {method}: status={status} body={raw_body}"
        ));
    }
    let body: Value = serde_json::from_str(&raw_body)
        .map_err(|e| format!("wallet RPC decode failed for {method}: {e}; body={raw_body}"))?;
    if let Some(error) = body.get("error") {
        return Err(format!("wallet RPC error for {method}: {error}"));
    }
    let result = body
        .get("result")
        .cloned()
        .ok_or_else(|| format!("wallet RPC missing result for {method}"))?;
    serde_json::from_value(result)
        .map_err(|e| format!("wallet RPC invalid result for {method}: {e}"))
}

/// Plain REST GET returning JSON or text.
pub async fn rest_get_text(url: &str) -> Result<String, String> {
    log::debug!("{LOG_PREFIX} rest_get url={}", redact_rpc_url(url));
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

pub async fn rest_get_json<T: DeserializeOwned>(url: &str) -> Result<T, String> {
    let body = rest_get_text(url).await?;
    serde_json::from_str(&body)
        .map_err(|e| format!("wallet REST GET decode failed: {e}; body={body}"))
}

/// Plain REST POST with a raw text body (e.g. Esplora /tx accepts hex).
pub async fn rest_post_text(url: &str, body: &str, content_type: &str) -> Result<String, String> {
    log::debug!(
        "{LOG_PREFIX} rest_post url={} body_len={}",
        redact_rpc_url(url),
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

/// REST POST with a JSON body.
pub async fn rest_post_json<T: DeserializeOwned>(url: &str, body: &Value) -> Result<T, String> {
    log::debug!("{LOG_PREFIX} rest_post_json url={}", redact_rpc_url(url));
    let response = client()
        .post(url)
        .json(body)
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
    serde_json::from_str(&text)
        .map_err(|e| format!("wallet REST POST decode failed: {e}; body={text}"))
}

#[cfg(test)]
mod tests {
    use super::redact_rpc_url;

    #[test]
    fn redact_rpc_url_strips_path_and_query() {
        assert_eq!(
            redact_rpc_url("https://user:pass@example.com/path/secret?apiKey=123"),
            "https://example.com"
        );
    }

    #[test]
    fn redact_rpc_url_handles_invalid_values() {
        assert_eq!(redact_rpc_url("not a url"), "<invalid-url>");
    }
}
