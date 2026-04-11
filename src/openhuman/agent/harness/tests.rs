use super::credentials::scrub_credentials;
use super::instructions::build_tool_instructions;
use super::parse::{
    extract_json_values, parse_arguments_value, parse_glm_style_tool_calls, parse_tool_call_value,
    parse_tool_calls, parse_tool_calls_from_json_value, tools_to_openai_format,
};
use super::tool_loop::{run_tool_call_loop, DEFAULT_MAX_TOOL_ITERATIONS};
use crate::openhuman::providers::traits::ProviderCapabilities;
use crate::openhuman::providers::{ChatMessage, ChatRequest, ChatResponse, Provider};
use crate::openhuman::tools::{self, Tool};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn test_scrub_credentials() {
    let input = "API_KEY=sk-1234567890abcdef; token: 1234567890; password=\"secret123456\"";
    let scrubbed = scrub_credentials(input);
    assert!(scrubbed.contains("API_KEY=sk-1*[REDACTED]"));
    assert!(scrubbed.contains("token: 1234*[REDACTED]"));
    assert!(scrubbed.contains("password=\"secr*[REDACTED]\""));
    assert!(!scrubbed.contains("abcdef"));
    assert!(!scrubbed.contains("secret123456"));
}

#[test]
fn test_scrub_credentials_json() {
    let input = r#"{"api_key": "sk-1234567890", "other": "public"}"#;
    let scrubbed = scrub_credentials(input);
    assert!(scrubbed.contains("\"api_key\": \"sk-1*[REDACTED]\""));
    assert!(scrubbed.contains("public"));
}

struct NonVisionProvider {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Provider for NonVisionProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok("ok".to_string())
    }
}

struct VisionProvider {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Provider for VisionProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: false,
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok("ok".to_string())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let marker_count =
            crate::openhuman::agent::multimodal::count_image_markers(request.messages);
        if marker_count == 0 {
            anyhow::bail!("expected image markers in request messages");
        }

        if request.tools.is_some() {
            anyhow::bail!("no tools should be attached for this test");
        }

        Ok(ChatResponse {
            text: Some("vision-ok".to_string()),
            tool_calls: Vec::new(),
            usage: None,
        })
    }
}

