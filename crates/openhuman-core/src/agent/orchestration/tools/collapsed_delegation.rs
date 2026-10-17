//! `delegate_to` — every archetype hand-off as one action-dispatched tool.
//!
//! Replaces the per-sub-agent fan-out where `collect_orchestrator_tools`
//! synthesised one [`ArchetypeDelegationTool`] per named sub-agent. On the
//! Master Agent that was **16 tools worth 17,746 bytes, 41% of its whole
//! tool-schema budget** — and the schemas were not 16 different things. Every
//! one of them carried a byte-identical copy of the delegation envelope
//! (`prompt` / `objective` / `evidence` / `constraints` / `must_not_assume` /
//! `expected_output` / `citation_requirement` / `model` / `blocking`), because
//! `ArchetypeDelegationTool::parameters_schema` is one `json!` literal that
//! does not read `self`. The only thing that differed between the 16 was the
//! name and the target's `when_to_use` line.
//!
//! So the envelope is emitted once here and the 16 names become an `agent`
//! enum, with each target's `when_to_use` kept verbatim in the description —
//! the routing information survives in full, the repetition does not.
//!
//! The integration axis went a step further: connected Composio actions are
//! `Deferred` tools reached through `tool_search` and called directly, so
//! that axis has no delegation tool at all (`orchestrator_tools`). This one
//! makes the sub-agent axis constant in the sub-agent dimension.
//!
//! # Why collapse rather than pack
//!
//! The toolpack mechanism (`load_skill` / `use_skill`) exists and would also
//! remove these bytes, but it is the wrong tool for this family. A pack costs
//! a round trip on first use, which is the right trade for a capability most
//! turns never touch — crypto, MCP setup, the `.pptx` writer. Delegation is
//! the orchestrator's *job*; putting a round trip in front of it would tax the
//! single most common thing it does, on almost every turn.
//!
//! Collapsing has the opposite cost profile: one extra enum field on a call
//! the model was making anyway, and no round trip at all. Frequency of use is
//! what separates the two mechanisms — see `toolpacks::registry`, whose
//! `DELIBERATELY_UNPACKED_FLEET_TOOLS` note draws the same line for the same
//! reason.
//!
//! # The members stay registered
//!
//! Each `delegate_*` / `research` / `plan` / … tool remains in the registry as
//! [`ToolExposure::Hidden`], exactly like the members of the collapsed `cron`
//! and `memory` tools. They are off the wire, not gone: a replayed transcript,
//! a saved skill, or a flow node that names `research` still resolves. Only
//! the advertised surface shrinks.
//!
//! # The name
//!
//! `delegate_to`, not `delegate`: a config-driven [`DelegateTool`] already
//! claims `delegate` whenever a user hand-writes an `[agents]` block, and the
//! builder's collision guard resolves a clash by dropping the *synthesised*
//! tool. Naming this one `delegate` would therefore have removed the
//! orchestrator's entire delegation surface for exactly those users, silently.
//!
//! [`DelegateTool`]: crate::agent::tools::DelegateTool
//! [`ArchetypeDelegationTool`]: super::ArchetypeDelegationTool
//! [`ToolExposure::Hidden`]: tinytools::ToolExposure::Hidden

use async_trait::async_trait;
use serde_json::{json, Value};

use super::archetype_delegation::{delegation_envelope_properties, render_structured_handoff};
use tinytools::ToolRunContext;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolResult, ToolTimeout};

/// The advertised name. A constant so the synthesis site, the prompt's
/// delegation section and the tests cannot disagree about it.
pub const DELEGATE_TO_TOOL_NAME: &str = "delegate_to";

/// JSON-Schema extension that carries the exact selector-to-agent mapping the
/// collapsed tool was built with. The harness registers tools behind `dyn
/// Tool`, so this keeps the concrete tool's routing table alongside its
/// advertised enum without a downcast or a later lookup against every global
/// definition.
pub(crate) const DISPATCH_TARGETS_SCHEMA_KEY: &str = "x-openhuman-delegation-targets";

/// One routable sub-agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DelegateTarget {
    /// The name this target had when it was its own tool, and the value the
    /// `agent` enum takes. Keeping the old name as the enum value is what lets
    /// the orchestrator prompt go on naming `research` and `schedule_task`
    /// without a rewrite, and keeps dispatch events reading as they did.
    pub tool_name: String,
    /// The registry id the work is actually handed to.
    pub agent_id: String,
    /// The target's `when_to_use`, verbatim.
    pub description: String,
}

/// Every archetype hand-off as one tool.
pub struct CollapsedDelegationTool {
    targets: Vec<DelegateTarget>,
    description: String,
}

