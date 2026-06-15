//! Browser-OAuth for HTTP-remote MCP servers (MCP authorization spec).
//!
//! Many remote MCP servers gate access behind OAuth 2.0 (authorization-code +
//! PKCE), advertised via a `401` challenge pointing at an
//! `oauth-protected-resource` document. This module runs that flow for the
//! desktop app using the **loopback redirect** approach (RFC 8252): the core's
//! own HTTP server hosts `/oauth/mcp/callback`, so no extra listener is needed.
//!
//! Flow:
//!   1. [`detect`]  — classify a server: `none` / `token` / `oauth`.
//!   2. [`begin`]   — discover the authorization server, **dynamically register**
//!                    a client (RFC 7591, capturing any issued `client_secret`),
//!                    generate PKCE, stash the pending state, and return the live
//!                    `/authorize` URL for the frontend to open in a browser.
//!   3. [`complete`]— called by the `/oauth/mcp/callback` route with `code`+`state`:
//!                    exchange the code (PKCE verifier + client creds) for an
//!                    access token, store it as the server's `Authorization`
//!                    header (reusing the `build_http_auth` connect path), and
//!                    reconnect.
//!
//! v1 stores the access token only (≈1h lifetime); refresh-token rotation is a
//! follow-up. The `client_secret` requirement was confirmed live against alpic,
//! which issues a confidential client despite a public-client request.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::openhuman::config::Config;
use crate::openhuman::mcp_client::McpHttpClient;

use super::store;
use super::types::Transport;

const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Pending authorization keyed by `state`, parked between [`begin`] and the
/// callback's [`complete`].
#[derive(Clone)]
struct PendingOAuth {
    server_id: String,
    code_verifier: String,
    client_id: String,
    client_secret: Option<String>,
    token_endpoint: String,
    redirect_uri: String,
}

fn pending() -> &'static Mutex<HashMap<String, PendingOAuth>> {
    static P: OnceLock<Mutex<HashMap<String, PendingOAuth>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Reserved env key holding the OAuth refresh bundle as JSON. Prefixed with
/// `__` so [`super::connections::build_http_auth`] skips it (it must NOT be
/// sent as a request header) and the UI hides it from the env-var list.
pub const OAUTH_BUNDLE_KEY: &str = "__oauth__";

/// Locally-stored (encrypted) OAuth bookkeeping for silent token refresh. The
/// access token itself lives in the `Authorization` env value; this carries
/// everything needed to mint a new one without another browser sign-in.
#[derive(Serialize, Deserialize)]
struct OAuthBundle {
    refresh_token: Option<String>,
    client_id: String,
    client_secret: Option<String>,
    token_endpoint: String,
    /// Unix seconds when the current access token expires (best-effort).
    expires_at: u64,
}

/// Parsed token-endpoint response.
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Result of [`detect`] — drives which control the connect modal renders.
#[derive(Debug, Serialize)]
pub struct AuthDetection {
    /// `none` (open) · `token` (static bearer/API key) · `oauth` (browser sign-in).
    pub kind: String,
    pub authorization_endpoint: Option<String>,
    pub grant_types: Vec<String>,
}

