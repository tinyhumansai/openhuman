//! Zhipu GLM provider with JWT authentication.
//! The GLM API requires JWT tokens generated from the `id.secret` API key format
//! with a custom `sign_type: "SIGN"` header, and uses `/v4/chat/completions`.

use crate::openhuman::providers::traits::{ChatMessage, Provider};
use async_trait::async_trait;
use reqwest::Client;
use ring::hmac;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct GlmProvider {
    api_key_id: String,
    api_key_secret: String,
    base_url: String,
    /// Cached JWT token + expiry timestamp (ms)
    token_cache: Mutex<Option<(String, u64)>>,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    temperature: f64,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: String,
}

/// Base64url encode without padding (per JWT spec).
fn base64url_encode_bytes(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i] as u32;
        let b1 = if i + 1 < data.len() { data[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < data.len() { data[i + 2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);

        if i + 1 < data.len() {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        }
        if i + 2 < data.len() {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        }

        i += 3;
    }

    // Convert to base64url: replace + with -, / with _, strip =
    result.replace('+', "-").replace('/', "_")
}

fn base64url_encode_str(s: &str) -> String {
    base64url_encode_bytes(s.as_bytes())
}

impl GlmProvider {
    pub fn new(api_key: Option<&str>) -> Self {
        let (id, secret) = api_key
            .and_then(|k| k.split_once('.'))
            .map(|(id, secret)| (id.to_string(), secret.to_string()))
            .unwrap_or_default();

        Self {
            api_key_id: id,
            api_key_secret: secret,
            base_url: "https://api.z.ai/api/paas/v4".to_string(),
            token_cache: Mutex::new(None),
        }
    }

    fn generate_token(&self) -> anyhow::Result<String> {
        if self.api_key_id.is_empty() || self.api_key_secret.is_empty() {
            anyhow::bail!(
                "GLM API key not set or invalid format. Expected 'id.secret'. \
                 Configure via the web UI or set GLM_API_KEY env var."
            );
        }

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_millis() as u64;

        // Check cache (valid for 3 minutes, token expires at 3.5 min)
        if let Ok(cache) = self.token_cache.lock() {
            if let Some((ref token, expiry)) = *cache {
                if now_ms < expiry {
                    return Ok(token.clone());
                }
            }
        }

        let exp_ms = now_ms + 210_000; // 3.5 minutes

        // Build JWT manually to include custom sign_type header
        // Header: {"alg":"HS256","typ":"JWT","sign_type":"SIGN"}
        let header_json = r#"{"alg":"HS256","typ":"JWT","sign_type":"SIGN"}"#;
        let header_b64 = base64url_encode_str(header_json);

        // Payload: {"api_key":"...","exp":...,"timestamp":...}
        let payload_json = format!(
            r#"{{"api_key":"{}","exp":{},"timestamp":{}}}"#,
            self.api_key_id, exp_ms, now_ms
        );
        let payload_b64 = base64url_encode_str(&payload_json);

        // Sign: HMAC-SHA256(header.payload, secret)
        let signing_input = format!("{header_b64}.{payload_b64}");
        let key = hmac::Key::new(hmac::HMAC_SHA256, self.api_key_secret.as_bytes());
        let signature = hmac::sign(&key, signing_input.as_bytes());
        let sig_b64 = base64url_encode_bytes(signature.as_ref());

        let token = format!("{signing_input}.{sig_b64}");

        // Cache for 3 minutes
        if let Ok(mut cache) = self.token_cache.lock() {
            *cache = Some((token.clone(), now_ms + 180_000));
        }

        Ok(token)
    }

    fn http_client(&self) -> Client {
        crate::openhuman::config::build_runtime_proxy_client_with_timeouts("provider.glm", 120, 10)
    }
}

#[async_trait]
impl Provider for GlmProvider {
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let token = self.generate_token()?;

        let mut messages = Vec::new();

        if let Some(sys) = system_prompt {
            messages.push(Message {
                role: "system".to_string(),
                content: sys.to_string(),
            });
        }

        messages.push(Message {
            role: "user".to_string(),
            content: message.to_string(),
        });

        let request = ChatRequest {
            model: model.to_string(),
            messages,
            temperature,
        };

        let url = format!("{}/chat/completions", self.base_url);

        let response = self
            .http_client()
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await?;
            anyhow::bail!("GLM API error: {error}");
        }

        let chat_response: ChatResponse = response.json().await?;

        chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow::anyhow!("No response from GLM"))
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let token = self.generate_token()?;

        let api_messages: Vec<Message> = messages
            .iter()
            .map(|m| Message {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let request = ChatRequest {
            model: model.to_string(),
            messages: api_messages,
            temperature,
        };

        let url = format!("{}/chat/completions", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await?;
            anyhow::bail!("GLM API error: {error}");
        }

        let chat_response: ChatResponse = response.json().await?;

        chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow::anyhow!("No response from GLM"))
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        if self.api_key_id.is_empty() || self.api_key_secret.is_empty() {
            return Ok(());
        }

        // Generate and cache a JWT token, establishing TLS to the GLM API.
        let token = self.generate_token()?;
        let url = format!("{}/chat/completions", self.base_url);
        // GET will likely return 405 but establishes the TLS + HTTP/2 connection pool.
        let _ = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_api_key() {
        let p = GlmProvider::new(Some("abc123.secretXYZ"));
        assert_eq!(p.api_key_id, "abc123");
        assert_eq!(p.api_key_secret, "secretXYZ");
    }

    #[test]
    fn handles_no_key() {
        let p = GlmProvider::new(None);
        assert!(p.api_key_id.is_empty());
        assert!(p.api_key_secret.is_empty());
    }

    #[test]
    fn handles_invalid_key_format() {
        let p = GlmProvider::new(Some("no-dot-here"));
        assert!(p.api_key_id.is_empty());
        assert!(p.api_key_secret.is_empty());
    }

    #[test]
    fn generates_jwt_token() {
        let p = GlmProvider::new(Some("testid.testsecret"));
        let token = p.generate_token().unwrap();
        assert!(!token.is_empty());
        // JWT has 3 dot-separated parts
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "JWT should have 3 parts: {token}");
    }

    #[test]
    fn caches_token() {
        let p = GlmProvider::new(Some("testid.testsecret"));
        let token1 = p.generate_token().unwrap();
        let token2 = p.generate_token().unwrap();
        assert_eq!(token1, token2, "Cached token should be reused");
    }

    #[test]
    fn fails_without_key() {
        let p = GlmProvider::new(None);
        let result = p.generate_token();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("API key not set"));
    }

    #[tokio::test]
    async fn chat_fails_without_key() {
        let p = GlmProvider::new(None);
        let result = p
            .chat_with_system(None, "hello", "glm-4.7", 0.7)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn chat_with_history_fails_without_key() {
        let p = GlmProvider::new(None);
        let messages = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there!"),
            ChatMessage::user("What did I say?"),
        ];
        let result = p.chat_with_history(&messages, "glm-4.7", 0.7).await;
        assert!(result.is_err());
    }

    #[test]
    fn base64url_no_padding() {
        let encoded = base64url_encode_bytes(b"hello");
        assert!(!encoded.contains('='));
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
    }

    #[tokio::test]
    async fn warmup_without_key_is_noop() {
        let provider = GlmProvider::new(None);
        let result = provider.warmup().await;
        assert!(result.is_ok());
    }
}
