//! HTTP client for backend OAuth routes (`/auth/...`) and one-time token handoff decryption.

use anyhow::{Context, Result};
use base64::Engine;
use reqwest::header::AUTHORIZATION;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};

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
        Ok(Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| Client::new()),
            base,
        })
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

        let resp = self
            .client
            .get(url)
            .header(
                AUTHORIZATION,
                format!("Bearer {}", bearer_jwt.trim()),
            )
            .send()
            .await
            .context("auth connect request")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("auth connect failed ({status}): {text}");
        }

        let env: ConnectEnvelope = serde_json::from_str(&text)
            .with_context(|| format!("parse connect JSON: {text}"))?;
        if !env.success {
            anyhow::bail!("auth connect unsuccessful: {text}");
        }
        let oauth_url = env
            .oauth_url
            .filter(|u| !u.is_empty())
            .context("missing oauthUrl in response")?;
        let state = env.state.filter(|s| !s.is_empty()).context("missing state")?;
        Ok(ConnectResponse { oauth_url, state })
    }

    /// `GET /auth/integrations`
    pub async fn list_integrations(
        &self,
        bearer_jwt: &str,
    ) -> Result<Vec<IntegrationSummary>> {
        let url = self
            .base
            .join("auth/integrations")
            .context("build integrations URL")?;
        let resp = self
            .client
            .get(url)
            .header(
                AUTHORIZATION,
                format!("Bearer {}", bearer_jwt.trim()),
            )
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
            .header(
                AUTHORIZATION,
                format!("Bearer {}", bearer_jwt.trim()),
            )
            .json(&body)
            .send()
            .await
            .context("integration tokens handoff")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("integration tokens failed ({status}): {text}");
        }
        let env: TokensEnvelope = serde_json::from_str(&text)
            .with_context(|| format!("parse tokens JSON: {text}"))?;
        if !env.success {
            anyhow::bail!("integration tokens unsuccessful: {text}");
        }
        let plaintext = decrypt_handoff_blob(&env.data.encrypted, encryption_key.trim())?;
        serde_json::from_str(&plaintext).context("parse decrypted token JSON")
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
            .header(
                AUTHORIZATION,
                format!("Bearer {}", bearer_jwt.trim()),
            )
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

    let plain = openssl::symm::decrypt_aead(
        openssl::symm::Cipher::aes_256_gcm(),
        &key,
        Some(iv),
        &[],
        ciphertext,
        tag,
    )
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
    anyhow::bail!(
        "encryption key must be 32 raw bytes or 44-char base64 (decoded to 32 bytes)"
    );
}
