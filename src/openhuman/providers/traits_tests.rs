use super::*;

struct CapabilityMockProvider;

#[async_trait]
impl Provider for CapabilityMockProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            vision: true,
        }
    }

    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("ok".into())
    }
}

#[test]
fn chat_message_constructors() {
    let sys = ChatMessage::system("Be helpful");
    assert_eq!(sys.role, "system");
    assert_eq!(sys.content, "Be helpful");

    let user = ChatMessage::user("Hello");
    assert_eq!(user.role, "user");

    let asst = ChatMessage::assistant("Hi there");
    assert_eq!(asst.role, "assistant");

    let tool = ChatMessage::tool("{}");
    assert_eq!(tool.role, "tool");
}

#[test]
fn chat_response_helpers() {
    let empty = ChatResponse {
        text: None,
        tool_calls: vec![],
        usage: None,
    };
    assert!(!empty.has_tool_calls());
    assert_eq!(empty.text_or_empty(), "");

    let with_tools = ChatResponse {
        text: Some("Let me check".into()),
        tool_calls: vec![ToolCall {
            id: "1".into(),
            name: "shell".into(),
            arguments: "{}".into(),
        }],
        usage: None,
    };
    assert!(with_tools.has_tool_calls());
    assert_eq!(with_tools.text_or_empty(), "Let me check");
}

#[test]
fn tool_call_serialization() {
    let tc = ToolCall {
        id: "call_123".into(),
        name: "file_read".into(),
        arguments: r#"{"path":"test.txt"}"#.into(),
    };
    let json = serde_json::to_string(&tc).unwrap();
    assert!(json.contains("call_123"));
    assert!(json.contains("file_read"));
}

#[test]
fn conversation_message_variants() {
    let chat = ConversationMessage::Chat(ChatMessage::user("hi"));
    let json = serde_json::to_string(&chat).unwrap();
    assert!(json.contains("\"type\":\"Chat\""));

    let tool_result = ConversationMessage::ToolResults(vec![ToolResultMessage {
        tool_call_id: "1".into(),
        content: "done".into(),
    }]);
    let json = serde_json::to_string(&tool_result).unwrap();
    assert!(json.contains("\"type\":\"ToolResults\""));
}

#[test]
fn provider_capabilities_default() {
    let caps = ProviderCapabilities::default();
    assert!(!caps.native_tool_calling);
    assert!(!caps.vision);
}

#[test]
fn provider_capabilities_equality() {
    let caps1 = ProviderCapabilities {
        native_tool_calling: true,
        vision: false,
    };
    let caps2 = ProviderCapabilities {
        native_tool_calling: true,
        vision: false,
    };
    let caps3 = ProviderCapabilities {
        native_tool_calling: false,
        vision: false,
    };

    assert_eq!(caps1, caps2);
    assert_ne!(caps1, caps3);
}

#[test]
fn supports_native_tools_reflects_capabilities_default_mapping() {
    let provider = CapabilityMockProvider;
    assert!(provider.supports_native_tools());
}

#[test]
fn supports_vision_reflects_capabilities_default_mapping() {
    let provider = CapabilityMockProvider;
    assert!(provider.supports_vision());
}

#[test]
fn tools_payload_variants() {
    // Test Gemini variant
    let gemini = ToolsPayload::Gemini {
        function_declarations: vec![serde_json::json!({"name": "test"})],
    };
    assert!(matches!(gemini, ToolsPayload::Gemini { .. }));

    // Test Anthropic variant
    let anthropic = ToolsPayload::Anthropic {
        tools: vec![serde_json::json!({"name": "test"})],
    };
    assert!(matches!(anthropic, ToolsPayload::Anthropic { .. }));

    // Test OpenAI variant
    let openai = ToolsPayload::OpenAI {
        tools: vec![serde_json::json!({"type": "function"})],
    };
    assert!(matches!(openai, ToolsPayload::OpenAI { .. }));

    // Test PromptGuided variant
    let prompt_guided = ToolsPayload::PromptGuided {
        instructions: "Use tools...".to_string(),
    };
    assert!(matches!(prompt_guided, ToolsPayload::PromptGuided { .. }));
}

