use std::sync::Arc;
use std::time::Instant;

use crate::agent::tinyagents::host::OpenHumanRunContext;
use tinyagents_harness::context::{RunConfig, RunContext};
use tinyagents_harness::middleware::{
    BoxToolFuture, MiddlewareStack, ToolBaseCall, ToolMiddleware,
};
use tinyinference_llm::tool::ToolCall;
use tinytools::ToolResult;

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::modules::tinybox::host_runtime::{NativeRuntime, RuntimeAdapter};
use crate::security::policy::CommandClass;
use crate::security::SecurityPolicy;
use crate::tools;
use crate::tools::native_ops_types::{ExecuteToolOutcome, RuntimeToolSummary};
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolScope};
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

fn classify_shell_tool_call(args: &serde_json::Value) -> CommandClass {
    // This synchronous summary API cannot ask TinyBox to analyze the command.
    // Report the conservative class here; the invocation path performs exact
    // module analysis before approval and execution.
    let mut class = CommandClass::Write;
    if let Some(declared) = args
        .get("category")
        .and_then(|value| value.as_str())
        .and_then(SecurityPolicy::parse_declared_class)
    {
        class = class.max(declared);
    }
    class
}

fn command_class_for_tool(tool: &dyn Tool, args: &serde_json::Value) -> CommandClass {
    if tool.name() == "shell" {
        return classify_shell_tool_call(args);
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

pub fn build_runtime_tools(config: &Config) -> Result<Vec<Box<dyn Tool>>, String> {
    // Keep the config's privacy posture on the same policy snapshot as the
    // autonomy settings. `from_config` alone defaults privacy to Standard.
    let security = Arc::new(SecurityPolicy::from_full_config(config));
    build_runtime_tools_with_security(config, security)
}

/// Builds native tools with a caller-resolved policy snapshot.
pub fn build_runtime_tools_with_security(
    config: &Config,
    security: Arc<SecurityPolicy>,
) -> Result<Vec<Box<dyn Tool>>, String> {
    debug!(
        workspace = %config.workspace_dir.display(),
        "[native_ops] build_runtime_tools: start"
    );
    let runtime: Arc<dyn RuntimeAdapter> = Arc::new(NativeRuntime::new());
    trace!("[native_ops] build_runtime_tools: tools::ops::all_tools_with_runtime");
    let built = tools::ops::all_tools_with_runtime(
        Arc::new(config.clone()),
        &security,
        runtime,
        &config.browser,
        &config.http_request,
        &config.action_dir,
        &config.agents,
        config,
        None,
    );
    debug!(
        tool_count = built.len(),
        "[native_ops] build_runtime_tools: done"
    );
    Ok(built)
}

pub fn list_tools(config: &Config) -> Result<Vec<RuntimeToolSummary>, String> {
    debug!("[native_ops] list_tools: start");
    let mut summaries: Vec<RuntimeToolSummary> = build_runtime_tools(config)?
        .into_iter()
        .map(|tool| summarize_tool(tool.as_ref()))
        .collect();
    summaries.sort_by(|a, b| a.name.cmp(&b.name));
    debug!(count = summaries.len(), "[native_ops] list_tools: done");
    Ok(summaries)
}

pub fn classify_tool_call(
    config: &Config,
    tool_name: &str,
    args: &serde_json::Value,
) -> Result<CommandClass, String> {
    let security = Arc::new(SecurityPolicy::from_config(
        &config.security.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    classify_tool_call_with_security(config, security, tool_name, args)
}

/// Classifies a native call using the same policy snapshot that will execute it.
pub fn classify_tool_call_with_security(
    config: &Config,
    security: Arc<SecurityPolicy>,
    tool_name: &str,
    args: &serde_json::Value,
) -> Result<CommandClass, String> {
    debug!(tool_name, "[native_ops] classify_tool_call: start");
    let tools = build_runtime_tools_with_security(config, security)?;
    let tool = tools
        .into_iter()
        .find(|tool| tool.name() == tool_name)
        .ok_or_else(|| {
            debug!(tool_name, "[native_ops] classify_tool_call: tool not found");
            format!("unknown tool `{tool_name}`")
        })?;
    let class = command_class_for_tool(tool.as_ref(), args);
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
    let tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(build_runtime_tools(config)?);
    let gate = crate::agent::tinyagents::middleware::security_gate::tool_middleware(
        vec![Arc::clone(&tools)],
        None,
        None,
    );
    run_gated(&tools, gate, tool_name, args, prefer_markdown).await
}

/// Execute a direct host-dispatched tool call while retaining its tenant and
/// verified turn origin through the same TinySecurity and approval middleware
/// used for agent tool calls.
pub(crate) async fn execute_tool_in_context(
    config: &Config,
    dispatch: crate::core::runtime::DispatchCtx,
    origin: crate::security::turn_origin::AgentTurnOrigin,
    tool_name: &str,
    args: serde_json::Value,
) -> Result<ExecuteToolOutcome, String> {
    let tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(build_runtime_tools(config)?);
    let gate = crate::agent::tinyagents::middleware::security_gate::tool_middleware(
        vec![Arc::clone(&tools)],
        None,
        None,
    );
    let data = OpenHumanRunContext::new()
        .with_core(dispatch.core().cloned())
        .with_origin(origin);
    run_gated_with_context(&tools, gate, tool_name, args, false, data).await
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
    let tools = build_runtime_tools_with_security(config, security)?;
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

/// The tool, run once the middleware has judged the call. The base of the
/// middleware stack: reached only when the gate allowed the call.
struct DirectTool<'a> {
    tool: &'a dyn Tool,
    prefer_markdown: bool,
    /// The tool's own error, kept whole for the caller: the harness result type
    /// carries text only.
    failure: std::sync::Mutex<Option<String>>,
}

impl ToolBaseCall<(), OpenHumanRunContext> for DirectTool<'_> {
    fn call<'a>(
        &'a self,
        _ctx: &'a RunContext<OpenHumanRunContext>,
        _state: &'a (),
        call: ToolCall,
    ) -> BoxToolFuture<'a> {
        Box::pin(async move {
            match self
                .tool
                .execute_with_options(
                    call.arguments,
                    ToolCallOptions {
                        prefer_markdown: self.prefer_markdown,
                    },
                )
                .await
            {
                Ok(result) => Ok(result),
                Err(error) => {
                    let message = format!("{error:#}");
                    if let Ok(mut failure) = self.failure.lock() {
                        *failure = Some(message.clone());
                    }
                    Ok(ToolResult::error(message))
                }
            }
        })
    }
}

/// [`execute_tool`] over an explicit gate, so the gating is testable without a
/// loaded security module.
pub(crate) async fn run_gated(
    tools: &[Box<dyn Tool>],
    gate: Arc<dyn ToolMiddleware<(), OpenHumanRunContext>>,
    tool_name: &str,
    args: serde_json::Value,
    prefer_markdown: bool,
) -> Result<ExecuteToolOutcome, String> {
    run_gated_with_context(
        tools,
        gate,
        tool_name,
        args,
        prefer_markdown,
        OpenHumanRunContext::new(),
    )
    .await
}

async fn run_gated_with_context(
    tools: &[Box<dyn Tool>],
    gate: Arc<dyn ToolMiddleware<(), OpenHumanRunContext>>,
    tool_name: &str,
    args: serde_json::Value,
    prefer_markdown: bool,
    data: OpenHumanRunContext,
) -> Result<ExecuteToolOutcome, String> {
    let tool = tools
        .iter()
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

    let mut stack: MiddlewareStack<(), OpenHumanRunContext> = MiddlewareStack::new();
    stack.push_tool_middleware(gate);
    let ctx = data.into_tinyagents(RunConfig::new("direct-tool"));
    let call = ToolCall {
        id: format!("direct-{tool_name}"),
        name: tool_name.to_string(),
        arguments: args,
        invalid: None,
    };
    let base = DirectTool {
        tool: tool.as_ref(),
        prefer_markdown,
        failure: std::sync::Mutex::new(None),
    };
    trace!(
        tool_name,
        "[native_ops] execute_tool: dispatch through the security gate"
    );
    let execution = stack
        .run_wrapped_tool(&ctx, &(), call, &base)
        .await
        .map(|outcome| outcome.into_result())
        .map_err(|error| format!("tool `{tool_name}` failed: {error}"))
        .and_then(|result| match base.failure.lock().ok().and_then(|f| f.clone()) {
            Some(error) => {
                debug!(tool_name, error = %error, "[native_ops] execute_tool: tool execution failed");
                Err(format!("tool `{tool_name}` failed: {error}"))
            }
            None => Ok(result),
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

#[cfg(all(test, feature = "security-module"))]
#[path = "native_ops_tests.rs"]
mod tests;
