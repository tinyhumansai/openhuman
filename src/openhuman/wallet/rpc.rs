use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use super::defaults::rpc_url_for_chain;
use super::ops::WalletChain;

const LOG_PREFIX: &str = "[wallet::rpc]";

fn redact_rpc_url(raw: &str) -> String {
    match reqwest::Url::parse(raw) {
        Ok(url) => match url.host_str() {
            Some(host) => format!("{}://{}", url.scheme(), host),
            None => format!("{}://<unknown-host>", url.scheme()),
        },
        Err(_) => "<invalid-url>".to_string(),
    }
}

pub async fn rpc_call<T: DeserializeOwned>(
    chain: WalletChain,
    method: &str,
    params: Value,
) -> Result<T, String> {
    let url = rpc_url_for_chain(chain);
    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    log::debug!(
        "{LOG_PREFIX} chain={:?} method={} url={}",
        chain,
        method,
        redact_rpc_url(&url)
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("wallet RPC client build failed for {method}: {e}"))?;
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
        "{LOG_PREFIX} chain={:?} method={} status={} body_len={}",
        chain,
        method,
        status,
        raw_body.len()
    );
    if !status.is_success() {
        return Err(format!(
            "wallet RPC HTTP failure for {method}: status={status} body={raw_body}"
        ));
    }
    let body: Value = serde_json::from_str(&raw_body)
        .map_err(|e| format!("wallet RPC decode failed for {method}: {e}; body={raw_body}"))?;
    log::debug!(
        "{LOG_PREFIX} chain={:?} method={} decoded_json=true",
        chain,
        method
    );
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
