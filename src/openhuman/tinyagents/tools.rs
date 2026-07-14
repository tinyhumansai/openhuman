//! `tinyagents` [`Tool`] adapter over an openhuman [`Tool`] (issue #4249).
//!
//! Wraps `Arc<dyn openhuman::tools::Tool>` so the harness agent-loop can invoke
//! the exact same tools the legacy loop runs. The harness calls `call` with a
//! validated [`TaToolCall`] (parsed JSON arguments + correlation id); we execute
//! the underlying tool and render the [`ToolResult`] the way the LLM should see
//! it (rendered via `output_for_llm`, matching the legacy tool loop).

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use tinyagents::harness::steering::{SteeringCommand, SteeringHandle};
use tinyagents::harness::tool::{
    SandboxMode, Tool, ToolAccess, ToolCall as TaToolCall, ToolExecutionContext, ToolPolicy,
    ToolResult as TaToolResult, ToolRuntime, ToolSchema, ToolSideEffects,
    ToolTimeout as TaToolTimeout, WorkspaceAccess,
};

/// A captured early-exit: a sub-agent invoked an early-exit tool (e.g.
/// `ask_user_clarification`), so the loop should pause and surface `question`
/// to the user. Mirrors the legacy `run_turn_engine` `early_exit_tool` seam.
#[derive(Debug, Clone)]
pub(crate) struct EarlyExit {
    pub(crate) tool: String,
    pub(crate) question: String,
}

/// Shared early-exit hook handed to the adapters for the early-exit tool names.
/// On a successful call to one of those tools it records the [`EarlyExit`] and
/// sends a [`SteeringCommand::Pause`] so the harness loop short-circuits at the
/// next checkpoint (before the next model call) — the tinyagents analogue of the
/// legacy loop's "break on early-exit tool" behavior.
#[derive(Clone)]
pub(crate) struct EarlyExitHook {
    handle: SteeringHandle,
    slot: Arc<Mutex<Option<EarlyExit>>>,
}

impl EarlyExitHook {
    /// Build a hook that pauses `handle` and records into a fresh slot.
    pub(crate) fn new(handle: SteeringHandle) -> Self {
        Self {
            handle,
            slot: Arc::new(Mutex::new(None)),
        }
    }

    /// The captured early-exit, if one fired during the run.
    pub(crate) fn take(&self) -> Option<EarlyExit> {
        // #4469 item 3: recover a poisoned slot rather than panic — a panic while
        // some other tool held this lock must not swallow the early-exit.
        self.slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Record an early-exit and request a cooperative pause. Only the first
    /// early-exit in a run is kept (matching the legacy "halt on first").
    fn trigger(&self, tool: &str, question: String) {
        {
            // #4469 item 3: `into_inner` keeps early-exit recording working even
            // if the slot mutex was poisoned by an unrelated panic.
            let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
            if slot.is_none() {
                *slot = Some(EarlyExit {
                    tool: tool.to_string(),
                    question,
                });
            }
        }
        tracing::info!(tool, "[tinyagents] early-exit tool — requesting pause");
        self.handle.send(SteeringCommand::Pause);
    }
}

/// A harness tool backed by an openhuman [`Tool`].
#[cfg(test)]
pub(crate) struct ToolAdapter {
    inner: Arc<dyn crate::openhuman::tools::Tool>,
}

#[cfg(test)]
impl ToolAdapter {
    /// Wrap a resolved openhuman tool.
    pub(crate) fn new(inner: Arc<dyn crate::openhuman::tools::Tool>) -> Self {
        Self { inner }
    }
}

#[cfg(test)]
#[async_trait]
impl Tool<()> for ToolAdapter {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn schema(&self) -> ToolSchema {
        super::convert::spec_to_schema(&self.inner.spec())
    }

    fn policy(&self) -> ToolPolicy {
        tool_policy_from_openhuman_tool(self.inner.as_ref())
    }

    async fn call(&self, _state: &(), call: TaToolCall) -> tinyagents::Result<TaToolResult> {
        Ok(execute_openhuman_tool(self.inner.as_ref(), call, None).await)
    }

