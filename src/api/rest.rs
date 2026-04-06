//! HTTP client for TinyHumans / AlphaHuman API routes (`/auth/...`, etc.).

use anyhow::{Context, Result};
use base64::Engine;
use reqwest::header::AUTHORIZATION;
use reqwest::{Client, Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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

/// Best-effort user id from an authenticated profile payload.
///
/// Accepts a raw user object or an envelope that nests the user under `data`
/// or `user`.
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

pub fn user_id_from_auth_me_payload(payload: &Value) -> Option<String> {
    user_id_from_profile_payload(payload)
}

/// JSON body returned by the backend after OAuth connect starts.
#[derive(Debug, Clone, Deserialize)]
pub struct ConnectResponse {
    pub oauth_url: String,
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

/// Integration row from `GET /auth/integrations` (no tokens).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationSummary {
    pub id: String,
    pub provider: String,
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

/// Decrypted OAuth token payload from `POST /auth/integrations/:id/tokens`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationTokensHandoff {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    pub expires_at: String,
}

#[derive(Clone)]
pub struct BackendOAuthClient {
    client: Client,
    base: Url,
}

impl BackendOAuthClient {
    pub fn new(api_base: &str) -> Result<Self> {
        let base = Url::parse(api_base.trim()).context("Invalid API base URL")?;
        let client = build_backend_reqwest_client()?;
        Ok(Self { client, base })
    }

    /// `GET /auth/{provider}/login` — open in browser; Origin/Referer must be allowlisted on the server.
    pub fn login_url(&self, provider: &str) -> Result<Url> {
        let p = provider.trim().trim_matches('/');
        anyhow::ensure!(!p.is_empty(), "provider is required");
        self.base
            .join(&format!("auth/{p}/login"))
            .context("build login URL")
    }

    /// `GET /auth/{provider}/connect` with Bearer JWT.
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

    /// `GET /auth/me` — current authenticated user profile for the Bearer session JWT.
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

    /// `POST /telegram/login-tokens/:token/consume` — exchange a one-time login token for a JWT.
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

    /// Confirms the JWT is accepted by the API using `GET /auth/me`.
    pub async fn validate_session_token(&self, bearer_jwt: &str) -> Result<()> {
        let _ = self.fetch_current_user(bearer_jwt).await?;
        Ok(())
    }

    /// `POST /auth/channels/:channel/link-token` — create a short-lived channel link token.
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

    /// Generic authenticated JSON request helper for backend API routes that
    /// follow the standard `{ success, data, message }` envelope.
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

    /// `GET /auth/integrations`
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

    /// `POST /auth/integrations/:id/tokens` — one-time handoff; decrypt with same key format as backend.
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

    /// `POST /auth/integrations/:id/client-key` — one-time handoff of client key share (deleted from Redis after retrieval).
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

    /// `POST /channels/:channel/messages` — Send a rich message to a channel.
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

    /// `POST /channels/:channel/reactions` — React to a message in a channel.
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

    /// `POST /agent-integrations/tenor/search` — Search for GIFs via Tenor.
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

    /// `POST /channels/:channel/threads` — Create a thread in a channel.
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

    /// `PATCH /channels/:channel/threads/:thread_id` — Close or reopen a thread.
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

    /// `GET /channels/:channel/threads` — List threads, optionally filtered by active status.
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

    /// `DELETE /auth/integrations/:id`
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

fn key_bytes_from_string(key: &str) -> Result<Vec<u8>> {
    if key.len() == 44 {
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(key) {
            if decoded.len() == 32 {
                return Ok(decoded);
            }
        }
    }
    if key.len() == 32 {
        return Ok(key.as_bytes().to_vec());
    }
    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(key) {
        if decoded.len() == 32 {
            return Ok(decoded);
        }
    }
    anyhow::bail!("encryption key must be 32 raw bytes or 44-char base64 (decoded to 32 bytes)");
}
