//! AWS Bedrock provider using the Converse API.
//!
//! Authentication: AWS AKSK (Access Key ID + Secret Access Key)
//! via environment variables. SigV4 signing is implemented manually
//! using hmac/sha2 crates — no AWS SDK dependency.

use crate::openhuman::providers::traits::{
    ChatMessage, ChatRequest as ProviderChatRequest, ChatResponse as ProviderChatResponse,
    Provider, ProviderCapabilities, ToolCall as ProviderToolCall, ToolsPayload,
};
use crate::openhuman::tools::ToolSpec;
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Hostname prefix for the Bedrock Runtime endpoint.
const ENDPOINT_PREFIX: &str = "bedrock-runtime";
/// SigV4 signing service name (AWS uses "bedrock", not "bedrock-runtime").
const SIGNING_SERVICE: &str = "bedrock";
const DEFAULT_REGION: &str = "us-east-1";
const DEFAULT_MAX_TOKENS: u32 = 4096;

// ── AWS Credentials ─────────────────────────────────────────────

/// Resolved AWS credentials for SigV4 signing.
struct AwsCredentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
    region: String,
}

impl AwsCredentials {
    /// Resolve credentials from environment variables.
    ///
    /// Required: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`.
    /// Optional: `AWS_SESSION_TOKEN`, `AWS_REGION` / `AWS_DEFAULT_REGION`.
    fn from_env() -> anyhow::Result<Self> {
        let access_key_id = env_required("AWS_ACCESS_KEY_ID")?;
        let secret_access_key = env_required("AWS_SECRET_ACCESS_KEY")?;

        let session_token = env_optional("AWS_SESSION_TOKEN");

        let region = env_optional("AWS_REGION")
            .or_else(|| env_optional("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|| DEFAULT_REGION.to_string());

        Ok(Self {
            access_key_id,
            secret_access_key,
            session_token,
            region,
        })
    }

    fn host(&self) -> String {
        format!("{ENDPOINT_PREFIX}.{}.amazonaws.com", self.region)
    }
}

fn env_required(name: &str) -> anyhow::Result<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Environment variable {name} is required for Bedrock"))
}

fn env_optional(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

// ── AWS SigV4 Signing ───────────────────────────────────────────

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC can take key of any size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Derive the SigV4 signing key via HMAC chain.
fn derive_signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

/// Build the SigV4 `Authorization` header value.
///
/// `headers` must be sorted by lowercase header name.
fn build_authorization_header(
    credentials: &AwsCredentials,
    method: &str,
    canonical_uri: &str,
    query_string: &str,
    headers: &[(String, String)],
    payload: &[u8],
    timestamp: &chrono::DateTime<chrono::Utc>,
) -> String {
    let date_stamp = timestamp.format("%Y%m%d").to_string();
    let amz_date = timestamp.format("%Y%m%dT%H%M%SZ").to_string();

    let mut canonical_headers = String::new();
    for (k, v) in headers {
        canonical_headers.push_str(k);
        canonical_headers.push(':');
        canonical_headers.push_str(v);
        canonical_headers.push('\n');
    }

    let signed_headers: String = headers
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");

    let payload_hash = sha256_hex(payload);

    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{query_string}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );

    let credential_scope = format!(
        "{date_stamp}/{}/{SIGNING_SERVICE}/aws4_request",
        credentials.region
    );

    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );

    let signing_key = derive_signing_key(
        &credentials.secret_access_key,
        &date_stamp,
        &credentials.region,
        SIGNING_SERVICE,
    );

    let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));

    format!(
        "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
        credentials.access_key_id
    )
}

// ── Converse API Types (Request) ────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConverseRequest {
    messages: Vec<ConverseMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<Vec<SystemBlock>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inference_config: Option<InferenceConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_config: Option<ToolConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ConverseMessage {
    role: String,
    content: Vec<ContentBlock>,
}