    async fn call_with_context(
        &self,
        _state: &(),
        call: TaToolCall,
        context: ToolExecutionContext,
    ) -> tinyagents::Result<TaToolResult> {
        Ok(execute_openhuman_tool(self.inner.as_ref(), call, Some(&context)).await)
    }
}

pub(crate) fn tool_policy_from_openhuman_tool(
    tool: &dyn crate::openhuman::tools::Tool,
) -> ToolPolicy {
    use crate::openhuman::tools::traits::ToolTimeout;
    use crate::openhuman::tools::PermissionLevel;

    let permission = tool.permission_level();
    let external_effect = tool.external_effect();
    let read_only = matches!(
        permission,
        PermissionLevel::None | PermissionLevel::ReadOnly
    ) && !external_effect;

    let timeout_ms = match tool.timeout_policy(&serde_json::Value::Null) {
        ToolTimeout::Secs(seconds) => Some(seconds.saturating_mul(1000)),
        ToolTimeout::Inherit | ToolTimeout::Unbounded => None,
    };

    ToolPolicy::classified()
        .with_side_effects(ToolSideEffects {
            read_only,
            writes_files: matches!(
                permission,
                PermissionLevel::Write | PermissionLevel::Execute | PermissionLevel::Dangerous
            ),
            network: false,
            installs_dependencies: false,
            destructive: matches!(permission, PermissionLevel::Dangerous),
            external_service: external_effect,
            payment: false,
        })
        .with_runtime(ToolRuntime {
            timeout_ms,
            // tinyagents 1.7 added a structured timeout field alongside the
            // numeric timeout_ms. Inherit the run/global policy and keep the
            // numeric timeout_ms above as the only per-tool deadline signal.
            timeout: TaToolTimeout::Inherit,
            max_retries: None,
            idempotent: tool.is_concurrency_safe(&serde_json::Value::Null),
            cancelable: true,
            sandbox: SandboxMode::Inherit,
            max_result_bytes: tool.max_result_size_chars(),
            streaming: false,
        })
        .with_access(ToolAccess {
            workspace: match permission {
                PermissionLevel::None | PermissionLevel::ReadOnly => WorkspaceAccess::None,
                PermissionLevel::Write | PermissionLevel::Execute | PermissionLevel::Dangerous => {
                    WorkspaceAccess::Any
                }
            },
            trusted_roots: Vec::new(),
            credentials: Vec::new(),
            approval_required: external_effect || matches!(permission, PermissionLevel::Dangerous),
            background_safe: !external_effect && !matches!(permission, PermissionLevel::Dangerous),
        })
}

/// `session_id` stamped on the per-tool `DomainEvent`s this executor publishes.
/// The tinyagents adapter carries no per-turn session handle at this seam, so a
/// stable module label groups its tool-execution telemetry (matches the fixed
/// `"javascript"` label the node runtime uses for the same events).
const TINYAGENTS_TOOL_SESSION: &str = "tinyagents";

/// Execute an openhuman [`Tool`](crate::openhuman::tools::Tool) for a harness
/// [`TaToolCall`] and render the [`TaToolResult`] the way the LLM should see it
/// (mirrors the live-path `HarnessToolExecutor`).
pub(crate) async fn execute_openhuman_tool(
    tool: &dyn crate::openhuman::tools::Tool,
    call: TaToolCall,
    context: Option<&ToolExecutionContext>,
) -> TaToolResult {
    let workspace_root = context
        .and_then(|ctx| ctx.workspace.as_ref())
        .map(|workspace| workspace.root.display().to_string());
    tracing::debug!(
        tool = %call.name,
        call_id = %call.id,
        workspace_root = workspace_root.as_deref().unwrap_or("none"),
        "[tinyagents] executing openhuman tool via harness adapter"
    );

    // Measure the real execution duration (#4467, item 4) and re-publish the
    // per-tool `DomainEvent`s the legacy session loop emitted so SSE consumers
    // keep per-tool start/complete telemetry on the tinyagents path (#4467,
    // item 5). `tool_name` is cloned up front because `call.name` is moved into
    // the `TaToolResult` below.
    let started = std::time::Instant::now();
    let tool_name = call.name.clone();
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::ToolExecutionStarted {
            tool_name: tool_name.clone(),
            session_id: TINYAGENTS_TOOL_SESSION.to_string(),
        },
    );

