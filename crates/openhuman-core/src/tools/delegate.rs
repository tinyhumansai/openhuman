use crate::config::DelegateAgentConfig;
use crate::inference::provider::factory::make_openhuman_backend_model_for_options;
use crate::inference::provider::factory::INFERENCE_BACKEND_ID;
use crate::security::SecurityPolicy;
use crate::tools::timeout::tool_execution_timeout_secs;
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::tool::ToolDispatch;
use tinyinference_llm::message::Message;
use tinyinference_llm::model::ModelRequest;
use tinytools::{Tool, ToolCallOptions, ToolResult, ToolRunContext};

/// Runtime overrides applied when the delegate tool builds a provider.
#[derive(Debug, Clone)]
pub(crate) struct ProviderRuntimeOptions {
    pub(crate) auth_profile_override: Option<String>,
    pub(crate) openhuman_dir: Option<PathBuf>,
    pub(crate) secrets_encrypt: bool,
    pub(crate) reasoning_enabled: Option<bool>,
}

impl Default for ProviderRuntimeOptions {
    fn default() -> Self {
        Self {
            auth_profile_override: None,
            openhuman_dir: None,
            secrets_encrypt: true,
            reasoning_enabled: None,
        }
    }
}

/// Tool that delegates a subtask to a named agent with a different
/// provider/model configuration. Enables multi-agent workflows where
/// a primary agent can hand off specialized work (research, coding,
/// summarization) to purpose-built sub-agents.
pub struct DelegateTool {
    agents: Arc<HashMap<String, DelegateAgentConfig>>,
    /// Provider runtime options inherited from root config.
    provider_runtime_options: ProviderRuntimeOptions,
    /// Depth at which this tool instance lives in the delegation chain.
    depth: u32,
}

/// Host adapter that places the config-defined one-shot executor behind the
/// shared driver lifecycle while retaining the configured Tool instance.
pub(crate) struct DelegateToolDispatch {
    tool: Arc<dyn Tool>,
}

impl DelegateToolDispatch {
    pub(crate) fn new(tool: Arc<dyn Tool>) -> Self {
        Self { tool }
    }
}

#[async_trait]
impl ToolDispatch<(), crate::agent::tinyagents::host::OpenHumanRunContext>
    for DelegateToolDispatch
{
    fn tool(&self) -> Arc<dyn Tool> {
        self.tool.clone()
    }

    async fn execute(
        &self,
        _state: &(),
        _call_id: tinyagents_harness::CallId,
        arguments: serde_json::Value,
        _options: ToolCallOptions,
        parent: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let agent_id = arguments
            .get("agent")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|agent| !agent.is_empty())
            .unwrap_or("delegate")
            .to_owned();
        crate::agent::subagent_host::run_configured_delegate_via_driver_tool(
            parent,
            self.tool.clone(),
            arguments,
            agent_id,
        )
        .await
        .map_err(|error| anyhow::anyhow!("delegate: {error}"))
    }
}

impl DelegateTool {
    pub fn new(
        agents: HashMap<String, DelegateAgentConfig>,
        _security: Arc<SecurityPolicy>,
    ) -> Self {
        Self::new_with_options(agents, _security, ProviderRuntimeOptions::default())
    }

    pub fn new_with_options(
        agents: HashMap<String, DelegateAgentConfig>,
        _security: Arc<SecurityPolicy>,
        provider_runtime_options: ProviderRuntimeOptions,
    ) -> Self {
        Self {
            agents: Arc::new(agents),
            provider_runtime_options,
            depth: 0,
        }
    }

    /// Create a DelegateTool for a sub-agent (with incremented depth).
    /// When sub-agents eventually get their own tool registry, construct
    /// their DelegateTool via this method with `depth: parent.depth + 1`.
    pub fn with_depth(
        agents: HashMap<String, DelegateAgentConfig>,
        _security: Arc<SecurityPolicy>,
        depth: u32,
    ) -> Self {
        Self::with_depth_and_options(agents, _security, depth, ProviderRuntimeOptions::default())
    }

    pub fn with_depth_and_options(
        agents: HashMap<String, DelegateAgentConfig>,
        _security: Arc<SecurityPolicy>,
        depth: u32,
        provider_runtime_options: ProviderRuntimeOptions,
    ) -> Self {
        Self {
            agents: Arc::new(agents),
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
        self.execute_inner(args, None).await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.execute_inner(args, tool_context).await
    }
}

impl DelegateTool {
    async fn execute_inner(
        &self,
        args: serde_json::Value,
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
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

        // `metadata` below is diagnostic only; the managed model owns the
        // actual backend wire extension so configured delegates cannot lose
        // their parent's thread between typed dispatch and provider invoke.
        let thread_id = tool_context.and_then(ToolRunContext::thread_id);
        let model = match make_openhuman_backend_model_for_options(
            &self.provider_runtime_options,
            &agent_config.model,
            thread_id,
        ) {
            Ok((model, _resolved)) => model,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "Agent '{agent_name}' failed: {error}"
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
        let mut request = ModelRequest::new(
            agent_config
                .system_prompt
                .as_deref()
                .map(Message::system)
                .into_iter()
                .chain(std::iter::once(Message::user(full_prompt)))
                .collect(),
        )
        .with_model(agent_config.model.clone())
        .with_temperature(temperature);
        request.metadata = json!({
            "openhuman": {
                "thread_id": tool_context.and_then(ToolRunContext::thread_id),
                "workspace_root": tool_context
                    .and_then(ToolRunContext::workspace)
                    .map(|workspace| workspace.root.display().to_string()),
            }
        });

        let result = tokio::time::timeout(
            Duration::from_secs(delegate_timeout_secs),
            model.invoke(&(), request),
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
                let mut rendered = response.text();
                if rendered.trim().is_empty() {
                    rendered = "[Empty response]".to_string();
                }

                Ok(ToolResult::success(format!(
                    "[Agent '{agent_name}' ({}/{})]\n{rendered}",
                    INFERENCE_BACKEND_ID, agent_config.model
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
