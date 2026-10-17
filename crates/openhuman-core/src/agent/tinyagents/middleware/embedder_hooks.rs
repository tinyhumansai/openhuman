//! [`EmbedderToolHooksMiddleware`]: deliver tool lifecycle events to the hooks
//! an embedding host installed.

use async_trait::async_trait;

use tinyagents_harness::context::RunContext;
use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::middleware::{Middleware, ToolInvocationIdentity};
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinytools::ToolResult as TaToolResult;

/// Delivers tool lifecycle events to hooks installed by an embedding host.
pub(crate) struct EmbedderToolHooksMiddleware {
    hooks: Vec<std::sync::Arc<dyn crate::agent::hooks::ToolHook>>,
    /// Normalized pre-call arguments keyed by provider `call_id`, so the
    /// `PostToolUse` context can hand an embedding host the same arguments its
    /// `PreToolUse` hook saw. The crate's `ToolResult` does not carry the
    /// original call arguments, so without this cache every post-use event would
    /// report `Null` and an auditing/correlating host could not match inputs to
    /// outcomes.
    pub(super) arguments_by_call_id:
        std::sync::Mutex<std::collections::HashMap<String, serde_json::Value>>,
}

impl EmbedderToolHooksMiddleware {
    pub(crate) fn new(hooks: Vec<std::sync::Arc<dyn crate::agent::hooks::ToolHook>>) -> Self {
        Self {
            hooks,
            arguments_by_call_id: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

fn hook_identity(
    data: &crate::agent::tinyagents::host::OpenHumanRunContext,
) -> (Option<String>, Option<String>, Option<std::path::PathBuf>) {
    let session_id = data
        .thread_id
        .clone()
        .or_else(|| data.parent.as_ref().map(|parent| parent.session_id.clone()));
    let agent_id = data
        .parent
        .as_ref()
        .map(|parent| parent.agent_definition_id.clone());
    let cwd = data
        .workspace
        .as_ref()
        .map(|workspace| workspace.root.clone())
        .or_else(|| {
            data.parent.as_ref().and_then(|parent| {
                parent
                    .workspace_descriptor
                    .as_ref()
                    .map(|workspace| workspace.root.clone())
            })
        })
        .or_else(|| {
            crate::core::runtime::CoreContext::with_current_embedder_config(|config| {
                config.action_dir.clone()
            })
        });
    (session_id, agent_id, cwd)
}

#[async_trait]
impl Middleware<(), crate::agent::tinyagents::host::OpenHumanRunContext>
    for EmbedderToolHooksMiddleware
{
    fn name(&self) -> &str {
        "embedder_tool_hooks"
    }

    async fn before_tool(
        &self,
        ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        call: &mut TaToolCall,
    ) -> TaResult<()> {
        let (session_id, agent_id, cwd) = hook_identity(&ctx.data);
        let mut context = crate::agent::hooks::ToolHookContext {
            event: crate::agent::hooks::ToolHookEvent::PreToolUse,
            call_id: call.id.clone(),
            tool_name: call.name.clone(),
            arguments: call.arguments.clone(),
            success: None,
            duration_ms: None,
            output: None,
            error: None,
            session_id,
            agent_id,
            cwd,
        };
        let dispatch = ctx.data.dispatch_ctx();
        for hook in &self.hooks {
            match hook.before_tool_decision_in(&dispatch, &context).await {
                crate::agent::hooks::ToolHookDecision::Proceed => {}
                // A rewrite is applied to the live call *and* to the context
                // handed to later hooks, so a chain of hooks each narrowing the
                // arguments composes instead of the last one silently winning.
                crate::agent::hooks::ToolHookDecision::ProceedWith(arguments) => {
                    tracing::debug!(
                        hook = hook.name(),
                        tool = context.tool_name,
                        "[tinyagents::mw] tool hook rewrote call arguments"
                    );
                    call.arguments = arguments.clone();
                    context.arguments = arguments;
                }
                crate::agent::hooks::ToolHookDecision::Deny(reason) => {
                    return Err(tinyagents_harness::error::TinyAgentsError::Tool(format!(
                        "tool hook '{}' denied {}: {reason}",
                        hook.name(),
                        context.tool_name
                    )));
                }
                // There is no approval channel inside a middleware, and a hook
                // that asks for a human is asking for something stricter than
                // "proceed" — so an unresolvable `Ask` denies rather than
                // quietly allowing. The approval-gate path in
                // `security::approval` is where an interactive host resolves it.
                crate::agent::hooks::ToolHookDecision::Ask(reason) => {
                    tracing::info!(
                        hook = hook.name(),
                        tool = context.tool_name,
                        "[tinyagents::mw] tool hook requested approval; denying in a \
                         non-interactive middleware"
                    );
                    return Err(tinyagents_harness::error::TinyAgentsError::Tool(format!(
                        "tool hook '{}' requires approval for {}: {reason}",
                        hook.name(),
                        context.tool_name
                    )));
                }
            }
        }
        // Cache the (already-recovered) arguments only once every hook approved
        // the call: a vetoed call never reaches `after_tool`, so storing it here
        // would leak a cache entry for the turn.
        self.arguments_by_call_id
            .lock()
            .expect("embedder tool-hook arguments poisoned")
            .insert(call.id.clone(), call.arguments.clone());
        Ok(())
    }

    /// Nested-call form of the `before_tool` enforcement above.
    ///
    /// `before_tool` never runs for a call a tool makes through
    /// `ToolExecutionContext::call_tool`, so without this a `Deny` hook could
    /// be bypassed by any tool that calls another. Mirrors `before_tool`:
    /// `Deny` and `Ask` refuse (a nested call can never be parked for a human).
    /// A `ProceedWith` rewrite cannot be applied to a call the middleware may
    /// not mutate, so it refuses too rather than letting the un-narrowed
    /// arguments through.
    async fn check_nested_tool(
        &self,
        ctx: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        call: &TaToolCall,
    ) -> TaResult<()> {
        let (session_id, agent_id, cwd) = hook_identity(&ctx.data);
        let context = crate::agent::hooks::ToolHookContext {
            event: crate::agent::hooks::ToolHookEvent::PreToolUse,
            call_id: call.id.clone(),
            tool_name: call.name.clone(),
            arguments: call.arguments.clone(),
            success: None,
            duration_ms: None,
            output: None,
            error: None,
            session_id,
            agent_id,
            cwd,
        };
        let dispatch = ctx.data.dispatch_ctx();
        for hook in &self.hooks {
            let refusal = match hook.before_tool_decision_in(&dispatch, &context).await {
                crate::agent::hooks::ToolHookDecision::Proceed => continue,
                crate::agent::hooks::ToolHookDecision::ProceedWith(_) => {
                    "rewrites the call, which a nested call cannot apply".to_string()
                }
                crate::agent::hooks::ToolHookDecision::Deny(reason) => {
                    format!("denied: {reason}")
                }
                crate::agent::hooks::ToolHookDecision::Ask(reason) => {
                    format!("requires approval, unavailable for a nested call: {reason}")
                }
            };
            tracing::info!(
                hook = hook.name(),
                tool = context.tool_name,
                "[tinyagents::mw] nested tool call refused by tool hook"
            );
            return Err(tinyagents_harness::error::TinyAgentsError::Tool(format!(
                "tool hook '{}' refused nested call to {}: {refusal}",
                hook.name(),
                context.tool_name
            )));
        }
        Ok(())
    }

    async fn after_tool(
        &self,
        ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        invocation: &ToolInvocationIdentity,
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        let call_id = invocation.call_id().to_string();
        let tool_name = invocation.tool_name();
        let arguments = self
            .arguments_by_call_id
            .lock()
            .expect("embedder tool-hook arguments poisoned")
            .remove(&call_id)
            .unwrap_or(serde_json::Value::Null);
        let (session_id, agent_id, cwd) = hook_identity(&ctx.data);
        let context = crate::agent::hooks::ToolHookContext {
            event: crate::agent::hooks::ToolHookEvent::PostToolUse,
            call_id,
            tool_name: tool_name.to_string(),
            arguments,
            success: Some(!result.is_error),
            duration_ms: None,
            output: Some(crate::agent::tinyagents::middleware::tool_result_text(
                result,
            )),
            error: result
                .is_error
                .then(|| crate::agent::tinyagents::middleware::tool_result_text(result)),
            session_id,
            agent_id,
            cwd,
        };
        for hook in &self.hooks {
            // Text a hook returns is appended to the result the model reads —
            // the seam a "you edited a file, here is the linter output" hook
            // needs. It is appended rather than substituted so a hook cannot
            // erase what the tool actually said.
            if let Some(additional) = hook.after_tool_context(&context).await {
                if !additional.trim().is_empty() {
                    tracing::debug!(
                        hook = hook.name(),
                        tool = context.tool_name,
                        chars = additional.chars().count(),
                        "[tinyagents::mw] tool hook appended context to the result"
                    );
                    crate::agent::tinyagents::middleware::append_tool_result_text(
                        result,
                        format!("\n\n{}", additional.trim_end()),
                    );
                }
            }
        }
        Ok(())
    }
}
