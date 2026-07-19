use std::sync::Arc;
use std::time::Instant;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::host_runtime::{NativeRuntime, RuntimeAdapter};
use crate::openhuman::config::Config;
use crate::openhuman::memory::Memory;
use crate::openhuman::runtime_node::types::{ExecuteToolOutcome, RuntimeToolSummary};
use crate::openhuman::security::{CommandClass, SecurityPolicy};
use crate::openhuman::tools::{self, PermissionLevel, Tool, ToolCallOptions, ToolScope};
use tracing::{debug, trace};

fn tool_scope_label(scope: ToolScope) -> &'static str {
    match scope {
        ToolScope::All => "all",
        ToolScope::AgentOnly => "agent_only",
        ToolScope::CliRpcOnly => "cli_rpc_only",
    }
}

fn summarize_tool(tool: &dyn Tool) -> RuntimeToolSummary {
    RuntimeToolSummary {
        name: tool.name().to_string(),
        description: tool.description().to_string(),
        category: tool.category().to_string(),
        permission_level: tool.permission_level().to_string(),
        scope: tool_scope_label(tool.scope()).to_string(),
        supports_markdown: tool.supports_markdown(),
        parameters: tool.parameters_schema(),
    }
}

fn classify_shell_tool_call(security: &SecurityPolicy, args: &serde_json::Value) -> CommandClass {
    let command = args
        .get("command")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let mut class = security.classify_command(command);
    if let Some(declared) = args
        .get("category")
        .and_then(|value| value.as_str())
        .and_then(SecurityPolicy::parse_declared_class)
    {
        class = class.max(declared);
    }
    class
}

fn command_class_for_tool(
    security: &SecurityPolicy,
    tool: &dyn Tool,
    args: &serde_json::Value,
) -> CommandClass {
    if tool.name() == "shell" {
        return classify_shell_tool_call(security, args);
    }

    let permission = tool.permission_level_with_args(args);
    match permission {
        PermissionLevel::None | PermissionLevel::ReadOnly => {
            if tool.external_effect_with_args(args) {
                CommandClass::Network
            } else {
                CommandClass::Read
            }
        }
        PermissionLevel::Write | PermissionLevel::Execute => {
            if tool.external_effect_with_args(args) {
                CommandClass::Network
            } else {
                CommandClass::Write
            }
        }
        PermissionLevel::Dangerous => CommandClass::Destructive,
    }
}

fn build_runtime_tools(config: &Config) -> Result<Vec<Box<dyn Tool>>, String> {
    debug!(
        workspace = %config.workspace_dir.display(),
        "[runtime_node::ops] build_runtime_tools: start"
    );
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    // Phase 1 of #1401: see comment in channels/runtime/startup.rs.
    let audit = crate::openhuman::security::get_or_create_workspace_audit_logger(
        crate::openhuman::config::AuditConfig::default(),
        config.workspace_dir.clone(),
    )
    .map_err(|e| e.to_string())?;
    let runtime: Arc<dyn RuntimeAdapter> = Arc::new(NativeRuntime::new());
    let local_embedding = config.workload_local_model("embeddings");
    let embedding_api_key =
        crate::openhuman::embeddings::resolve_api_key(config, &config.memory.embedding_provider);
    trace!("[runtime_node::ops] build_runtime_tools: create_memory_with_local_ai");
    let memory: Arc<dyn Memory> = Arc::from(
        crate::openhuman::memory_store::create_memory_with_local_ai(
            &config.memory,
            local_embedding.as_deref(),
            &embedding_api_key,
            &config.embedding_routes,
            Some(&config.storage.provider.config),
            &config.workspace_dir,
        )
        .map_err(|error| {
            debug!(
                error = %error,
                "[runtime_node::ops] build_runtime_tools: create_memory_with_local_ai failed"
            );
            error.to_string()
        })?,
    );
    trace!("[runtime_node::ops] build_runtime_tools: tools::all_tools_with_runtime");
    let built = tools::all_tools_with_runtime(
        Arc::new(config.clone()),
        &security,
        runtime,
        audit,
        memory,
        &config.browser,
        &config.http_request,
        &config.action_dir,
        &config.agents,
        config,
        None,
        None,
    );
    debug!(
        tool_count = built.len(),
        "[runtime_node::ops] build_runtime_tools: done"
    );
    Ok(built)
}

