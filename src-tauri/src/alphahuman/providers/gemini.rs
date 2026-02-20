//! Google Gemini provider with support for:
//! - Direct API key (`GEMINI_API_KEY` env var or config)
//! - Gemini app OAuth tokens (reuse existing ~/.gemini/ authentication)
//! - Google Cloud ADC (`GOOGLE_APPLICATION_CREDENTIALS`)

use crate::alphahuman::providers::traits::{ChatMessage, Provider};
use async_trait::async_trait;
use directories::UserDirs;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Gemini provider supporting multiple authentication methods.
pub struct GeminiProvider {
    auth: Option<GeminiAuth>,
}

/// Resolved credential — the variant determines both the HTTP auth method
/// and the diagnostic label returned by `auth_source()`.
#[derive(Debug)]
enum GeminiAuth {
    /// Explicit API key from config: sent as `?key=` query parameter.
    ExplicitKey(String),
    /// API key from `GEMINI_API_KEY` env var: sent as `?key=`.
    EnvGeminiKey(String),
    /// API key from `GOOGLE_API_KEY` env var: sent as `?key=`.
    EnvGoogleKey(String),
    /// OAuth access token from Gemini app: sent as `Authorization: Bearer`.
    OAuthToken(String),
}

impl GeminiAuth {
    /// Whether this credential is an API key (sent as `?key=` query param).
    fn is_api_key(&self) -> bool {
        matches!(
            self,
            GeminiAuth::ExplicitKey(_) | GeminiAuth::EnvGeminiKey(_) | GeminiAuth::EnvGoogleKey(_)
        )
    }

    /// Whether this credential is an OAuth token from Gemini app.
    fn is_oauth(&self) -> bool {
        matches!(self, GeminiAuth::OAuthToken(_))
    }

    /// The raw credential string.
    fn credential(&self) -> &str {
        match self {
            GeminiAuth::ExplicitKey(s)
            | GeminiAuth::EnvGeminiKey(s)
            | GeminiAuth::EnvGoogleKey(s)
            | GeminiAuth::OAuthToken(s) => s,
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// API REQUEST/RESPONSE TYPES
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Serialize)]
struct GenerateContentRequest {
    contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<Content>,
    #[serde(rename = "generationConfig")]
    generation_config: GenerationConfig,
}

/// Request envelope for the internal cloudcode-pa API.
/// OAuth tokens from Gemini app are scoped for this endpoint.
#[derive(Debug, Serialize)]
struct InternalGenerateContentRequest {
    model: String,
    #[serde(rename = "generationConfig")]
    generation_config: GenerationConfig,
    contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<Content>,
}

#[derive(Debug, Serialize)]
struct Content {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<Part>,
}

#[derive(Debug, Serialize)]
struct Part {
    text: String,
}

#[derive(Debug, Serialize, Clone)]
struct GenerationConfig {
    temperature: f64,
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct GenerateContentResponse {
    candidates: Option<Vec<Candidate>>,
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    content: CandidateContent,
}

#[derive(Debug, Deserialize)]
struct CandidateContent {
    parts: Vec<ResponsePart>,
}

#[derive(Debug, Deserialize)]
struct ResponsePart {
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    message: String,
}

// ══════════════════════════════════════════════════════════════════════════════
// GEMINI TOKEN STRUCTURES
// ══════════════════════════════════════════════════════════════════════════════

/// OAuth token stored by Gemini app in `~/.gemini/oauth_creds.json`
#[derive(Debug, Deserialize)]
struct GeminiCliOAuthCreds {
    access_token: Option<String>,
    expiry: Option<String>,
}

/// Internal API endpoint used by Gemini app for OAuth users.
/// See: https://github.com/google-gemini/gemini-cli/issues/19200
const CLOUDCODE_PA_ENDPOINT: &str = "https://cloudcode-pa.googleapis.com/v1internal";

/// Public API endpoint for API key users.
const PUBLIC_API_ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta";

impl GeminiProvider {
    /// Create a new Gemini provider.
    ///
    /// Authentication priority:
    /// 1. Explicit API key passed in
    /// 2. `GEMINI_API_KEY` environment variable
    /// 3. `GOOGLE_API_KEY` environment variable
    /// 4. Gemini app OAuth tokens (`~/.gemini/oauth_creds.json`)
    pub fn new(api_key: Option<&str>) -> Self {
        let resolved_auth = api_key
            .and_then(Self::normalize_non_empty)
            .map(GeminiAuth::ExplicitKey)
            .or_else(|| Self::load_non_empty_env("GEMINI_API_KEY").map(GeminiAuth::EnvGeminiKey))
            .or_else(|| Self::load_non_empty_env("GOOGLE_API_KEY").map(GeminiAuth::EnvGoogleKey))
            .or_else(|| Self::try_load_gemini_cli_token().map(GeminiAuth::OAuthToken));

        Self {
            auth: resolved_auth,
        }
    }

