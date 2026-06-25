//! Tool: `agent_prepare_context` — "plan mode as a subagent".
//!
//! Before answering or delegating a non-trivial request, the parent agent
//! (orchestrator / planner) calls `agent_prepare_context`. This runs the
//! read-only `context_scout` sub-agent inline (blocking), which gathers
//! context from memory, the user's goals/profile, connected integrations, and
//! the web, then returns a tight `[context_bundle]` envelope: whether there's
//! enough context to act, a compact context summary, and an ordered set of
//! recommended next tool calls drawn from the *parent's own* tool catalogue.
//!
//! The scout's output is bounded by `context_scout`'s `max_result_chars`
//! (≈1000 tokens) so the parent's context only grows by a bounded amount.

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{
    run_subagent, SubagentRunOptions, SubagentRunStatus,
};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::inference::provider::thread_context::current_thread_id;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::fmt::Write as _;

/// The sub-agent archetype this tool drives.
const SCOUT_AGENT_ID: &str = "context_scout";

/// Whether `output` is exactly one well-formed `[context_bundle] …
/// [/context_bundle]` envelope and *nothing else*: the trimmed output must
/// start with the open tag, end with the close tag, and contain exactly one of
/// each.
///
/// The harness prepends every non-error `run_context_scout` result to turn 1
/// as "Prepared context", so a prompt drift / model regression in
/// `context_scout` that emits free-form prose (e.g. `Sure, here's what I
/// found:\n[context_bundle]…[/context_bundle]`), or a malformed/duplicated
/// envelope, would otherwise silently inject arbitrary text. We reject those so
/// the caller falls back to the un-augmented message instead. The scout's own
/// contract is "emit the single envelope and nothing outside it".
fn is_well_formed_context_bundle(output: &str) -> bool {
    const OPEN: &str = "[context_bundle]";
    const CLOSE: &str = "[/context_bundle]";
    let trimmed = output.trim();
    // Exactly one open + one close tag, with no text outside the envelope.
    trimmed.starts_with(OPEN)
        && trimmed.ends_with(CLOSE)
        && trimmed.matches(OPEN).count() == 1
        && trimmed.matches(CLOSE).count() == 1
        // Guard the degenerate case where OPEN/CLOSE overlap on too-short input.
        && trimmed.len() >= OPEN.len() + CLOSE.len()
}

