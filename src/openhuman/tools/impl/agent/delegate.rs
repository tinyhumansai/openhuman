use crate::openhuman::config::DelegateAgentConfig;
use crate::openhuman::providers::{self, Provider};
use crate::openhuman::security::policy::ToolOperation;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tool_timeout::tool_execution_timeout_secs;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Tool that delegates a subtask to a named agent with a different
/// provider/model configuration. Enables multi-agent workflows where
/// a primary agent can hand off specialized work (research, coding,
/// summarization) to purpose-built sub-agents.
pub struct DelegateTool {
    agents: Arc<HashMap<String, DelegateAgentConfig>>,
    security: Arc<SecurityPolicy>,
    /// Provider runtime options inherited from root config.
    provider_runtime_options: providers::ProviderRuntimeOptions,
    /// Depth at which this tool instance lives in the delegation chain.
    depth: u32,
}

impl DelegateTool {
    pub fn new(
        agents: HashMap<String, DelegateAgentConfig>,
        security: Arc<SecurityPolicy>,
    ) -> Self {
        Self::new_with_options(
            agents,
            security,
            providers::ProviderRuntimeOptions::default(),
        )
    }

    pub fn new_with_options(
        agents: HashMap<String, DelegateAgentConfig>,
        security: Arc<SecurityPolicy>,
        provider_runtime_options: providers::ProviderRuntimeOptions,
    ) -> Self {
        Self {
            agents: Arc::new(agents),
            security,
            provider_runtime_options,
            depth: 0,
        }
    }

    /// Create a DelegateTool for a sub-agent (with incremented depth).
    /// When sub-agents eventually get their own tool registry, construct
    /// their DelegateTool via this method with `depth: parent.depth + 1`.
    pub fn with_depth(
        agents: HashMap<String, DelegateAgentConfig>,
        security: Arc<SecurityPolicy>,
        depth: u32,
    ) -> Self {
        Self::with_depth_and_options(
            agents,
            security,
            depth,
            providers::ProviderRuntimeOptions::default(),
        )
    }

    pub fn with_depth_and_options(
        agents: HashMap<String, DelegateAgentConfig>,
        security: Arc<SecurityPolicy>,
        depth: u32,
        provider_runtime_options: providers::ProviderRuntimeOptions,
    ) -> Self {
        Self {
            agents: Arc::new(agents),
            security,
            provider_runtime_options,
            depth,
        }
    }
}

#[async_trait]
impl Tool for DelegateTool {
    fn name(&self) -> &str {
        "delegate"
    }

    fn description(&self) -> &str {
        "Delegate a subtask to a specialized agent. Use when: a task benefits from a different model \
         (e.g. fast summarization, deep reasoning, code generation). The sub-agent runs a single \
         prompt and returns its response."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let agent_names: Vec<&str> = self.agents.keys().map(|s: &String| s.as_str()).collect();
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "agent": {
                    "type": "string",
                    "minLength": 1,
                    "description": format!(
                        "Name of the agent to delegate to. Available: {}",
                        if agent_names.is_empty() {
                            "(none configured)".to_string()
                        } else {
                            agent_names.join(", ")
                        }
                    )
                },
                "prompt": {
                    "type": "string",
                    "minLength": 1,
                    "description": "The task/prompt to send to the sub-agent"
                },
                "context": {
                    "type": "string",
                    "description": "Optional context to prepend (e.g. relevant code, prior findings)"
                }
            },
            "required": ["agent", "prompt"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let agent_name = args
            .get("agent")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .ok_or_else(|| anyhow::anyhow!("Missing 'agent' parameter"))?;

        if agent_name.is_empty() {
            return Ok(ToolResult::error("'agent' parameter must not be empty"));
        }

        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .ok_or_else(|| anyhow::anyhow!("Missing 'prompt' parameter"))?;

        if prompt.is_empty() {
            return Ok(ToolResult::error("'prompt' parameter must not be empty"));
        }

        let context = args
            .get("context")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("");

        // Look up agent config
        let agent_config = match self.agents.get(agent_name) {
            Some(cfg) => cfg,
            None => {
                let available: Vec<&str> =
                    self.agents.keys().map(|s: &String| s.as_str()).collect();
                return Ok(ToolResult::error(format!(
                    "Unknown agent '{agent_name}'. Available agents: {}",
                    if available.is_empty() {
                        "(none configured)".to_string()
                    } else {
                        available.join(", ")
                    }
                )));
            }
        };

        // Check recursion depth (immutable — set at construction, incremented for sub-agents)
        if self.depth >= agent_config.max_depth {
            return Ok(ToolResult::error(format!(
                "Delegation depth limit reached ({depth}/{max}). \
                     Cannot delegate further to prevent infinite loops.",
                depth = self.depth,
                max = agent_config.max_depth
            )));
        }

        if let Err(error) = self
            .security
            .enforce_tool_operation(ToolOperation::Act, "delegate")
        {
            return Ok(ToolResult::error(error));
        }

        let provider: Box<dyn Provider> = match providers::create_backend_inference_provider(
            None,
            None,
            &self.provider_runtime_options,
        ) {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Failed to create inference client for delegate agent '{agent_name}': {e}"
                )));
            }
        };

        // Build the message
        let full_prompt = if context.is_empty() {
            prompt.to_string()
        } else {
            format!("[Context]\n{context}\n\n[Task]\n{prompt}")
        };

        let temperature = agent_config.temperature.unwrap_or(0.7);

        let delegate_timeout_secs = tool_execution_timeout_secs();
        // Wrap the provider call in a timeout to prevent indefinite blocking
        let result = tokio::time::timeout(
            Duration::from_secs(delegate_timeout_secs),
            provider.chat_with_system(
                agent_config.system_prompt.as_deref(),
                &full_prompt,
                &agent_config.model,
                temperature,
            ),
        )
        .await;

        let result = match result {
            Ok(inner) => inner,
            Err(_elapsed) => {
                return Ok(ToolResult::error(format!(
                    "Agent '{agent_name}' timed out after {delegate_timeout_secs}s"
                )));
            }
        };

        match result {
            Ok(response) => {
                let mut rendered = response;
                if rendered.trim().is_empty() {
                    rendered = "[Empty response]".to_string();
                }

                Ok(ToolResult::success(format!(
                    "[Agent '{agent_name}' ({}/{})]\n{rendered}",
                    providers::INFERENCE_BACKEND_ID,
                    agent_config.model
                )))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "Agent '{agent_name}' failed: {e}",
            ))),
        }
    }
}

#[cfg(test)]
#[path = "delegate_tests.rs"]
mod tests;
