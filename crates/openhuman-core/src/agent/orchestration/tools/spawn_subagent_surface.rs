//! The single asynchronous sub-agent spawn surface.

use super::subagent_abort_report::AbortReport;
use crate::agent::orchestration::running_subagents;
use crate::agent::orchestration::subagent_sessions::{
    self, DurableSubagentSession, DurableSubagentStatus, SubagentSessionSelector,
    SubagentSessionStore, SubagentSessionUpsert,
};
use crate::agent::orchestration::tools::dispatch::incomplete_stop;
use crate::agent::subagent_host::SubagentRunOptions;
use crate::agent_registry::host_definition::AgentDefinitionRegistry;
use crate::core::runtime::DispatchCtx;
use crate::progress::AgentProgress;
use crate::threads::store::{self as conversations, ConversationMessage};
use async_trait::async_trait;
use serde_json::json;
use std::borrow::Borrow;
use std::sync::Arc;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::tool::{ToolDispatch, ToolExecutionContext};
use tinyagents_session::transcript::TranscriptMessage;
use tinytools::ToolRunContext;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult};

pub struct SpawnSubagentTool {
    /// The ids this instance advertises in its `agent_id` enum. Empty means
    /// the whole registry. See [`SpawnSubagentTool::scoped`].
    advertised_ids: Vec<String>,
}

/// Harness dispatch for the detached child path. It owns the typed parent run
/// so the spawned child receives the caller's carrier before `tokio::spawn`.
pub(crate) struct SpawnSubagentDispatch {
    tool: Arc<dyn Tool>,
}

impl SpawnSubagentDispatch {
    pub(crate) fn new(tool: Arc<dyn Tool>) -> Self {
        Self { tool }
    }
}

#[async_trait]
impl ToolDispatch<(), crate::agent::tinyagents::host::OpenHumanRunContext>
    for SpawnSubagentDispatch
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
        let context = ToolExecutionContext::from_run_context(parent, _call_id.clone());
        let detached_parent =
            crate::agent::subagent_host::detached_subagent_parent(parent, "async-subagent")?;
        // Headless owners wait for their children before returning, so keep
        // the actual native root as the driver's parent. Ordinary chat keeps
        // its detached carrier so background work outlives the turn.
        let driver_parent = if parent.data.headless_descendant_settlement {
            parent
        } else {
            &detached_parent
        };
        SpawnSubagentTool::new()
            .execute_with_live_parent_context(
                arguments,
                Some(&context),
                parent.data.child(),
                driver_parent,
            )
            .await
    }
}

impl SpawnSubagentTool {
    pub fn new() -> Self {
        Self {
            advertised_ids: Vec::new(),
        }
    }

    /// An instance whose schema advertises only `ids`, the parent's
    /// `[subagents]` allowlist.
    ///
    /// [`scope_spawn_subagent_spec`] narrows the session's spec view,
    /// but on native tool calling the wire schema is read from the registered
    /// tool itself (`CanonicalSharedToolAdapter::for_name` → `Tool::spec`),
    /// so the narrowed view never reached the provider: a captured
    /// orchestrator request carried all 19 registry ids. The session builder
    /// swaps this instance in so both views agree. Execution is unchanged;
    /// `execute` enforces the allowlist either way.
    pub fn scoped(mut ids: Vec<String>) -> Self {
        ids.sort();
        ids.dedup();
        Self {
            advertised_ids: ids,
        }
    }
}

/// Route reusable-session input through the same generic Message operation
/// installed for model calls. Durable session id lookup keeps the host from
/// steering an unrelated live task by a raw task-id string.
async fn steer_reusable_session(
    parent: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    session_id: &str,
    message: String,
) -> Result<(), String> {
    use tinyagents_harness::tool::ToolDispatch;
    use tinyagents_harness_tools::subagents::SubAgentMessageTool;
    use tinyagents_orchestration::subagent::SubAgentJobRegistry;
    use tinytools::{ToolCallOptions, ToolTimeout};

    let context = &parent.data;
    let product_parent = context
        .parent
        .as_ref()
        .ok_or_else(|| "reusable async subagent has no durable parent".to_owned())?;
    let thread_id = parent.thread_id().map(|id| id.as_str().to_owned());
    let owner = context.subagent_owner_scope(
        &product_parent.workspace_dir,
        &product_parent.session_id,
        thread_id.as_deref(),
    );
    let owner_scope: Arc<dyn Fn(u64) -> String + Send + Sync> = Arc::new(move |_| owner.clone());
    let jobs =
        (*crate::core::runtime::slot_for::<SubAgentJobRegistry>(&context.dispatch_ctx())).clone();
    let store = crate::agent::orchestration::subagent_sessions::open_for_context(
        &context.dispatch_ctx(),
        product_parent.workspace_dir.clone(),
        product_parent.session_id.clone(),
        thread_id.clone(),
        Some(jobs.clone()),
    )
    .map_err(|error| format!("open scoped subagent sessions: {error}"))?;
    let parent_session = product_parent.session_id.clone();
    let dispatch = SubAgentMessageTool::new(jobs)
        .with_tool_name("steer_subagent")
        .with_argument_fields("task_id", "subagent_session_id", "message", "__request_id")
        .with_lane_field("mode")
        .with_owner_scope(owner_scope)
        .with_sessions(store, move |_| parent_session.clone())
        .with_metadata(
            PermissionLevel::Execute,
            tinytools::ToolScope::All,
            tinytools::ToolCategory::System,
            ToolTimeout::Inherit,
        );
    let result = ToolDispatch::execute(
        &dispatch,
        &(),
        tinyagents_harness::CallId::new(format!("reuse-steer-{session_id}")),
        json!({
            "subagent_session_id": session_id,
            "message": message,
            "mode": "steer",
        }),
        ToolCallOptions::default(),
        parent,
    )
    .await
    .map_err(|error| error.to_string())?;
    if result.is_error {
        Err(result.output())
    } else {
        Ok(())
    }
}

