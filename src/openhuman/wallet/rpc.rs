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

pub(crate) fn redact_rpc_url(raw: &str) -> String {
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

tokio::task_local! {
    /// Ordered Solana RPC endpoints active for the current async task. Set only
    /// by the tiny.place settlement path (see [`with_tinyplace_solana_endpoints`])
    /// so Solana `rpc_call`s made while broadcasting a tiny.place payment try the
    /// tiny.place RPC first and fall back to the public cluster. Unset for all
    /// general wallet operations.
    static TINYPLACE_SOLANA_ENDPOINTS: Vec<String>;
}

/// Run `fut` with the tiny.place Solana failover endpoint list active for any
/// Solana [`rpc_call`] made within it. Scoped to the current async task, so
/// concurrent general wallet operations are unaffected.
pub async fn with_tinyplace_solana_endpoints<F, T>(endpoints: Vec<String>, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    TINYPLACE_SOLANA_ENDPOINTS.scope(endpoints, fut).await
}

fn active_tinyplace_endpoints() -> Option<Vec<String>> {
    TINYPLACE_SOLANA_ENDPOINTS.try_with(|e| e.clone()).ok()
}

/// Distinguishes an endpoint being unreachable/broken (retry the next fallback)
/// from a healthy endpoint returning an authoritative JSON-RPC error (do NOT
/// fall back — e.g. a real insufficient-funds result must surface as-is).
enum RpcCallError {
    Transport(String),
    Rpc(String),
}

/// JSON-RPC POST against a chain's default/override endpoint.
///
/// For Solana, when a tiny.place settlement scope is active
/// ([`with_tinyplace_solana_endpoints`]) the call fails over across the
/// tiny.place → public endpoint list on transport errors. All other chains and
/// non-tiny.place Solana calls use the single configured endpoint.
pub async fn rpc_call<T: DeserializeOwned>(
    chain: WalletChain,
    method: &str,
    params: Value,
) -> Result<T, String> {
    if chain == WalletChain::Solana {
        if let Some(endpoints) = active_tinyplace_endpoints() {
            return rpc_call_failover(&endpoints, method, params).await;
        }
    }
    rpc_call_to(&rpc_url_for_chain(chain), method, params).await
}

/// Try each endpoint in order, advancing only on transport-level failures.
async fn rpc_call_failover<T: DeserializeOwned>(
    endpoints: &[String],
    method: &str,
    params: Value,
) -> Result<T, String> {
    let total = endpoints.len();
    let mut last_transport: Option<String> = None;
    for (idx, url) in endpoints.iter().enumerate() {
        match rpc_call_to_inner::<T>(url, method, params.clone()).await {
            Ok(value) => {
                if idx > 0 {
                    log::warn!(
                        "{LOG_PREFIX} tinyplace solana rpc: {method} succeeded on fallback \
                         endpoint #{idx} ({})",
                        redact_rpc_url(url)
                    );
                }
                return Ok(value);
            }
            // Authoritative answer from a healthy endpoint — surface it, never
            // fall back (e.g. a genuine simulation/insufficient-funds error).
            Err(RpcCallError::Rpc(message)) => return Err(message),
            Err(RpcCallError::Transport(message)) => {
                log::warn!(
                    "{LOG_PREFIX} tinyplace solana rpc: endpoint #{idx}/{total} ({}) unreachable \
                     for {method}, trying fallback: {message}",
                    redact_rpc_url(url)
                );
                last_transport = Some(message);
            }
        }
    }
    Err(last_transport
        .unwrap_or_else(|| format!("wallet RPC: no Solana endpoints configured for {method}")))
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
        redact_rpc_url(url)
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