/// Content blocks use Bedrock's union style:
/// `{"text": "..."}`, `{"toolUse": {...}}`, `{"toolResult": {...}}`, `{"cachePoint": {...}}`.
///
/// Note: `text` is a simple string value, not a nested object. `toolUse` and `toolResult`
/// are nested objects. We use `#[serde(untagged)]` with manual struct wrappers to
/// match this mixed format.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum ContentBlock {
    Text(TextBlock),
    ToolUse(ToolUseWrapper),
    ToolResult(ToolResultWrapper),
    CachePointBlock(CachePointWrapper),
}

#[derive(Debug, Serialize, Deserialize)]
struct TextBlock {
    text: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolUseWrapper {
    tool_use: ToolUseBlock,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolUseBlock {
    tool_use_id: String,
    name: String,
    input: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolResultWrapper {
    tool_result: ToolResultBlock,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolResultBlock {
    tool_use_id: String,
    content: Vec<ToolResultContent>,
    status: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachePointWrapper {
    cache_point: CachePoint,
}

#[derive(Debug, Serialize, Deserialize)]
struct ToolResultContent {
    text: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachePoint {
    #[serde(rename = "type")]
    cache_type: String,
}

impl CachePoint {
    fn default_cache() -> Self {
        Self {
            cache_type: "default".to_string(),
        }
    }
}

/// System prompt blocks: either `{"text": "..."}` or `{"cachePoint": {...}}`.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum SystemBlock {
    Text(TextBlock),
    CachePoint(CachePointWrapper),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InferenceConfig {
    max_tokens: u32,
    temperature: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolConfig {
    tools: Vec<ToolDefinition>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolDefinition {
    tool_spec: ToolSpecDef,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolSpecDef {
    name: String,
    description: String,
    input_schema: InputSchema,
}

#[derive(Debug, Serialize)]
struct InputSchema {
    json: serde_json::Value,
}

// ── Converse API Types (Response) ───────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConverseResponse {
    #[serde(default)]
    output: Option<ConverseOutput>,
    #[serde(default)]
    #[allow(dead_code)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConverseOutput {
    #[serde(default)]
    message: Option<ConverseOutputMessage>,
}

#[derive(Debug, Deserialize)]
struct ConverseOutputMessage {
    #[allow(dead_code)]
    role: String,
    content: Vec<ResponseContentBlock>,
}

/// Response content blocks from the Converse API.
///
/// Uses `#[serde(untagged)]` to match Bedrock's union format where `text` is a
/// simple string value and `toolUse` is a nested object. Unknown block types
/// (e.g. `reasoningContent`, `guardContent`) are captured as `Other` to prevent
/// deserialization failures.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ResponseContentBlock {
    ToolUse(ResponseToolUseWrapper),
    Text(TextBlock),
    Other(serde_json::Value),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseToolUseWrapper {
    tool_use: ToolUseBlock,
}

// ── BedrockProvider ─────────────────────────────────────────────

pub struct BedrockProvider {
    credentials: Option<AwsCredentials>,
}

impl BedrockProvider {
    pub fn new() -> Self {
        Self {
            credentials: AwsCredentials::from_env().ok(),
        }
    }

    fn http_client(&self) -> Client {
        crate::openhuman::config::build_runtime_proxy_client_with_timeouts("provider.bedrock", 120, 10)
    }

    /// Percent-encode the model ID for URL path: only encode `:` to `%3A`.
    /// Colons in model IDs (e.g. `v1:0`) must be encoded because `reqwest::Url`
    /// may misparse them. Dots, hyphens, and alphanumerics are safe.
    fn encode_model_path(model_id: &str) -> String {
        model_id.replace(':', "%3A")
    }

    /// Build the actual request URL. Uses raw model ID (reqwest sends colons as-is).
    fn endpoint_url(region: &str, model_id: &str) -> String {
        format!("https://{ENDPOINT_PREFIX}.{region}.amazonaws.com/model/{model_id}/converse")
    }

    /// Build the canonical URI for SigV4 signing. Must URI-encode the path
    /// per SigV4 spec: colons become `%3A`. AWS verifies the signature against
    /// the encoded form even though the wire request uses raw colons.
    fn canonical_uri(model_id: &str) -> String {
        let encoded = Self::encode_model_path(model_id);
        format!("/model/{encoded}/converse")
    }

    fn require_credentials(&self) -> anyhow::Result<&AwsCredentials> {
        self.credentials.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "AWS Bedrock credentials not set. Set AWS_ACCESS_KEY_ID and \
                 AWS_SECRET_ACCESS_KEY environment variables."
            )
        })
    }

    // ── Cache heuristics (same thresholds as AnthropicProvider) ──

    /// Cache system prompts larger than ~1024 tokens (3KB of text).
    fn should_cache_system(text: &str) -> bool {
        text.len() > 3072
    }

    /// Cache conversations with more than 4 messages (excluding system).
    fn should_cache_conversation(messages: &[ChatMessage]) -> bool {
        messages.iter().filter(|m| m.role != "system").count() > 4
    }

    // ── Message conversion ──────────────────────────────────────

    fn convert_messages(
        messages: &[ChatMessage],
    ) -> (Option<Vec<SystemBlock>>, Vec<ConverseMessage>) {
        let mut system_blocks = Vec::new();
        let mut converse_messages = Vec::new();

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    if system_blocks.is_empty() {
                        system_blocks.push(SystemBlock::Text(TextBlock {
                            text: msg.content.clone(),
                        }));
                    }
                }
                "assistant" => {
                    if let Some(blocks) = Self::parse_assistant_tool_call_message(&msg.content) {
                        converse_messages.push(ConverseMessage {
                            role: "assistant".to_string(),
                            content: blocks,
                        });
                    } else {
                        converse_messages.push(ConverseMessage {
                            role: "assistant".to_string(),
                            content: vec![ContentBlock::Text(TextBlock {
                                text: msg.content.clone(),
                            })],
                        });
                    }
                }
                "tool" => {
                    if let Some(tool_result_msg) = Self::parse_tool_result_message(&msg.content) {
                        converse_messages.push(tool_result_msg);
                    } else {
                        converse_messages.push(ConverseMessage {
                            role: "user".to_string(),
                            content: vec![ContentBlock::Text(TextBlock {
                                text: msg.content.clone(),
                            })],
                        });
                    }
                }
                _ => {
                    converse_messages.push(ConverseMessage {
                        role: "user".to_string(),
                        content: vec![ContentBlock::Text(TextBlock {
                            text: msg.content.clone(),
                        })],
                    });
                }
            }
        }

        let system = if system_blocks.is_empty() {
            None
        } else {
            Some(system_blocks)
        };
        (system, converse_messages)
    }

    /// Parse assistant message containing structured tool calls.
    fn parse_assistant_tool_call_message(content: &str) -> Option<Vec<ContentBlock>> {
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
            blocks.push(ContentBlock::Text(TextBlock {
                text: text.to_string(),
            }));
        }
        for call in tool_calls {
            let input = serde_json::from_str::<serde_json::Value>(&call.arguments)
                .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));
            blocks.push(ContentBlock::ToolUse(ToolUseWrapper {
                tool_use: ToolUseBlock {
                    tool_use_id: call.id,
                    name: call.name,
                    input,
                },
            }));
        }
        Some(blocks)
    }

    /// Parse tool result message into a user message with ToolResult block.
    fn parse_tool_result_message(content: &str) -> Option<ConverseMessage> {
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
        Some(ConverseMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::ToolResult(ToolResultWrapper {
                tool_result: ToolResultBlock {
                    tool_use_id,
                    content: vec![ToolResultContent { text: result }],
                    status: "success".to_string(),
                },
            })],
        })
    }

    // ── Tool conversion ─────────────────────────────────────────

    fn convert_tools_to_converse(tools: Option<&[ToolSpec]>) -> Option<ToolConfig> {
        let items = tools?;
        if items.is_empty() {
            return None;
        }
        let tool_defs: Vec<ToolDefinition> = items
            .iter()
            .map(|tool| ToolDefinition {
                tool_spec: ToolSpecDef {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    input_schema: InputSchema {
                        json: tool.parameters.clone(),
                    },
                },
            })
            .collect();
        Some(ToolConfig { tools: tool_defs })
    }

    // ── Response parsing ────────────────────────────────────────

    fn parse_converse_response(response: ConverseResponse) -> ProviderChatResponse {
        let mut text_parts = Vec::new();
        let mut tool_calls = Vec::new();

        if let Some(output) = response.output {
            if let Some(message) = output.message {
                for block in message.content {
                    match block {
                        ResponseContentBlock::Text(tb) => {
                            let trimmed = tb.text.trim().to_string();
                            if !trimmed.is_empty() {
                                text_parts.push(trimmed);
                            }
                        }
                        ResponseContentBlock::ToolUse(wrapper) => {
                            if !wrapper.tool_use.name.is_empty() {
                                tool_calls.push(ProviderToolCall {
                                    id: wrapper.tool_use.tool_use_id,
                                    name: wrapper.tool_use.name,
                                    arguments: wrapper.tool_use.input.to_string(),
                                });
                            }
                        }
                        ResponseContentBlock::Other(_) => {}
                    }
                }
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

    // ── HTTP request ────────────────────────────────────────────

    async fn send_converse_request(
        &self,
        credentials: &AwsCredentials,
        model: &str,
        request_body: &ConverseRequest,
    ) -> anyhow::Result<ConverseResponse> {
        let payload = serde_json::to_vec(request_body)?;
        let url = Self::endpoint_url(&credentials.region, model);
        let canonical_uri = Self::canonical_uri(model);
        let now = chrono::Utc::now();
        let host = credentials.host();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();

        let mut headers_to_sign = vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("host".to_string(), host),
            ("x-amz-date".to_string(), amz_date.clone()),
        ];
        if let Some(ref token) = credentials.session_token {
            headers_to_sign.push(("x-amz-security-token".to_string(), token.clone()));
        }
        headers_to_sign.sort_by(|a, b| a.0.cmp(&b.0));

        let authorization = build_authorization_header(
            credentials,
            "POST",
            &canonical_uri,
            "",
            &headers_to_sign,
            &payload,
            &now,
        );

        let mut request = self
            .http_client()
            .post(&url)
            .header("content-type", "application/json")
            .header("x-amz-date", &amz_date)
            .header("authorization", &authorization);

        if let Some(ref token) = credentials.session_token {
            request = request.header("x-amz-security-token", token);
        }

        let response: reqwest::Response = request.body(payload).send().await?;

        if !response.status().is_success() {
            return Err(super::api_error("Bedrock", response).await);
        }

        let converse_response: ConverseResponse = response.json().await?;
        Ok(converse_response)
    }
}

// ── Provider trait implementation ───────────────────────────────

#[async_trait]
impl Provider for BedrockProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            vision: false,
        }
    }

    fn supports_native_tools(&self) -> bool {
        true
    }

    fn convert_tools(&self, tools: &[ToolSpec]) -> ToolsPayload {
        let tool_values: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "toolSpec": {
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": { "json": t.parameters }
                    }
                })
            })
            .collect();
        ToolsPayload::Anthropic { tools: tool_values }
    }

    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let credentials = self.require_credentials()?;

        let system = system_prompt.map(|text| {
            let mut blocks = vec![SystemBlock::Text(TextBlock {
                text: text.to_string(),
            })];
            if Self::should_cache_system(text) {
                blocks.push(SystemBlock::CachePoint(CachePointWrapper {
                    cache_point: CachePoint::default_cache(),
                }));
            }
            blocks
        });

        let request = ConverseRequest {
            system,
            messages: vec![ConverseMessage {
                role: "user".to_string(),
                content: vec![ContentBlock::Text(TextBlock {
                    text: message.to_string(),
                })],
            }],
            inference_config: Some(InferenceConfig {
                max_tokens: DEFAULT_MAX_TOKENS,
                temperature,
            }),
            tool_config: None,
        };

        let response = self
            .send_converse_request(credentials, model, &request)
            .await?;

        Self::parse_converse_response(response)
            .text
            .ok_or_else(|| anyhow::anyhow!("No response from Bedrock"))
    }

    async fn chat(
        &self,
        request: ProviderChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ProviderChatResponse> {
        let credentials = self.require_credentials()?;

        let (system_blocks, mut converse_messages) = Self::convert_messages(request.messages);

        // Apply cachePoint to system if large.
        let system = system_blocks.map(|mut blocks| {
            let has_large_system = blocks
                .iter()
                .any(|b| matches!(b, SystemBlock::Text(tb) if Self::should_cache_system(&tb.text)));
            if has_large_system {
                blocks.push(SystemBlock::CachePoint(CachePointWrapper {
                    cache_point: CachePoint::default_cache(),
                }));
            }
            blocks
        });

        // Apply cachePoint to last message if conversation is long.
        if Self::should_cache_conversation(request.messages) {
            if let Some(last_msg) = converse_messages.last_mut() {
                last_msg
                    .content
                    .push(ContentBlock::CachePointBlock(CachePointWrapper {
                        cache_point: CachePoint::default_cache(),
                    }));
            }
        }

        let tool_config = Self::convert_tools_to_converse(request.tools);

        let converse_request = ConverseRequest {
            system,
            messages: converse_messages,
            inference_config: Some(InferenceConfig {
                max_tokens: DEFAULT_MAX_TOKENS,
                temperature,
            }),
            tool_config,
        };

        let response = self
            .send_converse_request(credentials, model, &converse_request)
            .await?;

        Ok(Self::parse_converse_response(response))
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        if let Some(ref creds) = self.credentials {
            let url = format!("https://{ENDPOINT_PREFIX}.{}.amazonaws.com/", creds.region);
            let _ = self.http_client().get(&url).send().await;
        }
        Ok(())
    }
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::providers::traits::ChatMessage;

    // ── SigV4 signing tests ─────────────────────────────────────

    #[test]
    fn sha256_hex_empty_string() {
        // Known SHA-256 of empty input
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_hex_known_input() {
        // SHA-256 of "hello"
        assert_eq!(
            sha256_hex(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn hmac_sha256_known_input() {
        let result = hmac_sha256(b"key", b"message");
        assert_eq!(
            hex::encode(&result),
            "6e9ef29b75fffc5b7abae527d58fdadb2fe42e7219011976917343065f58ed4a"
        );
    }

    #[test]
    fn derive_signing_key_structure() {
        // Verify the key derivation produces a 32-byte key (SHA-256 output).
        let key = derive_signing_key(
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "20150830",
            "us-east-1",
            "iam",
        );
        assert_eq!(key.len(), 32);
    }

    #[test]
    fn derive_signing_key_known_test_vector() {
        // AWS SigV4 test vector from documentation.
        // Secret: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY"
        // Date: "20150830", Region: "us-east-1", Service: "iam"
        let key = derive_signing_key(
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "20150830",
            "us-east-1",
            "iam",
        );
        assert_eq!(
            hex::encode(&key),
            "c4afb1cc5771d871763a393e44b703571b55cc28424d1a5e86da6ed3c154a4b9"
        );
    }

    #[test]
    fn build_authorization_header_format() {
        let credentials = AwsCredentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_string(),
            session_token: None,
            region: "us-east-1".to_string(),
        };

        let timestamp = chrono::DateTime::parse_from_rfc3339("2024-01-15T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let headers = vec![
            ("content-type".to_string(), "application/json".to_string()),
            (
                "host".to_string(),
                "bedrock-runtime.us-east-1.amazonaws.com".to_string(),
            ),
            ("x-amz-date".to_string(), "20240115T120000Z".to_string()),
        ];

        let auth = build_authorization_header(
            &credentials,
            "POST",
            "/model/anthropic.claude-3-sonnet/converse",
            "",
            &headers,
            b"{}",
            &timestamp,
        );

        // Verify structure
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/"));
        assert!(auth.contains("SignedHeaders=content-type;host;x-amz-date"));
        assert!(auth.contains("Signature="));
        assert!(auth.contains("/us-east-1/bedrock/aws4_request"));
    }

    #[test]
    fn build_authorization_header_includes_security_token_in_signed_headers() {
        let credentials = AwsCredentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_string(),
            session_token: Some("session-token-value".to_string()),
            region: "us-east-1".to_string(),
        };

        let timestamp = chrono::DateTime::parse_from_rfc3339("2024-01-15T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let headers = vec![
            ("content-type".to_string(), "application/json".to_string()),
            (
                "host".to_string(),
                "bedrock-runtime.us-east-1.amazonaws.com".to_string(),
            ),
            ("x-amz-date".to_string(), "20240115T120000Z".to_string()),
            (
                "x-amz-security-token".to_string(),
                "session-token-value".to_string(),
            ),
        ];

        let auth = build_authorization_header(
            &credentials,
            "POST",
            "/model/test-model/converse",
            "",
            &headers,
            b"{}",
            &timestamp,
        );

        assert!(auth.contains("x-amz-security-token"));
    }

    // ── Credential tests ────────────────────────────────────────

    #[test]
    fn credentials_host_formats_correctly() {
        let creds = AwsCredentials {
            access_key_id: "AKID".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: None,
            region: "us-west-2".to_string(),
        };
        assert_eq!(creds.host(), "bedrock-runtime.us-west-2.amazonaws.com");
    }

    // ── Provider construction tests ─────────────────────────────

    #[test]
    fn creates_without_credentials() {
        // Provider should construct even without env vars.
        let _provider = BedrockProvider::new();
    }

    #[tokio::test]
    async fn chat_fails_without_credentials() {
        let provider = BedrockProvider { credentials: None };
        let result = provider
            .chat_with_system(None, "hello", "anthropic.claude-sonnet-4-6", 0.7)
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("credentials not set"),
            "Expected credentials error, got: {err}"
        );
    }

    // ── Endpoint URL tests ──────────────────────────────────────

    #[test]
    fn endpoint_url_formats_correctly() {
        let url = BedrockProvider::endpoint_url("us-east-1", "anthropic.claude-sonnet-4-6");
        assert_eq!(
            url,
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-sonnet-4-6/converse"
        );
    }

    #[test]
    fn endpoint_url_keeps_raw_colon() {
        // Endpoint URL uses raw colon so reqwest sends `:` on the wire.
        let url =
            BedrockProvider::endpoint_url("us-west-2", "anthropic.claude-3-5-haiku-20241022-v1:0");
        assert!(url.contains("/model/anthropic.claude-3-5-haiku-20241022-v1:0/converse"));
    }

    #[test]
    fn canonical_uri_encodes_colon() {
        // Canonical URI must encode `:` as `%3A` for SigV4 signing.
        let uri = BedrockProvider::canonical_uri("anthropic.claude-3-5-haiku-20241022-v1:0");
        assert_eq!(
            uri,
            "/model/anthropic.claude-3-5-haiku-20241022-v1%3A0/converse"
        );
    }

    #[test]
    fn canonical_uri_no_colon_unchanged() {
        let uri = BedrockProvider::canonical_uri("anthropic.claude-sonnet-4-6");
        assert_eq!(uri, "/model/anthropic.claude-sonnet-4-6/converse");
    }

    // ── Message conversion tests ────────────────────────────────

    #[test]
    fn convert_messages_system_extracted() {
        let messages = vec![
            ChatMessage::system("You are helpful"),
            ChatMessage::user("Hello"),
        ];
        let (system, msgs) = BedrockProvider::convert_messages(&messages);
        assert!(system.is_some());
        let system_blocks = system.unwrap();
        assert_eq!(system_blocks.len(), 1);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
    }

    #[test]
    fn convert_messages_user_and_assistant() {
        let messages = vec![
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there"),
        ];
        let (system, msgs) = BedrockProvider::convert_messages(&messages);
        assert!(system.is_none());
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
    }

    #[test]
    fn convert_messages_tool_role_to_tool_result() {
        let tool_json = r#"{"tool_call_id": "call_123", "content": "Result data"}"#;
        let messages = vec![ChatMessage::tool(tool_json)];
        let (_, msgs) = BedrockProvider::convert_messages(&messages);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert!(matches!(msgs[0].content[0], ContentBlock::ToolResult(_)));
    }

    #[test]
    fn convert_messages_assistant_tool_calls_parsed() {
        let tool_call_json = r#"{"content": "Let me check", "tool_calls": [{"id": "call_1", "name": "shell", "arguments": "{\"command\":\"ls\"}"}]}"#;
        let messages = vec![ChatMessage::assistant(tool_call_json)];
        let (_, msgs) = BedrockProvider::convert_messages(&messages);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "assistant");
        assert_eq!(msgs[0].content.len(), 2);
        assert!(matches!(msgs[0].content[0], ContentBlock::Text(_)));
        assert!(matches!(msgs[0].content[1], ContentBlock::ToolUse(_)));
    }

    #[test]
    fn convert_messages_plain_assistant_text() {
        let messages = vec![ChatMessage::assistant("Just text")];
        let (_, msgs) = BedrockProvider::convert_messages(&messages);
        assert_eq!(msgs.len(), 1);
        assert!(matches!(msgs[0].content[0], ContentBlock::Text(_)));
    }

    // ── Cache tests ─────────────────────────────────────────────

    #[test]
    fn should_cache_system_small_prompt() {
        assert!(!BedrockProvider::should_cache_system("Short prompt"));
    }

    #[test]
    fn should_cache_system_large_prompt() {
        let large = "a".repeat(3073);
        assert!(BedrockProvider::should_cache_system(&large));
    }

    #[test]
    fn should_cache_system_boundary() {
        assert!(!BedrockProvider::should_cache_system(&"a".repeat(3072)));
        assert!(BedrockProvider::should_cache_system(&"a".repeat(3073)));
    }

    #[test]
    fn should_cache_conversation_short() {
        let messages = vec![
            ChatMessage::system("System"),
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi"),
        ];
        assert!(!BedrockProvider::should_cache_conversation(&messages));
    }

    #[test]
    fn should_cache_conversation_long() {
        let mut messages = vec![ChatMessage::system("System")];
        for i in 0..5 {
            messages.push(ChatMessage {
                role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
                content: format!("Message {i}"),
            });
        }
        assert!(BedrockProvider::should_cache_conversation(&messages));
    }

    // ── Tool conversion tests ───────────────────────────────────

    #[test]
    fn convert_tools_to_converse_formats_correctly() {
        let tools = vec![ToolSpec {
            name: "shell".to_string(),
            description: "Run commands".to_string(),
            parameters: serde_json::json!({"type": "object", "properties": {"command": {"type": "string"}}}),
        }];
        let config = BedrockProvider::convert_tools_to_converse(Some(&tools));
        assert!(config.is_some());
        let config = config.unwrap();
        assert_eq!(config.tools.len(), 1);
        assert_eq!(config.tools[0].tool_spec.name, "shell");
    }

    #[test]
    fn convert_tools_to_converse_empty_returns_none() {
        assert!(BedrockProvider::convert_tools_to_converse(Some(&[])).is_none());
        assert!(BedrockProvider::convert_tools_to_converse(None).is_none());
    }

    // ── Serde tests ─────────────────────────────────────────────

    #[test]
    fn converse_request_serializes_without_system() {
        let req = ConverseRequest {
            system: None,
            messages: vec![ConverseMessage {
                role: "user".to_string(),
                content: vec![ContentBlock::Text(TextBlock {
                    text: "Hello".to_string(),
                })],
            }],
            inference_config: Some(InferenceConfig {
                max_tokens: 4096,
                temperature: 0.7,
            }),
            tool_config: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("system"));
        assert!(json.contains("Hello"));
        assert!(json.contains("maxTokens"));
    }

    #[test]
    fn converse_response_deserializes_text() {
        let json = r#"{
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [{"text": "Hello from Bedrock"}]
                }
            },
            "stopReason": "end_turn"
        }"#;
        let resp: ConverseResponse = serde_json::from_str(json).unwrap();
        let parsed = BedrockProvider::parse_converse_response(resp);
        assert_eq!(parsed.text.as_deref(), Some("Hello from Bedrock"));
        assert!(parsed.tool_calls.is_empty());
    }

    #[test]
    fn converse_response_deserializes_tool_use() {
        let json = r#"{
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [
                        {"toolUse": {"toolUseId": "call_1", "name": "shell", "input": {"command": "ls"}}}
                    ]
                }
            },
            "stopReason": "tool_use"
        }"#;
        let resp: ConverseResponse = serde_json::from_str(json).unwrap();
        let parsed = BedrockProvider::parse_converse_response(resp);
        assert!(parsed.text.is_none());
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "shell");
        assert_eq!(parsed.tool_calls[0].id, "call_1");
    }

    #[test]
    fn converse_response_empty_output() {
        let json = r#"{"output": null, "stopReason": null}"#;
        let resp: ConverseResponse = serde_json::from_str(json).unwrap();
        let parsed = BedrockProvider::parse_converse_response(resp);
        assert!(parsed.text.is_none());
        assert!(parsed.tool_calls.is_empty());
    }

    #[test]
    fn content_block_text_serializes_as_flat_string() {
        let block = ContentBlock::Text(TextBlock {
            text: "Hello".to_string(),
        });
        let json = serde_json::to_string(&block).unwrap();
        // Must be {"text":"Hello"}, NOT {"text":{"text":"Hello"}}
        assert_eq!(json, r#"{"text":"Hello"}"#);
    }

    #[test]
    fn content_block_tool_use_serializes_with_nested_object() {
        let block = ContentBlock::ToolUse(ToolUseWrapper {
            tool_use: ToolUseBlock {
                tool_use_id: "call_1".to_string(),
                name: "shell".to_string(),
                input: serde_json::json!({"command": "ls"}),
            },
        });
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains(r#""toolUse""#));
        assert!(json.contains(r#""toolUseId":"call_1""#));
    }

    #[test]
    fn content_block_cache_point_serializes() {
        let block = ContentBlock::CachePointBlock(CachePointWrapper {
            cache_point: CachePoint::default_cache(),
        });
        let json = serde_json::to_string(&block).unwrap();
        assert_eq!(json, r#"{"cachePoint":{"type":"default"}}"#);
    }

    #[test]
    fn content_block_text_round_trips() {
        let original = ContentBlock::Text(TextBlock {
            text: "Hello".to_string(),
        });
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: ContentBlock = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized, ContentBlock::Text(tb) if tb.text == "Hello"));
    }

    #[test]
    fn cache_point_serializes() {
        let cp = CachePoint::default_cache();
        let json = serde_json::to_string(&cp).unwrap();
        assert_eq!(json, r#"{"type":"default"}"#);
    }

    #[tokio::test]
    async fn warmup_without_credentials_is_noop() {
        let provider = BedrockProvider { credentials: None };
        let result = provider.warmup().await;
        assert!(result.is_ok());
    }

    #[test]
    fn capabilities_reports_native_tool_calling() {
        let provider = BedrockProvider { credentials: None };
        let caps = provider.capabilities();
        assert!(caps.native_tool_calling);
    }
}
