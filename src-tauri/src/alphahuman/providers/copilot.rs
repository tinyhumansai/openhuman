//! GitHub Copilot provider with OAuth device-flow authentication.
//!
//! Authenticates via GitHub's device code flow (same as VS Code Copilot),
//! then exchanges the OAuth token for short-lived Copilot API keys.
//! Tokens are cached to disk and auto-refreshed.
//!
//! **Note:** This uses VS Code's OAuth client ID (`Iv1.b507a08c87ecfe98`) and
//! editor headers. This is the same approach used by LiteLLM, Codex desktop app,
//! and other third-party Copilot integrations. The Copilot token endpoint is
//! private; there is no public OAuth scope or app registration for it.
//! GitHub could change or revoke this at any time, which would break all
//! third-party integrations simultaneously.

use crate::alphahuman::providers::traits::{
    ChatMessage, ChatRequest as ProviderChatRequest, ChatResponse as ProviderChatResponse,
    Provider, ToolCall as ProviderToolCall,
};
use crate::alphahuman::tools::ToolSpec;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::warn;

/// GitHub OAuth client ID for Copilot (VS Code extension).
const GITHUB_UIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const GITHUB_API_KEY_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const DEFAULT_API: &str = "https://api.githubcopilot.com";

// ── Token types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default = "default_interval")]
    interval: u64,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
}

fn default_interval() -> u64 {
    5
}

fn default_expires_in() -> u64 {
    900
}

#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiKeyInfo {
    token: String,
    expires_at: i64,
    #[serde(default)]
    endpoints: Option<ApiEndpoints>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiEndpoints {
    api: Option<String>,
}

struct CachedApiKey {
    token: String,
    api_endpoint: String,
    expires_at: i64,
}

// ── Chat completions types ───────────────────────────────────────

#[derive(Debug, Serialize)]
struct ApiChatRequest<'a> {
    model: String,
    messages: Vec<ApiMessage>,
    temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<NativeToolSpec<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Debug, Serialize)]
struct ApiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<NativeToolCall>>,
}

#[derive(Debug, Serialize)]
struct NativeToolSpec<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: NativeToolFunctionSpec<'a>,
}

#[derive(Debug, Serialize)]
struct NativeToolFunctionSpec<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct NativeToolCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    function: NativeFunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct NativeFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct ApiChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<NativeToolCall>>,
}

// ── Provider ─────────────────────────────────────────────────────

/// GitHub Copilot provider with automatic OAuth and token refresh.
///
/// On first use, prompts the user to visit github.com/login/device.
/// Tokens are cached to `~/.config/alphahuman/copilot/` and refreshed
/// automatically.
pub struct CopilotProvider {
    github_token: Option<String>,
    /// Mutex ensures only one caller refreshes tokens at a time,
    /// preventing duplicate device flow prompts or redundant API calls.
    refresh_lock: Arc<Mutex<Option<CachedApiKey>>>,
    token_dir: PathBuf,
}

impl CopilotProvider {
    pub fn new(github_token: Option<&str>) -> Self {
        let token_dir = directories::ProjectDirs::from("", "", "alphahuman")
            .map(|dir| dir.config_dir().join("copilot"))
            .unwrap_or_else(|| {
                // Fall back to a user-specific temp directory to avoid
                // shared-directory symlink attacks.
                let user = std::env::var("USER")
                    .or_else(|_| std::env::var("USERNAME"))
                    .unwrap_or_else(|_| "unknown".to_string());
                std::env::temp_dir().join(format!("alphahuman-copilot-{user}"))
            });

        if let Err(err) = std::fs::create_dir_all(&token_dir) {
            warn!(
                "Failed to create Copilot token directory {:?}: {err}. Token caching is disabled.",
                token_dir
            );
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                if let Err(err) =
                    std::fs::set_permissions(&token_dir, std::fs::Permissions::from_mode(0o700))
                {
                    warn!(
                        "Failed to set Copilot token directory permissions on {:?}: {err}",
                        token_dir
                    );
                }
            }
        }

