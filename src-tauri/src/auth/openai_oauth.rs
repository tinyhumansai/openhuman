use crate::auth::profiles::TokenSet;
use anyhow::{Context, Result};
use base64::Engine;
use chrono::Utc;
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const OPENAI_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const OPENAI_OAUTH_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const OPENAI_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const OPENAI_OAUTH_DEVICE_CODE_URL: &str = "https://auth.openai.com/oauth/device/code";
pub const OPENAI_OAUTH_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";

#[derive(Debug, Clone)]
pub struct PkceState {
    pub code_verifier: String,
    pub code_challenge: String,
    pub state: String,
}

#[derive(Debug, Clone)]
pub struct DeviceCodeStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    expires_in: u64,
    #[serde(default)]
    interval: Option<u64>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

pub fn generate_pkce_state() -> PkceState {
    let code_verifier = random_base64url(64);
    let digest = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);

    PkceState {
        code_verifier,
        code_challenge,
        state: random_base64url(24),
    }
}

pub fn build_authorize_url(pkce: &PkceState) -> String {
    let mut params = BTreeMap::new();
    params.insert("response_type", "code");
    params.insert("client_id", OPENAI_OAUTH_CLIENT_ID);
    params.insert("redirect_uri", OPENAI_OAUTH_REDIRECT_URI);
    params.insert("scope", "openid profile email offline_access");
    params.insert("code_challenge", pkce.code_challenge.as_str());
    params.insert("code_challenge_method", "S256");
    params.insert("state", pkce.state.as_str());
    params.insert("codex_cli_simplified_flow", "true");
    params.insert("id_token_add_organizations", "true");

    let mut encoded: Vec<String> = Vec::with_capacity(params.len());
    for (k, v) in params {
        encoded.push(format!("{}={}", url_encode(k), url_encode(v)));
    }

    format!("{OPENAI_OAUTH_AUTHORIZE_URL}?{}", encoded.join("&"))
}

pub async fn exchange_code_for_tokens(
    client: &Client,
    code: &str,
    pkce: &PkceState,
) -> Result<TokenSet> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("client_id", OPENAI_OAUTH_CLIENT_ID),
        ("redirect_uri", OPENAI_OAUTH_REDIRECT_URI),
        ("code_verifier", pkce.code_verifier.as_str()),
    ];

    let response = client
        .post(OPENAI_OAUTH_TOKEN_URL)
        .form(&form)
        .send()
        .await
        .context("Failed to exchange OpenAI OAuth authorization code")?;

    parse_token_response(response).await
}

pub async fn refresh_access_token(client: &Client, refresh_token: &str) -> Result<TokenSet> {
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", OPENAI_OAUTH_CLIENT_ID),
    ];

    let response = client
        .post(OPENAI_OAUTH_TOKEN_URL)
        .form(&form)
        .send()
        .await
        .context("Failed to refresh OpenAI OAuth token")?;

    parse_token_response(response).await
}

pub async fn start_device_code_flow(client: &Client) -> Result<DeviceCodeStart> {
    let form = [
        ("client_id", OPENAI_OAUTH_CLIENT_ID),
        ("scope", "openid profile email offline_access"),
    ];

    let response = client
        .post(OPENAI_OAUTH_DEVICE_CODE_URL)
        .form(&form)
        .send()
        .await
        .context("Failed to start OpenAI OAuth device-code flow")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("OpenAI device-code start failed ({status}): {body}");
    }

    let parsed: DeviceCodeResponse = response
        .json()
        .await
        .context("Failed to parse OpenAI device-code response")?;

    Ok(DeviceCodeStart {
        device_code: parsed.device_code,
        user_code: parsed.user_code,
        verification_uri: parsed.verification_uri,
        verification_uri_complete: parsed.verification_uri_complete,
        expires_in: parsed.expires_in,
        interval: parsed.interval.unwrap_or(5).max(1),
        message: parsed.message,
    })
}

pub async fn poll_device_code_tokens(
    client: &Client,
    device: &DeviceCodeStart,
) -> Result<TokenSet> {
    let started = Instant::now();
    let mut interval_secs = device.interval.max(1);

    loop {
        if started.elapsed() > Duration::from_secs(device.expires_in) {
            anyhow::bail!("Device-code flow timed out before authorization completed");
        }

        tokio::time::sleep(Duration::from_secs(interval_secs)).await;

        let form = [
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", device.device_code.as_str()),
            ("client_id", OPENAI_OAUTH_CLIENT_ID),
        ];

        let response = client
            .post(OPENAI_OAUTH_TOKEN_URL)
            .form(&form)
            .send()
            .await
            .context("Failed polling OpenAI device-code token endpoint")?;

        if response.status().is_success() {
            return parse_token_response(response).await;
        }

        let status = response.status();
        let text = response.text().await.unwrap_or_default();

        if let Ok(err) = serde_json::from_str::<OAuthErrorResponse>(&text) {
            match err.error.as_str() {
                "authorization_pending" => {
                    continue;
                }
                "slow_down" => {
                    interval_secs = interval_secs.saturating_add(5);
                    continue;
                }
                "access_denied" => {
                    anyhow::bail!("OpenAI device-code authorization was denied")
                }
                "expired_token" => {
                    anyhow::bail!("OpenAI device-code expired")
                }
                _ => {
                    anyhow::bail!(
                        "OpenAI device-code polling failed ({status}): {}",
                        err.error_description.unwrap_or(err.error)
                    )
                }
            }
        }

        anyhow::bail!("OpenAI device-code polling failed ({status}): {text}");
    }
}