pub fn list_tools(config: &Config) -> Result<Vec<RuntimeToolSummary>, String> {
    debug!("[runtime_node::ops] list_tools: start");
    let mut summaries: Vec<RuntimeToolSummary> = build_runtime_tools(config)?
        .into_iter()
        .map(|tool| summarize_tool(tool.as_ref()))
        .collect();
    summaries.sort_by(|a, b| a.name.cmp(&b.name));
    debug!(
        count = summaries.len(),
        "[runtime_node::ops] list_tools: done"
    );
    Ok(summaries)
}

pub fn classify_tool_call(
    config: &Config,
    tool_name: &str,
    args: &serde_json::Value,
) -> Result<CommandClass, String> {
    debug!(tool_name, "[runtime_node::ops] classify_tool_call: start");
    let security =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    let tools = build_runtime_tools(config)?;
    let tool = tools
        .into_iter()
        .find(|tool| tool.name() == tool_name)
        .ok_or_else(|| {
            debug!(
                tool_name,
                "[runtime_node::ops] classify_tool_call: tool not found"
            );
            format!("unknown tool `{tool_name}`")
        })?;
    let class = command_class_for_tool(&security, tool.as_ref(), args);
    debug!(
        tool_name,
        ?class,
        permission = %tool.permission_level_with_args(args),
        external_effect = tool.external_effect_with_args(args),
        "[runtime_node::ops] classify_tool_call: done"
    );
    Ok(class)
}

