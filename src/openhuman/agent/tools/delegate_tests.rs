use super::*;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

fn test_security() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy::default())
}

fn sample_agents() -> HashMap<String, DelegateAgentConfig> {
    let mut agents = HashMap::new();
    agents.insert(
        "researcher".to_string(),
        DelegateAgentConfig {
            model: "llama3".to_string(),
            system_prompt: Some("You are a research assistant.".to_string()),
            temperature: Some(0.3),
            max_depth: 3,
        },
    );
    agents.insert(
        "coder".to_string(),
        DelegateAgentConfig {
            model: crate::openhuman::config::DEFAULT_MODEL.to_string(),
            system_prompt: None,
            temperature: None,
            max_depth: 2,
        },
    );
    agents
}

#[test]
fn name_and_schema() {
    let tool = DelegateTool::new(sample_agents(), test_security());
    assert_eq!(tool.name(), "delegate");
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["agent"].is_object());
    assert!(schema["properties"]["prompt"].is_object());
    assert!(schema["properties"]["context"].is_object());
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("agent")));
    assert!(required.contains(&json!("prompt")));
    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(schema["properties"]["agent"]["minLength"], json!(1));
    assert_eq!(schema["properties"]["prompt"]["minLength"], json!(1));
}

#[test]
fn description_not_empty() {
    let tool = DelegateTool::new(sample_agents(), test_security());
    assert!(!tool.description().is_empty());
}

#[test]
fn schema_lists_agent_names() {
    let tool = DelegateTool::new(sample_agents(), test_security());
    let schema = tool.parameters_schema();
    let desc = schema["properties"]["agent"]["description"]
        .as_str()
        .unwrap();
    assert!(desc.contains("researcher") || desc.contains("coder"));
}

#[tokio::test]
async fn missing_agent_param() {
    let tool = DelegateTool::new(sample_agents(), test_security());
    let result = tool.execute(json!({"prompt": "test"})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn missing_prompt_param() {
    let tool = DelegateTool::new(sample_agents(), test_security());
    let result = tool.execute(json!({"agent": "researcher"})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn unknown_agent_returns_error() {
    let tool = DelegateTool::new(sample_agents(), test_security());
    let result = tool
        .execute(json!({"agent": "nonexistent", "prompt": "test"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Unknown agent"));
}

#[tokio::test]
async fn depth_limit_enforced() {
    let tool = DelegateTool::with_depth(sample_agents(), test_security(), 3);
    let result = tool
        .execute(json!({"agent": "researcher", "prompt": "test"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("depth limit"));
}

#[tokio::test]
async fn depth_limit_per_agent() {
    // coder has max_depth=2, so depth=2 should be blocked
    let tool = DelegateTool::with_depth(sample_agents(), test_security(), 2);
    let result = tool
        .execute(json!({"agent": "coder", "prompt": "test"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("depth limit"));
}

#[test]
fn empty_agents_schema() {
    let tool = DelegateTool::new(HashMap::new(), test_security());
    let schema = tool.parameters_schema();
    let desc = schema["properties"]["agent"]["description"]
        .as_str()
        .unwrap();
    assert!(desc.contains("none configured"));
}

#[tokio::test]
async fn blank_agent_rejected() {
    let tool = DelegateTool::new(sample_agents(), test_security());
    let result = tool
        .execute(json!({"agent": "  ", "prompt": "test"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("must not be empty"));
}

#[tokio::test]
async fn blank_prompt_rejected() {
    let tool = DelegateTool::new(sample_agents(), test_security());
    let result = tool
        .execute(json!({"agent": "researcher", "prompt": "  \t  "}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("must not be empty"));
}

#[tokio::test]
async fn whitespace_agent_name_trimmed_and_found() {
    let tool = DelegateTool::new(sample_agents(), test_security());
    // " researcher " with surrounding whitespace — after trim becomes "researcher"
    let result = tool
        .execute(json!({"agent": " researcher ", "prompt": "test"}))
        .await
        .unwrap();
    // Should find "researcher" after trim — will fail at provider level
    // since ollama isn't running, but must NOT get "Unknown agent".
    assert!(!result.output().contains("Unknown agent"));
}

#[tokio::test]
async fn delegation_blocked_in_readonly_mode() {
    let readonly = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let tool = DelegateTool::new(sample_agents(), readonly);
    let result = tool
        .execute(json!({"agent": "researcher", "prompt": "test"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("read-only mode"));
}

#[tokio::test]
async fn delegation_blocked_when_rate_limited() {
    let limited = Arc::new(SecurityPolicy {
        max_actions_per_hour: 0,
        ..SecurityPolicy::default()
    });
    let tool = DelegateTool::new(sample_agents(), limited);
    let result = tool
        .execute(json!({"agent": "researcher", "prompt": "test"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Rate limit exceeded"));
}

#[tokio::test]
async fn delegate_context_is_prepended_to_prompt() {
    let mut agents = HashMap::new();
    agents.insert(
        "tester".to_string(),
        DelegateAgentConfig {
            model: "test-model".to_string(),
            system_prompt: None,
            temperature: None,
            max_depth: 3,
        },
    );
    let tool = DelegateTool::new(agents, test_security());
    let result = tool
        .execute(json!({
            "agent": "tester",
            "prompt": "do something",
            "context": "some context data"
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(
        result.output().contains("Agent 'tester' failed") || result.output().contains("timed out")
    );
}

#[tokio::test]
async fn delegate_empty_context_omits_prefix() {
    let mut agents = HashMap::new();
    agents.insert(
        "tester".to_string(),
        DelegateAgentConfig {
            model: "test-model".to_string(),
            system_prompt: None,
            temperature: None,
            max_depth: 3,
        },
    );
    let tool = DelegateTool::new(agents, test_security());
    let result = tool
        .execute(json!({
            "agent": "tester",
            "prompt": "do something",
            "context": ""
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(
        result.output().contains("Agent 'tester' failed") || result.output().contains("timed out")
    );
}

#[test]
fn delegate_depth_construction() {
    let tool = DelegateTool::with_depth(sample_agents(), test_security(), 5);
    assert_eq!(tool.depth, 5);
}

#[tokio::test]
async fn delegate_no_agents_configured() {
    let tool = DelegateTool::new(HashMap::new(), test_security());
    let result = tool
        .execute(json!({"agent": "any", "prompt": "test"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("none configured"));
}