        Self {
            github_token: github_token
                .filter(|token| !token.is_empty())
                .map(String::from),
            refresh_lock: Arc::new(Mutex::new(None)),
            token_dir,
        }
    }

    fn http_client(&self) -> Client {
        crate::alphahuman::config::build_runtime_proxy_client_with_timeouts("provider.copilot", 120, 10)
    }

    /// Required headers for Copilot API requests (editor identification).
    const COPILOT_HEADERS: [(&str, &str); 4] = [
        ("Editor-Version", "vscode/1.85.1"),
        ("Editor-Plugin-Version", "copilot/1.155.0"),
        ("User-Agent", "GithubCopilot/1.155.0"),
        ("Accept", "application/json"),
    ];

    fn convert_tools<'a>(tools: Option<&'a [ToolSpec]>) -> Option<Vec<NativeToolSpec<'a>>> {
        tools.map(|items| {
            items
                .iter()
                .map(|tool| NativeToolSpec {
                    kind: "function",
                    function: NativeToolFunctionSpec {
                        name: &tool.name,
                        description: &tool.description,
                        parameters: &tool.parameters,
                    },
                })
                .collect()
        })
    }

    fn convert_messages(messages: &[ChatMessage]) -> Vec<ApiMessage> {
        messages
            .iter()
            .map(|message| {
                if message.role == "assistant" {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&message.content) {
                        if let Some(tool_calls_value) = value.get("tool_calls") {
                            if let Ok(parsed_calls) =
                                serde_json::from_value::<Vec<ProviderToolCall>>(tool_calls_value.clone())
                            {
                                let tool_calls = parsed_calls
                                    .into_iter()
                                    .map(|tool_call| NativeToolCall {
                                        id: Some(tool_call.id),
                                        kind: Some("function".to_string()),
                                        function: NativeFunctionCall {
                                            name: tool_call.name,
                                            arguments: tool_call.arguments,
                                        },
                                    })
                                    .collect::<Vec<_>>();

                                let content = value
                                    .get("content")
                                    .and_then(serde_json::Value::as_str)
                                    .map(ToString::to_string);

                                return ApiMessage {
                                    role: "assistant".to_string(),
                                    content,
                                    tool_call_id: None,
                                    tool_calls: Some(tool_calls),
                                };
                            }
                        }
                    }
                }

                if message.role == "tool" {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&message.content) {
                        let tool_call_id = value
                            .get("tool_call_id")
                            .and_then(serde_json::Value::as_str)
                            .map(ToString::to_string);
                        let content = value
                            .get("content")
                            .and_then(serde_json::Value::as_str)
                            .map(ToString::to_string);

                        return ApiMessage {
                            role: "tool".to_string(),
                            content,
                            tool_call_id,
                            tool_calls: None,
                        };
                    }
                }

                ApiMessage {
                    role: message.role.clone(),
                    content: Some(message.content.clone()),
                    tool_call_id: None,
                    tool_calls: None,
                }
            })
            .collect()
    }

    /// Send a chat completions request with required Copilot headers.
    async fn send_chat_request(
        &self,
        messages: Vec<ApiMessage>,
        tools: Option<&[ToolSpec]>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ProviderChatResponse> {
        let (token, endpoint) = self.get_api_key().await?;
        let url = format!("{}/chat/completions", endpoint.trim_end_matches('/'));

        let native_tools = Self::convert_tools(tools);
        let request = ApiChatRequest {
            model: model.to_string(),
            messages,
            temperature,
            tool_choice: native_tools.as_ref().map(|_| "auto".to_string()),
            tools: native_tools,
        };

        let mut req = self
            .http_client()
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&request);

        for (header, value) in &Self::COPILOT_HEADERS {
            req = req.header(*header, *value);
        }

        let response = req.send().await?;

        if !response.status().is_success() {
            return Err(super::api_error("GitHub Copilot", response).await);
        }

        let api_response: ApiChatResponse = response.json().await?;
        let choice = api_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No response from GitHub Copilot"))?;

        let tool_calls = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tool_call| ProviderToolCall {
                id: tool_call
                    .id
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                name: tool_call.function.name,
                arguments: tool_call.function.arguments,
            })
            .collect();

        Ok(ProviderChatResponse {
            text: choice.message.content,
            tool_calls,
        })
    }

    /// Get a valid Copilot API key, refreshing or re-authenticating as needed.
    /// Uses a Mutex to ensure only one caller refreshes at a time.
    async fn get_api_key(&self) -> anyhow::Result<(String, String)> {
        let mut cached = self.refresh_lock.lock().await;

        if let Some(cached_key) = cached.as_ref() {
            if chrono::Utc::now().timestamp() + 120 < cached_key.expires_at {
                return Ok((cached_key.token.clone(), cached_key.api_endpoint.clone()));
            }
        }

        if let Some(info) = self.load_api_key_from_disk().await {
            if chrono::Utc::now().timestamp() + 120 < info.expires_at {
                let endpoint = info
                    .endpoints
                    .as_ref()
                    .and_then(|e| e.api.clone())
                    .unwrap_or_else(|| DEFAULT_API.to_string());
                let token = info.token;

                *cached = Some(CachedApiKey {
                    token: token.clone(),
                    api_endpoint: endpoint.clone(),
                    expires_at: info.expires_at,
                });
                return Ok((token, endpoint));
            }
        }

        let access_token = self.get_github_access_token().await?;
        let api_key_info = self.exchange_for_api_key(&access_token).await?;
        self.save_api_key_to_disk(&api_key_info).await;

        let endpoint = api_key_info
            .endpoints
            .as_ref()
            .and_then(|e| e.api.clone())
            .unwrap_or_else(|| DEFAULT_API.to_string());

        *cached = Some(CachedApiKey {
            token: api_key_info.token.clone(),
            api_endpoint: endpoint.clone(),
            expires_at: api_key_info.expires_at,
        });

        Ok((api_key_info.token, endpoint))
    }

    /// Get a GitHub access token from config, cache, or device flow.
    async fn get_github_access_token(&self) -> anyhow::Result<String> {
        if let Some(token) = &self.github_token {
            return Ok(token.clone());
        }

        let access_token_path = self.token_dir.join("access-token");
        if let Ok(cached) = tokio::fs::read_to_string(&access_token_path).await {
            let token = cached.trim();
            if !token.is_empty() {
                return Ok(token.to_string());
            }
        }

        let token = self.device_code_login().await?;
        write_file_secure(&access_token_path, &token).await;
        Ok(token)
    }

    /// Run GitHub OAuth device code flow.
    async fn device_code_login(&self) -> anyhow::Result<String> {
        let response: DeviceCodeResponse = self
            .http_client()
            .post(GITHUB_DEVICE_CODE_URL)
            .header("Accept", "application/json")
            .json(&serde_json::json!({
                "client_id": GITHUB_UIENT_ID,
                "scope": "read:user"
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let mut poll_interval = Duration::from_secs(response.interval.max(5));
        let expires_in = response.expires_in.max(1);
        let expires_at = tokio::time::Instant::now() + Duration::from_secs(expires_in);

        eprintln!(
            "\nGitHub Copilot authentication is required.\n\
             Visit: {}\n\
             Code: {}\n\
             Waiting for authorization...\n",
            response.verification_uri, response.user_code
        );

        while tokio::time::Instant::now() < expires_at {
            tokio::time::sleep(poll_interval).await;

            let token_response: AccessTokenResponse = self
                .http_client()
                .post(GITHUB_ACCESS_TOKEN_URL)
                .header("Accept", "application/json")
                .json(&serde_json::json!({
                    "client_id": GITHUB_UIENT_ID,
                    "device_code": response.device_code,
                    "grant_type": "urn:ietf:params:oauth:grant-type:device_code"
                }))
                .send()
                .await?
                .json()
                .await?;

            if let Some(token) = token_response.access_token {
                eprintln!("Authentication succeeded.\n");
                return Ok(token);
            }

            match token_response.error.as_deref() {
                Some("slow_down") => {
                    poll_interval += Duration::from_secs(5);
                }
                Some("authorization_pending") | None => {}
                Some("expired_token") => {
                    anyhow::bail!("GitHub device authorization expired")
                }
                Some(error) => anyhow::bail!("GitHub auth failed: {error}"),
            }
        }

        anyhow::bail!("Timed out waiting for GitHub authorization")
    }

    /// Exchange a GitHub access token for a Copilot API key.
    async fn exchange_for_api_key(&self, access_token: &str) -> anyhow::Result<ApiKeyInfo> {
        let mut request = self.http_client().get(GITHUB_API_KEY_URL);
        for (header, value) in &Self::COPILOT_HEADERS {
            request = request.header(*header, *value);
        }
        request = request.header("Authorization", format!("token {access_token}"));

        let response = request.send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let sanitized = super::sanitize_api_error(&body);

            if status.as_u16() == 401 || status.as_u16() == 403 {
                let access_token_path = self.token_dir.join("access-token");
                tokio::fs::remove_file(&access_token_path).await.ok();
            }

            anyhow::bail!(
                "Failed to get Copilot API key ({status}): {sanitized}. \
                 Ensure your GitHub account has an active Copilot subscription."
            );
        }

        let info: ApiKeyInfo = response.json().await?;
        Ok(info)
    }

    async fn load_api_key_from_disk(&self) -> Option<ApiKeyInfo> {
        let path = self.token_dir.join("api-key.json");
        let data = tokio::fs::read_to_string(&path).await.ok()?;
        serde_json::from_str(&data).ok()
    }

    async fn save_api_key_to_disk(&self, info: &ApiKeyInfo) {
        let path = self.token_dir.join("api-key.json");
        if let Ok(json) = serde_json::to_string_pretty(info) {
            write_file_secure(&path, &json).await;
        }
    }
}

/// Write a file with 0600 permissions (owner read/write only).
/// Uses `spawn_blocking` to avoid blocking the async runtime.
async fn write_file_secure(path: &Path, content: &str) {
    let path = path.to_path_buf();
    let content = content.to_string();

    let result = tokio::task::spawn_blocking(move || {
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)?;
            file.write_all(content.as_bytes())?;

            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            Ok::<(), std::io::Error>(())
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&path, &content)?;
            Ok::<(), std::io::Error>(())
        }
    })
    .await;

    match result {
        Ok(Ok(())) => {}
        Ok(Err(err)) => warn!("Failed to write secure file: {err}"),
        Err(err) => warn!("Failed to spawn blocking write: {err}"),
    }
}

