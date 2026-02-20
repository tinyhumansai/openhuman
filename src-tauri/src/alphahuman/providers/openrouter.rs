use crate::alphahuman::providers::traits::{
    ChatMessage, ChatRequest as ProviderChatRequest, ChatResponse as ProviderChatResponse,
    Provider, ToolCall as ProviderToolCall,
};
use crate::alphahuman::tools::ToolSpec;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub struct OpenRouterProvider {
    credential: Option<String>,
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
struct ApiChatResponse {
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

#[derive(Debug, Serialize)]
struct NativeChatRequest {
    model: String,
    messages: Vec<NativeMessage>,
    temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<NativeToolSpec>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Debug, Serialize)]
struct NativeMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<NativeToolCall>>,
}

#[derive(Debug, Serialize)]
struct NativeToolSpec {
    #[serde(rename = "type")]
    kind: String,
    function: NativeToolFunctionSpec,
}

#[derive(Debug, Serialize)]
struct NativeToolFunctionSpec {
    name: String,
    description: String,
    parameters: serde_json::Value,
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
struct NativeChatResponse {
    choices: Vec<NativeChoice>,
}

#[derive(Debug, Deserialize)]
struct NativeChoice {
    message: NativeResponseMessage,
}

#[derive(Debug, Deserialize)]
struct NativeResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<NativeToolCall>>,
}

impl OpenRouterProvider {
    pub fn new(credential: Option<&str>) -> Self {
        Self {
            credential: credential.map(ToString::to_string),
        }
    }

    fn convert_tools(tools: Option<&[ToolSpec]>) -> Option<Vec<NativeToolSpec>> {
        let items = tools?;
        if items.is_empty() {
            return None;
        }
        Some(
            items
                .iter()
                .map(|tool| NativeToolSpec {
                    kind: "function".to_string(),
                    function: NativeToolFunctionSpec {
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                        parameters: tool.parameters.clone(),
                    },
                })
                .collect(),
        )
    }

    fn convert_messages(messages: &[ChatMessage]) -> Vec<NativeMessage> {
        messages
            .iter()
            .map(|m| {
                if m.role == "assistant" {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&m.content) {
                        if let Some(tool_calls_value) = value.get("tool_calls") {
                            if let Ok(parsed_calls) =
                                serde_json::from_value::<Vec<ProviderToolCall>>(
                                    tool_calls_value.clone(),
                                )
                            {
                                let tool_calls = parsed_calls
                                    .into_iter()
                                    .map(|tc| NativeToolCall {
                                        id: Some(tc.id),
                                        kind: Some("function".to_string()),
                                        function: NativeFunctionCall {
                                            name: tc.name,
                                            arguments: tc.arguments,
                                        },
                                    })
                                    .collect::<Vec<_>>();
                                let content = value
                                    .get("content")
                                    .and_then(serde_json::Value::as_str)
                                    .map(ToString::to_string);
                                return NativeMessage {
                                    role: "assistant".to_string(),
                                    content,
                                    tool_call_id: None,
                                    tool_calls: Some(tool_calls),
                                };
                            }
                        }
                    }
                }

                if m.role == "tool" {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&m.content) {
                        let tool_call_id = value
                            .get("tool_call_id")
                            .and_then(serde_json::Value::as_str)
                            .map(ToString::to_string);
                        let content = value
                            .get("content")
                            .and_then(serde_json::Value::as_str)
                            .map(ToString::to_string);
                        return NativeMessage {
                            role: "tool".to_string(),
                            content,
                            tool_call_id,
                            tool_calls: None,
                        };
                    }
                }

                NativeMessage {
                    role: m.role.clone(),
                    content: Some(m.content.clone()),
                    tool_call_id: None,
                    tool_calls: None,
                }
            })
            .collect()
    }

    fn parse_native_response(message: NativeResponseMessage) -> ProviderChatResponse {
        let tool_calls = message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tc| ProviderToolCall {
                id: tc.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                name: tc.function.name,
                arguments: tc.function.arguments,
            })
            .collect::<Vec<_>>();

        ProviderChatResponse {
            text: message.content,
            tool_calls,
        }
    }

    fn http_client(&self) -> Client {
        crate::alphahuman::config::build_runtime_proxy_client_with_timeouts("provider.openrouter", 120, 10)
    }
}

