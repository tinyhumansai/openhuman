use async_trait::async_trait;
use serde_json::json;
use serde_json::Value;

use crate::openhuman::tools::traits::{
    PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolResult, ToolTimeout,
};
use tinytools::ToolRunContext;

pub struct ArchetypeDelegationTool {
    pub tool_name: String,
    /// The agent this tool routes to, in the shape
    /// [`crate::openhuman::tools::traits::delegation_target`] reads back off the
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

    /// Publishes the routing target on the erased host-extension slot, the same
    /// way `UseSkillTool` publishes its pack handle. `traits::delegation_target`
    /// reads it back; every other tool returns `None` and pays nothing.
    fn host_extension(&self) -> Option<&(dyn std::any::Any + Send + Sync)> {
        Some(&self.agent_id)
    }

    /// The delegation envelope — deliberately description-light.
    ///
    /// This one literal is emitted for **every** synthesised `delegate_*` tool
    /// (19 of them on the Master Agent after tool-pack withholding), so each
    /// word of `description` here is billed 19× on every single turn. Fully
    /// described the envelope was 356 tokens × 19 = 6,764 tokens — 39% of the
    /// orchestrator's whole tool-schema budget, for the same JSON 19 times.
    ///
    /// The field *semantics* now live once in the parent's system prompt
    /// (`registry/agents/orchestrator/prompt.md`, "Structured handoffs"),
    /// which is where policy like "only observed facts" belonged anyway. The
    /// property names stay self-describing, and they are the only thing
    /// `render_structured_handoff` below reads.
    ///
    /// Four descriptions survive, each well under the 50-token cap, because
    /// their property name does not carry the meaning:
    ///
    /// * `blocking` — the default is behaviour-critical and not inferable from
    ///   the name. Getting it wrong is silent and asymmetric: async when it
    ///   should have blocked finalizes the turn before the result lands, the
    ///   exact failure the prompt's result-gating rule exists to prevent.
    /// * `evidence` — "actually observed" is the anti-fabrication contract,
    ///   not a label.
    /// * `citation_requirement` / `model` — a bare name reads as neither.
    ///
    /// Enforced by `envelope_descriptions_stay_within_budget` below. If you
    /// are about to add a description here, put it in prompt.md instead.
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["prompt"],
            "properties": {
                "prompt": { "type": "string" },
                "objective": { "type": "string" },
                "evidence": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Only facts, paths, URLs, ids or tool outputs you actually observed."
                },
                "constraints": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "must_not_assume": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "expected_output": { "type": "string" },
                "citation_requirement": {
                    "type": "string",
                    "enum": ["none", "file_paths", "urls", "retrieval_hits", "tool_outputs"],
                    "description": "Evidence style the child must preserve in its result."
                },
                "model": {
                    "type": "string",
                    "description": "Pin the child to this exact model id. Omit unless you have a reason."
                },
                "blocking": {
                    "type": "boolean",
                    "description": "Default false: async worker, result arrives as a later turn. true: waits, and the result gates this reply."
                }
            }
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
    /// fan-out and the long-running scripting tools (`shell`, `node_exec`).
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
        let raw_prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        if raw_prompt.is_empty() {
            return Ok(ToolResult::error(format!(
                "{}: `prompt` is required",
                self.tool_name
            )));
        }
        let prompt = render_structured_handoff(&raw_prompt, &args);

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

        super::dispatch_subagent(
            &self.agent_id.0,
            &self.tool_name,
            &prompt,
            None,
            model_override,
            tool_context,
            mode,
        )
        .await
    }
}

fn render_structured_handoff(prompt: &str, args: &Value) -> String {
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
