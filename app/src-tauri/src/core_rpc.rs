//! Shared helpers for authenticated calls from the Tauri host to the local core RPC.

use std::time::Duration;

use reqwest::RequestBuilder;
use serde::Serialize;

const CORE_RPC_URL_ENV: &str = "OPENHUMAN_CORE_RPC_URL";
pub(crate) fn core_rpc_url_value() -> String {
    std::env::var(CORE_RPC_URL_ENV).unwrap_or_else(|_| {
        format!(
            "http://127.0.0.1:{}/rpc",
            crate::core_process::default_core_port()
        )
    })
}

pub(crate) fn apply_auth(builder: RequestBuilder) -> Result<RequestBuilder, String> {
    let token = crate::core_process::current_rpc_token()
        .ok_or_else(|| "core RPC token is not initialized".to_string())?;
    Ok(builder.header("Authorization", format!("Bearer {token}")))
}

/// Verbatim status + body from an upstream runtime, mirrored back to the
/// renderer so it can reuse its existing JSON-RPC envelope parsing.
#[derive(Serialize)]
pub(crate) struct RelayHttpResponse {
    pub status: u16,
    pub body: String,
}

/// Normalize an optional bearer token into the header value to send, if any.
/// A `None` or blank/whitespace token yields `None` so we never emit an empty
/// `Authorization` header (local OpenAI-compatible runtimes that need no auth
/// would otherwise see a malformed bearer).
fn relay_bearer_header(token: Option<&str>) -> Option<String> {
    token
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| format!("Bearer {t}"))
}

/// Redact a relay URL before it lands in a log line or error string: drop the
/// query, fragment, and any userinfo (which can carry tokens/credentials),
/// keeping just `scheme://host[:port]/path` so transport diagnostics stay
/// useful without persisting secrets. Falls back to a coarse sentinel when the
/// URL can't be parsed.
fn redact_url_for_log(url: &str) -> String {
    url.parse::<url::Url>()
        .map(|mut parsed| {
            parsed.set_query(None);
            parsed.set_fragment(None);
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.to_string()
        })
        .unwrap_or_else(|_| "<invalid relay url>".to_string())
}

/// POST a JSON-RPC body to an arbitrary self-hosted runtime URL from the Rust
/// host instead of the webview.
///
/// Why this exists (#3865): the desktop webview origin is `tauri://localhost`,
/// a *secure context*. Chromium treats `http://127.0.0.1` / `localhost` as
/// "potentially trustworthy", so browser `fetch()` to the embedded local core
/// works — but a self-hosted runtime on a LAN IP (e.g.
/// `http://192.168.1.74:7788`) is plain cleartext from a secure context, so the
/// fetch is blocked as mixed content before any request leaves the browser
/// ("Failed to fetch", and the runtime never logs a `/rpc` hit) even though the
/// endpoint is healthy and reachable from curl/Safari. Issuing the request from
/// the Rust host with `reqwest` bypasses the webview's mixed-content / CORS
/// restrictions entirely — the same way the shell already talks to the local
/// core.
///
/// Returns the upstream status + body verbatim (including JSON-RPC error
/// envelopes and any 4xx/5xx) so the renderer keeps its existing handling; only
/// transport-level failures (DNS, connect, timeout) surface as `Err`.
#[tauri::command]
pub(crate) async fn relay_http_rpc(
    url: String,
    token: Option<String>,
    body: String,
) -> Result<RelayHttpResponse, String> {
    post_json_rpc(&url, token.as_deref(), body).await
}

/// Transport core of [`relay_http_rpc`]: POST a JSON body to `url` with an
/// optional bearer, returning the upstream status + body verbatim. Factored out
/// so in-process shell callers (e.g. the desktop companion pipeline) can reach
/// the local core over the same path the renderer's relay uses, without going
/// through the Tauri command boundary.
pub(crate) async fn post_json_rpc(
    url: &str,
    token: Option<&str>,
    body: String,
) -> Result<RelayHttpResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let mut builder = client
        .post(url)
        .header("Content-Type", "application/json")
        .body(body);

    let bearer = relay_bearer_header(token);
    if let Some(value) = bearer.as_deref() {
        builder = builder.header("Authorization", value);
    }

    let safe_url = redact_url_for_log(url);
    log::debug!(
        "[core_rpc][relay] POST {safe_url} (auth={})",
        bearer.is_some()
    );

    let resp = builder
        .send()
        .await
        .map_err(|e| format!("request to {safe_url} failed: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("failed to read response body from {safe_url}: {e}"))?;
    log::debug!(
        "[core_rpc][relay] ← {safe_url} status={status} body_len={}",
        text.len()
    );
    Ok(RelayHttpResponse { status, body: text })
}

/// Invoke a core JSON-RPC method against the embedded local core, returning the
/// inner result value. Handles building the JSON-RPC envelope, applying the
/// per-launch bearer, and unwrapping the `{ "result": <v>, "logs": [...] }`
/// `RpcOutcome` envelope some core handlers emit.
pub(crate) async fn call_core_rpc(
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let url = core_rpc_url_value();
    let token = crate::core_process::current_rpc_token();
    let envelope = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let resp = post_json_rpc(&url, token.as_deref(), envelope.to_string()).await?;
    if !(200..300).contains(&resp.status) {
        return Err(format!("core rpc {method} returned status {}", resp.status));
    }
    let parsed: serde_json::Value = serde_json::from_str(&resp.body)
        .map_err(|e| format!("core rpc {method}: invalid JSON response: {e}"))?;
    if let Some(err) = parsed.get("error") {
        if !err.is_null() {
            return Err(format!("core rpc {method} error: {err}"));
        }
    }
    let result = parsed
        .get("result")
        .cloned()
        .ok_or_else(|| format!("core rpc {method}: response has no result field"))?;
    Ok(unwrap_rpc_outcome(result))
}

/// Unwrap the `{ "result": <value>, "logs": [...] }` `RpcOutcome` envelope that
/// logged core handlers wrap their payload in; passes bare values through
/// unchanged.
fn unwrap_rpc_outcome(value: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = value.as_object() {
        if obj.len() == 2
            && obj.contains_key("result")
            && obj.get("logs").map(|l| l.is_array()).unwrap_or(false)
        {
            return obj
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::relay_bearer_header;

    #[test]
    fn bearer_header_present_for_real_token() {
        assert_eq!(
            relay_bearer_header(Some("tok123")).as_deref(),
            Some("Bearer tok123")
        );
        // Surrounding whitespace is trimmed before formatting.
        assert_eq!(
            relay_bearer_header(Some("  tok123  ")).as_deref(),
            Some("Bearer tok123")
        );
    }

    #[test]
    fn bearer_header_absent_for_missing_or_blank_token() {
        assert_eq!(relay_bearer_header(None), None);
        assert_eq!(relay_bearer_header(Some("")), None);
        assert_eq!(relay_bearer_header(Some("   ")), None);
    }
}
