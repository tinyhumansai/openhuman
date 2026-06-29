//! The Agent's per-call tool executor, extracted as a free function so both
//! [`super::types::Agent::execute_tool_call`] and the turn engine's
//! `AgentToolSource` run the exact same path (visibility gate → session policy
//! → per-call permission → pluggable `ToolPolicy` → `execute_with_options` +
//! payload summarizer → per-result byte budget), without one borrowing the
//! `Agent` while the turn observer borrows it mutably.
//!
//! Progress is emitted through a [`ProgressReporter`] (the channel/web flavor),
//! matching the `Agent::turn` events 1:1.

use std::collections::HashSet;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::dispatcher::{ParsedToolCall, ToolExecutionResult};
use crate::openhuman::agent::harness::engine::ProgressReporter;
use crate::openhuman::agent::harness::payload_summarizer::PayloadSummarizer;
use crate::openhuman::agent::harness::tool_result_artifacts::{
    apply_per_result_persistence, ToolResultArtifactStore,
};
use crate::openhuman::agent::hooks::{self, ToolCallRecord};
use crate::openhuman::agent::tool_policy::{
    ToolCallContext, ToolPolicy, ToolPolicyDecision, ToolPolicyRequest,
};
use crate::openhuman::agent_tool_policy::ToolPolicySession;
use crate::openhuman::tools::{Tool, ToolCallOptions};
use crate::openhuman::util::truncate_with_ellipsis;

/// Read-only context the Agent tool executor needs, captured up front so it
/// never borrows the `Agent` (whose history/context the turn observer mutates).
pub(super) struct AgentToolExecCtx<'a> {
    pub tools: &'a [Box<dyn Tool>],
    pub visible_tool_names: &'a HashSet<String>,
    pub tool_policy_session: &'a ToolPolicySession,
    pub tool_policy: &'a dyn ToolPolicy,
    pub payload_summarizer: Option<&'a dyn PayloadSummarizer>,
    pub event_session_id: &'a str,
    pub event_channel: &'a str,
    pub agent_definition_id: &'a str,
    pub prefer_markdown: bool,
    pub budget_bytes: usize,
    /// Whether Stage 1a (native content-aware compaction) runs before the
    /// byte budget. Sourced from `ContextManager::compaction_enabled`.
    pub compaction_enabled: bool,
    /// Agent-level TokenJuice profile for this session's tool results.
    pub tokenjuice_compression: crate::openhuman::tokenjuice::AgentTokenjuiceCompression,
    pub artifact_store: Option<&'a ToolResultArtifactStore>,
}

