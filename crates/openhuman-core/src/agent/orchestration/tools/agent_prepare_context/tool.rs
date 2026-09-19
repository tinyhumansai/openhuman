//! The `agent_prepare_context` [`Tool`] wrapper: schema, permission level,
//! and the parent tool-catalogue / scout-prompt / proposed-goal rendering
//! the scout engine (see [`super::scout_run`]) is built from.

use std::fmt::Write as _;
use std::sync::Arc;

use crate::agent::harness::fork_context::ParentExecutionContext;
use async_trait::async_trait;
use serde_json::json;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::tool::{ToolDispatch, ToolExecutionContext};
use tinytools::ToolRunContext;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult};

use super::scout_run::{
    already_prepared_context_bundle, run_context_scout_with_catalog_and_workspace,
};

/// Spawns the `context_scout` sub-agent to collect context and propose a plan.
pub struct AgentPrepareContextTool;

pub(crate) struct AgentPrepareContextDispatch {
    tool: Arc<dyn Tool>,
}
impl AgentPrepareContextDispatch {
    pub(crate) fn new(tool: Arc<dyn Tool>) -> Self {
        Self { tool }
    }
}
#[async_trait]
impl ToolDispatch<(), crate::agent::tinyagents::host::OpenHumanRunContext>
    for AgentPrepareContextDispatch
{
    fn tool(&self) -> Arc<dyn Tool> {
        self.tool.clone()
    }
    async fn execute(
        &self,
        _state: &(),
        arguments: serde_json::Value,
        _options: ToolCallOptions,
        parent: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let context = ToolExecutionContext::from_run_context(parent);
        AgentPrepareContextTool::new()
            .execute_with_parent_context(arguments, Some(&context), parent.data.child())
            .await
    }
}

impl Default for AgentPrepareContextTool {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentPrepareContextTool {
    pub fn new() -> Self {
        Self
    }

    /// Render the parent agent's tool catalogue into a compact
    /// `- name: description` list the scout can recommend *back* to the
    /// parent. Excludes this tool itself (recommending another scout pass
    /// would be circular). Returns an empty string when there's no parent
    /// context (e.g. a direct CLI/RPC tool call outside an agent turn) — the
    /// subsequent `run_subagent` call surfaces the no-parent error.
    ///
    /// Restricted to the parent's **visible** tool set (what it actually
    /// advertises and will execute this turn), not the full registry —
    /// otherwise the scout could recommend hidden direct-exec/spawn tools
    /// the parent can't call, which the runtime would reject or which would
    /// bypass specialist routing. Read from `visible_tool_specs`, the parent's
    /// own advertised list, because `all_tool_specs` describes what a *child*
    /// may inherit and deliberately carries none of the parent's synthesised
    /// `delegate_*` tools — the very tools the scout most often recommends.
    /// Falls back to that registry, name-filtered, when a context does not
    /// carry the visible list, to preserve behaviour for builders that don't
    /// populate it.
    pub(super) fn render_parent_tool_catalog(parent: Option<&ParentExecutionContext>) -> String {
        let Some(parent) = parent else {
            return String::new();
        };
        let visible = &parent.visible_tool_names;
        let specs: &[std::sync::Arc<tinytools::ToolSpec>] = if parent.visible_tool_specs.is_empty()
        {
            &parent.all_tool_specs
        } else {
            &parent.visible_tool_specs
        };
        let mut out = String::with_capacity(2048);
        for spec in specs.iter() {
            if spec.name == "agent_prepare_context" {
                continue;
            }
            if !visible.is_empty() && !visible.contains(&spec.name) {
                continue;
            }
            // One line per tool; trim the description to keep the catalogue
            // from dwarfing the scout's own prompt.
            let desc: String = spec
                .description
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let desc = if desc.chars().count() > 160 {
                let cut = desc
                    .char_indices()
                    .nth(160)
                    .map(|(i, _)| i)
                    .unwrap_or(desc.len());
                format!("{}…", &desc[..cut])
            } else {
                desc
            };
            let _ = writeln!(out, "- {}: {}", spec.name, desc);
        }
        out
    }