/// Run the `context_scout` sub-agent inline (blocking) for `question` and
/// return its bounded `[context_bundle]` envelope as a [`ToolResult`].
///
/// This is the shared engine behind two callers:
///
/// 1. The [`AgentPrepareContextTool`] — invoked *autonomously by the LLM*
///    when it decides to scout context mid-turn.
/// 2. The agent harness itself — when "super context" is enabled it calls
///    this directly on the first turn of a new thread (see
///    [`crate::openhuman::config::ContextConfig::super_context_enabled`]),
///    so the collection happens regardless of the model's decision.
///
/// Must be called from within an active agent turn (i.e. with the
/// [`crate::openhuman::agent::harness::fork_context::PARENT_CONTEXT`]
/// task-local installed) — it reads the parent's visible tool catalogue
/// and runs the scout against the parent's provider. Outside a turn the
/// `run_subagent` call surfaces a no-parent error as a [`ToolResult::error`].
pub async fn run_context_scout(question: &str, focus: Option<&str>) -> anyhow::Result<ToolResult> {
    let question = question.trim().to_string();
    let focus = focus.map(|s| s.to_string());

    tracing::info!(
        target: "agent_prepare_context",
        question_chars = question.chars().count(),
        has_focus = focus.as_deref().map(|f| !f.trim().is_empty()).unwrap_or(false),
        "[agent_prepare_context] invoked"
    );

    if question.is_empty() {
        return Ok(ToolResult::error(
            "agent_prepare_context: `question` is required",
        ));
    }

    let registry = match AgentDefinitionRegistry::global() {
        Some(reg) => reg,
        None => {
            return Ok(ToolResult::error(
                "agent_prepare_context: AgentDefinitionRegistry has not been initialised.",
            ));
        }
    };
    let definition = match registry.get(SCOUT_AGENT_ID) {
        Some(def) => def,
        None => {
            return Ok(ToolResult::error(format!(
                "agent_prepare_context: built-in agent `{SCOUT_AGENT_ID}` is not registered.",
            )));
        }
    };

    let tool_catalog = AgentPrepareContextTool::render_parent_tool_catalog();
    let catalog_tool_count = tool_catalog.lines().filter(|l| !l.is_empty()).count();
    let scout_prompt =
        AgentPrepareContextTool::build_scout_prompt(&question, focus.as_deref(), &tool_catalog);

    tracing::debug!(
        target: "agent_prepare_context",
        catalog_tool_count,
        scout_prompt_chars = scout_prompt.chars().count(),
        "[agent_prepare_context] spawning context_scout (blocking)"
    );

    let task_id = format!("ctx-{}", uuid::Uuid::new_v4());
    let parent_session = current_parent()
        .map(|p| p.session_id.clone())
        .unwrap_or_else(|| "standalone".into());
    let progress_sink = current_parent().and_then(|p| p.on_progress.clone());

    // Surface the scout as a live subagent row in the parent thread. The
    // child's own iterations/tool-calls already stream to this sink from
    // inside run_subagent; we bookend them with spawned/completed so the
    // UI opens and closes the card. Best-effort — a closed sink is fine.
    publish_global(DomainEvent::SubagentSpawned {
        parent_session: parent_session.clone(),
        agent_id: definition.id.clone(),
        mode: "typed".to_string(),
        task_id: task_id.clone(),
        prompt_chars: scout_prompt.chars().count(),
    });
    if let Some(ref tx) = progress_sink {
        let _ = tx
            .send(AgentProgress::SubagentSpawned {
                agent_id: definition.id.clone(),
                task_id: task_id.clone(),
                mode: "typed".to_string(),
                dedicated_thread: false,
                prompt_chars: scout_prompt.chars().count(),
                worker_thread_id: None,
                display_name: Some(definition.display_name().to_string()),
            })
            .await;
    }

    let options = SubagentRunOptions {
        task_id: Some(task_id.clone()),
        ..Default::default()
    };

    match run_subagent(definition, &scout_prompt, options).await {
        Ok(outcome) => match &outcome.status {
            SubagentRunStatus::Completed => {
                // Guard the contract: the scout MUST return exactly one
                // `[context_bundle] … [/context_bundle]` envelope. Reject
                // free-form / malformed output so the harness (which prepends
                // any non-error result to turn 1 as "Prepared context") cannot
                // inject arbitrary prose on a scout prompt drift.
                if !is_well_formed_context_bundle(&outcome.output) {
                    tracing::warn!(
                        target: "agent_prepare_context",
                        task_id = %outcome.task_id,
                        output_chars = outcome.output.chars().count(),
                        "[agent_prepare_context] scout returned a malformed/absent context_bundle — rejecting"
                    );
                    publish_global(DomainEvent::SubagentCompleted {
                        parent_session: parent_session.clone(),
                        task_id: outcome.task_id.clone(),
                        agent_id: outcome.agent_id.clone(),
                        elapsed_ms: outcome.elapsed.as_millis() as u64,
                        output_chars: 0,
                        iterations: outcome.iterations,
                    });
                    if let Some(ref tx) = progress_sink {
                        let _ = tx
                            .send(AgentProgress::SubagentCompleted {
                                agent_id: outcome.agent_id.clone(),
                                task_id: outcome.task_id.clone(),
                                elapsed_ms: outcome.elapsed.as_millis() as u64,
                                iterations: outcome.iterations as u32,
                                output_chars: 0,
                                worktree_path: None,
                                changed_files: Vec::new(),
                                dirty_status: None,
                            })
                            .await;
                    }
                    return Ok(ToolResult::error(
                        "agent_prepare_context: context_scout did not return a well-formed \
                         [context_bundle] envelope",
                    ));
                }
                tracing::info!(
                    target: "agent_prepare_context",
                    task_id = %outcome.task_id,
                    elapsed_ms = outcome.elapsed.as_millis() as u64,
                    iterations = outcome.iterations,
                    output_chars = outcome.output.chars().count(),
                    "[agent_prepare_context] context bundle ready"
                );
                publish_global(DomainEvent::SubagentCompleted {
                    parent_session: parent_session.clone(),
                    task_id: outcome.task_id.clone(),
                    agent_id: outcome.agent_id.clone(),
                    elapsed_ms: outcome.elapsed.as_millis() as u64,
                    output_chars: outcome.output.chars().count(),
                    iterations: outcome.iterations,
                });
                if let Some(ref tx) = progress_sink {
                    let _ = tx
                        .send(AgentProgress::SubagentCompleted {
                            agent_id: outcome.agent_id.clone(),
                            task_id: outcome.task_id.clone(),
                            elapsed_ms: outcome.elapsed.as_millis() as u64,
                            iterations: outcome.iterations as u32,
                            output_chars: outcome.output.chars().count(),
                            worktree_path: None,
                            changed_files: Vec::new(),
                            dirty_status: None,
                        })
                        .await;
                }

                // Bootstrap this thread's goal from the scout's proposal — but
                // ONLY when the thread has none yet. The orchestrator stays
                // authoritative (it sets/replaces via `goal_set`); the
                // context-gathering path just seeds a goal on the first scout of
                // a fresh chat so the harness has something to steer toward.
                // Runs for both entry points (LLM-invoked tool + harness
                // super-context first turn). Best-effort — never fails the call.
                if let (Some(parent), Some(thread_id)) = (current_parent(), current_thread_id()) {
                    if let Some(objective) =
                        AgentPrepareContextTool::parse_proposed_goal(&outcome.output)
                    {
                        match crate::openhuman::thread_goals::store::set_if_absent(
                            &parent.workspace_dir,
                            &thread_id,
                            &objective,
                            None,
                        )
                        .await
                        {
                            Ok(Some(goal)) => {
                                tracing::info!(
                                    target: "agent_prepare_context",
                                    thread_id = %thread_id,
                                    goal_id = %goal.goal_id,
                                    "[agent_prepare_context] bootstrapped thread goal from scout proposal"
                                );
                                publish_global(DomainEvent::ThreadGoalUpdated {
                                    thread_id: goal.thread_id.clone(),
                                    goal_id: goal.goal_id.clone(),
                                    status: goal.status.as_str().to_string(),
                                });
                            }
                            Ok(None) => {
                                tracing::debug!(
                                    target: "agent_prepare_context",
                                    thread_id = %thread_id,
                                    "[agent_prepare_context] thread already has a goal — scout proposal not applied"
                                );
                            }
                            Err(e) => {
                                tracing::debug!(
                                    target: "agent_prepare_context",
                                    error = %e,
                                    "[agent_prepare_context] failed to persist scout-proposed goal"
                                );
                            }
                        }
                    }
                }

                Ok(ToolResult::success(outcome.output))
            }
            // The scout has no `ask_user_clarification` tool, so this
            // branch should not fire — handle defensively rather than
            // leaking a confusing checkpoint envelope to the parent.
            SubagentRunStatus::AwaitingUser { question, .. } => {
                tracing::warn!(
                    target: "agent_prepare_context",
                    task_id = %outcome.task_id,
                    "[agent_prepare_context] scout unexpectedly awaited user input"
                );
                // Close the domain-event lifecycle too — a SubagentSpawned
                // was already published, so emit Completed to avoid a
                // dangling spawned state for event-bus consumers.
                publish_global(DomainEvent::SubagentCompleted {
                    parent_session: parent_session.clone(),
                    task_id: outcome.task_id.clone(),
                    agent_id: outcome.agent_id.clone(),
                    elapsed_ms: outcome.elapsed.as_millis() as u64,
                    output_chars: 0,
                    iterations: outcome.iterations,
                });
                if let Some(ref tx) = progress_sink {
                    let _ = tx
                        .send(AgentProgress::SubagentCompleted {
                            agent_id: outcome.agent_id.clone(),
                            task_id: outcome.task_id.clone(),
                            elapsed_ms: outcome.elapsed.as_millis() as u64,
                            iterations: outcome.iterations as u32,
                            output_chars: 0,
                            worktree_path: None,
                            changed_files: Vec::new(),
                            dirty_status: None,
                        })
                        .await;
                }
                Ok(ToolResult::success(format!(
                    "[context_bundle]\nhas_enough_context: false\n\
                     summary: The context scout could not complete without clarification: {question}\n\
                     recommended_tool_calls:\n[/context_bundle]"
                )))
            }
        },
        Err(err) => {
            let message = err.to_string();
            let error_kind = message
                .split(':')
                .next()
                .map(str::trim)
                .unwrap_or("unknown");
            tracing::error!(
                target: "agent_prepare_context",
                error_kind = %error_kind,
                "[agent_prepare_context] context_scout run failed"
            );
            publish_global(DomainEvent::SubagentFailed {
                parent_session: parent_session.clone(),
                task_id: task_id.clone(),
                agent_id: definition.id.clone(),
                error: message.clone(),
            });
            if let Some(ref tx) = progress_sink {
                let _ = tx
                    .send(AgentProgress::SubagentFailed {
                        agent_id: definition.id.clone(),
                        task_id: task_id.clone(),
                        error: message.clone(),
                    })
                    .await;
            }
            Ok(ToolResult::error(format!(
                "agent_prepare_context failed: {message}"
            )))
        }
    }
}