#[async_trait]
impl Provider for CopilotProvider {
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let mut messages = Vec::new();
        if let Some(system) = system_prompt {
            messages.push(ApiMessage {
                role: "system".to_string(),
                content: Some(system.to_string()),
                tool_call_id: None,
                tool_calls: None,
            });
        }
        messages.push(ApiMessage {
            role: "user".to_string(),
            content: Some(message.to_string()),
            tool_call_id: None,
            tool_calls: None,
        });

        let response = self
            .send_chat_request(messages, None, model, temperature)
            .await?;
        Ok(response.text.unwrap_or_default())
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let response = self
            .send_chat_request(Self::convert_messages(messages), None, model, temperature)
            .await?;
        Ok(response.text.unwrap_or_default())
    }

    async fn chat(
        &self,
        request: ProviderChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ProviderChatResponse> {
        self.send_chat_request(
            Self::convert_messages(request.messages),
            request.tools,
            model,
            temperature,
        )
        .await
    }

    fn supports_native_tools(&self) -> bool {
        true
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        let _ = self.get_api_key().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_without_token() {
        let provider = CopilotProvider::new(None);
        assert!(provider.github_token.is_none());
    }

    #[test]
    fn new_with_token() {
        let provider = CopilotProvider::new(Some("ghp_test"));
        assert_eq!(provider.github_token.as_deref(), Some("ghp_test"));
    }

    #[test]
    fn empty_token_treated_as_none() {
        let provider = CopilotProvider::new(Some(""));
        assert!(provider.github_token.is_none());
    }

    #[tokio::test]
    async fn cache_starts_empty() {
        let provider = CopilotProvider::new(None);
        let cached = provider.refresh_lock.lock().await;
        assert!(cached.is_none());
    }

    #[test]
    fn copilot_headers_include_required_fields() {
        let headers = CopilotProvider::COPILOT_HEADERS;
        assert!(headers
            .iter()
            .any(|(header, _)| *header == "Editor-Version"));
        assert!(headers
            .iter()
            .any(|(header, _)| *header == "Editor-Plugin-Version"));
        assert!(headers.iter().any(|(header, _)| *header == "User-Agent"));
    }

    #[test]
    fn default_interval_and_expiry() {
        assert_eq!(default_interval(), 5);
        assert_eq!(default_expires_in(), 900);
    }

    #[test]
    fn supports_native_tools() {
        let provider = CopilotProvider::new(None);
        assert!(provider.supports_native_tools());
    }
}