#[async_trait]
impl Provider for OpenRouterProvider {
    async fn warmup(&self) -> anyhow::Result<()> {
        // Hit a lightweight endpoint to establish TLS + HTTP/2 connection pool.
        // This prevents the first real chat request from timing out on cold start.
        if let Some(credential) = self.credential.as_ref() {
            self.http_client()
                .get("https://openrouter.ai/api/v1/auth/key")
                .header("Authorization", format!("Bearer {credential}"))
                .send()
                .await?
                .error_for_status()?;
        }
        Ok(())
    }

    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let credential = self.credential.as_ref()
            .ok_or_else(|| anyhow::anyhow!("OpenRouter API key not set. Configure via the web UI or set OPENROUTER_API_KEY env var."))?;

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

        let response = self
            .http_client()
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {credential}"))
            .header(
                "HTTP-Referer",
                "https://github.com/theonlyhennygod/alphahuman",
            )
            .header("X-Title", "Alphahuman")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(super::api_error("OpenRouter", response).await);
        }

        let chat_response: ApiChatResponse = response.json().await?;

        chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow::anyhow!("No response from OpenRouter"))
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let credential = self.credential.as_ref()
            .ok_or_else(|| anyhow::anyhow!("OpenRouter API key not set. Configure via the web UI or set OPENROUTER_API_KEY env var."))?;

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

        let response = self
            .http_client()
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {credential}"))
            .header(
                "HTTP-Referer",
                "https://github.com/theonlyhennygod/alphahuman",
            )
            .header("X-Title", "Alphahuman")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(super::api_error("OpenRouter", response).await);
        }

        let chat_response: ApiChatResponse = response.json().await?;

        chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow::anyhow!("No response from OpenRouter"))
    }

    async fn chat(
        &self,
        request: ProviderChatRequest<'_>,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ProviderChatResponse> {
        let credential = self.credential.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
            "OpenRouter API key not set. Configure via the web UI or set OPENROUTER_API_KEY env var."
        )
        })?;

        let tools = Self::convert_tools(request.tools);
        let native_request = NativeChatRequest {
            model: model.to_string(),
            messages: Self::convert_messages(request.messages),
            temperature,
            tool_choice: tools.as_ref().map(|_| "auto".to_string()),
            tools,
        };

        let response = self
            .http_client()
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {credential}"))
            .header(
                "HTTP-Referer",
                "https://github.com/theonlyhennygod/alphahuman",
            )
            .header("X-Title", "Alphahuman")
            .json(&native_request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(super::api_error("OpenRouter", response).await);
        }

        let native_response: NativeChatResponse = response.json().await?;
        let message = native_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message)
            .ok_or_else(|| anyhow::anyhow!("No response from OpenRouter"))?;
        Ok(Self::parse_native_response(message))
    }

    fn supports_native_tools(&self) -> bool {
        true
    }

    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<ProviderChatResponse> {
        let credential = self.credential.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "OpenRouter API key not set. Configure via the web UI or set OPENROUTER_API_KEY env var."
            )
        })?;

        // Convert tool JSON values to NativeToolSpec
        let native_tools: Option<Vec<NativeToolSpec>> = if tools.is_empty() {
            None
        } else {
            let specs: Vec<NativeToolSpec> = tools
                .iter()
                .filter_map(|t| {
                    let func = t.get("function")?;
                    Some(NativeToolSpec {
                        kind: "function".to_string(),
                        function: NativeToolFunctionSpec {
                            name: func.get("name")?.as_str()?.to_string(),
                            description: func
                                .get("description")
                                .and_then(|d| d.as_str())
                                .unwrap_or("")
                                .to_string(),
                            parameters: func
                                .get("parameters")
                                .cloned()
                                .unwrap_or(serde_json::json!({})),
                        },
                    })
                })
                .collect();
            if specs.is_empty() {
                None
            } else {
                Some(specs)
            }
        };

        // Convert ChatMessage to NativeMessage, preserving structured assistant/tool entries
        // when history contains native tool-call metadata.
        let native_messages = Self::convert_messages(messages);

        let native_request = NativeChatRequest {
            model: model.to_string(),
            messages: native_messages,
            temperature,
            tool_choice: native_tools.as_ref().map(|_| "auto".to_string()),
            tools: native_tools,
        };

        let response = self
            .http_client()
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {credential}"))
            .header(
                "HTTP-Referer",
                "https://github.com/theonlyhennygod/alphahuman",
            )
            .header("X-Title", "Alphahuman")
            .json(&native_request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(super::api_error("OpenRouter", response).await);
        }

        let native_response: NativeChatResponse = response.json().await?;
        let message = native_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message)
            .ok_or_else(|| anyhow::anyhow!("No response from OpenRouter"))?;
        Ok(Self::parse_native_response(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alphahuman::providers::traits::{ChatMessage, Provider};

    #[test]
    fn creates_with_key() {
        let provider = OpenRouterProvider::new(Some("openrouter-test-credential"));
        assert_eq!(
            provider.credential.as_deref(),
            Some("openrouter-test-credential")
        );
    }

    #[test]
    fn creates_without_key() {
        let provider = OpenRouterProvider::new(None);
        assert!(provider.credential.is_none());
    }

    #[tokio::test]
    async fn warmup_without_key_is_noop() {
        let provider = OpenRouterProvider::new(None);
        let result = provider.warmup().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn chat_with_system_fails_without_key() {
        let provider = OpenRouterProvider::new(None);
        let result = provider
            .chat_with_system(Some("system"), "hello", "openai/gpt-4o", 0.2)
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("API key not set"));
    }

    #[tokio::test]
    async fn chat_with_history_fails_without_key() {
        let provider = OpenRouterProvider::new(None);
        let messages = vec![
            ChatMessage {
                role: "system".into(),
                content: "be concise".into(),
            },
            ChatMessage {
                role: "user".into(),
                content: "hello".into(),
            },
        ];

        let result = provider
            .chat_with_history(&messages, "anthropic/claude-sonnet-4", 0.7)
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("API key not set"));
    }

    #[test]
    fn chat_request_serializes_with_system_and_user() {
        let request = ChatRequest {
            model: "anthropic/claude-sonnet-4".into(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: "You are helpful".into(),
                },
                Message {
                    role: "user".into(),
                    content: "Summarize this".into(),
                },
            ],
            temperature: 0.5,
        };

        let json = serde_json::to_string(&request).unwrap();

        assert!(json.contains("anthropic/claude-sonnet-4"));
        assert!(json.contains("\"role\":\"system\""));
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"temperature\":0.5"));
    }

    #[test]
    fn chat_request_serializes_history_messages() {
        let messages = [
            ChatMessage {
                role: "assistant".into(),
                content: "Previous answer".into(),
            },
            ChatMessage {
                role: "user".into(),
                content: "Follow-up".into(),
            },
        ];

        let request = ChatRequest {
            model: "google/gemini-2.5-pro".into(),
            messages: messages
                .iter()
                .map(|msg| Message {
                    role: msg.role.clone(),
                    content: msg.content.clone(),
                })
                .collect(),
            temperature: 0.0,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"role\":\"assistant\""));
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("google/gemini-2.5-pro"));
    }

    #[test]
    fn response_deserializes_single_choice() {
        let json = r#"{"choices":[{"message":{"content":"Hi from OpenRouter"}}]}"#;

        let response: ApiChatResponse = serde_json::from_str(json).unwrap();

        assert_eq!(response.choices.len(), 1);
        assert_eq!(response.choices[0].message.content, "Hi from OpenRouter");
    }

    #[test]
    fn response_deserializes_empty_choices() {
        let json = r#"{"choices":[]}"#;

        let response: ApiChatResponse = serde_json::from_str(json).unwrap();

        assert!(response.choices.is_empty());
    }

    #[tokio::test]
    async fn chat_with_tools_fails_without_key() {
        let provider = OpenRouterProvider::new(None);
        let messages = vec![ChatMessage {
            role: "user".into(),
            content: "What is the date?".into(),
        }];
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": {
                "name": "shell",
                "description": "Run a shell command",
                "parameters": {"type": "object", "properties": {"command": {"type": "string"}}}
            }
        })];

        let result = provider
            .chat_with_tools(&messages, &tools, "deepseek/deepseek-chat", 0.5)
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("API key not set"));
    }

    #[test]
    fn native_response_deserializes_with_tool_calls() {
        let json = r#"{
            "choices":[{
                "message":{
                    "content":null,
                    "tool_calls":[
                        {"id":"call_123","type":"function","function":{"name":"get_price","arguments":"{\"symbol\":\"BTC\"}"}}
                    ]
                }
            }]
        }"#;

        let response: NativeChatResponse = serde_json::from_str(json).unwrap();

        assert_eq!(response.choices.len(), 1);
        let message = &response.choices[0].message;
        assert!(message.content.is_none());
        let tool_calls = message.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id.as_deref(), Some("call_123"));
        assert_eq!(tool_calls[0].function.name, "get_price");
        assert_eq!(tool_calls[0].function.arguments, "{\"symbol\":\"BTC\"}");
    }

    #[test]
    fn native_response_deserializes_with_text_and_tool_calls() {
        let json = r#"{
            "choices":[{
                "message":{
                    "content":"I'll get that for you.",
                    "tool_calls":[
                        {"id":"call_456","type":"function","function":{"name":"shell","arguments":"{\"command\":\"date\"}"}}
                    ]
                }
            }]
        }"#;

        let response: NativeChatResponse = serde_json::from_str(json).unwrap();

        assert_eq!(response.choices.len(), 1);
        let message = &response.choices[0].message;
        assert_eq!(message.content.as_deref(), Some("I'll get that for you."));
        let tool_calls = message.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "shell");
    }

    #[test]
    fn parse_native_response_converts_to_chat_response() {
        let message = NativeResponseMessage {
            content: Some("Here you go.".into()),
            tool_calls: Some(vec![NativeToolCall {
                id: Some("call_789".into()),
                kind: Some("function".into()),
                function: NativeFunctionCall {
                    name: "file_read".into(),
                    arguments: r#"{"path":"test.txt"}"#.into(),
                },
            }]),
        };

        let response = OpenRouterProvider::parse_native_response(message);

        assert_eq!(response.text.as_deref(), Some("Here you go."));
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].id, "call_789");
        assert_eq!(response.tool_calls[0].name, "file_read");
    }

    #[test]
    fn convert_messages_parses_assistant_tool_call_payload() {
        let messages = vec![ChatMessage {
            role: "assistant".into(),
            content: r#"{"content":"Using tool","tool_calls":[{"id":"call_abc","name":"shell","arguments":"{\"command\":\"pwd\"}"}]}"#
                .into(),
        }];

        let converted = OpenRouterProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "assistant");
        assert_eq!(converted[0].content.as_deref(), Some("Using tool"));

        let tool_calls = converted[0].tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id.as_deref(), Some("call_abc"));
        assert_eq!(tool_calls[0].function.name, "shell");
        assert_eq!(tool_calls[0].function.arguments, r#"{"command":"pwd"}"#);
    }

    #[test]
    fn convert_messages_parses_tool_result_payload() {
        let messages = vec![ChatMessage {
            role: "tool".into(),
            content: r#"{"tool_call_id":"call_xyz","content":"done"}"#.into(),
        }];

        let converted = OpenRouterProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "tool");
        assert_eq!(converted[0].tool_call_id.as_deref(), Some("call_xyz"));
        assert_eq!(converted[0].content.as_deref(), Some("done"));
        assert!(converted[0].tool_calls.is_none());
    }
}
