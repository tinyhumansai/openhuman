//! HTTP client for TinyHumans / AlphaHuman API routes (`/auth/...`, etc.).

use anyhow::{Context, Result};
use base64::Engine;
use reqwest::header::AUTHORIZATION;
use reqwest::{Client, Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

use super::jwt::bearer_authorization_value;

fn build_backend_reqwest_client() -> Result<Client> {
    // Force rustls for consistent cross-platform TLS behavior.
    Client::builder()
        .use_rustls_tls()
        .http1_only()
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build HTTP client: {e}"))
}

fn parse_api_response_json(text: &str) -> Result<Value> {
    let v: Value = serde_json::from_str(text).with_context(|| format!("parse API JSON: {text}"))?;
    let Some(obj) = v.as_object() else {
        return Ok(v);
    };
    if let Some(success) = obj.get("success").and_then(|x| x.as_bool()) {
        if !success {
            let msg = obj
                .get("message")
                .or_else(|| obj.get("error"))
                .and_then(|x| x.as_str())
                .unwrap_or("request unsuccessful");
            anyhow::bail!("API request failed: {msg}");
        }
        if let Some(data) = obj.get("data") {
            if !data.is_null() {
                return Ok(data.clone());
            }
        }
        if let Some(user) = obj.get("user") {
            if !user.is_null() {
                return Ok(user.clone());
            }
        }
        let mut m = obj.clone();
        m.remove("success");
        return Ok(Value::Object(m));
    }
    Ok(v)
}

fn user_id_from_object(obj: &serde_json::Map<String, Value>) -> Option<String> {
    for key in ["id", "_id", "userId"] {
        if let Some(s) = obj.get(key).and_then(|x| x.as_str()) {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

/// Best-effort extraction of a user ID from an authenticated profile payload.
///
/// This function handles various envelope formats, including raw user objects
/// or those nested under `data` or `user` keys.
pub fn user_id_from_profile_payload(payload: &Value) -> Option<String> {
    let obj = payload.as_object()?;
    if let Some(data) = obj.get("data").and_then(|v| v.as_object()) {
        return user_id_from_object(data).or_else(|| {
            data.get("user")
                .and_then(|u| u.as_object())
                .and_then(user_id_from_object)
        });
    }

    user_id_from_object(obj).or_else(|| {
        obj.get("user")
            .and_then(|u| u.as_object())
            .and_then(user_id_from_object)
    })
}

/// Alias for [`user_id_from_profile_payload`] for semantic clarity in auth flows.
pub fn user_id_from_auth_me_payload(payload: &Value) -> Option<String> {
    user_id_from_profile_payload(payload)
}

/// JSON body returned by the backend when an OAuth connection process is initiated.
#[derive(Debug, Clone, Deserialize)]
pub struct ConnectResponse {
    /// The URL to redirect the user to for OAuth authorization.
    pub oauth_url: String,
    /// The state parameter used to prevent CSRF and correlate the callback.
    pub state: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ConnectEnvelope {
    success: bool,
    #[serde(default, alias = "oauthUrl")]
    oauth_url: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct IntegrationsEnvelope {
    success: bool,
    data: IntegrationsData,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IntegrationsData {
    integrations: Vec<IntegrationSummary>,
}

/// A summary of an active integration, as returned by the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationSummary {
    /// Unique identifier for the integration.
    pub id: String,
    /// The name of the integration provider (e.g., "google", "slack").
    pub provider: String,
    /// RFC3339 timestamp of when the integration was created.
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TokensEnvelope {
    success: bool,
    data: TokensData,
}

#[derive(Debug, Clone, Deserialize)]
struct TokensData {
    encrypted: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LoginTokenConsumeEnvelope {
    success: bool,
    data: LoginTokenConsumeData,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginTokenConsumeData {
    jwt_token: String,
}

/// Decrypted OAuth token payload for handing off tokens to a local service or skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationTokensHandoff {
    /// The OAuth access token.
    pub access_token: String,
    /// The optional OAuth refresh token.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// RFC3339 timestamp of when the access token expires.
    pub expires_at: String,
}

/// A client for interacting with the TinyHumans / AlphaHuman backend API.
#[derive(Clone)]
pub struct BackendOAuthClient {
    client: Client,
    base: Url,
}

impl BackendOAuthClient {
    /// Creates a new `BackendOAuthClient` with the given API base URL.
    pub fn new(api_base: &str) -> Result<Self> {
        let base = Url::parse(api_base.trim()).context("Invalid API base URL")?;
        let client = build_backend_reqwest_client()?;
        Ok(Self { client, base })
    }

    /// Returns the URL for initiating a login flow for a specific provider.
    pub fn login_url(&self, provider: &str) -> Result<Url> {
        let p = provider.trim().trim_matches('/');
        anyhow::ensure!(!p.is_empty(), "provider is required");
        self.base
            .join(&format!("auth/{p}/login"))
            .context("build login URL")
    }

    /// Initiates an OAuth connection flow for the current user and a specific provider.
    pub async fn connect(
        &self,
        provider: &str,
        bearer_jwt: &str,
        skill_id: Option<&str>,
        response_type: Option<&str>,
        encryption_mode: Option<&str>,
    ) -> Result<ConnectResponse> {
        let p = provider.trim().trim_matches('/');
        anyhow::ensure!(!p.is_empty(), "provider is required");
        let mut url = self
            .base
            .join(&format!("auth/{p}/connect"))
            .context("build connect URL")?;
        if let Some(s) = skill_id.filter(|s| !s.is_empty()) {
            url.query_pairs_mut().append_pair("skillId", s);
        }
        if let Some(r) = response_type.filter(|r| !r.is_empty()) {
            url.query_pairs_mut().append_pair("responseType", r);
        }
        if let Some(e) = encryption_mode.filter(|e| !e.is_empty()) {
            url.query_pairs_mut().append_pair("encryptionMode", e);
        }

        let resp = self
            .client
            .get(url)
            .header(AUTHORIZATION, bearer_authorization_value(bearer_jwt))
            .send()
            .await
            .context("auth connect request")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("auth connect failed ({status}): {text}");
        }

        let env: ConnectEnvelope =
            serde_json::from_str(&text).with_context(|| format!("parse connect JSON: {text}"))?;
        if !env.success {
            anyhow::bail!("auth connect unsuccessful: {text}");
        }
        let oauth_url = env
            .oauth_url
            .filter(|u| !u.is_empty())
            .context("missing oauthUrl in response")?;
        let state = env
            .state
            .filter(|s| !s.is_empty())
            .context("missing state")?;
        Ok(ConnectResponse { oauth_url, state })
    }

    /// Fetches the current authenticated user profile using the provided JWT.
    pub async fn fetch_current_user(&self, bearer_jwt: &str) -> Result<Value> {
        let url = self.base.join("auth/me").context("build /auth/me URL")?;
        let resp = self
            .client
            .get(url)
            .header(AUTHORIZATION, bearer_authorization_value(bearer_jwt))
            .send()
            .await
            .context("GET /auth/me")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("GET /auth/me failed ({status}): {text}");
        }
        parse_api_response_json(&text)
    }

    /// Exchanges a one-time login token (e.g. from Telegram) for a long-lived JWT.
    pub async fn consume_login_token(&self, login_token: &str) -> Result<String> {
        let token = login_token.trim();
        anyhow::ensure!(!token.is_empty(), "login token is required");

        let url = self
            .base
            .join(&format!(
                "telegram/login-tokens/{}/consume",
                urlencoding::encode(token)
            ))
            .context("build login-token consume URL")?;

        let resp = self
            .client
            .post(url)
            .send()
            .await
            .context("consume login token")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("consume login token failed ({status}): {text}");
        }

        let env: LoginTokenConsumeEnvelope = serde_json::from_str(&text)
            .with_context(|| format!("parse consume-login-token JSON: {text}"))?;
        if !env.success {
            anyhow::bail!("consume login token unsuccessful: {text}");
        }

        let jwt = env.data.jwt_token.trim().to_string();
        anyhow::ensure!(
            !jwt.is_empty(),
            "consume login token response missing jwtToken"
        );
        Ok(jwt)
    }

    /// Validates that the provided session token is still active and accepted.
    pub async fn validate_session_token(&self, bearer_jwt: &str) -> Result<()> {
        let _ = self.fetch_current_user(bearer_jwt).await?;
        Ok(())
    }

    /// Creates a short-lived link token for connecting a specific communication channel.
    pub async fn create_channel_link_token(
        &self,
        channel: &str,
        bearer_jwt: &str,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        let encoded_channel = urlencoding::encode(channel);

        let url = self
            .base
            .join(&format!("auth/channels/{encoded_channel}/link-token"))
            .context("build channel link-token URL")?;

        let resp = self
            .client
            .post(url)
            .header(AUTHORIZATION, bearer_authorization_value(bearer_jwt))
            .send()
            .await
            .context("create channel link token")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("create channel link token failed ({status}): {text}");
        }

        parse_api_response_json(&text)
    }

    /// Generic authenticated JSON request helper for backend API routes.
    pub async fn authed_json(
        &self,
        bearer_jwt: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let url = self
            .base
            .join(path.trim_start_matches('/'))
            .with_context(|| format!("build URL for {path}"))?;

        let mut request = self
            .client
            .request(method.clone(), url.clone())
            .header(AUTHORIZATION, bearer_authorization_value(bearer_jwt));

        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("backend request {} {}", method.as_str(), url.path()))?;

        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!(
                "{} {} failed ({status}): {text}",
                method.as_str(),
                url.path()
            );
        }

        parse_api_response_json(&text)
    }

    /// Lists all active integrations for the current user.
    pub async fn list_integrations(&self, bearer_jwt: &str) -> Result<Vec<IntegrationSummary>> {
        let url = self
            .base
            .join("auth/integrations")
            .context("build integrations URL")?;
        let resp = self
            .client
            .get(url)
            .header(AUTHORIZATION, bearer_authorization_value(bearer_jwt))
            .send()
            .await
            .context("list integrations")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("list integrations failed ({status}): {text}");
        }
        let env: IntegrationsEnvelope = serde_json::from_str(&text)
            .with_context(|| format!("parse integrations JSON: {text}"))?;
        if !env.success {
            anyhow::bail!("list integrations unsuccessful: {text}");
        }
        Ok(env.data.integrations)
    }

    /// Fetches the decrypted OAuth tokens for a specific integration.
    ///
    /// This is a one-time handoff process. The encryption key must match the
    /// one used by the backend to encrypt the tokens.
    pub async fn fetch_integration_tokens_handoff(
        &self,
        integration_id: &str,
        bearer_jwt: &str,
        encryption_key: &str,
    ) -> Result<IntegrationTokensHandoff> {
        let id = integration_id.trim();
        anyhow::ensure!(
            !id.is_empty() && id.len() == 24,
            "integrationId must be a 24-char hex id"
        );
        let url = self
            .base
            .join(&format!("auth/integrations/{id}/tokens"))
            .context("build tokens URL")?;
        let body = serde_json::json!({ "key": encryption_key.trim() });
        let resp = self
            .client
            .post(url)
            .header(AUTHORIZATION, bearer_authorization_value(bearer_jwt))
            .json(&body)
            .send()
            .await
            .context("integration tokens handoff")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("integration tokens failed ({status}): {text}");
        }
        let env: TokensEnvelope =
            serde_json::from_str(&text).with_context(|| format!("parse tokens JSON: {text}"))?;
        if !env.success {
            anyhow::bail!("integration tokens unsuccessful: {text}");
        }
        let plaintext = decrypt_handoff_blob(&env.data.encrypted, encryption_key.trim())?;
        serde_json::from_str(&plaintext).context("parse decrypted token JSON")
    }

    /// Fetches the client key share for a specific integration.
    ///
    /// This is a one-time handoff; the key is deleted from the backend's
    /// temporary storage (Redis) after retrieval.
    pub async fn fetch_client_key(&self, integration_id: &str, bearer_jwt: &str) -> Result<String> {
        let id = integration_id.trim();
        anyhow::ensure!(
            !id.is_empty() && id.len() == 24,
            "integrationId must be a 24-char hex id"
        );
        let url = self
            .base
            .join(&format!("auth/integrations/{id}/client-key"))
            .context("build client-key URL")?;
        let resp = self
            .client
            .post(url)
            .header(AUTHORIZATION, bearer_authorization_value(bearer_jwt))
            .send()
            .await
            .context("fetch client key")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("fetch client key failed ({status}): {text}");
        }
        let v: Value = serde_json::from_str(&text)
            .with_context(|| format!("parse client-key JSON: {text}"))?;
        let obj = v.as_object().context("expected JSON object")?;
        let success = obj
            .get("success")
            .and_then(|s| s.as_bool())
            .unwrap_or(false);
        if !success {
            let msg = obj
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("client key retrieval unsuccessful");
            anyhow::bail!("fetch client key failed: {msg}");
        }
        let client_key = obj
            .get("data")
            .and_then(|d| d.get("clientKey"))
            .and_then(|k| k.as_str())
            .context("missing data.clientKey in response")?;
        Ok(client_key.to_string())
    }

    /// Sends a message to a communication channel.
    pub async fn send_channel_message(
        &self,
        channel: &str,
        bearer_jwt: &str,
        message_body: Value,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        let encoded = urlencoding::encode(channel);
        self.authed_json(
            bearer_jwt,
            Method::POST,
            &format!("channels/{encoded}/messages"),
            Some(message_body),
        )
        .await
    }

    /// Signals "the agent is typing…" on a channel that supports it
    /// (Telegram's `sendChatAction`, Slack's typing event, …). The backend
    /// resolves the target chat from the channel integration metadata and
    /// is responsible for hitting the provider-native API.
    ///
    /// Telegram keeps the typing indicator alive for ~5 seconds per call,
    /// so callers should re-invoke every ~4 s for as long as the turn is
    /// in flight. Returns `Err` if the backend doesn't support typing for
    /// this channel — caller should swallow the error silently.
    pub async fn send_channel_typing(&self, channel: &str, bearer_jwt: &str) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        let encoded = urlencoding::encode(channel);
        self.authed_json(
            bearer_jwt,
            Method::POST,
            &format!("channels/{encoded}/typing"),
            Some(json!({})),
        )
        .await
    }

    /// Edits an existing channel message. Used by the progressive-edit
    /// streaming path (Telegram / Slack) to coalesce live deltas into a
    /// single evolving outbound message rather than spamming the chat
    /// with one bubble per token.
    ///
    /// `message_id` is the backend-returned id of the message that was
    /// first sent via [`Self::send_channel_message`]. Returns the
    /// updated message record, or an `Err` if the backend does not
    /// support editing for this channel (caller should fall back to
    /// atomic-final delivery).
    pub async fn send_channel_edit(
        &self,
        channel: &str,
        message_id: &str,
        bearer_jwt: &str,
        edit_body: Value,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        anyhow::ensure!(!message_id.is_empty(), "message_id is required");
        let encoded_channel = urlencoding::encode(channel);
        let encoded_id = urlencoding::encode(message_id);
        self.authed_json(
            bearer_jwt,
            Method::PATCH,
            &format!("channels/{encoded_channel}/messages/{encoded_id}"),
            Some(edit_body),
        )
        .await
    }

    /// Sends a reaction (e.g. emoji) to a message in a channel.
    pub async fn send_channel_reaction(
        &self,
        channel: &str,
        bearer_jwt: &str,
        reaction_body: Value,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        let encoded = urlencoding::encode(channel);
        self.authed_json(
            bearer_jwt,
            Method::POST,
            &format!("channels/{encoded}/reactions"),
            Some(reaction_body),
        )
        .await
    }

    /// Searches for GIFs using the Tenor integration.
    pub async fn search_tenor_gifs(
        &self,
        bearer_jwt: &str,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Value> {
        anyhow::ensure!(!query.trim().is_empty(), "query is required");
        let body = serde_json::json!({
            "query": query.trim(),
            "limit": limit.unwrap_or(5),
            "contentFilter": "medium",
        });
        self.authed_json(
            bearer_jwt,
            Method::POST,
            "agent-integrations/tenor/search",
            Some(body),
        )
        .await
    }

    /// Creates a new thread in a communication channel.
    pub async fn create_channel_thread(
        &self,
        channel: &str,
        bearer_jwt: &str,
        title: &str,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        anyhow::ensure!(!title.trim().is_empty(), "title is required");
        let encoded = urlencoding::encode(channel);
        let body = serde_json::json!({ "title": title.trim() });
        self.authed_json(
            bearer_jwt,
            Method::POST,
            &format!("channels/{encoded}/threads"),
            Some(body),
        )
        .await
    }

    /// Updates an existing thread (e.g., closing or reopening it).
    pub async fn update_channel_thread(
        &self,
        channel: &str,
        bearer_jwt: &str,
        thread_id: &str,
        action: &str,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        anyhow::ensure!(!thread_id.trim().is_empty(), "threadId is required");
        anyhow::ensure!(
            action == "close" || action == "reopen",
            "action must be 'close' or 'reopen'"
        );
        let encoded_channel = urlencoding::encode(channel);
        let encoded_thread = urlencoding::encode(thread_id.trim());
        let body = serde_json::json!({ "action": action });
        self.authed_json(
            bearer_jwt,
            Method::PATCH,
            &format!("channels/{encoded_channel}/threads/{encoded_thread}"),
            Some(body),
        )
        .await
    }

    /// Lists threads in a communication channel, optionally filtering by status.
    pub async fn list_channel_threads(
        &self,
        channel: &str,
        bearer_jwt: &str,
        active_filter: Option<bool>,
    ) -> Result<Value> {
        let channel = channel.trim().trim_matches('/');
        anyhow::ensure!(!channel.is_empty(), "channel is required");
        let encoded = urlencoding::encode(channel);
        let mut path = format!("channels/{encoded}/threads");
        if let Some(active) = active_filter {
            path.push_str(if active {
                "?active=true"
            } else {
                "?active=false"
            });
        }
        self.authed_json(bearer_jwt, Method::GET, &path, None).await
    }

    /// Revokes (deletes) an active integration.
    pub async fn revoke_integration(&self, integration_id: &str, bearer_jwt: &str) -> Result<()> {
        let id = integration_id.trim();
        anyhow::ensure!(!id.is_empty(), "integration id is required");
        let url = self
            .base
            .join(&format!("auth/integrations/{id}"))
            .context("build revoke URL")?;
        let resp = self
            .client
            .delete(url)
            .header(AUTHORIZATION, bearer_authorization_value(bearer_jwt))
            .send()
            .await
            .context("revoke integration")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("revoke integration failed ({status}): {text}");
        }
        Ok(())
    }
}

