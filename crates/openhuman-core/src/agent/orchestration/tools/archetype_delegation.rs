use async_trait::async_trait;
use serde_json::json;
use serde_json::Value;

use tinytools::ToolRunContext;
use tinytools::{
    PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolExposure, ToolResult, ToolTimeout,
};

pub struct ArchetypeDelegationTool {
    pub tool_name: String,
    /// The agent this tool routes to, in the shape
    /// [`crate::tools::host_extensions::delegation_target`] reads back off the
    /// erased host-extension slot.
    ///
    /// A newtype rather than a bare `String` because that slot is one `Any` per
    /// tool: a downcast to `String` would happily match any *other* tool that
    /// parked a string there. It holds the id rather than deriving it because
    /// [`Tool::host_extension`] hands out a borrow, so there must be something
    /// to borrow from — and one field, not two, is what stops the exposed
    /// target drifting from the routed one.
    pub agent_id: DelegationTarget,
    pub tool_description: String,
}

/// The agent a synthesised `delegate_*` tool routes to.
///
/// Lets a caller that holds only `&dyn Tool` ask "which agent does this reach?"
/// — the question the toolpack route hint needs answered, and the reason the
/// hint does not need its own copy of every agent's `delegate_name`. The tool
/// set a session was actually built with is the single source of truth: a
/// delegate that is not in it cannot be named as a route, which is exactly the
/// property we want.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegationTarget(pub String);

#[async_trait]
impl Tool for ArchetypeDelegationTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Hidden
    }

    /// Publishes the routing target on the erased host-extension slot, the same
    /// way `UseSkillTool` publishes its pack handle. `traits::delegation_target`
    /// reads it back; every other tool returns `None` and pays nothing.
    fn host_extension(&self) -> Option<&(dyn std::any::Any + Send + Sync)> {
        Some(&self.agent_id)
    }

    /// The delegation envelope: `prompt` and `blocking` only.
    ///
    /// This schema is emitted for every synthesised `delegate_*` tool on the
    /// wire, so each word is billed per delegate on every request. Fully
    /// described it was 356 tokens per delegate; the structured hand-off fields
    /// (`objective`, `evidence`, `constraints`, `must_not_assume`,
    /// `expected_output`, `citation_requirement`, `model`) are no longer
    /// advertised because a self-contained `prompt` carries the same content.
    /// They are still read by [`render_structured_handoff`], so a caller that
    /// sends them keeps working. `blocking` keeps its description because its
    /// default is behaviour-critical and not inferable from the name.
    ///
    /// Shared with the collapsed `delegate_to` through
    /// [`delegation_envelope_properties`]; enforced by
    /// `envelope_descriptions_stay_within_budget`.
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["prompt"],
            "properties": delegation_envelope_properties()
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    /// Run **without** the global per-tool wall-clock deadline. This tool is a
    /// delegation primitive: it hands a task to a bounded sub-agent
    /// (`tools_agent` → `delegate_tools_agent`, `code_executor` → `run_code`,
    /// …) and awaits that agent's full run. Under the default `Inherit` policy
    /// the whole delegation is hard-killed at the single-tool timeout (120s) —
    /// so any sub-agent run that legitimately exceeds two minutes is truncated
    /// mid-flight (Sentry TAURI-RUST-K29 `delegate_tools_agent` and
    /// TAURI-RUST-8HB `run_code`: thousands of 120.000s truncations). The
    /// child's lifetime is already bounded internally — by its `max_iterations`,
    /// the run cancellation token, and each inner tool's own timeout — so it
    /// governs its own duration, exactly like the sibling `spawn_parallel_agents`
    /// fan-out and the long-running shell tool.
    fn timeout_policy(&self, _args: &serde_json::Value) -> ToolTimeout {
        ToolTimeout::Unbounded
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
        let mut run_context = crate::agent::tinyagents::host::OpenHumanRunContext::new();
        run_context.thread_id = tool_context
            .and_then(ToolRunContext::thread_id)
            .map(ToOwned::to_owned);
        execute_archetype_delegation(
            &self.agent_id.0,
            &self.tool_name,
            args,
            tool_context,
            run_context,
        )
        .await
    }
}

/// Execute an archetype hand-off with an explicit child run carrier.
pub(crate) async fn execute_archetype_delegation(
    agent_id: &str,
    tool_name: &str,
    args: serde_json::Value,
    tool_context: Option<&dyn ToolRunContext>,
    run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
) -> anyhow::Result<ToolResult> {
    if let Some(live_parent) = super::ambient_parent_run_context("direct-archetype-delegation") {
        let run_context = live_parent.data.child();
        return execute_archetype_delegation_with_live_parent(
            agent_id,
            tool_name,
            args,
            tool_context,
            run_context,
            Some(&live_parent),
        )
        .await;
    }
    execute_archetype_delegation_with_live_parent(
        agent_id,
        tool_name,
        args,
        tool_context,
        run_context,
        None,
    )
    .await
}