    fn normalize_non_empty(value: &str) -> Option<String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn load_non_empty_env(name: &str) -> Option<String> {
        std::env::var(name)
            .ok()
            .and_then(|value| Self::normalize_non_empty(&value))
    }

    /// Try to load OAuth access token from Gemini app's cached credentials.
    /// Location: `~/.gemini/oauth_creds.json`
    fn try_load_gemini_cli_token() -> Option<String> {
        let gemini_dir = Self::gemini_cli_dir()?;
        let creds_path = gemini_dir.join("oauth_creds.json");

        if !creds_path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&creds_path).ok()?;
        let creds: GeminiCliOAuthCreds = serde_json::from_str(&content).ok()?;

        // Check if token is expired (basic check)
        if let Some(ref expiry) = creds.expiry {
            if let Ok(expiry_time) = chrono::DateTime::parse_from_rfc3339(expiry) {
                if expiry_time < chrono::Utc::now() {
                    tracing::warn!(
                        "Gemini OAuth token expired — refresh authentication in the web UI"
                    );
                    return None;
                }
            }
        }

        creds
            .access_token
            .and_then(|token| Self::normalize_non_empty(&token))
    }

    /// Get the Gemini app config directory (~/.gemini)
    fn gemini_cli_dir() -> Option<PathBuf> {
        UserDirs::new().map(|u| u.home_dir().join(".gemini"))
    }

    /// Check if Gemini app is configured and has valid credentials
    pub fn has_cli_credentials() -> bool {
        Self::try_load_gemini_cli_token().is_some()
    }

    /// Check if any Gemini authentication is available
    pub fn has_any_auth() -> bool {
        Self::load_non_empty_env("GEMINI_API_KEY").is_some()
            || Self::load_non_empty_env("GOOGLE_API_KEY").is_some()
            || Self::has_cli_credentials()
    }

    /// Get authentication source description for diagnostics.
    /// Uses the stored enum variant — no env var re-reading at call time.
    pub fn auth_source(&self) -> &'static str {
        match self.auth.as_ref() {
            Some(GeminiAuth::ExplicitKey(_)) => "config",
            Some(GeminiAuth::EnvGeminiKey(_)) => "GEMINI_API_KEY env var",
            Some(GeminiAuth::EnvGoogleKey(_)) => "GOOGLE_API_KEY env var",
            Some(GeminiAuth::OAuthToken(_)) => "Gemini app OAuth",
            None => "none",
        }
    }

    fn format_model_name(model: &str) -> String {
        if model.starts_with("models/") {
            model.to_string()
        } else {
            format!("models/{model}")
        }
    }

    /// Build the API URL based on auth type.
    ///
    /// - API key users → public `generativelanguage.googleapis.com/v1beta`
    /// - OAuth users → internal `cloudcode-pa.googleapis.com/v1internal`
    ///
    /// The Gemini app OAuth tokens are scoped for the internal Code Assist API,
    /// not the public API. Sending them to the public endpoint results in
    /// "400 Bad Request: API key not valid" errors.
    /// See: https://github.com/google-gemini/gemini-cli/issues/19200
    fn build_generate_content_url(model: &str, auth: &GeminiAuth) -> String {
        match auth {
            GeminiAuth::OAuthToken(_) => {
                // OAuth tokens from Gemini app are scoped for the internal
                // Code Assist API. The model is passed in the request body,
                // not the URL path.
                format!("{CLOUDCODE_PA_ENDPOINT}:generateContent")
            }
            _ => {
                let model_name = Self::format_model_name(model);
                let base_url = format!("{PUBLIC_API_ENDPOINT}/{model_name}:generateContent");

                if auth.is_api_key() {
                    format!("{base_url}?key={}", auth.credential())
                } else {
                    base_url
                }
            }
        }
    }