/// Execute one parsed tool call end-to-end with the Agent's semantics, emitting
/// `ToolCallStarted` / `ToolCallCompleted` through `progress`. Returns the
/// result (for history formatting) + the call record (for post-turn hooks).
pub(super) async fn run_agent_tool_call(
    ctx: &AgentToolExecCtx<'_>,
    progress: &dyn ProgressReporter,
    call: &ParsedToolCall,
    iteration: usize,
) -> (ToolExecutionResult, ToolCallRecord) {
    let started = std::time::Instant::now();
    publish_global(DomainEvent::ToolExecutionStarted {
        tool_name: call.name.clone(),
        session_id: ctx.event_session_id.to_string(),
    });
    // Synthesise a fallback id for prompt-guided (non-native) tool calls so
    // downstream consumers always have a stable key to reconcile rows by.
    let call_id = call.tool_call_id.clone().unwrap_or_else(|| {
        format!(
            "turn-{iteration}-{}-{}",
            call.name,
            uuid::Uuid::new_v4().simple()
        )
    });
    // Resolve the tool (when visible) to compute the server-side human label
    // + contextual detail once, so dynamic Composio/MCP tools never surface as
    // raw snake_case. `None` when the tool isn't found → client formatter.
    let (display_label, display_detail) = match ctx.tools.iter().find(|t| t.name() == call.name) {
        Some(tool) => (
            tool.display_label(&call.arguments),
            tool.display_detail(&call.arguments),
        ),
        None => (None, None),
    };
    progress
        .tool_started(
            &call_id,
            &call.name,
            &call.arguments,
            (iteration + 1) as u32,
            display_label.as_deref(),
            display_detail.as_deref(),
        )
        .await;
    log::info!("[agent] executing tool: {}", call.name);

    let (raw_result, success) = if !ctx.visible_tool_names.is_empty()
        && !ctx.visible_tool_names.contains(&call.name)
    {
        log::warn!(
            "[agent] blocked tool call '{}' — not in visible tool set",
            call.name
        );
        (
            format!("Tool '{}' is not available to this agent", call.name),
            false,
        )
    } else if let Some(tool) = ctx.tools.iter().find(|t| t.name() == call.name) {
        let session_decision = ctx.tool_policy_session.decision_for(&call.name);
        if session_decision.is_denied() {
            let required = session_decision
                .required_permission
                .map(|permission| permission.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            (
                format!(
                    "Tool '{}' blocked by tool policy: requires {}, channel '{}' allows {}",
                    call.name, required, ctx.event_channel, session_decision.allowed_permission
                ),
                false,
            )
        } else {
            let call_required = tool.permission_level_with_args(&call.arguments);
            if call_required > session_decision.allowed_permission {
                tracing::debug!(
                    tool = call.name.as_str(),
                    call_required = %call_required,
                    allowed = %session_decision.allowed_permission,
                    "[agent_loop] tool action blocked by per-call permission check"
                );
                (
                    format!(
                        "Tool '{}' action requires {} permission, channel '{}' allows {}",
                        call.name,
                        call_required,
                        ctx.event_channel,
                        session_decision.allowed_permission
                    ),
                    false,
                )
            } else {
                let context = ToolCallContext::session(
                    ctx.event_session_id,
                    ctx.event_channel,
                    ctx.agent_definition_id.to_string(),
                    call_id.clone(),
                    (iteration + 1) as u32,
                );
                let mut policy_request =
                    ToolPolicyRequest::new(call.name.clone(), call.arguments.clone(), context);
                if let Some(generated_context) = tool.generated_runtime_context(&call.arguments) {
                    policy_request = policy_request.with_generated_tool_context(generated_context);
                }
                let policy_decision = ctx.tool_policy.check(&policy_request).await;
                if let Some(reason) = policy_decision.blocking_reason() {
                    let blocked_action = match &policy_decision {
                        ToolPolicyDecision::RequireApproval { .. } => "requires approval",
                        ToolPolicyDecision::Deny { .. } => "denied",
                        ToolPolicyDecision::Allow => "allowed",
                    };
                    crate::openhuman::tool_registry::denials::record(
                        call.name.as_str(),
                        ctx.tool_policy.name(),
                        blocked_action,
                        reason,
                    );
                    tracing::debug!(
                        tool = call.name.as_str(),
                        policy = ctx.tool_policy.name(),
                        action = blocked_action,
                        reason = %reason,
                        "[agent_loop] tool blocked by policy"
                    );
                    (
                        format!(
                            "Tool '{}' {blocked_action} by policy '{}': {reason}",
                            call.name,
                            ctx.tool_policy.name()
                        ),
                        false,
                    )
                } else {
                    let options = ToolCallOptions {
                        prefer_markdown: ctx.prefer_markdown,
                    };
                    let policy = tool.timeout_policy(&call.arguments);
                    let (tool_deadline, timeout_secs) =
                        crate::openhuman::agent::harness::engine::tools::resolve_tool_deadline(
                            policy,
                        );
                    match policy {
                        crate::openhuman::tools::traits::ToolTimeout::Secs(req) => {
                            tracing::debug!(
                                tool = call.name.as_str(),
                                requested_timeout_secs = req,
                                effective_timeout_secs = timeout_secs,
                                "[agent_loop] session tool requested explicit per-call timeout"
                            );
                        }
                        crate::openhuman::tools::traits::ToolTimeout::Unbounded => {
                            tracing::debug!(
                                tool = call.name.as_str(),
                                "[agent_loop] session tool runs unbounded"
                            );
                        }
                        crate::openhuman::tools::traits::ToolTimeout::Inherit => {
                            tracing::trace!(
                                tool = call.name.as_str(),
                                effective_timeout_secs = timeout_secs,
                                "[agent_loop] session tool inherits global timeout"
                            );
                        }
                    }
                    let outcome = match tool_deadline {
                        Some(deadline) => {
                            tokio::time::timeout(
                                deadline,
                                tool.execute_with_options(call.arguments.clone(), options),
                            )
                            .await
                        }
                        None => Ok(tool
                            .execute_with_options(call.arguments.clone(), options)
                            .await),
                    };
                    match outcome {
                        Ok(Ok(r)) => {
                            if !r.is_error {
                                let mut output = r.output_for_llm(ctx.prefer_markdown);
                                if ctx.prefer_markdown && r.markdown_formatted.is_some() {
                                    log::debug!(
                                        "[agent_loop] tool={} returned markdown payload bytes={}",
                                        call.name,
                                        output.len()
                                    );
                                }
                                if let Some(ps) = ctx.payload_summarizer {
                                    log::debug!(
                                        "[agent_loop] payload_summarizer intercepting tool={} bytes={}",
                                        call.name,
                                        output.len()
                                    );
                                    match ps.maybe_summarize(&call.name, None, &output).await {
                                        Ok(Some(payload)) => {
                                            log::info!(
                                                "[agent_loop] payload_summarizer compressed tool={} {}->{} bytes",
                                                call.name,
                                                payload.original_bytes,
                                                payload.summary_bytes
                                            );
                                            output = payload.summary;
                                        }
                                        Ok(None) => {
                                            log::debug!(
                                                "[agent_loop] payload_summarizer pass-through tool={} bytes={}",
                                                call.name,
                                                output.len()
                                            );
                                        }
                                        Err(e) => {
                                            log::warn!(
                                                "[agent_loop] payload_summarizer error tool={} err={} (passing raw payload through)",
                                                call.name,
                                                e
                                            );
                                        }
                                    }
                                }
                                (output, true)
                            } else {
                                (
                                    format!("Error: {}", r.output_for_llm(ctx.prefer_markdown)),
                                    false,
                                )
                            }
                        }
                        Ok(Err(e)) => (format!("Error executing {}: {e}", call.name), false),
                        Err(_) => {
                            let message = format!(
                                "tool '{}' timed out after {} seconds",
                                call.name, timeout_secs
                            );
                            tracing::warn!(
                                tool = call.name.as_str(),
                                timeout_secs,
                                "[agent_loop] session tool timed out"
                            );
                            crate::core::observability::report_error(
                                message.as_str(),
                                "tool",
                                "execute",
                                &[
                                    ("tool", call.name.as_str()),
                                    ("outcome", "timeout"),
                                    ("timeout_secs", &timeout_secs.to_string()),
                                    ("iteration", &(iteration + 1).to_string()),
                                ],
                            );
                            (format!("Error: {message}"), false)
                        }
                    }
                }
            }
        }
    } else {
        (format!("Unknown tool: {}", call.name), false)
    };

    // Stage 1a — content-aware compaction via the TokenJuice content router.
    // Runs before the byte budget on the fresh tool output (never sent to the
    // backend yet, so it's cache-safe like the budget below). Detects the
    // content kind, routes to the matching compressor, and offloads the
    // original to CCR (recoverable via `tokenjuice_retrieve`); only ever
    // shrinks, otherwise passes the original through.
    let raw_result = crate::openhuman::tokenjuice::compact_output_with_policy(
        raw_result,
        &call.name,
        ctx.compaction_enabled,
        ctx.tokenjuice_compression,
    )
    .await;

    // Per-result byte budget — the only cache-safe reduction stage (the full
    // body has never been sent to the backend). Oversized outputs are persisted
    // into the action workspace when possible, with truncation as fallback.
    let (result, budget_outcome) = apply_per_result_persistence(
        raw_result,
        ctx.artifact_store,
        &call.name,
        Some(&call_id),
        ctx.budget_bytes,
    )
    .await;
    if budget_outcome.persisted {
        log::info!(
            "[agent_loop] tool_result_artifact applied name={} original_bytes={} final_bytes={}",
            call.name,
            budget_outcome.original_bytes,
            budget_outcome.final_bytes,
        );
    } else if budget_outcome.original_bytes != budget_outcome.final_bytes {
        log::info!(
            "[agent_loop] tool_result_budget applied name={} original_bytes={} final_bytes={} dropped_bytes={}",
            call.name,
            budget_outcome.original_bytes,
            budget_outcome.final_bytes,
            budget_outcome.original_bytes - budget_outcome.final_bytes
        );
    }

    let elapsed_ms = started.elapsed().as_millis() as u64;
    publish_global(DomainEvent::ToolExecutionCompleted {
        tool_name: call.name.clone(),
        session_id: ctx.event_session_id.to_string(),
        success,
        elapsed_ms,
    });
    progress
        .tool_completed(
            &call_id,
            &call.name,
            success,
            &result,
            elapsed_ms,
            (iteration + 1) as u32,
        )
        .await;
    log::info!(
        "[agent] tool completed: {} success={} elapsed_ms={}",
        call.name,
        success,
        elapsed_ms
    );
    log::debug!(
        "[agent] tool output for {}: {}",
        call.name,
        truncate_with_ellipsis(&result, 500)
    );

    let output_summary = hooks::sanitize_tool_output(&result, &call.name, success);
    let record = ToolCallRecord {
        name: call.name.clone(),
        arguments: call.arguments.clone(),
        success,
        output_summary,
        duration_ms: elapsed_ms,
    };
    let exec_result = ToolExecutionResult {
        name: call.name.clone(),
        output: result,
        success,
        tool_call_id: call.tool_call_id.clone(),
    };
    (exec_result, record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::dispatcher::ParsedToolCall;
    use crate::openhuman::agent::tool_policy::AllowAllToolPolicy;
    use crate::openhuman::agent_tool_policy::ToolPolicyEngine;
    use crate::openhuman::tools::traits::{ToolResult, ToolTimeout};
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct HangingTool;
    struct TestProgress {
        completed: AtomicUsize,
        timeout_completions: AtomicUsize,
    }

    #[async_trait]
    impl ProgressReporter for TestProgress {
        async fn tool_completed(
            &self,
            _call_id: &str,
            _tool_name: &str,
            success: bool,
            output: &str,
            _elapsed_ms: u64,
            _iteration: u32,
        ) {
            self.completed.fetch_add(1, Ordering::Relaxed);
            if !success && output.contains("timed out after 1 seconds") {
                self.timeout_completions.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    #[async_trait]
    impl Tool for HangingTool {
        fn name(&self) -> &str {
            "memory_tree"
        }

        fn description(&self) -> &str {
            "test tool that never finishes before the harness timeout"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({ "type": "object", "properties": {} })
        }

        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(ToolResult::success("late result"))
        }

        fn timeout_policy(&self, _args: &serde_json::Value) -> ToolTimeout {
            ToolTimeout::Secs(1)
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn session_tool_executor_enforces_tool_timeout_policy() {
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(HangingTool)];
        let visible_tool_names = HashSet::new();
        let policy_session = ToolPolicyEngine::build_session(
            "context_scout",
            "web",
            "test",
            &HashMap::new(),
            &tools,
            &visible_tool_names,
        );
        let tool_policy = AllowAllToolPolicy;
        let ctx = AgentToolExecCtx {
            tools: &tools,
            visible_tool_names: &visible_tool_names,
            tool_policy_session: &policy_session,
            tool_policy: &tool_policy,
            payload_summarizer: None,
            event_session_id: "session-1",
            event_channel: "web",
            agent_definition_id: "context_scout",
            prefer_markdown: false,
            budget_bytes: 4096,
            compaction_enabled: false,
            tokenjuice_compression: crate::openhuman::tokenjuice::AgentTokenjuiceCompression::Off,
            artifact_store: None,
        };
        let call = ParsedToolCall {
            name: "memory_tree".to_string(),
            arguments: json!({"mode": "walk", "query": "project context"}),
            tool_call_id: Some("call-1".to_string()),
        };
        let progress = TestProgress {
            completed: AtomicUsize::new(0),
            timeout_completions: AtomicUsize::new(0),
        };

        let outcome = tokio::time::timeout(
            Duration::from_secs(8),
            run_agent_tool_call(&ctx, &progress, &call, 0),
        )
        .await;

        let (result, record) =
            outcome.expect("session tool executor should return a terminal timeout result");
        assert!(!result.success);
        assert!(result.output.contains("timed out after 1 seconds"));
        assert!(!record.success);
        assert_eq!(progress.completed.load(Ordering::Relaxed), 1);
        assert_eq!(progress.timeout_completions.load(Ordering::Relaxed), 1);
    }
}