impl Default for SpawnSubagentTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Narrow a session's `spawn_subagent` schema to the ids the parent may
/// actually dispatch.
///
/// The tool is registered once per process, so its `agent_id` enum is built
/// from the whole registry: 30-odd ids, of which the orchestrator's
/// `[subagents]` allowlist admits about twenty. `execute` already refuses the
/// rest, so advertising them only bought a refused call and a slice of schema
/// on every turn. Called from the per-session spec view
/// (`builder::visible_tool_specs_for_policy`), the same place `use_skill`'s
/// pack index is narrowed. A missing or empty allowlist leaves the spec alone:
/// wildcard parents keep the full registry.
pub fn scope_spawn_subagent_spec(spec: &mut tinytools::ToolSpec, allowed: &[String]) {
    if allowed.is_empty() {
        return;
    }
    let Some(enum_slot) = spec
        .parameters
        .pointer_mut("/properties/agent_id/enum")
        .filter(|value| value.is_array())
    else {
        return;
    };
    let mut ids: Vec<String> = allowed.to_vec();
    ids.sort();
    ids.dedup();
    *enum_slot = serde_json::Value::Array(ids.into_iter().map(serde_json::Value::String).collect());
    if let Some(description) = spec
        .parameters
        .pointer_mut("/properties/agent_id/description")
    {
        *description = serde_json::Value::String(
            "Sub-agent id (only these are dispatchable from here).".to_string(),
        );
    }
}