    fn http_client(&self) -> Client {
        crate::alphahuman::config::build_runtime_proxy_client_with_timeouts("provider.gemini", 120, 10)
    }

    fn build_generate_content_request(
        &self,
        auth: &GeminiAuth,
        url: &str,
        request: &GenerateContentRequest,
        model: &str,
    ) -> reqwest::RequestBuilder {
        let req = self.http_client().post(url).json(request);
        match auth {
            GeminiAuth::OAuthToken(token) => {
                // Internal API expects the model in the request body envelope
                let internal_request = InternalGenerateContentRequest {
                    model: Self::format_model_name(model),
                    generation_config: request.generation_config.clone(),
                    contents: request
                        .contents
                        .iter()
                        .map(|c| Content {
                            role: c.role.clone(),
                            parts: c
                                .parts
                                .iter()
                                .map(|p| Part {
                                    text: p.text.clone(),
                                })
                                .collect(),
                        })
                        .collect(),
                    system_instruction: request.system_instruction.as_ref().map(|si| Content {
                        role: si.role.clone(),
                        parts: si
                            .parts
                            .iter()
                            .map(|p| Part {
                                text: p.text.clone(),
                            })
                            .collect(),
                    }),
                };
                self.http_client()
                    .post(url)
                    .json(&internal_request)
                    .bearer_auth(token)
            }
            _ => req,
        }
    }
}

impl GeminiProvider {
    async fn send_generate_content(
        &self,
        contents: Vec<Content>,
        system_instruction: Option<Content>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let auth = self.auth.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "Gemini API key not found. Options:\n\
                 1. Set GEMINI_API_KEY env var\n\
                 2. Authenticate in the web UI (tokens will be reused)\n\
                 3. Get an API key from https://aistudio.google.com/app/apikey\n\
                 4. Configure via the web UI to configure"
            )
        })?;

        let request = GenerateContentRequest {
            contents,
            system_instruction,
            generation_config: GenerationConfig {
                temperature,
                max_output_tokens: 8192,
            },
        };

        let url = Self::build_generate_content_url(model, auth);

        let response = self
            .build_generate_content_request(auth, &url, &request, model)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Gemini API error ({status}): {error_text}");
        }

        let result: GenerateContentResponse = response.json().await?;

        if let Some(err) = result.error {
            anyhow::bail!("Gemini API error: {}", err.message);
        }

        result
            .candidates
            .and_then(|c| c.into_iter().next())
            .and_then(|c| c.content.parts.into_iter().next())
            .and_then(|p| p.text)
            .ok_or_else(|| anyhow::anyhow!("No response from Gemini"))
    }
}

#[async_trait]
impl Provider for GeminiProvider {
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let system_instruction = system_prompt.map(|sys| Content {
            role: None,
            parts: vec![Part {
                text: sys.to_string(),
            }],
        });

        let contents = vec![Content {
            role: Some("user".to_string()),
            parts: vec![Part {
                text: message.to_string(),
            }],
        }];

