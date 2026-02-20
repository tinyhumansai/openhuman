use crate::alphahuman::providers::traits::{
    ChatMessage, ChatRequest as ProviderChatRequest, ChatResponse as ProviderChatResponse,
    Provider, ToolCall as ProviderToolCall,
};
use crate::alphahuman::tools::ToolSpec;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub struct AnthropicProvider {
    credential: Option<String>,
    base_url: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
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
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Serialize)]
struct NativeChatRequest<'a> {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<SystemPrompt>,
    messages: Vec<NativeMessage>,
    temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<NativeToolSpec<'a>>>,
}

#[derive(Debug, Serialize)]
struct NativeMessage {
    role: String,
    content: Vec<NativeContentOut>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum NativeContentOut {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
}

#[derive(Debug, Serialize)]
struct NativeToolSpec<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Debug, Clone, Serialize)]
struct CacheControl {
    #[serde(rename = "type")]
    cache_type: String,
}

impl CacheControl {
    fn ephemeral() -> Self {
        Self {
            cache_type: "ephemeral".to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum SystemPrompt {
    String(String),
    Blocks(Vec<SystemBlock>),
}

#[derive(Debug, Serialize)]
struct SystemBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Debug, Deserialize)]
struct NativeChatResponse {
    #[serde(default)]
    content: Vec<NativeContentIn>,
}

#[derive(Debug, Deserialize)]
struct NativeContentIn {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
}

impl AnthropicProvider {
    pub fn new(credential: Option<&str>) -> Self {
        Self::with_base_url(credential, None)
    }

    pub fn with_base_url(credential: Option<&str>, base_url: Option<&str>) -> Self {
        let base_url = base_url
            .map(|u| u.trim_end_matches('/'))
            .unwrap_or("https://api.anthropic.com")
            .to_string();
        Self {
            credential: credential
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(ToString::to_string),
            base_url,
        }
    }

    fn is_setup_token(token: &str) -> bool {
        token.starts_with("sk-ant-oat01-")
    }

    fn apply_auth(
        &self,
        request: reqwest::RequestBuilder,
        credential: &str,
    ) -> reqwest::RequestBuilder {
        if Self::is_setup_token(credential) {
            request
                .header("Authorization", format!("Bearer {credential}"))
                .header("anthropic-beta", "oauth-2025-04-20")
        } else {
            request.header("x-api-key", credential)
        }
    }

    /// Cache system prompts larger than ~1024 tokens (3KB of text)
    fn should_cache_system(text: &str) -> bool {
        text.len() > 3072
    }

    /// Cache conversations with more than 4 messages (excluding system)
    fn should_cache_conversation(messages: &[ChatMessage]) -> bool {
        messages.iter().filter(|m| m.role != "system").count() > 4
    }

    /// Apply cache control to the last message content block
    fn apply_cache_to_last_message(messages: &mut [NativeMessage]) {
        if let Some(last_msg) = messages.last_mut() {
            if let Some(last_content) = last_msg.content.last_mut() {
                match last_content {
                    NativeContentOut::Text { cache_control, .. }
                    | NativeContentOut::ToolResult { cache_control, .. } => {
                        *cache_control = Some(CacheControl::ephemeral());
                    }
                    NativeContentOut::ToolUse { .. } => {}
                }
            }
        }
    }

    fn convert_tools<'a>(tools: Option<&'a [ToolSpec]>) -> Option<Vec<NativeToolSpec<'a>>> {
        let items = tools?;
        if items.is_empty() {
            return None;
        }
        let mut native_tools: Vec<NativeToolSpec<'a>> = items
            .iter()
            .map(|tool| NativeToolSpec {
                name: &tool.name,
                description: &tool.description,
                input_schema: &tool.parameters,
                cache_control: None,
            })
            .collect();

        // Cache the last tool definition (caches all tools)
        if let Some(last_tool) = native_tools.last_mut() {
            last_tool.cache_control = Some(CacheControl::ephemeral());
        }

