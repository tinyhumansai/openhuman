use std::sync::Arc;
use std::time::Instant;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::host_runtime::{NativeRuntime, RuntimeAdapter};
use crate::openhuman::config::Config;
use crate::openhuman::memory::Memory;
use crate::openhuman::runtime_node::types::{ExecuteToolOutcome, RuntimeToolSummary};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::{self, Tool, ToolCallOptions, ToolScope};
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

fn build_runtime_tools(config: &Config) -> Result<Vec<Box<dyn Tool>>, String> {
    debug!(
        workspace = %config.workspace_dir.display(),
        "[runtime_node::ops] build_runtime_tools: start"
    );
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
    ));
    // Phase 1 of #1401: see comment in channels/runtime/startup.rs.
    let audit = crate::openhuman::security::get_or_create_workspace_audit_logger(
        crate::openhuman::config::AuditConfig::default(),
        config.workspace_dir.clone(),
    )
    .map_err(|e| e.to_string())?;
    let runtime: Arc<dyn RuntimeAdapter> = Arc::new(NativeRuntime::new());
    let local_embedding = config.workload_local_model("embeddings");
    trace!("[runtime_node::ops] build_runtime_tools: create_memory_with_local_ai");
    let memory: Arc<dyn Memory> = Arc::from(
        crate::openhuman::memory_store::create_memory_with_local_ai(
            &config.memory,
            local_embedding.as_deref(),
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
        &config.workspace_dir,
        &config.agents,
        config,
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
}