/// AES-256-GCM decrypt compatible with backend `encryptMessageFromString` (IV 16 + tag 16 + ciphertext, base64).
pub fn decrypt_handoff_blob(b64_ciphertext: &str, key_str: &str) -> Result<String> {
    let key = key_bytes_from_string(key_str)?;
    let combined = base64::engine::general_purpose::STANDARD
        .decode(b64_ciphertext.trim())
        .context("base64-decode encrypted payload")?;
    if combined.len() < 32 {
        anyhow::bail!("encrypted payload too short");
    }
    let iv = &combined[0..16];
    let tag = &combined[16..32];
    let ciphertext = &combined[32..];

    // aes-gcm expects ciphertext || tag
    let mut ct_with_tag = Vec::with_capacity(ciphertext.len() + tag.len());
    ct_with_tag.extend_from_slice(ciphertext);
    ct_with_tag.extend_from_slice(tag);

    use aes_gcm::aead::generic_array::typenum::U16;
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::aes::Aes256;
    use aes_gcm::AesGcm;
    type Aes256Gcm16 = AesGcm<Aes256, U16>;

    let cipher =
        Aes256Gcm16::new_from_slice(&key).map_err(|e| anyhow::anyhow!("invalid AES key: {e}"))?;
    let nonce = aes_gcm::aead::generic_array::GenericArray::from_slice(iv);
    let plain = cipher
        .decrypt(nonce, ct_with_tag.as_ref())
        .map_err(|e| anyhow::anyhow!("AES-GCM decrypt failed: {e}"))?;

    String::from_utf8(plain).context("handoff plaintext is not UTF-8")
}