#[tokio::test]
async fn run_tool_call_loop_returns_structured_error_for_non_vision_provider() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = NonVisionProvider {
        calls: Arc::clone(&calls),
    };

    let mut history = vec![ChatMessage::user(
        "please inspect [IMAGE:data:image/png;base64,iVBORw0KGgo=]".to_string(),
    )];
    let tools_registry: Vec<Box<dyn Tool>> = Vec::new();

    let err = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "cli",
        &crate::openhuman::config::MultimodalConfig::default(),
        3,
        None,
    )
    .await
    .expect_err("provider without vision support should fail");

    assert!(err.to_string().contains("provider_capability_error"));
    assert!(err.to_string().contains("capability=vision"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn run_tool_call_loop_rejects_oversized_image_payload() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = VisionProvider {
        calls: Arc::clone(&calls),
    };

    let oversized_payload = STANDARD.encode(vec![0_u8; (1024 * 1024) + 1]);
    let mut history = vec![ChatMessage::user(format!(
        "[IMAGE:data:image/png;base64,{oversized_payload}]"
    ))];

    let tools_registry: Vec<Box<dyn Tool>> = Vec::new();
    let multimodal = crate::openhuman::config::MultimodalConfig {
        max_images: 4,
        max_image_size_mb: 1,
        allow_remote_fetch: false,
    };

    let err = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "cli",
        &multimodal,
        3,
        None,
    )
    .await
    .expect_err("oversized payload must fail");

    assert!(err
        .to_string()
        .contains("multimodal image size limit exceeded"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn run_tool_call_loop_accepts_valid_multimodal_request_flow() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = VisionProvider {
        calls: Arc::clone(&calls),
    };

    let mut history = vec![ChatMessage::user(
        "Analyze this [IMAGE:data:image/png;base64,iVBORw0KGgo=]".to_string(),
    )];
    let tools_registry: Vec<Box<dyn Tool>> = Vec::new();

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "cli",
        &crate::openhuman::config::MultimodalConfig::default(),
        3,
        None,
    )
    .await
    .expect("valid multimodal payload should pass");

    assert_eq!(result, "vision-ok");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn parse_tool_calls_extracts_single_call() {
    let response = r#"Let me check that.
<tool_call>
{"name": "shell", "arguments": {"command": "ls -la"}}
</tool_call>"#;

    let (text, calls) = parse_tool_calls(response);
    assert_eq!(text, "Let me check that.");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "ls -la"
    );
}

#[test]
fn parse_tool_calls_extracts_multiple_calls() {
    let response = r#"<tool_call>
{"name": "file_read", "arguments": {"path": "a.txt"}}
</tool_call>
<tool_call>
{"name": "file_read", "arguments": {"path": "b.txt"}}
</tool_call>"#;

    let (_, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "file_read");
    assert_eq!(calls[1].name, "file_read");
}

#[test]
fn parse_tool_calls_returns_text_only_when_no_calls() {
    let response = "Just a normal response with no tools.";
    let (text, calls) = parse_tool_calls(response);
    assert_eq!(text, "Just a normal response with no tools.");
    assert!(calls.is_empty());
}

#[test]
fn parse_tool_calls_handles_malformed_json() {
    let response = r#"<tool_call>
not valid json
</tool_call>
Some text after."#;

    let (text, calls) = parse_tool_calls(response);
    assert!(calls.is_empty());
    assert!(text.contains("Some text after."));
}

#[test]
fn parse_tool_calls_text_before_and_after() {
    let response = r#"Before text.
<tool_call>
{"name": "shell", "arguments": {"command": "echo hi"}}
</tool_call>
After text."#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.contains("Before text."));
    assert!(text.contains("After text."));
    assert_eq!(calls.len(), 1);
}

#[test]
fn parse_tool_calls_handles_openai_format() {
    // OpenAI-style response with tool_calls array
    let response = r#"{"content": "Let me check that for you.", "tool_calls": [{"type": "function", "function": {"name": "shell", "arguments": "{\"command\": \"ls -la\"}"}}]}"#;

    let (text, calls) = parse_tool_calls(response);
    assert_eq!(text, "Let me check that for you.");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "ls -la"
    );
}

#[test]
fn parse_tool_calls_handles_openai_format_multiple_calls() {
    let response = r#"{"tool_calls": [{"type": "function", "function": {"name": "file_read", "arguments": "{\"path\": \"a.txt\"}"}}, {"type": "function", "function": {"name": "file_read", "arguments": "{\"path\": \"b.txt\"}"}}]}"#;

    let (_, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "file_read");
    assert_eq!(calls[1].name, "file_read");
}

#[test]
fn parse_tool_calls_openai_format_without_content() {
    // Some providers don't include content field with tool_calls
    let response = r#"{"tool_calls": [{"type": "function", "function": {"name": "memory_recall", "arguments": "{}"}}]}"#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.is_empty()); // No content field
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "memory_recall");
}

#[test]
fn parse_tool_calls_handles_markdown_json_inside_tool_call_tag() {
    let response = r#"<tool_call>
```json
{"name": "file_write", "arguments": {"path": "test.py", "content": "print('ok')"}}
```
</tool_call>"#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.is_empty());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "file_write");
    assert_eq!(
        calls[0].arguments.get("path").unwrap().as_str().unwrap(),
        "test.py"
    );
}

#[test]
fn parse_tool_calls_handles_noisy_tool_call_tag_body() {
    let response = r#"<tool_call>
I will now call the tool with this payload:
{"name": "shell", "arguments": {"command": "pwd"}}
</tool_call>"#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.is_empty());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "pwd"
    );
}

#[test]
fn parse_tool_calls_handles_markdown_tool_call_fence() {
    let response = r#"I'll check that.
```tool_call
{"name": "shell", "arguments": {"command": "pwd"}}
```
Done."#;

    let (text, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "pwd"
    );
    assert!(text.contains("I'll check that."));
    assert!(text.contains("Done."));
    assert!(!text.contains("```tool_call"));
}

