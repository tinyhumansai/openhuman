use std::sync::Arc;
use std::time::Instant;

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::modules::tinybox::host_runtime::{NativeRuntime, RuntimeAdapter};
use crate::security::policy::CommandClass;
use crate::security::SecurityPolicy;
use crate::tools;
use crate::tools::native_ops_types::{ExecuteToolOutcome, RuntimeToolSummary};
use tinytools::{Tool, ToolCallOptions, ToolScope};
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

async fn command_class_for_tool(
    security: &SecurityPolicy,
    tool: &dyn Tool,
    args: &serde_json::Value,
) -> Result<CommandClass, String> {
    if tool.name() == "shell" {
        let declared = args
            .get("category")
            .and_then(|value| value.as_str())
            .and_then(|raw| match raw.trim().to_ascii_lowercase().as_str() {
                "read" => Some(CommandClass::Read),
                "write" => Some(CommandClass::Write),
                "network" => Some(CommandClass::Network),
                "install" => Some(CommandClass::Install),
                "destructive" => Some(CommandClass::Destructive),
                _ => None,
            });
        return security
            .classify_native_command(crate::security::policy::CommandQuery {
                command: args
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default(),
                declared,
                flow: crate::security::policy::CommandFlow::Harness,
                stage: crate::security::policy::CommandStage::Authorize,
            })
            .await
            .map_err(|reason| reason.to_string());
    }

    let answer = crate::security::policy::host_action_class(
        tool.permission_level_with_args(args),
        tool.external_effect_with_args(args),
    )
    .await?;
    Ok(match answer {
        tinysecurity_bus::CommandClass::Read => CommandClass::Read,
        tinysecurity_bus::CommandClass::Write => CommandClass::Write,
        tinysecurity_bus::CommandClass::Network => CommandClass::Network,
        tinysecurity_bus::CommandClass::Install => CommandClass::Install,
        tinysecurity_bus::CommandClass::Destructive => CommandClass::Destructive,
    })
}

pub async fn build_runtime_tools(config: &Config) -> Result<Vec<Box<dyn Tool>>, String> {
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    build_runtime_tools_with_security(config, security).await
}

/// Builds native tools with a caller-resolved policy snapshot.
pub async fn build_runtime_tools_with_security(
    config: &Config,
    security: Arc<SecurityPolicy>,
) -> Result<Vec<Box<dyn Tool>>, String> {
    debug!(
        workspace = %config.workspace_dir.display(),
        "[native_ops] build_runtime_tools: start"
    );
    // Phase 1 of #1401: see comment in channels/runtime/startup.rs.
    let audit = crate::security::get_or_create_workspace_audit_sink(
        config.audit.clone(),
        config.workspace_dir.clone(),
    )
    .map_err(|e| e.to_string())?;
    let runtime: Arc<dyn RuntimeAdapter> = Arc::new(NativeRuntime::new());
    trace!("[native_ops] build_runtime_tools: tools::ops::all_tools_with_runtime");
    let built = tools::ops::all_tools_with_runtime(
        Arc::new(config.clone()),
        &security,
        runtime,
        audit,
        &config.browser,
        &config.http_request,
        &config.action_dir,
        config,
        None,
    )
    .await;
    debug!(
        tool_count = built.len(),
        "[native_ops] build_runtime_tools: done"
    );
    Ok(built)
}

pub async fn list_tools(config: &Config) -> Result<Vec<RuntimeToolSummary>, String> {
    debug!("[native_ops] list_tools: start");
    let mut summaries: Vec<RuntimeToolSummary> = build_runtime_tools(config)
        .await?
        .into_iter()
        .map(|tool| summarize_tool(tool.as_ref()))
        .collect();
    summaries.sort_by(|a, b| a.name.cmp(&b.name));
    debug!(count = summaries.len(), "[native_ops] list_tools: done");
    Ok(summaries)
}

pub async fn classify_tool_call(
    config: &Config,
    tool_name: &str,
    args: &serde_json::Value,
) -> Result<CommandClass, String> {
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    classify_tool_call_with_security(config, security, tool_name, args).await
}

/// Classifies a native call using the same policy snapshot that will execute it.
pub async fn classify_tool_call_with_security(
    config: &Config,
    security: Arc<SecurityPolicy>,
    tool_name: &str,
    args: &serde_json::Value,
) -> Result<CommandClass, String> {
    debug!(tool_name, "[native_ops] classify_tool_call: start");
    let tools = build_runtime_tools_with_security(config, Arc::clone(&security)).await?;
    let tool = tools
        .into_iter()
        .find(|tool| tool.name() == tool_name)
        .ok_or_else(|| {
            debug!(tool_name, "[native_ops] classify_tool_call: tool not found");
            format!("unknown tool `{tool_name}`")
        })?;
    let class = command_class_for_tool(&security, tool.as_ref(), args).await?;
    debug!(
        tool_name,
        ?class,
        permission = %tool.permission_level_with_args(args),
        external_effect = tool.external_effect_with_args(args),
        "[native_ops] classify_tool_call: done"
    );
    Ok(class)
}

pub async fn execute_tool(
    config: &Config,
    tool_name: &str,
    args: serde_json::Value,
    prefer_markdown: bool,
) -> Result<ExecuteToolOutcome, String> {
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    execute_tool_with_security(config, security, tool_name, args, prefer_markdown).await
}

/// Executes a native call using the same policy snapshot used for its gate.
pub async fn execute_tool_with_security(
    config: &Config,
    security: Arc<SecurityPolicy>,
    tool_name: &str,
    args: serde_json::Value,
    prefer_markdown: bool,
) -> Result<ExecuteToolOutcome, String> {
    debug!(
        tool_name,
        prefer_markdown, "[native_ops] execute_tool: start"
    );
    let tools = build_runtime_tools_with_security(config, security).await?;
    trace!(
        tool_count = tools.len(),
        tool_name,
        "[native_ops] execute_tool: runtime tools built"
    );
    let tool = tools
        .into_iter()
        .find(|tool| tool.name() == tool_name)
        .ok_or_else(|| {
            debug!(tool_name, "[native_ops] execute_tool: tool not found");
            format!("unknown tool `{tool_name}`")
        })?;

    let started = Instant::now();
    debug!(
        tool_name,
        "[native_ops] execute_tool: publish ToolExecutionStarted"
    );
    BUS.publish(DomainEvent::ToolExecutionStarted {
        tool_name: tool_name.to_string(),
        session_id: "javascript".to_string(),
    });

    trace!(
        tool_name,
        "[native_ops] execute_tool: dispatch execute_with_options"
    );
    let execution = tool
        .execute_with_options(args, ToolCallOptions { prefer_markdown })
        .await
        .map_err(|error| {
            debug!(
                tool_name,
                error = %error,
                "[native_ops] execute_tool: tool execution failed"
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
        success, elapsed_ms, "[native_ops] execute_tool: publish ToolExecutionCompleted"
    );
    BUS.publish(DomainEvent::ToolExecutionCompleted {
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
        "[native_ops] execute_tool: returning outcome"
    );
    Ok(ExecuteToolOutcome {
        tool_name: tool_name.to_string(),
        elapsed_ms,
        result,
    })
}
