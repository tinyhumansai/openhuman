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
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::fmt::Write as _;

/// The sub-agent archetype this tool drives.
const SCOUT_AGENT_ID: &str = "context_scout";

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
}

#[async_trait]
impl Tool for AgentPrepareContextTool {
    fn name(&self) -> &str {
        "agent_prepare_context"
    }

    fn description(&self) -> &str {
        "Before answering or delegating, scout existing context. Runs a fast \
         read-only context-collector that checks memory, your goals/profile, \
         connected integrations, and the web, then returns whether there's \
         enough context to answer, a compact context summary, and an ordered \
         list of recommended next tool calls (your own tools, by exact name, \
         with args). Use at the start of non-trivial turns."
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
        let question = args
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let focus = args
            .get("focus")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

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

        let tool_catalog = Self::render_parent_tool_catalog();
        let catalog_tool_count = tool_catalog.lines().filter(|l| !l.is_empty()).count();
        let scout_prompt = Self::build_scout_prompt(&question, focus.as_deref(), &tool_catalog);

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

    #[tokio::test]
    async fn missing_question_returns_error() {
        let tool = AgentPrepareContextTool::new();
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("question"));
    }
}