#[test]
fn parse_tool_calls_handles_markdown_tool_call_hybrid_close_tag() {
    let response = r#"Preface
```tool-call
{"name": "shell", "arguments": {"command": "date"}}
</tool_call>
Tail"#;

    let (text, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "date"
    );
    assert!(text.contains("Preface"));
    assert!(text.contains("Tail"));
    assert!(!text.contains("```tool-call"));
}

#[test]
fn parse_tool_calls_handles_markdown_invoke_fence() {
    let response = r#"Checking.
```invoke
{"name": "shell", "arguments": {"command": "date"}}
```
Done."#;

    let (text, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "date"
    );
    assert!(text.contains("Checking."));
    assert!(text.contains("Done."));
}

#[test]
fn parse_tool_calls_handles_toolcall_tag_alias() {
    let response = r#"<toolcall>
{"name": "shell", "arguments": {"command": "date"}}
</toolcall>"#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.is_empty());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "date"
    );
}

#[test]
fn parse_tool_calls_handles_tool_dash_call_tag_alias() {
    let response = r#"<tool-call>
{"name": "shell", "arguments": {"command": "whoami"}}
</tool-call>"#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.is_empty());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "whoami"
    );
}

#[test]
fn parse_tool_calls_handles_invoke_tag_alias() {
    let response = r#"<invoke>
{"name": "shell", "arguments": {"command": "uptime"}}
</invoke>"#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.is_empty());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "uptime"
    );
}

#[test]
fn parse_tool_calls_recovers_unclosed_tool_call_with_json() {
    let response = r#"I will call the tool now.
<tool_call>
{"name": "shell", "arguments": {"command": "uptime -p"}}"#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.contains("I will call the tool now."));
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "uptime -p"
    );
}

#[test]
fn parse_tool_calls_recovers_mismatched_close_tag() {
    let response = r#"<tool_call>
{"name": "shell", "arguments": {"command": "uptime"}}
</arg_value>"#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.is_empty());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "uptime"
    );
}

#[test]
fn parse_tool_calls_recovers_cross_alias_closing_tags() {
    let response = r#"<toolcall>
{"name": "shell", "arguments": {"command": "date"}}
</tool_call>"#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.is_empty());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
}

#[test]
fn parse_tool_calls_rejects_raw_tool_json_without_tags() {
    // SECURITY: Raw JSON without explicit wrappers should NOT be parsed
    // This prevents prompt injection attacks where malicious content
    // could include JSON that mimics a tool call.
    let response = r#"Sure, creating the file now.
{"name": "file_write", "arguments": {"path": "hello.py", "content": "print('hello')"}}"#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.contains("Sure, creating the file now."));
    assert_eq!(
        calls.len(),
        0,
        "Raw JSON without wrappers should not be parsed"
    );
}

#[test]
fn build_tool_instructions_includes_all_tools() {
    use crate::openhuman::security::SecurityPolicy;
    let security = Arc::new(SecurityPolicy::from_config(
        &crate::openhuman::config::AutonomyConfig::default(),
        std::path::Path::new("/tmp"),
    ));
    let tools = tools::default_tools(security);
    let instructions = build_tool_instructions(&tools);

    assert!(instructions.contains("## Tool Use Protocol"));
    assert!(instructions.contains("<tool_call>"));
    assert!(instructions.contains("shell"));
    assert!(instructions.contains("file_read"));
    assert!(instructions.contains("file_write"));
}

#[test]
fn tools_to_openai_format_produces_valid_schema() {
    use crate::openhuman::security::SecurityPolicy;
    let security = Arc::new(SecurityPolicy::from_config(
        &crate::openhuman::config::AutonomyConfig::default(),
        std::path::Path::new("/tmp"),
    ));
    let tools = tools::default_tools(security);
    let formatted = tools_to_openai_format(&tools);

    assert!(!formatted.is_empty());
    for tool_json in &formatted {
        assert_eq!(tool_json["type"], "function");
        assert!(tool_json["function"]["name"].is_string());
        assert!(tool_json["function"]["description"].is_string());
        assert!(!tool_json["function"]["name"].as_str().unwrap().is_empty());
    }
    // Verify known tools are present
    let names: Vec<&str> = formatted
        .iter()
        .filter_map(|t| t["function"]["name"].as_str())
        .collect();
    assert!(names.contains(&"shell"));
    assert!(names.contains(&"file_read"));
}

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - Tool Call Parsing Edge Cases
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn parse_tool_calls_handles_empty_tool_result() {
    // Recovery: Empty tool_result tag should be handled gracefully
    let response = r#"I'll run that command.
<tool_result name="shell">

</tool_result>
Done."#;
    let (text, calls) = parse_tool_calls(response);
    assert!(text.contains("Done."));
    assert!(calls.is_empty());
}