#[test]
fn build_tool_instructions_text_format() {
    let tools = vec![
        ToolSpec {
            name: "shell".to_string(),
            description: "Execute commands".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"}
                }
            }),
        },
        ToolSpec {
            name: "file_read".to_string(),
            description: "Read files".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                }
            }),
        },
    ];

    let instructions = build_tool_instructions_text(&tools);

    // Check for protocol description
    assert!(instructions.contains("Tool Use Protocol"));
    assert!(instructions.contains("<tool_call>"));
    assert!(instructions.contains("</tool_call>"));

    // Check for tool listings
    assert!(instructions.contains("**shell**"));
    assert!(instructions.contains("Execute commands"));
    assert!(instructions.contains("**file_read**"));
    assert!(instructions.contains("Read files"));

    // Check for parameters
    assert!(instructions.contains("Parameters:"));
    assert!(instructions.contains(r#""type":"object""#));
}

#[test]
fn build_tool_instructions_text_empty() {
    let instructions = build_tool_instructions_text(&[]);

    // Should still have protocol description
    assert!(instructions.contains("Tool Use Protocol"));

    // Should have empty tools section
    assert!(instructions.contains("Available Tools"));
}

// Mock provider for testing.
struct MockProvider {
    supports_native: bool,
}

#[async_trait]
impl Provider for MockProvider {
    fn supports_native_tools(&self) -> bool {
        self.supports_native
    }

    async fn chat_with_system(
        &self,
        _system: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("response".to_string())
    }
}

#[test]
fn provider_convert_tools_default() {
    let provider = MockProvider {
        supports_native: false,
    };

    let tools = vec![ToolSpec {
        name: "test_tool".to_string(),
        description: "A test tool".to_string(),
        parameters: serde_json::json!({"type": "object"}),
    }];

    let payload = provider.convert_tools(&tools);

    // Default implementation should return PromptGuided.
    assert!(matches!(payload, ToolsPayload::PromptGuided { .. }));

    if let ToolsPayload::PromptGuided { instructions } = payload {
        assert!(instructions.contains("test_tool"));
        assert!(instructions.contains("A test tool"));
    }
}

#[tokio::test]
async fn provider_chat_prompt_guided_fallback() {
    let provider = MockProvider {
        supports_native: false,
    };

    let tools = vec![ToolSpec {
        name: "shell".to_string(),
        description: "Run commands".to_string(),
        parameters: serde_json::json!({"type": "object"}),
    }];

    let request = ChatRequest {
        messages: &[ChatMessage::user("Hello")],
        tools: Some(&tools),
        stream: None,
    };

    let response = provider.chat(request, "model", 0.7).await.unwrap();

    // Should return a response (default impl calls chat_with_history).
    assert!(response.text.is_some());
}

#[tokio::test]
async fn provider_chat_without_tools() {
    let provider = MockProvider {
        supports_native: true,
    };

    let request = ChatRequest {
        messages: &[ChatMessage::user("Hello")],
        tools: None,
        stream: None,
    };

    let response = provider.chat(request, "model", 0.7).await.unwrap();

    // Should work normally without tools.
    assert!(response.text.is_some());
}

// Provider that echoes the system prompt for assertions.
struct EchoSystemProvider {
    supports_native: bool,
}

#[async_trait]
impl Provider for EchoSystemProvider {
    fn supports_native_tools(&self) -> bool {
        self.supports_native
    }

    async fn chat_with_system(
        &self,
        system: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok(system.unwrap_or_default().to_string())
    }
}

// Provider with custom prompt-guided conversion.
struct CustomConvertProvider;

#[async_trait]
impl Provider for CustomConvertProvider {
    fn supports_native_tools(&self) -> bool {
        false
    }

    fn convert_tools(&self, _tools: &[ToolSpec]) -> ToolsPayload {
        ToolsPayload::PromptGuided {
            instructions: "CUSTOM_TOOL_INSTRUCTIONS".to_string(),
        }
    }

    async fn chat_with_system(
        &self,
        system: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok(system.unwrap_or_default().to_string())
    }
}

// Provider returning an invalid payload for non-native mode.
struct InvalidConvertProvider;

#[async_trait]
impl Provider for InvalidConvertProvider {
    fn supports_native_tools(&self) -> bool {
        false
    }

    fn convert_tools(&self, _tools: &[ToolSpec]) -> ToolsPayload {
        ToolsPayload::OpenAI {
            tools: vec![serde_json::json!({"type": "function"})],
        }
    }

    async fn chat_with_system(
        &self,
        _system: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("should_not_reach".to_string())
    }
}

#[tokio::test]
async fn provider_chat_prompt_guided_preserves_existing_system_not_first() {
    let provider = EchoSystemProvider {
        supports_native: false,
    };

    let tools = vec![ToolSpec {
        name: "shell".to_string(),
        description: "Run commands".to_string(),
        parameters: serde_json::json!({"type": "object"}),
    }];

    let request = ChatRequest {
        messages: &[
            ChatMessage::user("Hello"),
            ChatMessage::system("BASE_SYSTEM_PROMPT"),
        ],
        tools: Some(&tools),
        stream: None,
    };

    let response = provider.chat(request, "model", 0.7).await.unwrap();
    let text = response.text.unwrap_or_default();

    assert!(text.contains("BASE_SYSTEM_PROMPT"));
    assert!(text.contains("Tool Use Protocol"));
}

#[tokio::test]
async fn provider_chat_prompt_guided_uses_convert_tools_override() {
    let provider = CustomConvertProvider;

    let tools = vec![ToolSpec {
        name: "shell".to_string(),
        description: "Run commands".to_string(),
        parameters: serde_json::json!({"type": "object"}),
    }];

    let request = ChatRequest {
        messages: &[ChatMessage::system("BASE"), ChatMessage::user("Hello")],
        tools: Some(&tools),
        stream: None,
    };

    let response = provider.chat(request, "model", 0.7).await.unwrap();
    let text = response.text.unwrap_or_default();

    assert!(text.contains("BASE"));
    assert!(text.contains("CUSTOM_TOOL_INSTRUCTIONS"));
}

#[tokio::test]
async fn provider_chat_prompt_guided_rejects_non_prompt_payload() {
    let provider = InvalidConvertProvider;

    let tools = vec![ToolSpec {
        name: "shell".to_string(),
        description: "Run commands".to_string(),
        parameters: serde_json::json!({"type": "object"}),
    }];

    let request = ChatRequest {
        messages: &[ChatMessage::user("Hello")],
        tools: Some(&tools),
        stream: None,
    };

    let err = provider.chat(request, "model", 0.7).await.unwrap_err();
    let message = err.to_string();

    assert!(message.contains("non-prompt-guided"));
}