        self.send_generate_content(contents, system_instruction, model, temperature)
            .await
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let mut system_parts: Vec<&str> = Vec::new();
        let mut contents: Vec<Content> = Vec::new();

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    system_parts.push(&msg.content);
                }
                "user" => {
                    contents.push(Content {
                        role: Some("user".to_string()),
                        parts: vec![Part {
                            text: msg.content.clone(),
                        }],
                    });
                }
                "assistant" => {
                    // Gemini API uses "model" role instead of "assistant"
                    contents.push(Content {
                        role: Some("model".to_string()),
                        parts: vec![Part {
                            text: msg.content.clone(),
                        }],
                    });
                }
                _ => {}
            }
        }

        let system_instruction = if system_parts.is_empty() {
            None
        } else {
            Some(Content {
                role: None,
                parts: vec![Part {
                    text: system_parts.join("\n\n"),
                }],
            })
        };

        self.send_generate_content(contents, system_instruction, model, temperature)
            .await
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        if let Some(auth) = self.auth.as_ref() {
            let url = if auth.is_api_key() {
                format!(
                    "https://generativelanguage.googleapis.com/v1beta/models?key={}",
                    auth.credential()
                )
            } else {
                "https://generativelanguage.googleapis.com/v1beta/models".to_string()
            };

            let mut request = self.http_client().get(&url);
            if let GeminiAuth::OAuthToken(token) = auth {
                request = request.bearer_auth(token);
            }

            request.send().await?.error_for_status()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::AUTHORIZATION;

    #[test]
    fn normalize_non_empty_trims_and_filters() {
        assert_eq!(
            GeminiProvider::normalize_non_empty(" value "),
            Some("value".into())
        );
        assert_eq!(GeminiProvider::normalize_non_empty(""), None);
        assert_eq!(GeminiProvider::normalize_non_empty(" \t\n"), None);
    }

    #[test]
    fn provider_creates_without_key() {
        let provider = GeminiProvider::new(None);
        // May pick up env vars; just verify it doesn't panic
        let _ = provider.auth_source();
    }

    #[test]
    fn provider_creates_with_key() {
        let provider = GeminiProvider::new(Some("test-api-key"));
        assert!(matches!(
            provider.auth,
            Some(GeminiAuth::ExplicitKey(ref key)) if key == "test-api-key"
        ));
    }

    #[test]
    fn provider_rejects_empty_key() {
        let provider = GeminiProvider::new(Some(""));
        assert!(!matches!(provider.auth, Some(GeminiAuth::ExplicitKey(_))));
    }

    #[test]
    fn gemini_cli_dir_returns_path() {
        let dir = GeminiProvider::gemini_cli_dir();
        // Should return Some on systems with home dir
        if UserDirs::new().is_some() {
            assert!(dir.is_some());
            assert!(dir.unwrap().ends_with(".gemini"));
        }
    }

    #[test]
    fn auth_source_explicit_key() {
        let provider = GeminiProvider {
            auth: Some(GeminiAuth::ExplicitKey("key".into())),
        };
        assert_eq!(provider.auth_source(), "config");
    }

    #[test]
    fn auth_source_none_without_credentials() {
        let provider = GeminiProvider { auth: None };
        assert_eq!(provider.auth_source(), "none");
    }

    #[test]
    fn auth_source_oauth() {
        let provider = GeminiProvider {
            auth: Some(GeminiAuth::OAuthToken("ya29.mock".into())),
        };
        assert_eq!(provider.auth_source(), "Gemini app OAuth");
    }

    #[test]
    fn model_name_formatting() {
        assert_eq!(
            GeminiProvider::format_model_name("gemini-2.0-flash"),
            "models/gemini-2.0-flash"
        );
        assert_eq!(
            GeminiProvider::format_model_name("models/gemini-1.5-pro"),
            "models/gemini-1.5-pro"
        );
    }

    #[test]
    fn api_key_url_includes_key_query_param() {
        let auth = GeminiAuth::ExplicitKey("api-key-123".into());
        let url = GeminiProvider::build_generate_content_url("gemini-2.0-flash", &auth);
        assert!(url.contains(":generateContent?key=api-key-123"));
    }

    #[test]
    fn oauth_url_uses_internal_endpoint() {
        let auth = GeminiAuth::OAuthToken("ya29.test-token".into());
        let url = GeminiProvider::build_generate_content_url("gemini-2.0-flash", &auth);
        assert!(url.starts_with("https://cloudcode-pa.googleapis.com/v1internal"));
        assert!(url.ends_with(":generateContent"));
        assert!(!url.contains("generativelanguage.googleapis.com"));
        assert!(!url.contains("?key="));
    }

    #[test]
    fn api_key_url_uses_public_endpoint() {
        let auth = GeminiAuth::ExplicitKey("api-key-123".into());
        let url = GeminiProvider::build_generate_content_url("gemini-2.0-flash", &auth);
        assert!(url.contains("generativelanguage.googleapis.com/v1beta"));
        assert!(url.contains("models/gemini-2.0-flash"));
    }

    #[test]
    fn oauth_request_uses_bearer_auth_header() {
        let provider = GeminiProvider {
            auth: Some(GeminiAuth::OAuthToken("ya29.mock-token".into())),
        };
        let auth = GeminiAuth::OAuthToken("ya29.mock-token".into());
        let url = GeminiProvider::build_generate_content_url("gemini-2.0-flash", &auth);
        let body = GenerateContentRequest {
            contents: vec![Content {
                role: Some("user".into()),
                parts: vec![Part {
                    text: "hello".into(),
                }],
            }],
            system_instruction: None,
            generation_config: GenerationConfig {
                temperature: 0.7,
                max_output_tokens: 8192,
            },
        };

        let request = provider
            .build_generate_content_request(&auth, &url, &body, "gemini-2.0-flash")
            .build()
            .unwrap();

        assert_eq!(
            request
                .headers()
                .get(AUTHORIZATION)
                .and_then(|h| h.to_str().ok()),
            Some("Bearer ya29.mock-token")
        );
    }

    #[test]
    fn api_key_request_does_not_set_bearer_header() {
        let provider = GeminiProvider {
            auth: Some(GeminiAuth::ExplicitKey("api-key-123".into())),
        };
        let auth = GeminiAuth::ExplicitKey("api-key-123".into());
        let url = GeminiProvider::build_generate_content_url("gemini-2.0-flash", &auth);
        let body = GenerateContentRequest {
            contents: vec![Content {
                role: Some("user".into()),
                parts: vec![Part {
                    text: "hello".into(),
                }],
            }],
            system_instruction: None,
            generation_config: GenerationConfig {
                temperature: 0.7,
                max_output_tokens: 8192,
            },
        };

        let request = provider
            .build_generate_content_request(&auth, &url, &body, "gemini-2.0-flash")
            .build()
            .unwrap();

        assert!(request.headers().get(AUTHORIZATION).is_none());
    }

    #[test]
    fn request_serialization() {
        let request = GenerateContentRequest {
            contents: vec![Content {
                role: Some("user".to_string()),
                parts: vec![Part {
                    text: "Hello".to_string(),
                }],
            }],
            system_instruction: Some(Content {
                role: None,
                parts: vec![Part {
                    text: "You are helpful".to_string(),
                }],
            }),
            generation_config: GenerationConfig {
                temperature: 0.7,
                max_output_tokens: 8192,
            },
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"text\":\"Hello\""));
        assert!(json.contains("\"temperature\":0.7"));
        assert!(json.contains("\"maxOutputTokens\":8192"));
    }

    #[test]
    fn internal_request_includes_model() {
        let request = InternalGenerateContentRequest {
            model: "models/gemini-3-pro-preview".to_string(),
            generation_config: GenerationConfig {
                temperature: 0.7,
                max_output_tokens: 8192,
            },
            contents: vec![Content {
                role: Some("user".to_string()),
                parts: vec![Part {
                    text: "Hello".to_string(),
                }],
            }],
            system_instruction: None,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"model\":\"models/gemini-3-pro-preview\""));
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"temperature\":0.7"));
    }

    #[test]
    fn response_deserialization() {
        let json = r#"{
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello there!"}]
                }
            }]
        }"#;

        let response: GenerateContentResponse = serde_json::from_str(json).unwrap();
        assert!(response.candidates.is_some());
        let text = response
            .candidates
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
            .content
            .parts
            .into_iter()
            .next()
            .unwrap()
            .text;
        assert_eq!(text, Some("Hello there!".to_string()));
    }

    #[test]
    fn error_response_deserialization() {
        let json = r#"{
            "error": {
                "message": "Invalid API key"
            }
        }"#;

        let response: GenerateContentResponse = serde_json::from_str(json).unwrap();
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().message, "Invalid API key");
    }

    #[tokio::test]
    async fn warmup_without_key_is_noop() {
        let provider = GeminiProvider { auth: None };
        let result = provider.warmup().await;
        assert!(result.is_ok());
    }
}