#[test]
fn parse_arguments_value_handles_null() {
    // Recovery: null arguments are returned as-is (Value::Null)
    let value = serde_json::json!(null);
    let result = parse_arguments_value(Some(&value));
    assert!(result.is_null());
}

#[test]
fn parse_tool_calls_handles_empty_tool_calls_array() {
    // Recovery: Empty tool_calls array returns original response (no tool parsing)
    let response = r#"{"content": "Hello", "tool_calls": []}"#;
    let (text, calls) = parse_tool_calls(response);
    // When tool_calls is empty, the entire JSON is returned as text
    assert!(text.contains("Hello"));
    assert!(calls.is_empty());
}

#[test]
fn parse_tool_calls_handles_whitespace_only_name() {
    // Recovery: Whitespace-only tool name should return None
    let value = serde_json::json!({"function": {"name": "   ", "arguments": {}}});
    let result = parse_tool_call_value(&value);
    assert!(result.is_none());
}

#[test]
fn parse_tool_calls_handles_empty_string_arguments() {
    // Recovery: Empty string arguments should be handled
    let value = serde_json::json!({"name": "test", "arguments": ""});
    let result = parse_tool_call_value(&value);
    assert!(result.is_some());
    assert_eq!(result.unwrap().name, "test");
}

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - Arguments Parsing
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn parse_arguments_value_handles_invalid_json_string() {
    // Recovery: Invalid JSON string should return empty object
    let value = serde_json::Value::String("not valid json".to_string());
    let result = parse_arguments_value(Some(&value));
    assert!(result.is_object());
    assert!(result.as_object().unwrap().is_empty());
}

#[test]
fn parse_arguments_value_handles_none() {
    // Recovery: None arguments should return empty object
    let result = parse_arguments_value(None);
    assert!(result.is_object());
    assert!(result.as_object().unwrap().is_empty());
}

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - JSON Extraction
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn extract_json_values_handles_empty_string() {
    // Recovery: Empty input should return empty vec
    let result = extract_json_values("");
    assert!(result.is_empty());
}

#[test]
fn extract_json_values_handles_whitespace_only() {
    // Recovery: Whitespace only should return empty vec
    let result = extract_json_values("   \n\t  ");
    assert!(result.is_empty());
}

#[test]
fn extract_json_values_handles_multiple_objects() {
    // Recovery: Multiple JSON objects should all be extracted
    let input = r#"{"a": 1}{"b": 2}{"c": 3}"#;
    let result = extract_json_values(input);
    assert_eq!(result.len(), 3);
}

#[test]
fn extract_json_values_handles_arrays() {
    // Recovery: JSON arrays should be extracted
    let input = r#"[1, 2, 3]{"key": "value"}"#;
    let result = extract_json_values(input);
    assert_eq!(result.len(), 2);
}

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - Constants Validation
// ═══════════════════════════════════════════════════════════════════════

const _: () = {
    assert!(DEFAULT_MAX_TOOL_ITERATIONS > 0);
    assert!(DEFAULT_MAX_TOOL_ITERATIONS <= 100);
};

#[test]
fn constants_bounds_are_compile_time_checked() {
    // Bounds are enforced by the const assertions above.
}

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - Tool Call Value Parsing
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn parse_tool_call_value_handles_missing_name_field() {
    // Recovery: Missing name field should return None
    let value = serde_json::json!({"function": {"arguments": {}}});
    let result = parse_tool_call_value(&value);
    assert!(result.is_none());
}

#[test]
fn parse_tool_call_value_handles_top_level_name() {
    // Recovery: Tool call with name at top level (non-OpenAI format)
    let value = serde_json::json!({"name": "test_tool", "arguments": {}});
    let result = parse_tool_call_value(&value);
    assert!(result.is_some());
    assert_eq!(result.unwrap().name, "test_tool");
}