pub async fn receive_loopback_code(expected_state: &str, timeout: Duration) -> Result<String> {
    let listener = TcpListener::bind("127.0.0.1:1455")
        .await
        .context("Failed to bind callback listener at 127.0.0.1:1455")?;

    let accepted = tokio::time::timeout(timeout, listener.accept())
        .await
        .context("Timed out waiting for browser callback")?
        .context("Failed to accept callback connection")?;

    let (mut stream, _) = accepted;
    let mut buffer = vec![0_u8; 8192];
    let bytes_read = stream
        .read(&mut buffer)
        .await
        .context("Failed to read callback request")?;

    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let first_line = request
        .lines()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Malformed callback request"))?;

    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Callback request missing path"))?;

    let code = parse_code_from_redirect(path, Some(expected_state))?;

    let body =
        "<html><body><h2>Alphahuman login complete</h2><p>You can close this tab.</p></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;

    Ok(code)
}

pub fn parse_code_from_redirect(input: &str, expected_state: Option<&str>) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        anyhow::bail!("No OAuth code provided");
    }

    let query = if let Some((_, right)) = trimmed.split_once('?') {
        right
    } else {
        trimmed
    };

    let params = parse_query_params(query);
    let is_callback_payload = trimmed.contains('?')
        || params.contains_key("code")
        || params.contains_key("state")
        || params.contains_key("error");

    if let Some(err) = params.get("error") {
        let desc = params
            .get("error_description")
            .cloned()
            .unwrap_or_else(|| "OAuth authorization failed".to_string());
        anyhow::bail!("OpenAI OAuth error: {err} ({desc})");
    }

    if let Some(expected_state) = expected_state {
        if let Some(got) = params.get("state") {
            if got != expected_state {
                anyhow::bail!("OAuth state mismatch");
            }
        } else if is_callback_payload {
            anyhow::bail!("Missing OAuth state in callback");
        }
    }

    if let Some(code) = params.get("code").cloned() {
        return Ok(code);
    }

    if !is_callback_payload {
        return Ok(trimmed.to_string());
    }

    anyhow::bail!("Missing OAuth code in callback")
}

pub fn extract_account_id_from_jwt(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;

    for key in [
        "account_id",
        "accountId",
        "acct",
        "sub",
        "https://api.openai.com/account_id",
    ] {
        if let Some(value) = claims.get(key).and_then(|v| v.as_str()) {
            if !value.trim().is_empty() {
                return Some(value.to_string());
            }
        }
    }

    None
}

async fn parse_token_response(response: reqwest::Response) -> Result<TokenSet> {
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("OpenAI OAuth token request failed ({status}): {body}");
    }

    let token: TokenResponse = response
        .json()
        .await
        .context("Failed to parse OpenAI token response")?;

    let expires_at = token.expires_in.and_then(|seconds| {
        if seconds <= 0 {
            None
        } else {
            Some(Utc::now() + chrono::Duration::seconds(seconds))
        }
    });

    Ok(TokenSet {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        id_token: token.id_token,
        expires_at,
        token_type: token.token_type,
        scope: token.scope,
    })
}

fn parse_query_params(input: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for pair in input.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        out.insert(url_decode(key), url_decode(value));
    }
    out
}

fn random_base64url(byte_len: usize) -> String {
    use chacha20poly1305::aead::{rand_core::RngCore, OsRng};

    let mut bytes = vec![0_u8; byte_len];
    OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn url_encode(input: &str) -> String {
    input
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect::<String>()
}

fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = bytes[i + 1] as char;
                let lo = bytes[i + 2] as char;
                if let (Some(h), Some(l)) = (hi.to_digit(16), lo.to_digit(16)) {
                    if let Ok(value) = u8::try_from(h * 16 + l) {
                        out.push(value);
                        i += 3;
                        continue;
                    }
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }

    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_generation_is_valid() {
        let pkce = generate_pkce_state();
        assert!(pkce.code_verifier.len() >= 43);
        assert!(!pkce.code_challenge.is_empty());
        assert!(!pkce.state.is_empty());
    }

    #[test]
    fn parse_redirect_url_extracts_code() {
        let code = parse_code_from_redirect(
            "http://127.0.0.1:1455/auth/callback?code=abc123&state=xyz",
            Some("xyz"),
        )
        .unwrap();
        assert_eq!(code, "abc123");
    }

    #[test]
    fn parse_redirect_accepts_raw_code() {
        let code = parse_code_from_redirect("raw-code", None).unwrap();
        assert_eq!(code, "raw-code");
    }

    #[test]
    fn parse_redirect_rejects_state_mismatch() {
        let err = parse_code_from_redirect("/auth/callback?code=x&state=a", Some("b")).unwrap_err();
        assert!(err.to_string().contains("state mismatch"));
    }

    #[test]
    fn parse_redirect_rejects_error_without_code() {
        let err = parse_code_from_redirect(
            "/auth/callback?error=access_denied&error_description=user+cancelled",
            Some("xyz"),
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("OpenAI OAuth error: access_denied"));
    }

    #[test]
    fn extract_account_id_from_jwt_payload() {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("{}");
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode("{\"account_id\":\"acct_123\"}");
        let token = format!("{header}.{payload}.sig");

        let account = extract_account_id_from_jwt(&token);
        assert_eq!(account.as_deref(), Some("acct_123"));
    }
}
