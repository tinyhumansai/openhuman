//! Dynamic orchestrator tool generation.
//!
//! Instead of a single `spawn_subagent` mega-tool, this module generates
//! one tool per subagent archetype + one tool per installed skill. The
//! orchestrator's function-calling schema becomes a flat list of
//! well-named tools:
//!
//!   `notion`, `gmail`, `research`, `run_code`, `review_code`, `plan`
//!
//! Each tool's `execute()` internally calls `run_subagent` with the
//! correct definition + skill_filter. The LLM just picks the right tool
//! by name — no `agent_id` or `skill_filter` to remember.

use async_trait::async_trait;
use serde_json::json;
use std::collections::HashSet;

use super::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{run_subagent, SubagentRunOptions};
use crate::openhuman::event_bus::{publish_global, DomainEvent};
use crate::openhuman::tools::spawn_subagent::SpawnSubagentTool;

// ─────────────────────────────────────────────────────────────────────────────
// Skill-based orchestrator tools (one per installed skill)
// ─────────────────────────────────────────────────────────────────────────────

/// A tool that delegates to `skills_agent` with a fixed `skill_filter`.
/// The orchestrator sees this as e.g. `notion` or `gmail`.
struct SkillDelegationTool {
    /// Tool name the LLM calls (e.g. "notion", "gmail").
    tool_name: String,
    /// Skill id for `skill_filter` (same as tool_name usually).
    skill_id: String,
    /// Human-readable description for the LLM.
    tool_description: String,
}

#[async_trait]
impl Tool for SkillDelegationTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["prompt"],
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Clear instruction for what to do. Include all relevant context — the sub-agent has no memory of your conversation."
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

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        if prompt.is_empty() {
            return Ok(ToolResult::error(format!(
                "{}: `prompt` is required",
                self.tool_name
            )));
        }

        dispatch_subagent("skills_agent", &self.tool_name, &prompt, Some(&self.skill_id)).await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Archetype-based orchestrator tools (research, run_code, etc.)
// ─────────────────────────────────────────────────────────────────────────────

/// A tool that delegates to a fixed agent archetype.
struct ArchetypeDelegationTool {
    /// Tool name the LLM calls (e.g. "research", "run_code").
    tool_name: String,
    /// Agent id from the definition registry.
    agent_id: String,
    /// Human-readable description for the LLM.
    tool_description: String,
}

#[async_trait]
impl Tool for ArchetypeDelegationTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["prompt"],
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Clear instruction for what to do. Include all relevant context — the sub-agent has no memory of your conversation."
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

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        if prompt.is_empty() {
            return Ok(ToolResult::error(format!(
                "{}: `prompt` is required",
                self.tool_name
            )));
        }

        dispatch_subagent(&self.agent_id, &self.tool_name, &prompt, None).await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared dispatch logic
// ─────────────────────────────────────────────────────────────────────────────