#[test]
fn parse_tool_calls_from_json_value_handles_empty_array() {
    // Recovery: Empty tool_calls array should return empty vec
    let value = serde_json::json!({"tool_calls": []});
    let result = parse_tool_calls_from_json_value(&value);
    assert!(result.is_empty());
}

#[test]
fn parse_tool_calls_from_json_value_handles_missing_tool_calls() {
    // Recovery: Missing tool_calls field should fall through
    let value = serde_json::json!({"name": "test", "arguments": {}});
    let result = parse_tool_calls_from_json_value(&value);
    assert_eq!(result.len(), 1);
}

#[test]
fn parse_tool_calls_from_json_value_handles_top_level_array() {
    // Recovery: Top-level array of tool calls
    let value = serde_json::json!([
        {"name": "tool_a", "arguments": {}},
        {"name": "tool_b", "arguments": {}}
    ]);
    let result = parse_tool_calls_from_json_value(&value);
    assert_eq!(result.len(), 2);
}

// ═══════════════════════════════════════════════════════════════════════
// GLM-Style Tool Call Parsing
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn parse_glm_style_browser_open_url() {
    let response = "browser_open/url>https://example.com";
    let calls = parse_glm_style_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "shell");
    assert!(calls[0].1["command"].as_str().unwrap().contains("curl"));
    assert!(calls[0].1["command"]
        .as_str()
        .unwrap()
        .contains("example.com"));
}

#[test]
fn parse_glm_style_shell_command() {
    let response = "shell/command>ls -la";
    let calls = parse_glm_style_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "shell");
    assert_eq!(calls[0].1["command"], "ls -la");
}

#[test]
fn parse_glm_style_http_request() {
    let response = "http_request/url>https://api.example.com/data";
    let calls = parse_glm_style_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "http_request");
    assert_eq!(calls[0].1["url"], "https://api.example.com/data");
    assert_eq!(calls[0].1["method"], "GET");
}

#[test]
fn parse_glm_style_plain_url() {
    let response = "https://example.com/api";
    let calls = parse_glm_style_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "shell");
    assert!(calls[0].1["command"].as_str().unwrap().contains("curl"));
}

#[test]
fn parse_glm_style_json_args() {
    let response = r#"shell/{"command": "echo hello"}"#;
    let calls = parse_glm_style_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "shell");
    assert_eq!(calls[0].1["command"], "echo hello");
}

#[test]
fn parse_glm_style_multiple_calls() {
    let response = r#"shell/command>ls
browser_open/url>https://example.com"#;
    let calls = parse_glm_style_tool_calls(response);
    assert_eq!(calls.len(), 2);
}

#[test]
fn parse_glm_style_tool_call_integration() {
    // Integration test: GLM format should be parsed in parse_tool_calls
    let response = "Checking...\nbrowser_open/url>https://example.com\nDone";
    let (text, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert!(text.contains("Checking"));
    assert!(text.contains("Done"));
}

#[test]
fn parse_glm_style_rejects_non_http_url_param() {
    let response = "browser_open/url>javascript:alert(1)";
    let calls = parse_glm_style_tool_calls(response);
    assert!(calls.is_empty());
}

#[test]
fn parse_tool_calls_handles_unclosed_tool_call_tag() {
    let response = "<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"pwd\"}}\nDone";
    let (text, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].arguments["command"], "pwd");
    assert_eq!(text, "Done");
}

// ─────────────────────────────────────────────────────────────────────
// TG4 (inline): parse_tool_calls robustness — malformed/edge-case inputs
// Prevents: Pattern 4 issues #746, #418, #777, #848
// ─────────────────────────────────────────────────────────────────────

#[test]
fn parse_tool_calls_empty_input_returns_empty() {
    let (text, calls) = parse_tool_calls("");
    assert!(calls.is_empty(), "empty input should produce no tool calls");
    assert!(text.is_empty(), "empty input should produce no text");
}