/// 32 bytes of entropy from two v4 UUIDs, base64url-encoded (no `rand` churn).
fn random_b64(n_uuids: usize) -> String {
    let mut bytes = Vec::with_capacity(n_uuids * 16);
    for _ in 0..n_uuids {
        bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    B64.encode(bytes)
}

fn gen_pkce() -> (String, String) {
    let verifier = random_b64(3); // ~64 chars, within the 43..128 PKCE range
    let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// `http://127.0.0.1:<core_port>/oauth/mcp/callback` — the route the core HTTP
/// server hosts. The port MUST match where the core actually bound, or the
/// browser redirect lands on a dead listener and sign-in times out.
///
/// Source priority:
/// 1. `OPENHUMAN_CORE_RPC_URL` — set by the core to its *real* bound address
///    after startup, so it reflects any port-fallback (e.g. the embedded core
///    falling back off the preferred 7788). This is the authoritative value.
/// 2. `OPENHUMAN_CORE_PORT` — the configured/requested port hint.
/// 3. `7788` — the default.
fn callback_redirect_uri() -> String {
    let port = port_from_core_rpc_url()
        .or_else(|| {
            std::env::var("OPENHUMAN_CORE_PORT")
                .ok()
                .and_then(|v| v.trim().parse::<u16>().ok())
        })
        .unwrap_or(7788);
    format!("http://127.0.0.1:{port}/oauth/mcp/callback")
}

/// Parse the bound port out of `OPENHUMAN_CORE_RPC_URL` (e.g.
/// `http://127.0.0.1:7790/rpc` → `7790`). `None` when the var is unset or has
/// no explicit port.
fn port_from_core_rpc_url() -> Option<u16> {
    let url = std::env::var("OPENHUMAN_CORE_RPC_URL").ok()?;
    parse_explicit_port(&url)
}

/// Extract an explicit port from an `http(s)://host:port/...` URL. Returns
/// `None` if there is no explicit port. Kept pure so it is unit-testable
/// without touching process env.
fn parse_explicit_port(url: &str) -> Option<u16> {
    reqwest::Url::parse(url).ok().and_then(|u| u.port())
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .expect("reqwest client must build")
}

/// Resolve the server's HTTP-remote URL, erroring for non-remote installs.
fn remote_url(config: &Config, server_id: &str) -> Result<String, String> {
    let server = store::get_server(config, server_id).map_err(|e| e.to_string())?;
    match server.transport {
        Transport::HttpRemote { url } if !url.is_empty() => Ok(url),
        Transport::HttpRemote { .. } => Err("server has no deployment_url".to_string()),
        Transport::Stdio => Err("oauth only applies to http_remote servers".to_string()),
    }
}

/// Classify a server's auth requirement via an unauthenticated probe — the only
/// reliable signal (registry metadata is unreliable / mislabelled).
pub async fn detect(config: &Config, server_id: &str) -> Result<AuthDetection, String> {
    // A genuine lookup/store failure (invalid `server_id`, DB error) must
    // surface as an error — collapsing it to `kind="none"` would mislead the UI
    // into showing a false "open server" state and hide the real failure. Only
    // a non-HTTP / URL-less transport is legitimately "no HTTP auth".
    let server = store::get_server(config, server_id).map_err(|e| e.to_string())?;
    let url = match server.transport {
        Transport::HttpRemote { url } if !url.is_empty() => url,
        _ => {
            return Ok(AuthDetection {
                kind: "none".into(),
                authorization_endpoint: None,
                grant_types: vec![],
            })
        }
    };
    let client = McpHttpClient::new(url, 20);
    match client.discover_authorization().await {
        // initialize did not 401 → open server.
        Ok(None) => Ok(AuthDetection {
            kind: "none".into(),
            authorization_endpoint: None,
            grant_types: vec![],
        }),
        Ok(Some(ctx)) => {
            // A 401 that points at an OAuth authorization server exposing an
            // `authorization_endpoint` → browser OAuth. We do NOT require
            // `grant_types_supported` to list `authorization_code`: per RFC 8414
            // that field is optional and *defaults* to including
            // `authorization_code` (alpic, for one, omits it entirely). The
            // presence of an authorize endpoint is the real signal. Otherwise a
            // plain bearer/API-key 401 → static token.
            for asm in &ctx.authorization_server_metadata {
                let supports_code = asm.grant_types_supported.is_empty()
                    || asm
                        .grant_types_supported
                        .iter()
                        .any(|g| g == "authorization_code");
                if asm.authorization_endpoint.is_some() && supports_code {
                    return Ok(AuthDetection {
                        kind: "oauth".into(),
                        authorization_endpoint: asm.authorization_endpoint.clone(),
                        grant_types: asm.grant_types_supported.clone(),
                    });
                }
            }
            Ok(AuthDetection {
                kind: "token".into(),
                authorization_endpoint: None,
                grant_types: vec![],
            })
        }
        // 401 we couldn't fully parse, or a transient error: let the user paste
        // a token rather than block them.
        Err(e) => {
            tracing::debug!("[mcp-oauth] detect fell back to token for {server_id}: {e}");
            Ok(AuthDetection {
                kind: "token".into(),
                authorization_endpoint: None,
                grant_types: vec![],
            })
        }
    }
}

/// Begin the browser-OAuth flow: discover → dynamic client registration → PKCE,
/// park the pending state, and return the live `/authorize` URL.
pub async fn begin(config: &Config, server_id: &str) -> Result<String, String> {
    let url = remote_url(config, server_id)?;
    let client = McpHttpClient::new(url.clone(), 20);
    let ctx = client
        .discover_authorization()
        .await
        .map_err(|e| format!("oauth discovery failed: {e}"))?
        .ok_or_else(|| "server does not require authorization".to_string())?;
    // Pick by capability, not position: the first advertised authorization
    // server may be incomplete while a later one is fully usable. Require the
    // endpoints begin() actually needs (authorize + token + dynamic client
    // registration) and — when grant types are listed — `authorization_code`.
    let asm = ctx
        .authorization_server_metadata
        .into_iter()
        .find(|asm| {
            asm.authorization_endpoint.is_some()
                && asm.token_endpoint.is_some()
                && asm.registration_endpoint.is_some()
                && (asm.grant_types_supported.is_empty()
                    || asm
                        .grant_types_supported
                        .iter()
                        .any(|g| g == "authorization_code"))
        })
        .ok_or_else(|| {
            "no authorization server advertised a usable OAuth configuration \
             (authorize + token + dynamic registration)"
                .to_string()
        })?;
    let authorization_endpoint = asm
        .authorization_endpoint
        .ok_or_else(|| "authorization server has no authorization_endpoint".to_string())?;
    let token_endpoint = asm
        .token_endpoint
        .ok_or_else(|| "authorization server has no token_endpoint".to_string())?;
    let registration_endpoint = asm.registration_endpoint.ok_or_else(|| {
        "server requires OAuth but does not support dynamic client registration".to_string()
    })?;

    let redirect_uri = callback_redirect_uri();
    let (client_id, client_secret) = register_client(&registration_endpoint, &redirect_uri).await?;

    let (code_verifier, code_challenge) = gen_pkce();
    let state = uuid::Uuid::new_v4().to_string();

    pending().lock().unwrap().insert(
        state.clone(),
        PendingOAuth {
            server_id: server_id.to_string(),
            code_verifier,
            client_id: client_id.clone(),
            client_secret,
            token_endpoint,
            redirect_uri: redirect_uri.clone(),
        },
    );

    let authorize_url = url::Url::parse_with_params(
        &authorization_endpoint,
        &[
            ("response_type", "code"),
            ("client_id", client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_challenge", code_challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
            ("resource", url.as_str()),
        ],
    )
    .map_err(|e| format!("failed to build authorize url: {e}"))?;

    tracing::info!(
        "[mcp-oauth] begin server_id={server_id} client_id={client_id} authorize={authorization_endpoint}"
    );
    Ok(authorize_url.to_string())
}

/// RFC 7591 dynamic client registration. Returns `(client_id, client_secret?)`.
/// We request `client_secret_post`; servers that issue a confidential client
/// (e.g. alpic) return a `client_secret` we must keep for the token exchange.
async fn register_client(
    registration_endpoint: &str,
    redirect_uri: &str,
) -> Result<(String, Option<String>), String> {
    let body = json!({
        "client_name": "OpenHuman",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "client_secret_post",
    });
    let resp = http()
        .post(registration_endpoint)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("client registration request failed: {e}"))?;
    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("client registration returned non-JSON: {e}"))?;
    if !status.is_success() {
        return Err(format!("client registration HTTP {status}: {json}"));
    }
    let client_id = json
        .get("client_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "registration response missing client_id".to_string())?
        .to_string();
    let client_secret = json
        .get("client_secret")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((client_id, client_secret))
}

/// Complete the flow from the callback route: exchange `code` for a token,
/// store it as the server's `Authorization` header, and reconnect.
pub async fn complete(config: &Config, state: &str, code: &str) -> Result<String, String> {
    let p = pending()
        .lock()
        .unwrap()
        .remove(state)
        .ok_or_else(|| "unknown or expired OAuth state".to_string())?;

    let form: Vec<(&str, &str)> = {
        let mut f = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", p.redirect_uri.as_str()),
            ("client_id", p.client_id.as_str()),
            ("code_verifier", p.code_verifier.as_str()),
        ];
        if let Some(secret) = p.client_secret.as_deref() {
            f.push(("client_secret", secret));
        }
        f
    };
    let tokens = parse_token_response(&post_token_form(&p.token_endpoint, &form).await?)?;

    // Persist the access token (as the Authorization header) plus the refresh
    // bundle so the connection survives token expiry without re-signing-in.
    persist_tokens(
        config,
        &p.server_id,
        &p.client_id,
        p.client_secret.as_deref(),
        &p.token_endpoint,
        &tokens,
    )?;

    // Reconnect so tools come live immediately.
    let server = store::get_server(config, &p.server_id).map_err(|e| e.to_string())?;
    super::connections::connect(config, &server)
        .await
        .map_err(|e| format!("connected auth but MCP connect failed: {e}"))?;

    tracing::info!(
        "[mcp-oauth] complete server_id={} — token stored, reconnected",
        p.server_id
    );
    Ok(p.server_id)
}

/// If an installed server has an OAuth refresh bundle whose access token is
/// expired (or within 60s of it), mint a new access token via the refresh-token
/// grant and persist it. Returns `Ok(true)` when a refresh happened. No-op
/// (`Ok(false)`) for non-OAuth servers or when no refresh token is available.
/// Called from the connect path so the agent never hits an expired token.
pub async fn refresh_if_expired(config: &Config, server_id: &str) -> Result<bool, String> {
    let env = store::load_env_values(config, server_id).unwrap_or_default();
    let bundle_json = match env.get(OAUTH_BUNDLE_KEY) {
        Some(b) => b,
        None => return Ok(false), // not an OAuth-authenticated server
    };
    let bundle: OAuthBundle =
        serde_json::from_str(bundle_json).map_err(|e| format!("corrupt oauth bundle: {e}"))?;
    if bundle.expires_at > now_unix() + 60 {
        return Ok(false); // still valid
    }
    let refresh_token = match bundle.refresh_token.as_deref() {
        Some(r) if !r.is_empty() => r,
        _ => return Ok(false), // nothing to refresh with; a 401 will prompt re-auth
    };

    let form: Vec<(&str, &str)> = {
        let mut f = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", bundle.client_id.as_str()),
        ];
        if let Some(s) = bundle.client_secret.as_deref() {
            f.push(("client_secret", s));
        }
        f
    };
    let mut tokens = parse_token_response(&post_token_form(&bundle.token_endpoint, &form).await?)?;
    // Some servers omit a rotated refresh token — keep the existing one.
    if tokens.refresh_token.is_none() {
        tokens.refresh_token = bundle.refresh_token.clone();
    }
    persist_tokens(
        config,
        server_id,
        &bundle.client_id,
        bundle.client_secret.as_deref(),
        &bundle.token_endpoint,
        &tokens,
    )?;
    tracing::info!("[mcp-oauth] refreshed access token for server_id={server_id}");
    Ok(true)
}