impl CollapsedDelegationTool {
    /// Build the collapsed tool, or `None` when there is nothing to route to.
    ///
    /// `None` rather than an empty enum: a `delegate` tool whose `agent` has no
    /// valid value is a schema the model can only call wrongly.
    pub fn for_targets(targets: Vec<DelegateTarget>) -> Option<Self> {
        if targets.is_empty() {
            return None;
        }
        let description = build_description(&targets);
        Some(Self {
            targets,
            description,
        })
    }

    fn resolve(&self, agent: &str) -> Option<&DelegateTarget> {
        self.targets.iter().find(|t| t.tool_name == agent)
    }

    fn agent_enum(&self) -> Vec<&str> {
        self.targets.iter().map(|t| t.tool_name.as_str()).collect()
    }

    fn dispatch_targets_schema(&self) -> Value {
        Value::Array(
            self.targets
                .iter()
                .map(|target| {
                    json!({
                        "tool_name": target.tool_name,
                        "agent_id": target.agent_id,
                    })
                })
                .collect(),
        )
    }
}

fn build_description(targets: &[DelegateTarget]) -> String {
    let mut buf = String::from(
        "Hand a task to a specialist sub-agent. Set `agent` to one of the values below and pass \
         the task as `prompt`. Choose by what the task needs:",
    );
    for target in targets {
        buf.push_str("\n- `");
        buf.push_str(&prompt_safe(&target.tool_name));
        buf.push('`');
        let trimmed = target.description.trim();
        if !trimmed.is_empty() {
            buf.push_str(": ");
            buf.push_str(&prompt_safe(trimmed));
        }
    }
    buf
}

fn prompt_safe(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '`' => out.push('\''),
            ch if ch.is_control() => out.push_str(&format!("\\u{{{:x}}}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

#[async_trait]
impl Tool for CollapsedDelegationTool {
    fn name(&self) -> &str {
        DELEGATE_TO_TOOL_NAME
    }

    fn description(&self) -> &str {
        &self.description
    }

    /// The envelope, emitted **once**, plus the `agent` selector.
    ///
    /// The properties come from `delegation_envelope_properties` rather than a
    /// second literal: two copies of this object would be two places for the
    /// collapsed tool and its hidden members to disagree about what a hand-off
    /// carries, and `render_structured_handoff` reads the property names
    /// directly. One definition, both callers.
    fn parameters_schema(&self) -> Value {
        let mut schema = json!({
            "type": "object",
            "required": ["agent", "prompt"],
            "properties": {
                "agent": {
                    "type": "string",
                    "enum": self.agent_enum(),
                    "description": "Which specialist to hand this to."
                }
            }
        });
        schema[DISPATCH_TARGETS_SCHEMA_KEY] = self.dispatch_targets_schema();
        let properties = schema["properties"]
            .as_object_mut()
            .expect("properties is an object literal above");
        if let Value::Object(envelope) = delegation_envelope_properties() {
            for (key, value) in envelope {
                properties.insert(key, value);
            }
        }
        schema
    }

    fn permission_level(&self) -> PermissionLevel {
        // Every member declares `Execute`, so there is no per-action variation
        // to resolve here. If a target ever needs more, this must become an
        // args-aware lookup like `cron`'s — a single level would then be
        // laundering one target's risk down to another's.
        PermissionLevel::Execute
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    /// Unbounded, matching the member tools this replaces.
    ///
    /// Under the default `Inherit` policy the whole delegation is hard-killed
    /// at the single-tool timeout (120s), truncating any sub-agent run that
    /// legitimately takes longer — the Sentry regression (TAURI-RUST-K29,
    /// TAURI-RUST-8HB) that put `Unbounded` on `ArchetypeDelegationTool` in the
    /// first place. The child bounds its own lifetime through `max_iterations`,
    /// the run cancellation token and each inner tool's own timeout.
    fn timeout_policy(&self, _args: &Value) -> ToolTimeout {
        ToolTimeout::Unbounded
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: ToolCallOptions,
        tool_context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let mut run_context = crate::agent::tinyagents::host::OpenHumanRunContext::new().with_core(
            crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(tool_context)
                .into_core(),
        );
        run_context.thread_id = tool_context
            .and_then(ToolRunContext::thread_id)
            .map(ToOwned::to_owned);
        execute_collapsed_delegation(&self.targets, args, tool_context, run_context).await
    }
}

/// Recover the concrete collapsed target table retained in the advertised
/// schema for typed harness registration. Reject malformed metadata instead
/// of widening the route to every entry in the process-wide definition
/// registry.
pub(crate) fn dispatch_targets_from_schema(schema: &Value) -> Result<Vec<DelegateTarget>, String> {
    let enum_names = schema
        .pointer("/properties/agent/enum")
        .and_then(Value::as_array)
        .ok_or_else(|| "delegate_to schema is missing `properties.agent.enum`".to_string())?;
    let enum_names: Vec<&str> = enum_names
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| "delegate_to agent enum contains a non-string value".to_string())
        })
        .collect::<Result<_, _>>()?;
    let encoded_targets = schema
        .get(DISPATCH_TARGETS_SCHEMA_KEY)
        .and_then(Value::as_array)
        .ok_or_else(|| "delegate_to schema is missing its target mapping".to_string())?;
    let targets: Vec<DelegateTarget> = encoded_targets
        .iter()
        .map(|value| {
            let object = value
                .as_object()
                .ok_or_else(|| "delegate_to target mapping entry is not an object".to_string())?;
            let required = |key: &str| {
                object
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| format!("delegate_to target mapping is missing `{key}`"))
            };
            Ok(DelegateTarget {
                tool_name: required("tool_name")?,
                agent_id: required("agent_id")?,
                // Descriptions are prompt-only routing guidance. The typed
                // dispatch needs the exact selector-to-agent mapping, not a
                // second copy of model-visible prose.
                description: String::new(),
            })
        })
        .collect::<Result<_, String>>()?;
    if targets.len() != enum_names.len()
        || targets
            .iter()
            .map(|target| target.tool_name.as_str())
            .ne(enum_names.iter().copied())
        || targets.iter().any(|target| {
            targets
                .iter()
                .filter(|candidate| candidate.tool_name == target.tool_name)
                .count()
                != 1
        })
    {
        return Err(
            "delegate_to target mapping does not exactly match its advertised agent enum".into(),
        );
    }
    Ok(targets)
}

