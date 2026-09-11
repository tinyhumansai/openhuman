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
/// query, fragment, path, and any userinfo (which can carry tokens/credentials
/// or PII in the path itself), keeping just `scheme://host[:port]` so transport
/// diagnostics stay useful without persisting secrets. Falls back to a coarse
/// sentinel when the URL can't be parsed.
pub(crate) fn redact_url_for_log(url: &str) -> String {
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
    // Defense in depth behind `store::save`'s validation: never attach a
    // bearer over plain HTTP to a non-loopback host, whatever the caller is.
    // The local core (loopback) and any `https` endpoint keep working; a
    // Remote gateway that slipped past persistence is still rejected here.
    let relay_token = relay_bearer_header(token);
    #[cfg(feature = "gateways")]
    if relay_token.is_some()
        && crate::gateway::types::validate_remote_transport(url, token).is_err()
    {
        return Err(format!(
            "refusing to send a bearer to {} over an insecure transport",
            redact_url_for_log(url)
        ));
    }

    let mut client_builder = reqwest::Client::builder().timeout(Duration::from_secs(30));
    if relay_token.is_some() {
        // A bearer must never follow a redirect off the endpoint the user
        // configured. reqwest strips Authorization on cross-origin redirects
        // by default, but disabling redirects entirely for authenticated
        // requests is the deterministic, fail-closed choice.
        client_builder = client_builder.redirect(reqwest::redirect::Policy::none());
    }
    let client = client_builder
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let mut builder = client
        .post(url)
        .header("Content-Type", "application/json")
        .body(body);

    if let Some(value) = relay_token.as_deref() {
        builder = builder.header("Authorization", value);
    }

    let safe_url = redact_url_for_log(url);
    log::debug!(
        "[core_rpc][relay] POST {safe_url} (auth={})",
        relay_token.is_some()
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

#[cfg(test)]
mod tests {
    use super::redact_url_for_log;
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

    #[test]
    fn redact_strips_credentials_query_and_path() {
        // Userinfo, query, fragment, and path must not survive into logs; only
        // the scheme://host[:port] surface is kept for transport diagnostics.
        assert_eq!(
            redact_url_for_log("http://user:pass@192.168.1.74:7788/rpc/secret?token=t0k#frag"),
            "http://192.168.1.74:7788/"
        );
        assert_eq!(
            redact_url_for_log("https://core.example.com/rpc"),
            "https://core.example.com/"
        );
        // An unparseable URL degrades to the coarse sentinel.
        assert_eq!(redact_url_for_log("not a url"), "<invalid relay url>");
    }

    /// A non-loopback `http` URL carrying a bearer must be refused, and the
    /// surfaced error must carry the redacted `scheme://host[:port]` form —
    /// never the raw secret-bearing userinfo, path, or query (CWE-532).
    #[cfg(feature = "gateways")]
    #[tokio::test]
    async fn insecure_transport_refusal_redacts_url() {
        let err = match super::post_json_rpc(
            "http://user:pass@192.168.1.74:7788/rpc/secret?token=t0k",
            Some("bearer-tok"),
            "body".to_string(),
        )
        .await
        {
            Err(e) => e,
            Ok(_) => panic!("insecure non-loopback + bearer must be refused"),
        };
        assert!(
            !err.contains("pass"),
            "raw userinfo leaked into refusal error: {err}"
        );
        assert!(
            !err.contains("t0k"),
            "raw query token leaked into refusal error: {err}"
        );
        assert!(
            !err.contains("/secret"),
            "raw path leaked into refusal error: {err}"
        );
        assert!(
            err.contains("http://192.168.1.74:7788/"),
            "redacted host should remain for diagnostics: {err}"
        );
    }
}
