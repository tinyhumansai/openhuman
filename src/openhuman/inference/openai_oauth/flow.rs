//! OAuth start / complete / status for OpenAI Codex (ChatGPT subscription).

use std::path::PathBuf;
use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use motosan_ai_oauth::StateStrategy;
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::openhuman::config::Config;
use crate::openhuman::credentials::state_dir_from_config;

use super::config::{codex_oauth_config, REDIRECT_URI};
use super::store::{
    import_codex_cli_auth, persist_openai_oauth_token, OPENAI_OAUTH_PROFILE_NAME,
    OPENAI_PROVIDER_KEY,
};

const LOG_PREFIX: &str = "[inference][openai-oauth]";
const PENDING_FILENAME: &str = "openai-oauth-pending.json";
const PENDING_TTL_SECS: u64 = 600;
const OAUTH_HTTP_TIMEOUT_SECS: u64 = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingOAuth {
    state: String,
    verifier: String,
    redirect_uri: String,
    created_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAiOAuthStartResult {
    pub auth_url: String,
    pub state: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAiOAuthStatusResult {
    pub connected: bool,
    pub profile_id: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub auth_method: Option<String>,
}

fn pending_path(config: &Config) -> PathBuf {
    state_dir_from_config(config).join(PENDING_FILENAME)
}

fn generate_pkce() -> (String, String) {
    let mut bytes = [0u8; 64];
    rand::rng().fill(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let hash = Sha256::digest(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hash);
    (verifier, challenge)
}

fn random_state() -> String {
    let mut state_bytes = [0u8; 16];
    rand::rng().fill(&mut state_bytes);
    URL_SAFE_NO_PAD.encode(state_bytes)
}

fn write_pending(config: &Config, pending: &PendingOAuth) -> Result<(), String> {
    let path = pending_path(config);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(pending).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    log::debug!("{LOG_PREFIX} pending session written");
    Ok(())
}

fn read_pending(config: &Config) -> Result<Option<PendingOAuth>, String> {
    let path = pending_path(config);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let pending: PendingOAuth = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let now = unix_now_secs();
    if now.saturating_sub(pending.created_at) > PENDING_TTL_SECS {
        let _ = std::fs::remove_file(&path);
        return Ok(None);
    }
    Ok(Some(pending))
}

fn clear_pending(config: &Config) {
    let path = pending_path(config);
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}

pub fn start_openai_oauth(config: &Config) -> Result<OpenAiOAuthStartResult, String> {
    let oauth_cfg = codex_oauth_config();
    let (verifier, challenge) = generate_pkce();
    let state = match oauth_cfg.state_strategy {
        StateStrategy::Random => random_state(),
        StateStrategy::EqualsVerifier => verifier.clone(),
    };

    let pending = PendingOAuth {
        state: state.clone(),
        verifier,
        redirect_uri: REDIRECT_URI.to_string(),
        created_at: unix_now_secs(),
    };
    write_pending(config, &pending)?;

    let auth_url = build_authorize_url(&oauth_cfg, &challenge, &state, REDIRECT_URI);
    log::info!("{LOG_PREFIX} oauth start state_len={}", state.len());

    Ok(OpenAiOAuthStartResult {
        auth_url,
        state,
        redirect_uri: REDIRECT_URI.to_string(),
    })
}

pub fn parse_callback_input(input: &str) -> Result<(String, String), String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("callback URL is required".to_string());
    }

    let query = if let Ok(parsed) = url::Url::parse(trimmed) {
        parsed.query().unwrap_or("").to_string()
    } else if trimmed.contains('=') {
        trimmed.to_string()
    } else {
        return Err("invalid callback URL".to_string());
    };

    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "code" if !value.is_empty() => code = Some(value.into_owned()),
            "state" if !value.is_empty() => state = Some(value.into_owned()),
            _ => {}
        }
    }

    let code = code.ok_or_else(|| "callback URL missing code parameter".to_string())?;
    let state = state.ok_or_else(|| "callback URL missing state parameter".to_string())?;
    Ok((code, state))
}

pub async fn complete_openai_oauth(
    config: &Config,
    callback_input: &str,
) -> Result<serde_json::Value, String> {
    let pending = read_pending(config)?
        .ok_or_else(|| "no pending OAuth session; call openai_oauth_start first".to_string())?;

    let (code, returned_state) = parse_callback_input(callback_input)?;
    if returned_state != pending.state {
        clear_pending(config);
        return Err("OAuth state mismatch — try connecting again".to_string());
    }

    let oauth_cfg = codex_oauth_config();
    let token =
        exchange_authorization_code(&oauth_cfg, &code, &pending.verifier, &pending.redirect_uri)
            .await?;

    clear_pending(config);
    let profile = persist_openai_oauth_token(config, &token)?;
    log::info!("{LOG_PREFIX} oauth complete profile_id={}", profile.id);

    Ok(serde_json::json!({
        "connected": true,
        "profileId": profile.id,
        "provider": OPENAI_PROVIDER_KEY,
        "authMethod": "oauth",
    }))
}