pub async fn execute_tool(
    config: &Config,
    tool_name: &str,
    args: serde_json::Value,
    prefer_markdown: bool,
) -> Result<ExecuteToolOutcome, String> {
    debug!(
        tool_name,
        prefer_markdown, "[runtime_node::ops] execute_tool: start"
    );
    let tools = build_runtime_tools(config)?;
    trace!(
        tool_count = tools.len(),
        tool_name,
        "[runtime_node::ops] execute_tool: runtime tools built"
    );
    let tool = tools
        .into_iter()
        .find(|tool| tool.name() == tool_name)
        .ok_or_else(|| {
            debug!(
                tool_name,
                "[runtime_node::ops] execute_tool: tool not found"
            );
            format!("unknown tool `{tool_name}`")
        })?;

    let started = Instant::now();
    debug!(
        tool_name,
        "[runtime_node::ops] execute_tool: publish ToolExecutionStarted"
    );
    publish_global(DomainEvent::ToolExecutionStarted {
        tool_name: tool_name.to_string(),
        session_id: "javascript".to_string(),
    });

    trace!(
        tool_name,
        "[runtime_node::ops] execute_tool: dispatch execute_with_options"
    );
    let execution = tool
        .execute_with_options(args, ToolCallOptions { prefer_markdown })
        .await
        .map_err(|error| {
            debug!(
                tool_name,
                error = %error,
                "[runtime_node::ops] execute_tool: tool execution failed"
            );
            format!("tool `{tool_name}` failed: {error:#}")
        });

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let success = execution
        .as_ref()
        .map(|result| !result.is_error)
        .unwrap_or(false);
    debug!(
        tool_name,
        success, elapsed_ms, "[runtime_node::ops] execute_tool: publish ToolExecutionCompleted"
    );
    publish_global(DomainEvent::ToolExecutionCompleted {
        tool_name: tool_name.to_string(),
        session_id: "javascript".to_string(),
        success,
        elapsed_ms,
    });

    let result = execution?;
    trace!(
        tool_name,
        success = !result.is_error,
        elapsed_ms,
        "[runtime_node::ops] execute_tool: returning outcome"
    );
    Ok(ExecuteToolOutcome {
        tool_name: tool_name.to_string(),
        elapsed_ms,
        result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;

    struct DummyTool;

    #[async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            "dummy_tool"
        }

        fn description(&self) -> &str {
            "Dummy tool"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
        ) -> anyhow::Result<crate::openhuman::skills::types::ToolResult> {
            Ok(crate::openhuman::skills::types::ToolResult::success("ok"))
        }
    }

    struct PolicyTool {
        name: &'static str,
        permission: PermissionLevel,
        external_effect: bool,
    }

    #[async_trait]
    impl Tool for PolicyTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "Policy test tool"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }

        fn permission_level(&self) -> PermissionLevel {
            self.permission
        }

        fn external_effect_with_args(&self, _args: &serde_json::Value) -> bool {
            self.external_effect
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
        ) -> anyhow::Result<crate::openhuman::skills::types::ToolResult> {
            Ok(crate::openhuman::skills::types::ToolResult::success("ok"))
        }
    }

    #[test]
    fn summarize_tool_exposes_metadata() {
        let summary = summarize_tool(&DummyTool);
        assert_eq!(summary.name, "dummy_tool");
        assert_eq!(summary.category, "system");
        assert_eq!(summary.permission_level, "ReadOnly");
        assert_eq!(summary.scope, "all");
    }

    #[test]
    fn tool_scope_labels_are_stable() {
        assert_eq!(tool_scope_label(ToolScope::All), "all");
        assert_eq!(tool_scope_label(ToolScope::AgentOnly), "agent_only");
        assert_eq!(tool_scope_label(ToolScope::CliRpcOnly), "cli_rpc_only");
    }

    #[test]
    fn command_class_for_tool_maps_metadata_to_policy_buckets() {
        let security = SecurityPolicy::default();

        let read = PolicyTool {
            name: "read_tool",
            permission: PermissionLevel::ReadOnly,
            external_effect: false,
        };
        assert_eq!(
            command_class_for_tool(&security, &read, &json!({})),
            CommandClass::Read
        );

        let write = PolicyTool {
            name: "write_tool",
            permission: PermissionLevel::Write,
            external_effect: false,
        };
        assert_eq!(
            command_class_for_tool(&security, &write, &json!({})),
            CommandClass::Write
        );

        let outbound = PolicyTool {
            name: "outbound_tool",
            permission: PermissionLevel::Write,
            external_effect: true,
        };
        assert_eq!(
            command_class_for_tool(&security, &outbound, &json!({})),
            CommandClass::Network
        );

        let dangerous = PolicyTool {
            name: "dangerous_tool",
            permission: PermissionLevel::Dangerous,
            external_effect: true,
        };
        assert_eq!(
            command_class_for_tool(&security, &dangerous, &json!({})),
            CommandClass::Destructive
        );
    }

    #[test]
    fn command_class_for_shell_uses_command_args() {
        let security = SecurityPolicy::default();
        let shell = PolicyTool {
            name: "shell",
            permission: PermissionLevel::Execute,
            external_effect: true,
        };

        assert_eq!(
            command_class_for_tool(&security, &shell, &json!({"command": "ls src"})),
            CommandClass::Read
        );
        assert_eq!(
            command_class_for_tool(&security, &shell, &json!({"command": "touch out.txt"})),
            CommandClass::Write
        );
        assert_eq!(
            command_class_for_tool(
                &security,
                &shell,
                &json!({"command": "curl https://example.com"})
            ),
            CommandClass::Network
        );
        assert_eq!(
            command_class_for_tool(
                &security,
                &shell,
                &json!({"command": "cat Cargo.toml", "category": "destructive"})
            ),
            CommandClass::Destructive
        );
    }
}