/// Decode the shared encryption key into 32 raw AES bytes.
///
/// Accepts, in order of preference:
/// 1. base64url without padding — the current backend format (e.g.
///    a 43-char alphanumeric string using `-` / `_`). This must be tried
///    BEFORE standard base64 because `-`/`_` are invalid in the standard
///    alphabet and would fail cleanly, whereas a standard-base64 string
///    never contains `-`/`_` so base64url_no_pad will still decode it
///    correctly as long as there's no padding.
/// 2. base64url with padding.
/// 3. Standard base64 with padding (legacy backend format).
/// 4. Standard base64 without padding.
/// 5. A raw 32-byte ASCII key (len == 32, used as-is).
///
/// NOTE: the key is only decoded locally for AES-GCM key material in
/// `decrypt_handoff_blob`. The key sent back to the backend (in the
/// `{ key: ... }` POST body of `fetch_integration_tokens_handoff`) is the
/// original string — never re-encoded — so base64url keys stay base64url
/// on the wire.
fn key_bytes_from_string(key: &str) -> Result<Vec<u8>> {
    let trimmed = key.trim();

    // Raw 32-byte ASCII key
    if trimmed.len() == 32 && !trimmed.contains(['+', '/', '-', '_', '=']) {
        return Ok(trimmed.as_bytes().to_vec());
    }

    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};

    // `base64::Engine` has generic methods and therefore isn't
    // dyn-compatible, so we unroll the attempts instead of looping over
    // a slice of trait objects.
    macro_rules! try_decode {
        ($engine:expr) => {
            if let Ok(decoded) = $engine.decode(trimmed) {
                if decoded.len() == 32 {
                    return Ok(decoded);
                }
            }
        };
    }
    try_decode!(URL_SAFE_NO_PAD);
    try_decode!(URL_SAFE);
    try_decode!(STANDARD);
    try_decode!(STANDARD_NO_PAD);

    anyhow::bail!(
        "encryption key must decode to 32 raw bytes (raw, base64, or base64url accepted; got len={})",
        trimmed.len()
    );
}