    // Approval (HITL) now runs in `ApprovalSecurityMiddleware`
    // (`tinyagents/middleware.rs`, a `wrap_tool` middleware) so a denial
    // short-circuits before this executor is reached.
    //
    // Execute through the session tool semantics the live path used
    // (`agent_tool_exec`): `execute_with_context` (so markdown-capable tools
    // render markdown and context-aware tools can see TinyAgents run metadata)
    // under the tool's resolved timeout deadline. Without the deadline an
    // inherited/long-running tool call could hang the turn indefinitely.
    // Per-call `ToolPolicy`/permission gating needs the session policy context,
    // which the per-tool adapter does not carry; approval covers external
    // effects, and `RunPolicy::unknown_tool` recovers unregistered tool names
    // before execution reaches this adapter.
    let options = crate::openhuman::tools::ToolCallOptions {
        prefer_markdown: true,
    };
    let (deadline, timeout_secs) =
        crate::openhuman::tool_timeout::resolve_tool_deadline(tool.timeout_policy(&call.arguments));
    let exec = tool.execute_with_context(call.arguments.clone(), options, context);
    let outcome = match deadline {
        Some(d) => match tokio::time::timeout(d, exec).await {
            Ok(r) => r,
            Err(_) => {
                let elapsed_ms = started.elapsed().as_millis() as u64;
                tracing::warn!(
                    tool = %call.name,
                    timeout_secs,
                    elapsed_ms,
                    "[tinyagents] tool timed out"
                );
                crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::ToolExecutionCompleted {
                        tool_name: tool_name.clone(),
                        session_id: TINYAGENTS_TOOL_SESSION.to_string(),
                        success: false,
                        elapsed_ms,
                    },
                );
                return TaToolResult {
                    call_id: call.id,
                    name: call.name.clone(),
                    content: format!(
                        "Error: tool '{}' timed out after {timeout_secs}s",
                        call.name
                    ),
                    raw: None,
                    error: Some(format!("tool '{}' timed out", call.name)),
                    elapsed_ms,
                };
            }
        },
        None => exec.await,
    };
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let result = match outcome {
        Ok(result) => {
            let content = result.output_for_llm(true);
            let error = if result.is_error {
                Some(content.clone())
            } else {
                None
            };
            TaToolResult {
                call_id: call.id,
                name: call.name,
                content,
                raw: None,
                error,
                elapsed_ms,
            }
        }
        Err(e) => {
            tracing::warn!(tool = %call.name, error = %e, "[tinyagents] tool failed");
            TaToolResult {
                call_id: call.id,
                name: call.name.clone(),
                content: format!("Error executing {}: {e}", call.name),
                raw: None,
                error: Some(e.to_string()),
                elapsed_ms,
            }
        }
    };
    // Terminal per-tool telemetry (#4467, item 5): success is derived from the
    // rendered result's error channel so a tool-reported error surfaces as a
    // failed completion, mirroring the node-runtime bridge.
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::ToolExecutionCompleted {
            tool_name,
            session_id: TINYAGENTS_TOOL_SESSION.to_string(),
            success: result.error.is_none(),
            elapsed_ms,
        },
    );
    result
}

/// A harness tool backed by the routes' shared, `Arc`-owned tool registry sets
/// (`Arc<Vec<Box<dyn Tool>>>`). One adapter is registered per advertised tool
/// name; on call it locates the named tool across the shared sets and executes
/// it — the tinyagents analogue of the live path's `SharedToolExecutor`, which
/// lets a route reuse the same `Arc`-shared tools the legacy loop runs without
/// cloning them.
pub(crate) struct SharedToolAdapter {
    sets: Vec<Arc<Vec<Box<dyn crate::openhuman::tools::Tool>>>>,
    name: String,
    description: String,
    schema: ToolSchema,
    policy: ToolPolicy,
    /// When set, a successful call records an [`EarlyExit`] and pauses the loop.
    early_exit: Option<EarlyExitHook>,
}

impl SharedToolAdapter {
    /// Build an adapter for the tool named `name`, locating it across `sets` to
    /// capture its advertised spec. Returns `None` when no set contains it.
    pub(crate) fn for_name(
        sets: Vec<Arc<Vec<Box<dyn crate::openhuman::tools::Tool>>>>,
        name: &str,
    ) -> Option<Self> {
        let (spec, policy) = sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|t| t.name() == name)
            .map(|t| (t.spec(), tool_policy_from_openhuman_tool(t.as_ref())))?;
        Some(Self {
            sets,
            name: spec.name.clone(),
            description: spec.description.clone(),
            schema: super::convert::spec_to_schema(&spec),
            policy,
            early_exit: None,
        })
    }

    /// Treat this tool as an early-exit tool: a successful call records the
    /// question and pauses the run via `hook`.
    pub(crate) fn with_early_exit(mut self, hook: EarlyExitHook) -> Self {
        self.early_exit = Some(hook);
        self
    }
}