/// Typed-harness counterpart that preserves a live parent for the blocking
/// archetype child.
pub(crate) async fn execute_archetype_delegation_with_live_parent(
    agent_id: &str,
    tool_name: &str,
    args: serde_json::Value,
    tool_context: Option<&dyn ToolRunContext>,
    run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
    live_parent: Option<
        &tinyagents_harness::context::RunContext<
            crate::agent::tinyagents::host::OpenHumanRunContext,
        >,
    >,
) -> anyhow::Result<ToolResult> {
    let raw_prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if raw_prompt.is_empty() {
        return Ok(ToolResult::error(format!(
            "{}: `prompt` is required",
            tool_name
        )));
    }
    let prompt = render_structured_handoff(&raw_prompt, &args);
    let prompt = match crate::agent::attachments::delegation_prompt(
        &prompt,
        &args,
        tool_context
            .and_then(|ctx| ctx.workspace())
            .or(run_context.workspace.as_ref()),
        run_context.origin.as_ref(),
    )
    .await
    {
        Ok(prompt) => prompt,
        Err(error) => {
            return Ok(ToolResult::error(format!(
                "image forwarding failed: {error}"
            )));
        }
    };

    let model_override = args
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    // Async by default: the delegated specialist runs as a durable,
    // resumable worker and its result comes back as a new chat turn.
    // `blocking: true` is the opt-in for results that must gate this
    // reply. (`dispatch_subagent` itself falls back to blocking when
    // there is no chat thread to deliver an async result into.)
    let blocking = args
        .get("blocking")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mode = if blocking {
        super::dispatch::DispatchMode::Blocking
    } else {
        super::dispatch::DispatchMode::PreferAsync
    };

    super::dispatch::dispatch_subagent_with_live_parent(
        agent_id,
        tool_name,
        &prompt,
        model_override,
        tool_context,
        mode,
        run_context,
        live_parent,
    )
    .await
}

pub(super) fn render_structured_handoff(prompt: &str, args: &Value) -> String {
    let mut out = String::new();
    out.push_str("Task:\n");
    out.push_str(prompt.trim());

    push_optional_string(&mut out, "Objective", args.get("objective"));
    push_optional_array(&mut out, "Evidence", args.get("evidence"));
    push_optional_array(&mut out, "Constraints", args.get("constraints"));
    push_optional_array(&mut out, "Must not assume", args.get("must_not_assume"));
    push_optional_string(&mut out, "Expected output", args.get("expected_output"));
    push_optional_string(
        &mut out,
        "Citation requirement",
        args.get("citation_requirement"),
    );

    out
}

/// The advertised hand-off envelope, shared by every member delegate and the
/// collapsed `delegate_to` so the two cannot drift. Only `prompt` and
/// `blocking` are offered; the structured fields (`objective`, `evidence`,
/// `constraints`, `must_not_assume`, `expected_output`,
/// `citation_requirement`, `model`) are still read by
/// [`render_structured_handoff`] when a caller sends them.
pub(super) fn delegation_envelope_properties() -> Value {
    serde_json::json!({
        "prompt": {
            "type": "string",
            "description": "The whole task, self-contained: the worker has no memory of this chat."
        },
        "image_paths": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Image paths relative to the acting workspace."
        },
        "blocking": {
            "type": "boolean",
            "description": "Default false: async worker, result arrives as a later turn. true: waits, and the result gates this reply."
        }
    })
}

fn push_optional_string(out: &mut String, label: &str, value: Option<&Value>) {
    let Some(text) = value.and_then(Value::as_str).map(str::trim) else {
        return;
    };
    if text.is_empty() {
        return;
    }
    out.push_str("\n\n");
    out.push_str(label);
    out.push_str(":\n");
    out.push_str(text);
}

fn push_optional_array(out: &mut String, label: &str, value: Option<&Value>) {
    let Some(items) = value.and_then(Value::as_array) else {
        return;
    };
    let strings: Vec<&str> = items
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if strings.is_empty() {
        return;
    }
    out.push_str("\n\n");
    out.push_str(label);
    out.push_str(":\n");
    for item in strings {
        out.push_str("- ");
        out.push_str(item);
        out.push('\n');
    }
    if out.ends_with('\n') {
        out.pop();
    }
}

#[cfg(test)]
#[path = "archetype_delegation_tests.rs"]
mod tests;