#[cfg(test)]
mod key_bytes_from_string_tests {
    use super::key_bytes_from_string;
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    use base64::Engine;

    #[test]
    fn decodes_base64url_no_pad() {
        // A 32-byte key that, when base64url-encoded, contains both `-` and `_`.
        let raw = [
            0xff_u8, 0xfb, 0xef, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa,
            0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09,
            0x0a, 0x0b, 0x0c, 0x0d,
        ];
        let url_key = URL_SAFE_NO_PAD.encode(raw);
        assert!(url_key.contains('-') || url_key.contains('_'));
        let decoded = key_bytes_from_string(&url_key).unwrap();
        assert_eq!(decoded, raw);
    }

    #[test]
    fn decodes_standard_base64() {
        let raw = [0x41_u8; 32];
        let std_key = STANDARD.encode(raw);
        let decoded = key_bytes_from_string(&std_key).unwrap();
        assert_eq!(decoded, raw);
    }

    #[test]
    fn decodes_raw_32_byte_key() {
        let raw = "abcdefghijklmnopqrstuvwxyz012345";
        assert_eq!(raw.len(), 32);
        let decoded = key_bytes_from_string(raw).unwrap();
        assert_eq!(decoded, raw.as_bytes());
    }

    #[test]
    fn trims_whitespace() {
        let raw = [0x42_u8; 32];
        let url_key = format!("  {}\n", URL_SAFE_NO_PAD.encode(raw));
        let decoded = key_bytes_from_string(&url_key).unwrap();
        assert_eq!(decoded, raw);
    }

    #[test]
    fn rejects_wrong_length() {
        let err = key_bytes_from_string("tooshort").unwrap_err();
        assert!(err.to_string().contains("must decode to 32 raw bytes"));
    }
}