/// Execute a collapsed hand-off with an explicit child run carrier.
pub(crate) async fn execute_collapsed_delegation(
    targets: &[DelegateTarget],
    args: Value,
    tool_context: Option<&dyn ToolRunContext>,
    run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
) -> anyhow::Result<ToolResult> {
    execute_collapsed_delegation_with_live_parent(targets, args, tool_context, run_context, None)
        .await
}

/// Typed-harness counterpart that preserves the exact parent run for a
/// blocking child rather than recreating a synthetic root from host data.
pub(crate) async fn execute_collapsed_delegation_with_live_parent(
    targets: &[DelegateTarget],
    args: Value,
    tool_context: Option<&dyn ToolRunContext>,
    run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
    live_parent: Option<
        &tinyagents_harness::context::RunContext<
            crate::agent::tinyagents::host::OpenHumanRunContext,
        >,
    >,
) -> anyhow::Result<ToolResult> {
    let requested = args.get("agent").and_then(Value::as_str).map(str::trim);
    let Some(target) =
        requested.and_then(|agent| targets.iter().find(|target| target.tool_name == agent))
    else {
        return Ok(ToolResult::error(format!(
            "`agent` must be one of: {}. Got: {}",
            targets
                .iter()
                .map(|target| target.tool_name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            requested.filter(|s| !s.is_empty()).unwrap_or("(missing)")
        )));
    };

    let raw_prompt = args
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if raw_prompt.is_empty() {
        return Ok(ToolResult::error(format!(
            "{DELEGATE_TO_TOOL_NAME}: `prompt` is required"
        )));
    }
    let prompt = render_structured_handoff(&raw_prompt, &args);
    let prompt = match crate::agent::attachments::delegation_prompt(
        &run_context.dispatch_ctx(),
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
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    // Async by default, exactly as the member tools were: the specialist
    // runs as a durable, resumable worker and its result arrives as a new
    // chat turn. `blocking: true` is the opt-in for a result that must gate
    // this reply.
    let blocking = args
        .get("blocking")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mode = if blocking {
        super::dispatch::DispatchMode::Blocking
    } else {
        super::dispatch::DispatchMode::PreferAsync
    };

    tracing::debug!(
        agent = %target.agent_id,
        via = %target.tool_name,
        "[delegate] dispatch"
    );
    // `target.tool_name`, not `DELEGATE_TO_TOOL_NAME`: the dispatch name rides
    // into run records and the UI, and reporting every hand-off as
    // `delegate` would erase which specialist was chosen from every trace.
    super::dispatch::dispatch_subagent_with_live_parent(
        &target.agent_id,
        &target.tool_name,
        &prompt,
        model_override,
        tool_context,
        mode,
        run_context,
        live_parent,
    )
    .await
}

#[cfg(test)]
#[path = "collapsed_delegation_tests.rs"]
mod tests;