#[async_trait]
impl Tool<()> for SharedToolAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> ToolSchema {
        self.schema.clone()
    }

    fn policy(&self) -> ToolPolicy {
        self.policy.clone()
    }

    async fn call(&self, _state: &(), call: TaToolCall) -> tinyagents::Result<TaToolResult> {
        self.call_openhuman_tool(call, None).await
    }

    async fn call_with_context(
        &self,
        _state: &(),
        call: TaToolCall,
        context: ToolExecutionContext,
    ) -> tinyagents::Result<TaToolResult> {
        self.call_openhuman_tool(call, Some(&context)).await
    }
}

impl SharedToolAdapter {
    async fn call_openhuman_tool(
        &self,
        call: TaToolCall,
        context: Option<&ToolExecutionContext>,
    ) -> tinyagents::Result<TaToolResult> {
        let found = self
            .sets
            .iter()
            .flat_map(|set| set.iter())
            .find(|t| t.name() == self.name);
        match found {
            Some(tool) => {
                let result = execute_openhuman_tool(tool.as_ref(), call, context).await;
                // Early-exit (e.g. `ask_user_clarification`): on a successful
                // call, record the question and pause so the runner can
                // checkpoint and surface the prompt — matching the legacy seam.
                if let Some(hook) = &self.early_exit {
                    if result.error.is_none() {
                        hook.trigger(&self.name, result.content.clone());
                    }
                }
                Ok(result)
            }
            None => {
                tracing::warn!(tool = %self.name, "[tinyagents] shared tool not found");
                Ok(TaToolResult {
                    call_id: call.id,
                    name: call.name,
                    content: format!("Error: unknown tool '{}'", self.name),
                    raw: None,
                    error: Some("unknown tool".to_string()),
                    elapsed_ms: 0,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::ToolTimeout;
    use crate::openhuman::tools::ToolResult as OhToolResult;

    /// A tool whose `execute_with_options` sleeps forever but declares a short
    /// per-call timeout, so the adapter's deadline must fire.
    struct HangingTool;

    #[async_trait]
    impl crate::openhuman::tools::Tool for HangingTool {
        fn name(&self) -> &str {
            "hang"
        }
        fn description(&self) -> &str {
            "hangs"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }
        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<OhToolResult> {
            futures_util::future::pending::<()>().await;
            Ok(OhToolResult::success("never"))
        }
        fn timeout_policy(&self, _args: &serde_json::Value) -> ToolTimeout {
            ToolTimeout::Secs(1)
        }
    }

    /// A fast tool that echoes an argument, to prove the normal path still runs.
    struct EchoTool;

    #[async_trait]
    impl crate::openhuman::tools::Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echoes"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }
        async fn execute(&self, args: serde_json::Value) -> anyhow::Result<OhToolResult> {
            let m = args.get("msg").and_then(|v| v.as_str()).unwrap_or("");
            Ok(OhToolResult::success(format!("echoed:{m}")))
        }
    }

    fn call(name: &str, args: serde_json::Value) -> TaToolCall {
        TaToolCall {
            id: "c1".into(),
            name: name.into(),
            arguments: args,
            invalid: None,
        }
    }

    #[tokio::test]
    async fn tool_execution_respects_the_per_call_timeout() {
        let result =
            execute_openhuman_tool(&HangingTool, call("hang", serde_json::json!({})), None).await;
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|e| e.contains("timed out")),
            "a hanging tool must surface a timeout error, got {:?}",
            result.error
        );
        assert!(result.content.contains("timed out"));
    }

    #[tokio::test]
    async fn fast_tool_runs_to_completion() {
        let result = execute_openhuman_tool(
            &EchoTool,
            call("echo", serde_json::json!({ "msg": "hi" })),
            None,
        )
        .await;
        assert!(result.error.is_none());
        assert!(result.content.contains("echoed:hi"));
    }
}