#[test]
fn parse_tool_calls_whitespace_only_returns_empty_calls() {
    let (text, calls) = parse_tool_calls("   \n\t  ");
    assert!(calls.is_empty());
    assert!(text.is_empty() || text.trim().is_empty());
}

#[test]
fn parse_tool_calls_nested_xml_tags_handled() {
    // Double-wrapped tool call should still parse the inner call
    let response =
        r#"<tool_call><tool_call>{"name":"echo","arguments":{"msg":"hi"}}</tool_call></tool_call>"#;
    let (_text, calls) = parse_tool_calls(response);
    // Should find at least one tool call
    assert!(
        !calls.is_empty(),
        "nested XML tags should still yield at least one tool call"
    );
}

#[test]
fn parse_tool_calls_truncated_json_no_panic() {
    // Incomplete JSON inside tool_call tags
    let response = r#"<tool_call>{"name":"shell","arguments":{"command":"ls"</tool_call>"#;
    let (_text, _calls) = parse_tool_calls(response);
    // Should not panic — graceful handling of truncated JSON
}

#[test]
fn parse_tool_calls_empty_json_object_in_tag() {
    let response = "<tool_call>{}</tool_call>";
    let (_text, calls) = parse_tool_calls(response);
    // Empty JSON object has no name field — should not produce valid tool call
    assert!(
        calls.is_empty(),
        "empty JSON object should not produce a tool call"
    );
}

#[test]
fn parse_tool_calls_closing_tag_only_returns_text() {
    let response = "Some text </tool_call> more text";
    let (text, calls) = parse_tool_calls(response);
    assert!(
        calls.is_empty(),
        "closing tag only should not produce calls"
    );
    assert!(
        !text.is_empty(),
        "text around orphaned closing tag should be preserved"
    );
}

#[test]
fn parse_tool_calls_very_large_arguments_no_panic() {
    let large_arg = "x".repeat(100_000);
    let response = format!(
        r#"<tool_call>{{"name":"echo","arguments":{{"message":"{}"}}}}</tool_call>"#,
        large_arg
    );
    let (_text, calls) = parse_tool_calls(&response);
    assert_eq!(calls.len(), 1, "large arguments should still parse");
    assert_eq!(calls[0].name, "echo");
}

#[test]
fn parse_tool_calls_special_characters_in_arguments() {
    let response = r#"<tool_call>{"name":"echo","arguments":{"message":"hello \"world\" <>&'\n\t"}}</tool_call>"#;
    let (_text, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "echo");
}

#[test]
fn parse_tool_calls_text_with_embedded_json_not_extracted() {
    // Raw JSON without any tags should NOT be extracted as a tool call
    let response = r#"Here is some data: {"name":"echo","arguments":{"message":"hi"}} end."#;
    let (_text, calls) = parse_tool_calls(response);
    assert!(
        calls.is_empty(),
        "raw JSON in text without tags should not be extracted"
    );
}

#[test]
fn parse_tool_calls_multiple_formats_mixed() {
    // Mix of text and properly tagged tool call
    let response = r#"I'll help you with that.

<tool_call>
{"name":"shell","arguments":{"command":"echo hello"}}
</tool_call>

Let me check the result."#;
    let (text, calls) = parse_tool_calls(response);
    assert_eq!(
        calls.len(),
        1,
        "should extract one tool call from mixed content"
    );
    assert_eq!(calls[0].name, "shell");
    assert!(
        text.contains("help you"),
        "text before tool call should be preserved"
    );
}

// ─────────────────────────────────────────────────────────────────────
// TG4 (inline): scrub_credentials edge cases
// ─────────────────────────────────────────────────────────────────────

#[test]
fn scrub_credentials_empty_input() {
    let result = scrub_credentials("");
    assert_eq!(result, "");
}

#[test]
fn scrub_credentials_no_sensitive_data() {
    let input = "normal text without any secrets";
    let result = scrub_credentials(input);
    assert_eq!(
        result, input,
        "non-sensitive text should pass through unchanged"
    );
}

#[test]
fn scrub_credentials_short_values_not_redacted() {
    // Values shorter than 8 chars should not be redacted
    let input = r#"api_key="short""#;
    let result = scrub_credentials(input);
    assert_eq!(result, input, "short values should not be redacted");
}