async fn dispatch_subagent(
    agent_id: &str,
    tool_name: &str,
    prompt: &str,
    skill_filter: Option<&str>,
) -> anyhow::Result<ToolResult> {
    let registry = match AgentDefinitionRegistry::global() {
        Some(reg) => reg,
        None => {
            return Ok(ToolResult::error(
                "Agent registry not initialised. This usually means the \
                 core process started without calling \
                 AgentDefinitionRegistry::init_global at startup.",
            ));
        }
    };

    let definition = match registry.get(agent_id) {
        Some(def) => def,
        None => {
            return Ok(ToolResult::error(format!(
                "{tool_name}: agent '{agent_id}' not found in registry"
            )));
        }
    };

    // Validate skill filter if set.
    if let Some(skill) = skill_filter {
        if let Err(err) = crate::openhuman::tools::spawn_subagent::validate_skill_filter_public(skill) {
            return Ok(ToolResult::error(err));
        }
    }

    let parent_session = current_parent()
        .map(|p| p.session_id.clone())
        .unwrap_or_else(|| "standalone".into());
    let task_id = format!("sub-{}", uuid::Uuid::new_v4());

    publish_global(DomainEvent::SubagentSpawned {
        parent_session: parent_session.clone(),
        agent_id: definition.id.clone(),
        mode: "typed".to_string(),
        task_id: task_id.clone(),
        prompt_chars: prompt.chars().count(),
    });

    log::info!(
        "[agent] delegating to {} via {} prompt_chars={}",
        agent_id,
        tool_name,
        prompt.chars().count()
    );

    let options = SubagentRunOptions {
        skill_filter_override: skill_filter.map(|s| s.to_string()),
        category_filter_override: None,
        context: None, // memory context is auto-injected by the runner
        task_id: Some(task_id.clone()),
    };

    match run_subagent(definition, prompt, options).await {
        Ok(outcome) => {
            publish_global(DomainEvent::SubagentCompleted {
                parent_session,
                task_id: outcome.task_id.clone(),
                agent_id: outcome.agent_id.clone(),
                elapsed_ms: outcome.elapsed.as_millis() as u64,
                output_chars: outcome.output.chars().count(),
                iterations: outcome.iterations,
            });
            log::info!(
                "[agent] {} completed via {} iterations={} output_chars={}",
                agent_id,
                tool_name,
                outcome.iterations,
                outcome.output.chars().count()
            );
            Ok(ToolResult::success(outcome.output))
        }
        Err(err) => {
            let message = err.to_string();
            publish_global(DomainEvent::SubagentFailed {
                parent_session,
                task_id,
                agent_id: definition.id.clone(),
                error: message.clone(),
            });
            Ok(ToolResult::error(format!(
                "{tool_name} failed: {message}"
            )))
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API: generate orchestrator tools
// ─────────────────────────────────────────────────────────────────────────────

/// Static archetype tools the orchestrator always gets.
const ARCHETYPE_TOOLS: &[(&str, &str, &str)] = &[
    (
        "research",
        "researcher",
        "Search the web, read docs, and gather information. Returns a dense markdown summary with sources.",
    ),
    (
        "run_code",
        "code_executor",
        "Write, run, debug, and test code in a sandboxed environment. Has shell, file access, and git.",
    ),
    (
        "review_code",
        "critic",
        "Review code changes for quality, security, and correctness. Read-only — returns findings, never edits.",
    ),
    (
        "plan",
        "planner",
        "Break a complex goal into a structured step-by-step plan with dependencies. Use for tasks with 3+ steps.",
    ),
];

/// Skill name → description mapping for known skills.
/// Falls back to a generic description for unknown skills.
fn skill_description(skill_id: &str) -> String {
    match skill_id {
        "notion" => "Interact with Notion: search pages, create and update pages and databases, manage blocks and comments.".into(),
        "gmail" => "Interact with Gmail: read emails, send messages, search inbox, manage labels.".into(),
        "slack" => "Interact with Slack: send messages, read channels, manage conversations.".into(),
        "google-calendar" | "calendar" => "Interact with Google Calendar: view events, create meetings, manage schedules.".into(),
        "google-drive" | "drive" => "Interact with Google Drive: manage files, folders, and sharing.".into(),
        "github" => "Interact with GitHub: manage repos, issues, pull requests, and code.".into(),
        _ => format!("Interact with the {skill_id} integration."),
    }
}

/// Build the orchestrator's tool list: one tool per installed skill +
/// one tool per archetype. Also includes `spawn_subagent` as a fallback
/// for advanced use cases (fork mode, custom agent_ids).
///
/// Call this at agent build time when the visible-tool filter is active
/// (i.e. the main agent is an orchestrator).
pub fn collect_orchestrator_tools() -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    let mut seen_names: HashSet<String> = HashSet::new();

    // ── Skill-based tools (dynamic, from running skills) ──────────────
    if let Some(engine) = crate::openhuman::skills::global_engine() {
        let all_skill_tools = engine.all_tools();
        // Extract unique skill ids from tool names like "notion__search"
        let mut skill_ids: Vec<String> = all_skill_tools
            .iter()
            .filter_map(|(skill_id, _)| {
                if seen_names.insert(skill_id.clone()) {
                    Some(skill_id.clone())
                } else {
                    None
                }
            })
            .collect();
        skill_ids.sort();
        skill_ids.dedup();

        for skill_id in &skill_ids {
            log::info!(
                "[orchestrator_tools] registering skill delegation tool: {}",
                skill_id
            );
            tools.push(Box::new(SkillDelegationTool {
                tool_name: skill_id.clone(),
                skill_id: skill_id.clone(),
                tool_description: skill_description(skill_id),
            }));
        }
    }

    // ── Archetype-based tools (static) ────────────────────────────────
    for (tool_name, agent_id, description) in ARCHETYPE_TOOLS {
        log::info!(
            "[orchestrator_tools] registering archetype delegation tool: {} -> {}",
            tool_name,
            agent_id
        );
        tools.push(Box::new(ArchetypeDelegationTool {
            tool_name: tool_name.to_string(),
            agent_id: agent_id.to_string(),
            tool_description: description.to_string(),
        }));
    }

    // ── spawn_subagent as fallback for advanced use ────────────────────
    // Fork mode, custom agent_ids, explicit skill_filter, etc.
    tools.push(Box::new(SpawnSubagentTool::new()));

    log::info!(
        "[orchestrator_tools] total orchestrator tools: {}",
        tools.len()
    );

    tools
}