/// Spawns the `context_scout` sub-agent to collect context and propose a plan.
pub struct AgentPrepareContextTool;

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
    /// bypass specialist routing. Falls back to the full registry only when
    /// the visible set is unknown (empty), to preserve behaviour in contexts
    /// that don't populate it.
    fn render_parent_tool_catalog() -> String {
        let Some(parent) = current_parent() else {
            return String::new();
        };
        let visible = &parent.visible_tool_names;
        let mut out = String::with_capacity(2048);
        for spec in parent.all_tool_specs.iter() {
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
    fn build_scout_prompt(question: &str, focus: Option<&str>, tool_catalog: &str) -> String {
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
    fn parse_proposed_goal(bundle: &str) -> Option<String> {
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
         next tool calls (your own tools, by exact name, with args), and any \
         skills worth running. Use at the start of non-trivial turns."
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
        // Execute would make `ToolPolicyEngine` strip it from the provider-
        // visible set on a `ReadOnly`-capped channel, which would hide the
        // orchestrator's mandatory first-turn context-prep call and either
        // skip the pass or surface an unavailable-tool error.
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let question = args.get("question").and_then(|v| v.as_str()).unwrap_or("");
        let focus = args.get("focus").and_then(|v| v.as_str());
        run_context_scout(question, focus).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_requires_question_and_makes_focus_optional() {
        let tool = AgentPrepareContextTool::new();
        let schema = tool.parameters_schema();
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("schema has required array");
        assert!(required.iter().any(|v| v.as_str() == Some("question")));
        assert!(
            required.iter().all(|v| v.as_str() != Some("focus")),
            "focus must be optional"
        );
        let props = schema.get("properties").expect("schema has properties");
        assert!(props.get("question").is_some());
        assert!(props.get("focus").is_some());
    }

    #[test]
    fn build_scout_prompt_includes_request_focus_and_catalog() {
        let prompt = AgentPrepareContextTool::build_scout_prompt(
            "summarise my unread gmail",
            Some("last 24h"),
            "- delegate_to_integrations_agent: route to a connected integration\n",
        );
        assert!(prompt.contains("[Request]"));
        assert!(prompt.contains("summarise my unread gmail"));
        assert!(prompt.contains("[Focus]"));
        assert!(prompt.contains("last 24h"));
        assert!(prompt.contains("[Orchestrator tools]"));
        assert!(prompt.contains("delegate_to_integrations_agent"));
        assert!(prompt.contains("[context_bundle]"));
    }

    #[test]
    fn build_scout_prompt_handles_empty_catalog() {
        let prompt = AgentPrepareContextTool::build_scout_prompt("do a thing", None, "");
        assert!(prompt.contains("(none available"));
        assert!(!prompt.contains("[Focus]"));
    }

    #[test]
    fn parse_proposed_goal_extracts_objective_or_none() {
        let bundle = "[context_bundle]\nhas_enough_context: true\n\
                      proposed_goal: Ship the desktop release to all platforms\n\
                      summary: ...\n[/context_bundle]";
        assert_eq!(
            AgentPrepareContextTool::parse_proposed_goal(bundle).as_deref(),
            Some("Ship the desktop release to all platforms")
        );

        // Explicit `none` → no goal.
        let none_bundle = "[context_bundle]\nproposed_goal: none\nsummary: x\n[/context_bundle]";
        assert!(AgentPrepareContextTool::parse_proposed_goal(none_bundle).is_none());

        // Missing line → no goal.
        let no_line = "[context_bundle]\nhas_enough_context: true\n[/context_bundle]";
        assert!(AgentPrepareContextTool::parse_proposed_goal(no_line).is_none());

        // Case-insensitive prefix.
        let cased = "Proposed_Goal:  Land the migration  ";
        assert_eq!(
            AgentPrepareContextTool::parse_proposed_goal(cased).as_deref(),
            Some("Land the migration")
        );

        // Lines starting with a multibyte char must not panic the byte-prefix
        // match (regression for the `l[..14]` non-boundary slice).
        let multibyte = "[context_bundle]\n日本語の要約 summary line\nproposed_goal: 目標を達成する\n[/context_bundle]";
        assert_eq!(
            AgentPrepareContextTool::parse_proposed_goal(multibyte).as_deref(),
            Some("目標を達成する")
        );
    }

    #[tokio::test]
    async fn missing_question_returns_error() {
        let tool = AgentPrepareContextTool::new();
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("question"));
    }

    #[test]
    fn accepts_a_single_well_formed_bundle() {
        let out = "[context_bundle]\nhas_enough_context: true\nsummary: ok\n[/context_bundle]";
        assert!(is_well_formed_context_bundle(out));
    }

    #[test]
    fn rejects_free_form_prose_without_a_bundle() {
        assert!(!is_well_formed_context_bundle(
            "Sure! Here's what I found about your request..."
        ));
    }

    #[test]
    fn rejects_unterminated_or_reversed_envelope() {
        assert!(!is_well_formed_context_bundle(
            "[context_bundle]\nsummary: ..."
        ));
        assert!(!is_well_formed_context_bundle(
            "[/context_bundle] stray [context_bundle]"
        ));
    }

    #[test]
    fn rejects_duplicated_envelope() {
        assert!(!is_well_formed_context_bundle(
            "[context_bundle]a[/context_bundle][context_bundle]b[/context_bundle]"
        ));
    }

    #[test]
    fn rejects_prose_around_the_envelope() {
        // Leading prose before the bundle.
        assert!(!is_well_formed_context_bundle(
            "Sure, here's what I found:\n[context_bundle]\nsummary: x\n[/context_bundle]"
        ));
        // Trailing prose after the bundle.
        assert!(!is_well_formed_context_bundle(
            "[context_bundle]\nsummary: x\n[/context_bundle]\nHope that helps!"
        ));
    }

    #[test]
    fn accepts_envelope_with_surrounding_whitespace() {
        // Leading/trailing whitespace is trimmed, not treated as prose.
        assert!(is_well_formed_context_bundle(
            "\n  [context_bundle]\nsummary: x\n[/context_bundle]\n  "
        ));
    }
}