pub fn import_openai_oauth_from_codex_cli(config: &Config) -> Result<serde_json::Value, String> {
    let profile = import_codex_cli_auth(config)?;
    log::info!(
        "{LOG_PREFIX} codex cli auth imported profile_id={}",
        profile.id
    );

    Ok(serde_json::json!({
        "connected": true,
        "profileId": profile.id,
        "provider": OPENAI_PROVIDER_KEY,
        "authMethod": "oauth",
        "source": "codex_cli",
    }))
}

pub fn openai_oauth_status(config: &Config) -> Result<OpenAiOAuthStatusResult, String> {
    use crate::openhuman::credentials::profiles::AuthProfileKind;
    use crate::openhuman::credentials::AuthService;

    let auth = AuthService::from_config(config);
    let profile = auth
        .get_profile(OPENAI_PROVIDER_KEY, Some(OPENAI_OAUTH_PROFILE_NAME))
        .map_err(|e| e.to_string())?;

    let Some(profile) = profile else {
        return Ok(OpenAiOAuthStatusResult {
            connected: false,
            profile_id: None,
            expires_at: None,
            auth_method: None,
        });
    };

    if profile.kind != AuthProfileKind::OAuth {
        return Ok(OpenAiOAuthStatusResult {
            connected: false,
            profile_id: Some(profile.id),
            expires_at: None,
            auth_method: Some("token".to_string()),
        });
    }

    Ok(OpenAiOAuthStatusResult {
        connected: true,
        profile_id: Some(profile.id),
        expires_at: profile.token_set.as_ref().and_then(|t| t.expires_at),
        auth_method: Some("oauth".to_string()),
    })
}

pub fn disconnect_openai_oauth(config: &Config) -> Result<serde_json::Value, String> {
    use crate::openhuman::credentials::AuthService;

    let auth = AuthService::from_config(config);
    let removed = auth
        .remove_profile(OPENAI_PROVIDER_KEY, OPENAI_OAUTH_PROFILE_NAME)
        .map_err(|e| e.to_string())?;
    clear_pending(config);
    Ok(serde_json::json!({ "disconnected": removed }))
}

pub(super) fn build_authorize_url(
    config: &motosan_ai_oauth::OAuthConfig,
    challenge: &str,
    state: &str,
    redirect_uri: &str,
) -> String {
    let mut url = reqwest::Url::parse(config.auth_url).expect("auth_url must be valid");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", config.client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", &config.scopes.join(" "))
            .append_pair("state", state)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256");
        for (k, v) in config.extra_auth_params {
            q.append_pair(k, v);
        }
    }
    url.to_string()
}

pub(super) async fn exchange_authorization_code(
    config: &motosan_ai_oauth::OAuthConfig,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<motosan_ai_oauth::Token, String> {
    // Per RFC 6749 §4.1.3 the token request only requires grant_type, code,
    // redirect_uri, code_verifier (PKCE), and client_id. `state` belongs to the
    // authorization request / callback validation, not this exchange.
    let mut params = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", verifier),
        ("client_id", config.client_id),
    ];
    if let Some(secret) = config.client_secret {
        params.push(("client_secret", secret));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(OAUTH_HTTP_TIMEOUT_SECS))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .post(config.token_url)
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .await
        .map_err(|e| {
            log::warn!("{LOG_PREFIX} token exchange request failed: {e}");
            e.to_string()
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        log::warn!(
            "{LOG_PREFIX} token exchange http_status={status} body_len={}",
            body.len()
        );
        return Err(format!("HTTP {status}: {body}"));
    }

    #[derive(serde::Deserialize)]
    struct RawTokenResponse {
        access_token: String,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        id_token: Option<String>,
        expires_in: u64,
    }

    let raw: RawTokenResponse = resp.json().await.map_err(|e| e.to_string())?;
    Ok(motosan_ai_oauth::Token {
        access_token: raw.access_token,
        refresh_token: raw.refresh_token.unwrap_or_default(),
        id_token: raw.id_token,
        expires_in: raw.expires_in,
        issued_at: unix_now_secs(),
    })
}

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