    /// Build the scout's task prompt: the request, optional focus, and the
    /// parent tool catalogue the scout draws its recommendations from.
    pub(super) fn build_scout_prompt(
        question: &str,
        focus: Option<&str>,
        tool_catalog: &str,
    ) -> String {
        let mut prompt = String::with_capacity(question.len() + tool_catalog.len() + 512);
        let _ = writeln!(prompt, "[Request]\n{question}\n");
        if let Some(focus) = focus.filter(|f| !f.trim().is_empty()) {
            let _ = writeln!(prompt, "[Focus]\n{}\n", focus.trim());
        }
        if tool_catalog.trim().is_empty() {
            prompt.push_str(
                "[Orchestrator tools]\n(none available — return an empty \
                 recommended_tool_calls list)\n",
            );
        } else {
            let _ = writeln!(
                prompt,
                "[Orchestrator tools]\nThese are the tools the orchestrator can call next. \
                 Every `recommended_tool_calls[].tool` MUST be one of these exact names:\n{tool_catalog}"
            );
        }
        prompt.push_str(
            "\nGather what you need, then emit the single [context_bundle] … \
             [/context_bundle] block as specified. Do not answer the request yourself.",
        );
        prompt
    }

    /// Extract the scout's `proposed_goal:` line from a `[context_bundle]`, if
    /// present and meaningful. Returns `None` for a missing line or an explicit
    /// `none`. The prefix is matched case-insensitively; its byte length is
    /// fixed (no multibyte), so slicing past it is safe.
    pub(super) fn parse_proposed_goal(bundle: &str) -> Option<String> {
        const PREFIX: &str = "proposed_goal:";
        // Boundary-safe prefix match: `get(..len)` returns None rather than
        // panicking when the line begins with a multibyte char before byte 14.
        let line = bundle.lines().map(str::trim).find(|l| {
            l.get(..PREFIX.len())
                .is_some_and(|p| p.eq_ignore_ascii_case(PREFIX))
        })?;
        let value = line[PREFIX.len()..].trim();
        if value.is_empty() || value.eq_ignore_ascii_case("none") {
            return None;
        }
        Some(value.to_string())
    }
}

#[async_trait]
impl Tool for AgentPrepareContextTool {
    fn name(&self) -> &str {
        "agent_prepare_context"
    }

    fn description(&self) -> &str {
        "Before answering or delegating, scout existing context. Runs a fast \
         read-only context-collector that checks memory, past conversations \
         (transcripts), your goals/profile, installed/registry skills, connected \
         integrations, and the web, then returns whether there's enough context \
         to answer, a compact context summary, an ordered list of recommended \
         next tool calls (parent tools, by exact name, with args), and any \
         skills worth running. Use only when a caller explicitly needs an \
         ad hoc scout pass. If the current prompt says agent context has \
         already been prepared, use the prepared context and do not call this \
         tool again."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["question"],
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The user's request or goal to gather context for. Be specific — the scout has no memory of your conversation."
                },
                "focus": {
                    "type": "string",
                    "description": "Optional hint that narrows what to scout (e.g. a platform, time window, or sub-question)."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // ReadOnly, not Execute: this tool only ever runs the read-only
        // `context_scout` (read_only sandbox, no write/exec tools). Marking it
        // Execute would make `ToolPolicyEngine` strip it from any
        // provider-visible set on a `ReadOnly`-capped channel, which would hide
        // the scout from callers that still expose it explicitly.
        PermissionLevel::ReadOnly
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
        self.execute_with_parent_context(
            args,
            tool_context,
            crate::agent::tinyagents::host::OpenHumanRunContext::new(),
        )
        .await
    }
}

impl AgentPrepareContextTool {
    pub(crate) async fn execute_with_parent_context(
        &self,
        args: serde_json::Value,
        tool_context: Option<&dyn ToolRunContext>,
        run_context: crate::agent::tinyagents::host::OpenHumanRunContext,
    ) -> anyhow::Result<ToolResult> {
        let prepared_sources = run_context.prepared_context_sources.as_ref();
        if !prepared_sources.is_empty() {
            tracing::info!(
                target: "agent_prepare_context",
                sources = ?prepared_sources,
                "[agent_prepare_context] skipped because agent context is already prepared for this turn"
            );
            return Ok(ToolResult::success(already_prepared_context_bundle(
                prepared_sources,
            )));
        }

        let question = args.get("question").and_then(|v| v.as_str()).unwrap_or("");
        let focus = args.get("focus").and_then(|v| v.as_str());
        let tool_catalog =
            AgentPrepareContextTool::render_parent_tool_catalog(run_context.parent.as_ref());
        run_context_scout_with_catalog_and_workspace(
            question,
            focus,
            &tool_catalog,
            tool_context.and_then(|ctx| ctx.workspace().cloned()),
            tool_context
                .and_then(ToolRunContext::thread_id)
                .map(str::to_owned),
            run_context,
        )
        .await
    }
}