/// Persist the access token (`Authorization`) + refresh bundle (`__oauth__`)
/// for an OAuth server, MERGED over the existing env so any custom headers or
/// other stored keys survive the (re)connect/refresh — `set_env_values` is
/// replace-all, so starting from a blank map would silently erase them (#3648).
fn persist_tokens(
    config: &Config,
    server_id: &str,
    client_id: &str,
    client_secret: Option<&str>,
    token_endpoint: &str,
    tokens: &TokenResponse,
) -> Result<(), String> {
    let bundle = OAuthBundle {
        refresh_token: tokens.refresh_token.clone(),
        client_id: client_id.to_string(),
        client_secret: client_secret.map(str::to_string),
        token_endpoint: token_endpoint.to_string(),
        expires_at: now_unix() + tokens.expires_in.unwrap_or(3600),
    };
    let mut env = store::load_env_values(config, server_id).unwrap_or_default();
    env.insert(
        "Authorization".to_string(),
        format!("Bearer {}", tokens.access_token),
    );
    env.insert(
        OAUTH_BUNDLE_KEY.to_string(),
        serde_json::to_string(&bundle).map_err(|e| e.to_string())?,
    );
    store::set_env_values(config, server_id, &env).map_err(|e| e.to_string())?;
    let mut keys: Vec<String> = env.keys().cloned().collect();
    keys.sort();
    store::update_server_env_keys(config, server_id, &keys).map_err(|e| e.to_string())?;
    Ok(())
}

