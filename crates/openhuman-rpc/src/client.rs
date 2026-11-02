//! Shared authenticated HTTP transport for OpenHuman JSON-RPC clients.

use std::time::Duration;

use serde::Serialize;

/// Verbatim status and body returned by an OpenHuman JSON-RPC endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HttpRpcResponse {
    pub status: u16,
    pub body: String,
}

/// Normalize an optional token into a bearer header value.
pub fn bearer_header(token: Option<&str>) -> Option<String> {
    token
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| format!("Bearer {token}"))
}

/// Redact credentials and request-specific URL components before logging.
pub fn redact_url_for_log(url: &str) -> String {
    url.parse::<url::Url>()
        .map(|mut parsed| {
            parsed.set_query(None);
            parsed.set_fragment(None);
            parsed.set_path("");
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.to_string()
        })
        .unwrap_or_else(|_| "<invalid relay url>".to_string())
}

/// POST a JSON-RPC body and preserve the endpoint's status and body verbatim.
pub async fn post_json_rpc(
    url: &str,
    token: Option<&str>,
    body: impl Into<String>,
) -> Result<HttpRpcResponse, String> {
    let bearer = bearer_header(token);
    let mut client_builder = reqwest::Client::builder().timeout(Duration::from_secs(30));
    if bearer.is_some() {
        client_builder = client_builder.redirect(reqwest::redirect::Policy::none());
    }
    let client = client_builder
        .build()
        .map_err(|error| format!("failed to build HTTP client: {error}"))?;

    let mut request = client
        .post(url)
        .header("Content-Type", "application/json")
        .body(body.into());
    if let Some(value) = bearer.as_deref() {
        request = request.header("Authorization", value);
    }

    let safe_url = redact_url_for_log(url);
    log::debug!(
        "[openhuman_rpc] POST {safe_url} (auth={})",
        bearer.is_some()
    );
    let response = request
        .send()
        .await
        .map_err(|error| format!("request to {safe_url} failed: {error}"))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|error| format!("failed to read response body from {safe_url}: {error}"))?;
    log::debug!(
        "[openhuman_rpc] response from {safe_url} status={status} body_len={}",
        body.len()
    );
    Ok(HttpRpcResponse { status, body })
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