#[async_trait]
impl Tool for SpawnSubagentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Run a focused sub-agent task asynchronously. Returns immediately while the worker runs; \
         its result is delivered automatically. Reuse the same task_key to continue an existing task."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let scoped = !self.advertised_ids.is_empty();
        let agent_ids: Vec<String> = if scoped {
            self.advertised_ids.clone()
        } else {
            AgentDefinitionRegistry::current()
                .map(|reg| reg.list().iter().map(|d| d.id.clone()).collect())
                .unwrap_or_default()
        };

        let agent_id_schema = if agent_ids.is_empty() {
            json!({
                "type": "string",
                "description": "Sub-agent id (e.g. researcher, summarizer)."
            })
        } else {
            json!({
                "type": "string",
                "enum": agent_ids,
                "description": if scoped {
                    "Sub-agent id (only these are dispatchable from here)."
                } else {
                    "Sub-agent id from the registry."
                }
            })
        };

        json!({
            "type": "object",
            "required": ["agent_id", "prompt"],
            "properties": {
                "agent_id": agent_id_schema,
                "prompt": {
                    "type": "string",
                    "description": "Self-contained instruction with all needed context; the worker cannot ask the user."
                },
                "context": {
                    "type": "string",
                    "description": "Optional prior results, rendered as a `[Context]` block before the prompt."
                },
                "model": {
                    "type": "string",
                    "description": "Optional exact model id for this spawn only."
                },
                "task_title": {
                    "type": "string",
                    "description": "Optional short title for the worker thread."
                },
                "task_key": {
                    "type": "string",
                    "description": "Optional identity key for reusing an existing worker."
                },
                "image_paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional image paths to include with the worker's task."
                },
                "fresh": {
                    "type": "boolean",
                    "description": "Force a fresh worker instead of reusing a matching one."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.execute_with_context_inner(
            args,
            tool_context,
            crate::agent::tinyagents::host::OpenHumanRunContext::new().with_core(
                crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(tool_context)
                    .into_core(),
            ),
            None,
        )
        .await
    }
}

impl SpawnSubagentTool {
    /// Drive the tool the way a live parent turn does, over an explicit
    /// `parent` execution context. Tests only: production turns reach the tool
    /// through [`SpawnSubagentDispatch`] with their own carrier.
    #[cfg(test)]
    pub(crate) async fn execute_under_parent(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        tool_context: Option<&dyn ToolRunContext>,
        parent: crate::agent::subagent_host::ParentExecutionContext,
    ) -> anyhow::Result<ToolResult> {
        let live_parent = super::live_parent_run_context("direct-async-subagent", parent);
        let detached_parent =
            crate::agent::subagent_host::detached_subagent_parent(&live_parent, "async-subagent")?;
        self.execute_with_context_inner(
            args,
            tool_context,
            live_parent.data.child(),
            Some(&detached_parent),
        )
        .await
    }

    pub(crate) async fn execute_with_live_parent_context<P>(
        &self,
        args: serde_json::Value,
        tool_context: Option<&dyn ToolRunContext>,
        run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
        detached_parent: P,
    ) -> anyhow::Result<ToolResult>
    where
        P: Borrow<RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>>,
    {
        self.execute_with_context_inner(
            args,
            tool_context,
            run_context,
            Some(detached_parent.borrow()),
        )
        .await
    }
}

#[path = "spawn_subagent_completion.rs"]
mod completion;

include!("spawn_subagent_driver_lookup.rs");
include!("spawn_subagent_preflight.rs");
include!("spawn_subagent_host_impl.rs");
include!("spawn_subagent_request.rs");
include!("spawn_subagent_projection.rs");
include!("spawn_subagent_workflow.rs");

/// Format the user-facing acceptance text around a structured async sub-agent reference.
///
/// The wording follows the tool contract: the child runs asynchronously and
/// its result arrives through the parent's completion channel.
fn format_async_subagent_accepted(agent_id: &str, payload_json: &str) -> String {
    format!(
        "Accepted async sub-agent `{agent_id}`. Its result is delivered automatically. To continue or answer a clarification, call `spawn_subagent` again with the same task_key.

[async_subagent_ref]
{payload_json}
[/async_subagent_ref]"
    )
}

/// Build the machine-readable reference the orchestrator uses to follow up on a worker.
///
/// Only tools in `fleet` are offered: an instruction naming a tool the parent
/// cannot see costs an iteration of confused reasoning per delegation.
fn async_subagent_ref_payload(
    task_id: &str,
    subagent_session_id: &str,
    task_key: &str,
    agent_id: &str,
    worker_thread_id: Option<&str>,
    reused: bool,
    reuse_decision: &str,
    status: &str,
) -> serde_json::Value {
    let instructions = json!({
        "answer_or_resume": {
            "tool": "spawn_subagent",
            "description": "Continue this task or answer a clarification while preserving its context.",
            "arguments": { "agent_id": agent_id, "task_key": task_key, "prompt": "<follow-up or user answer>" }
        }
    });
    let next_actions = [
        "reuse the same task_key to continue or answer a clarification".to_string(),
        "continue without waiting; the result is delivered automatically".to_string(),
    ];

    json!({
        "task_id": task_id,
        "taskId": task_id,
        "subagent_session_id": subagent_session_id,
        "subagentSessionId": subagent_session_id,
        "task_key": task_key,
        "taskKey": task_key,
        "agent_id": agent_id,
        "agentId": agent_id,
        "mode": "async",
        "status": status,
        "worker_thread_id": worker_thread_id,
        "workerThreadId": worker_thread_id,
        "reused": reused,
        "reuse_decision": reuse_decision,
        "reuseDecision": reuse_decision,
        "result_delivery": "automatic",
        "instructions": instructions,
        "next_actions": next_actions
    })
}

fn add_background_contract(prompt: &str) -> String {
    format!(
        "[Background Contract]\n\
         Run this task without requiring attention from the parent or user. \
         Do not call ask_user_clarification. If required information is missing, \
         make the safest best-effort progress and record the limitation in your final output.\n\n\
         [Task]\n{prompt}"
    )
}

fn durable_task_key_source(
    args: &serde_json::Value,
    prompt: &str,
    context: Option<&str>,
) -> String {
    if let Some(task_key) = args
        .get("task_key")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return task_key.to_string();
    }

    match context.map(str::trim).filter(|s| !s.is_empty()) {
        Some(context) => format!("{prompt}\n\n[Context]\n{context}"),
        None => prompt.to_string(),
    }
}

fn reusable_follow_up_message(prompt: &str, context: Option<&str>) -> String {
    let mut message = String::from("[Follow-up instruction for reusable sub-agent]\n");
    if let Some(context) = context.map(str::trim).filter(|s| !s.is_empty()) {
        message.push_str("\n[Context]\n");
        message.push_str(context);
        message.push_str("\n\n");
    }
    message.push_str("[Task]\n");
    message.push_str(prompt);
    message
}

#[cfg(test)]
#[path = "spawn_subagent_async_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "spawn_subagent_tests.rs"]
mod surface_tests;

#[cfg(all(test, feature = "flows"))]
#[path = "spawn_subagent_proposal_tests.rs"]
mod proposal_tests;