/// POST a form to a token endpoint and return the JSON body (erroring on non-2xx).
async fn post_token_form(endpoint: &str, form: &[(&str, &str)]) -> Result<Value, String> {
    let resp = http()
        .post(endpoint)
        .form(form)
        .send()
        .await
        .map_err(|e| format!("token request failed: {e}"))?;
    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("token endpoint returned non-JSON: {e}"))?;
    if !status.is_success() {
        return Err(format!("token request HTTP {status}: {json}"));
    }
    Ok(json)
}

fn parse_token_response(json: &Value) -> Result<TokenResponse, String> {
    let access_token = json
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "token response missing access_token".to_string())?
        .to_string();
    Ok(TokenResponse {
        access_token,
        refresh_token: json
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::to_string),
        expires_in: json.get("expires_in").and_then(Value::as_u64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        let (verifier, challenge) = gen_pkce();
        assert!(
            (43..=128).contains(&verifier.len()),
            "verifier in PKCE range"
        );
        let expected = B64.encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, expected);
    }

    #[test]
    fn parse_token_response_extracts_fields() {
        let v = json!({"access_token":"a","refresh_token":"r","expires_in":3600});
        let t = parse_token_response(&v).unwrap();
        assert_eq!(t.access_token, "a");
        assert_eq!(t.refresh_token.as_deref(), Some("r"));
        assert_eq!(t.expires_in, Some(3600));
        // refresh_token / expires_in are optional.
        let minimal = parse_token_response(&json!({"access_token":"x"})).unwrap();
        assert_eq!(minimal.access_token, "x");
        assert!(minimal.refresh_token.is_none());
        assert!(minimal.expires_in.is_none());
        // access_token is required.
        assert!(parse_token_response(&json!({"token_type":"bearer"})).is_err());
    }

    #[test]
    fn oauth_bundle_round_trips_through_env_json() {
        let bundle = OAuthBundle {
            refresh_token: Some("r".into()),
            client_id: "cli_x".into(),
            client_secret: Some("sec".into()),
            token_endpoint: "https://as/token".into(),
            expires_at: 1_700_000_000,
        };
        let json = serde_json::to_string(&bundle).unwrap();
        let back: OAuthBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(back.client_id, "cli_x");
        assert_eq!(back.refresh_token.as_deref(), Some("r"));
        assert_eq!(back.expires_at, 1_700_000_000);
    }

    /// Serialize the env-mutating callback tests — they share the process-global
    /// `OPENHUMAN_CORE_RPC_URL` / `OPENHUMAN_CORE_PORT` vars, and cargo runs
    /// tests in the same binary concurrently. Poison-recovery keeps a panicking
    /// test from wedging the others.
    fn callback_env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn callback_uri_uses_core_port_env() {
        let _guard = callback_env_lock();
        // With no RPC URL advertised, fall back to the CORE_PORT hint.
        std::env::remove_var("OPENHUMAN_CORE_RPC_URL");
        std::env::set_var("OPENHUMAN_CORE_PORT", "7790");
        assert_eq!(
            callback_redirect_uri(),
            "http://127.0.0.1:7790/oauth/mcp/callback"
        );
        std::env::remove_var("OPENHUMAN_CORE_PORT");
    }

    #[test]
    fn parse_explicit_port_reads_bound_port() {
        // Authoritative real bound address (with explicit port) → that port.
        assert_eq!(parse_explicit_port("http://127.0.0.1:7790/rpc"), Some(7790));
        assert_eq!(parse_explicit_port("http://127.0.0.1:1422/rpc"), Some(1422));
        // No explicit port (default) or unparseable → None, so the caller
        // falls back to CORE_PORT / 7788.
        assert_eq!(parse_explicit_port("http://127.0.0.1/rpc"), None);
        assert_eq!(parse_explicit_port("not-a-url"), None);
    }

    #[test]
    fn callback_uri_prefers_real_bound_port_over_core_port_hint() {
        let _guard = callback_env_lock();
        // The core fell back to 7791 (advertised via OPENHUMAN_CORE_RPC_URL)
        // even though the requested CORE_PORT was 7788 — the callback must use
        // the REAL bound port so the browser redirect actually reaches it.
        std::env::set_var("OPENHUMAN_CORE_RPC_URL", "http://127.0.0.1:7791/rpc");
        std::env::set_var("OPENHUMAN_CORE_PORT", "7788");
        assert_eq!(
            callback_redirect_uri(),
            "http://127.0.0.1:7791/oauth/mcp/callback"
        );
        std::env::remove_var("OPENHUMAN_CORE_RPC_URL");
        std::env::remove_var("OPENHUMAN_CORE_PORT");
    }
}