        Some(native_tools)
    }

    fn parse_assistant_tool_call_message(content: &str) -> Option<Vec<NativeContentOut>> {
        let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
        let tool_calls = value
            .get("tool_calls")
            .and_then(|v| serde_json::from_value::<Vec<ProviderToolCall>>(v.clone()).ok())?;

        let mut blocks = Vec::new();
        if let Some(text) = value
            .get("content")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            blocks.push(NativeContentOut::Text {
                text: text.to_string(),
                cache_control: None,
            });
        }
        for call in tool_calls {
            let input = serde_json::from_str::<serde_json::Value>(&call.arguments)
                .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));
            blocks.push(NativeContentOut::ToolUse {
                id: call.id,
                name: call.name,
                input,
                cache_control: None,
            });
        }
        Some(blocks)
    }

    fn parse_tool_result_message(content: &str) -> Option<NativeMessage> {
        let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
        let tool_use_id = value
            .get("tool_call_id")
            .and_then(serde_json::Value::as_str)?
            .to_string();
        let result = value
            .get("content")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        Some(NativeMessage {
            role: "user".to_string(),
            content: vec![NativeContentOut::ToolResult {
                tool_use_id,
                content: result,
                cache_control: None,
            }],
        })
    }

    fn convert_messages(messages: &[ChatMessage]) -> (Option<SystemPrompt>, Vec<NativeMessage>) {
        let mut system_text = None;
        let mut native_messages = Vec::new();

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    if system_text.is_none() {
                        system_text = Some(msg.content.clone());
                    }
                }
                "assistant" => {
                    if let Some(blocks) = Self::parse_assistant_tool_call_message(&msg.content) {
                        native_messages.push(NativeMessage {
                            role: "assistant".to_string(),
                            content: blocks,
                        });
                    } else {
                        native_messages.push(NativeMessage {
                            role: "assistant".to_string(),
                            content: vec![NativeContentOut::Text {
                                text: msg.content.clone(),
                                cache_control: None,
                            }],
                        });
                    }
                }
                "tool" => {
                    if let Some(tool_result) = Self::parse_tool_result_message(&msg.content) {
                        native_messages.push(tool_result);
                    } else {
                        native_messages.push(NativeMessage {
                            role: "user".to_string(),
                            content: vec![NativeContentOut::Text {
                                text: msg.content.clone(),
                                cache_control: None,
                            }],
                        });
                    }
                }
                _ => {
                    native_messages.push(NativeMessage {
                        role: "user".to_string(),
                        content: vec![NativeContentOut::Text {
                            text: msg.content.clone(),
                            cache_control: None,
                        }],
                    });
                }
            }
        }

        // Convert system text to SystemPrompt with cache control if large
        let system_prompt = system_text.map(|text| {
            if Self::should_cache_system(&text) {
                SystemPrompt::Blocks(vec![SystemBlock {
                    block_type: "text".to_string(),
                    text,
                    cache_control: Some(CacheControl::ephemeral()),
                }])
            } else {
                SystemPrompt::String(text)
            }
        });

        (system_prompt, native_messages)
    }

    fn parse_text_response(response: ChatResponse) -> anyhow::Result<String> {
        response
            .content
            .into_iter()
            .find(|c| c.kind == "text")
            .and_then(|c| c.text)
            .ok_or_else(|| anyhow::anyhow!("No response from Anthropic"))
    }

    fn parse_native_response(response: NativeChatResponse) -> ProviderChatResponse {
        let mut text_parts = Vec::new();
        let mut tool_calls = Vec::new();

        for block in response.content {
            match block.kind.as_str() {
                "text" => {
                    if let Some(text) = block.text.map(|t| t.trim().to_string()) {
                        if !text.is_empty() {
                            text_parts.push(text);
                        }
                    }
                }
                "tool_use" => {
                    let name = block.name.unwrap_or_default();
                    if name.is_empty() {
                        continue;
                    }
                    let arguments = block
                        .input
                        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
                    tool_calls.push(ProviderToolCall {
                        id: block.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                        name,
                        arguments: arguments.to_string(),
                    });
                }
                _ => {}
            }
        }

        ProviderChatResponse {
            text: if text_parts.is_empty() {
                None
            } else {
                Some(text_parts.join("\n"))
            },
            tool_calls,
        }
    }

    fn http_client(&self) -> Client {
        crate::alphahuman::config::build_runtime_proxy_client_with_timeouts("provider.anthropic", 120, 10)
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let credential = self.credential.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "Anthropic credentials not set. Set ANTHROPIC_API_KEY or ANTHROPIC_OAUTH_TOKEN (setup-token)."
            )
        })?;

        let request = ChatRequest {
            model: model.to_string(),
            max_tokens: 4096,
            system: system_prompt.map(ToString::to_string),
            messages: vec![Message {
                role: "user".to_string(),
                content: message.to_string(),
            }],
            temperature,
        };

        let mut request = self
            .http_client()
            .post(format!("{}/v1/messages", self.base_url))
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request);

        request = self.apply_auth(request, credential);

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(super::api_error("Anthropic", response).await);
        }

        let chat_response: ChatResponse = response.json().await?;
        Self::parse_text_response(chat_response)
    }

    async fn chat(
        &self,
        request: ProviderChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ProviderChatResponse> {
        let credential = self.credential.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "Anthropic credentials not set. Set ANTHROPIC_API_KEY or ANTHROPIC_OAUTH_TOKEN (setup-token)."
            )
        })?;

        let (system_prompt, mut messages) = Self::convert_messages(request.messages);

        // Auto-cache last message if conversation is long
        if Self::should_cache_conversation(request.messages) {
            Self::apply_cache_to_last_message(&mut messages);
        }

        let native_request = NativeChatRequest {
            model: model.to_string(),
            max_tokens: 4096,
            system: system_prompt,
            messages,
            temperature,
            tools: Self::convert_tools(request.tools),
        };

        let req = self
            .http_client()
            .post(format!("{}/v1/messages", self.base_url))
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&native_request);

        let response = self.apply_auth(req, credential).send().await?;
        if !response.status().is_success() {
            return Err(super::api_error("Anthropic", response).await);
        }

        let native_response: NativeChatResponse = response.json().await?;
        Ok(Self::parse_native_response(native_response))
    }

    fn supports_native_tools(&self) -> bool {
        true
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        if let Some(credential) = self.credential.as_ref() {
            let mut request = self
                .http_client()
                .post(format!("{}/v1/messages", self.base_url))
                .header("anthropic-version", "2023-06-01");
            request = self.apply_auth(request, credential);
            // Send a minimal request; the goal is TLS + HTTP/2 setup, not a valid response.
            // Anthropic has no lightweight GET endpoint, so we accept any non-network error.
            let _ = request.send().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::anthropic_token::{detect_auth_kind, AnthropicAuthKind};

    #[test]
    fn creates_with_key() {
        let p = AnthropicProvider::new(Some("anthropic-test-credential"));
        assert!(p.credential.is_some());
        assert_eq!(p.credential.as_deref(), Some("anthropic-test-credential"));
        assert_eq!(p.base_url, "https://api.anthropic.com");
    }

    #[test]
    fn creates_without_key() {
        let p = AnthropicProvider::new(None);
        assert!(p.credential.is_none());
        assert_eq!(p.base_url, "https://api.anthropic.com");
    }

    #[test]
    fn creates_with_empty_key() {
        let p = AnthropicProvider::new(Some(""));
        assert!(p.credential.is_none());
    }

    #[test]
    fn creates_with_whitespace_key() {
        let p = AnthropicProvider::new(Some("  anthropic-test-credential  "));
        assert!(p.credential.is_some());
        assert_eq!(p.credential.as_deref(), Some("anthropic-test-credential"));
    }

    #[test]
    fn creates_with_custom_base_url() {
        let p = AnthropicProvider::with_base_url(
            Some("anthropic-credential"),
            Some("https://api.example.com"),
        );
        assert_eq!(p.base_url, "https://api.example.com");
        assert_eq!(p.credential.as_deref(), Some("anthropic-credential"));
    }

    #[test]
    fn custom_base_url_trims_trailing_slash() {
        let p = AnthropicProvider::with_base_url(None, Some("https://api.example.com/"));
        assert_eq!(p.base_url, "https://api.example.com");
    }

    #[test]
    fn default_base_url_when_none_provided() {
        let p = AnthropicProvider::with_base_url(None, None);
        assert_eq!(p.base_url, "https://api.anthropic.com");
    }

    #[tokio::test]
    async fn chat_fails_without_key() {
        let p = AnthropicProvider::new(None);
        let result = p
            .chat_with_system(None, "hello", "claude-3-opus", 0.7)
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("credentials not set"),
            "Expected key error, got: {err}"
        );
    }

    #[test]
    fn setup_token_detection_works() {
        assert!(AnthropicProvider::is_setup_token("sk-ant-oat01-abcdef"));
        assert!(!AnthropicProvider::is_setup_token("sk-ant-api-key"));
    }

    #[test]
    fn apply_auth_uses_bearer_and_beta_for_setup_tokens() {
        let provider = AnthropicProvider::new(None);
        let request = provider
            .apply_auth(
                provider
                    .http_client()
                    .get("https://api.anthropic.com/v1/models"),
                "sk-ant-oat01-test-token",
            )
            .build()
            .expect("request should build");

        assert_eq!(
            request
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer sk-ant-oat01-test-token")
        );
        assert_eq!(
            request
                .headers()
                .get("anthropic-beta")
                .and_then(|v| v.to_str().ok()),
            Some("oauth-2025-04-20")
        );
        assert!(request.headers().get("x-api-key").is_none());
    }

    #[test]
    fn apply_auth_uses_x_api_key_for_regular_tokens() {
        let provider = AnthropicProvider::new(None);
        let request = provider
            .apply_auth(
                provider
                    .http_client()
                    .get("https://api.anthropic.com/v1/models"),
                "sk-ant-api-key",
            )
            .build()
            .expect("request should build");

        assert_eq!(
            request
                .headers()
                .get("x-api-key")
                .and_then(|v| v.to_str().ok()),
            Some("sk-ant-api-key")
        );
        assert!(request.headers().get("authorization").is_none());
        assert!(request.headers().get("anthropic-beta").is_none());
    }

    #[tokio::test]
    async fn chat_with_system_fails_without_key() {
        let p = AnthropicProvider::new(None);
        let result = p
            .chat_with_system(Some("You are Alphahuman"), "hello", "claude-3-opus", 0.7)
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn chat_request_serializes_without_system() {
        let req = ChatRequest {
            model: "claude-3-opus".to_string(),
            max_tokens: 4096,
            system: None,
            messages: vec![Message {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            temperature: 0.7,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            !json.contains("system"),
            "system field should be skipped when None"
        );
        assert!(json.contains("claude-3-opus"));
        assert!(json.contains("hello"));
    }

    #[test]
    fn chat_request_serializes_with_system() {
        let req = ChatRequest {
            model: "claude-3-opus".to_string(),
            max_tokens: 4096,
            system: Some("You are Alphahuman".to_string()),
            messages: vec![Message {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            temperature: 0.7,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"system\":\"You are Alphahuman\""));
    }

    #[test]
    fn chat_response_deserializes() {
        let json = r#"{"content":[{"type":"text","text":"Hello there!"}]}"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.content.len(), 1);
        assert_eq!(resp.content[0].kind, "text");
        assert_eq!(resp.content[0].text.as_deref(), Some("Hello there!"));
    }

    #[test]
    fn chat_response_empty_content() {
        let json = r#"{"content":[]}"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        assert!(resp.content.is_empty());
    }

    #[test]
    fn chat_response_multiple_blocks() {
        let json =
            r#"{"content":[{"type":"text","text":"First"},{"type":"text","text":"Second"}]}"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.content.len(), 2);
        assert_eq!(resp.content[0].text.as_deref(), Some("First"));
        assert_eq!(resp.content[1].text.as_deref(), Some("Second"));
    }

    #[test]
    fn temperature_range_serializes() {
        for temp in [0.0, 0.5, 1.0, 2.0] {
            let req = ChatRequest {
                model: "claude-3-opus".to_string(),
                max_tokens: 4096,
                system: None,
                messages: vec![],
                temperature: temp,
            };
            let json = serde_json::to_string(&req).unwrap();
            assert!(json.contains(&format!("{temp}")));
        }
    }

    #[test]
    fn detects_auth_from_jwt_shape() {
        let kind = detect_auth_kind("a.b.c", None);
        assert_eq!(kind, AnthropicAuthKind::Authorization);
    }

    #[test]
    fn cache_control_serializes_correctly() {
        let cache = CacheControl::ephemeral();
        let json = serde_json::to_string(&cache).unwrap();
        assert_eq!(json, r#"{"type":"ephemeral"}"#);
    }

    #[test]
    fn system_prompt_string_variant_serializes() {
        let prompt = SystemPrompt::String("You are a helpful assistant".to_string());
        let json = serde_json::to_string(&prompt).unwrap();
        assert_eq!(json, r#""You are a helpful assistant""#);
    }

    #[test]
    fn system_prompt_blocks_variant_serializes() {
        let prompt = SystemPrompt::Blocks(vec![SystemBlock {
            block_type: "text".to_string(),
            text: "You are a helpful assistant".to_string(),
            cache_control: Some(CacheControl::ephemeral()),
        }]);
        let json = serde_json::to_string(&prompt).unwrap();
        assert!(json.contains(r#""type":"text""#));
        assert!(json.contains("You are a helpful assistant"));
        assert!(json.contains(r#""type":"ephemeral""#));
    }

    #[test]
    fn system_prompt_blocks_without_cache_control() {
        let prompt = SystemPrompt::Blocks(vec![SystemBlock {
            block_type: "text".to_string(),
            text: "Short prompt".to_string(),
            cache_control: None,
        }]);
        let json = serde_json::to_string(&prompt).unwrap();
        assert!(json.contains("Short prompt"));
        assert!(!json.contains("cache_control"));
    }

    #[test]
    fn native_content_text_without_cache_control() {
        let content = NativeContentOut::Text {
            text: "Hello".to_string(),
            cache_control: None,
        };
        let json = serde_json::to_string(&content).unwrap();
        assert!(json.contains(r#""type":"text""#));
        assert!(json.contains("Hello"));
        assert!(!json.contains("cache_control"));
    }

    #[test]
    fn native_content_text_with_cache_control() {
        let content = NativeContentOut::Text {
            text: "Hello".to_string(),
            cache_control: Some(CacheControl::ephemeral()),
        };
        let json = serde_json::to_string(&content).unwrap();
        assert!(json.contains(r#""type":"text""#));
        assert!(json.contains("Hello"));
        assert!(json.contains(r#""cache_control":{"type":"ephemeral"}"#));
    }

    #[test]
    fn native_content_tool_use_without_cache_control() {
        let content = NativeContentOut::ToolUse {
            id: "tool_123".to_string(),
            name: "get_weather".to_string(),
            input: serde_json::json!({"location": "San Francisco"}),
            cache_control: None,
        };
        let json = serde_json::to_string(&content).unwrap();
        assert!(json.contains(r#""type":"tool_use""#));
        assert!(json.contains("tool_123"));
        assert!(json.contains("get_weather"));
        assert!(!json.contains("cache_control"));
    }

    #[test]
    fn native_content_tool_result_with_cache_control() {
        let content = NativeContentOut::ToolResult {
            tool_use_id: "tool_123".to_string(),
            content: "Result data".to_string(),
            cache_control: Some(CacheControl::ephemeral()),
        };
        let json = serde_json::to_string(&content).unwrap();
        assert!(json.contains(r#""type":"tool_result""#));
        assert!(json.contains("tool_123"));
        assert!(json.contains("Result data"));
        assert!(json.contains(r#""cache_control":{"type":"ephemeral"}"#));
    }

    #[test]
    fn native_tool_spec_without_cache_control() {
        let schema = serde_json::json!({"type": "object"});
        let tool = NativeToolSpec {
            name: "get_weather",
            description: "Get weather info",
            input_schema: &schema,
            cache_control: None,
        };
        let json = serde_json::to_string(&tool).unwrap();
        assert!(json.contains("get_weather"));
        assert!(!json.contains("cache_control"));
    }

    #[test]
    fn native_tool_spec_with_cache_control() {
        let schema = serde_json::json!({"type": "object"});
        let tool = NativeToolSpec {
            name: "get_weather",
            description: "Get weather info",
            input_schema: &schema,
            cache_control: Some(CacheControl::ephemeral()),
        };
        let json = serde_json::to_string(&tool).unwrap();
        assert!(json.contains("get_weather"));
        assert!(json.contains(r#""cache_control":{"type":"ephemeral"}"#));
    }

    #[test]
    fn should_cache_system_small_prompt() {
        let small_prompt = "You are a helpful assistant.";
        assert!(!AnthropicProvider::should_cache_system(small_prompt));
    }

    #[test]
    fn should_cache_system_large_prompt() {
        let large_prompt = "a".repeat(3073); // Just over 3072 bytes
        assert!(AnthropicProvider::should_cache_system(&large_prompt));
    }

    #[test]
    fn should_cache_system_boundary() {
        let boundary_prompt = "a".repeat(3072); // Exactly 3072 bytes
        assert!(!AnthropicProvider::should_cache_system(&boundary_prompt));

        let over_boundary = "a".repeat(3073);
        assert!(AnthropicProvider::should_cache_system(&over_boundary));
    }

    #[test]
    fn should_cache_conversation_short() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "System prompt".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: "Hi".to_string(),
            },
        ];
        // Only 2 non-system messages
        assert!(!AnthropicProvider::should_cache_conversation(&messages));
    }

    #[test]
    fn should_cache_conversation_long() {
        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: "System prompt".to_string(),
        }];
        // Add 5 non-system messages
        for i in 0..5 {
            messages.push(ChatMessage {
                role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
                content: format!("Message {i}"),
            });
        }
        assert!(AnthropicProvider::should_cache_conversation(&messages));
    }

    #[test]
    fn should_cache_conversation_boundary() {
        let mut messages = vec![];
        // Add exactly 4 non-system messages
        for i in 0..4 {
            messages.push(ChatMessage {
                role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
                content: format!("Message {i}"),
            });
        }
        assert!(!AnthropicProvider::should_cache_conversation(&messages));

        // Add one more to cross boundary
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: "One more".to_string(),
        });
        assert!(AnthropicProvider::should_cache_conversation(&messages));
    }

    #[test]
    fn apply_cache_to_last_message_text() {
        let mut messages = vec![NativeMessage {
            role: "user".to_string(),
            content: vec![NativeContentOut::Text {
                text: "Hello".to_string(),
                cache_control: None,
            }],
        }];

        AnthropicProvider::apply_cache_to_last_message(&mut messages);

        match &messages[0].content[0] {
            NativeContentOut::Text { cache_control, .. } => {
                assert!(cache_control.is_some());
            }
            _ => panic!("Expected Text variant"),
        }
    }

    #[test]
    fn apply_cache_to_last_message_tool_result() {
        let mut messages = vec![NativeMessage {
            role: "user".to_string(),
            content: vec![NativeContentOut::ToolResult {
                tool_use_id: "tool_123".to_string(),
                content: "Result".to_string(),
                cache_control: None,
            }],
        }];

        AnthropicProvider::apply_cache_to_last_message(&mut messages);

        match &messages[0].content[0] {
            NativeContentOut::ToolResult { cache_control, .. } => {
                assert!(cache_control.is_some());
            }
            _ => panic!("Expected ToolResult variant"),
        }
    }

    #[test]
    fn apply_cache_to_last_message_does_not_affect_tool_use() {
        let mut messages = vec![NativeMessage {
            role: "assistant".to_string(),
            content: vec![NativeContentOut::ToolUse {
                id: "tool_123".to_string(),
                name: "get_weather".to_string(),
                input: serde_json::json!({}),
                cache_control: None,
            }],
        }];

        AnthropicProvider::apply_cache_to_last_message(&mut messages);

        // ToolUse should not be affected
        match &messages[0].content[0] {
            NativeContentOut::ToolUse { cache_control, .. } => {
                assert!(cache_control.is_none());
            }
            _ => panic!("Expected ToolUse variant"),
        }
    }

    #[test]
    fn apply_cache_empty_messages() {
        let mut messages = vec![];
        AnthropicProvider::apply_cache_to_last_message(&mut messages);
        // Should not panic
        assert!(messages.is_empty());
    }

    #[test]
    fn convert_tools_adds_cache_to_last_tool() {
        let tools = vec![
            ToolSpec {
                name: "tool1".to_string(),
                description: "First tool".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            },
            ToolSpec {
                name: "tool2".to_string(),
                description: "Second tool".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            },
        ];

        let native_tools = AnthropicProvider::convert_tools(Some(&tools)).unwrap();

        assert_eq!(native_tools.len(), 2);
        assert!(native_tools[0].cache_control.is_none());
        assert!(native_tools[1].cache_control.is_some());
    }

    #[test]
    fn convert_tools_single_tool_gets_cache() {
        let tools = vec![ToolSpec {
            name: "tool1".to_string(),
            description: "Only tool".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }];

        let native_tools = AnthropicProvider::convert_tools(Some(&tools)).unwrap();

        assert_eq!(native_tools.len(), 1);
        assert!(native_tools[0].cache_control.is_some());
    }

    #[test]
    fn convert_messages_small_system_prompt() {
        let messages = vec![ChatMessage {
            role: "system".to_string(),
            content: "Short system prompt".to_string(),
        }];

        let (system_prompt, _) = AnthropicProvider::convert_messages(&messages);

        match system_prompt.unwrap() {
            SystemPrompt::String(s) => {
                assert_eq!(s, "Short system prompt");
            }
            SystemPrompt::Blocks(_) => panic!("Expected String variant for small prompt"),
        }
    }

    #[test]
    fn convert_messages_large_system_prompt() {
        let large_content = "a".repeat(3073);
        let messages = vec![ChatMessage {
            role: "system".to_string(),
            content: large_content.clone(),
        }];

        let (system_prompt, _) = AnthropicProvider::convert_messages(&messages);

        match system_prompt.unwrap() {
            SystemPrompt::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                assert_eq!(blocks[0].text, large_content);
                assert!(blocks[0].cache_control.is_some());
            }
            SystemPrompt::String(_) => panic!("Expected Blocks variant for large prompt"),
        }
    }

    #[test]
    fn backward_compatibility_native_chat_request() {
        // Test that requests without cache_control serialize identically to old format
        let req = NativeChatRequest {
            model: "claude-3-opus".to_string(),
            max_tokens: 4096,
            system: Some(SystemPrompt::String("System".to_string())),
            messages: vec![NativeMessage {
                role: "user".to_string(),
                content: vec![NativeContentOut::Text {
                    text: "Hello".to_string(),
                    cache_control: None,
                }],
            }],
            temperature: 0.7,
            tools: None,
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("cache_control"));
        assert!(json.contains(r#""system":"System""#));
    }

    #[tokio::test]
    async fn warmup_without_key_is_noop() {
        let provider = AnthropicProvider::new(None);
        let result = provider.warmup().await;
        assert!(result.is_ok());
    }
}
