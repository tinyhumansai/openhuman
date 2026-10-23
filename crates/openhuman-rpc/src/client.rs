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

/// `scheme://host[:port]` of `url` for logging: credentials, path, query and
/// fragment never survive. Answered by the TinySecurity module's
/// `EndpointOrigin` redaction (the one URL redactor); until the module serves, or
/// when it cannot answer, the fixed suppression marker is returned instead.
pub fn redact_url_for_log(url: &str) -> String {
    crate::core_host::util::log_redaction::endpoint_origin(url)
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
