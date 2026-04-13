mod archetype_delegation;
mod ask_clarification;
mod complete_onboarding;
mod delegate;
mod spawn_subagent;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{run_subagent, SubagentRunOptions};
use crate::openhuman::tools::traits::ToolResult;

pub(crate) const ARCHETYPE_TOOLS: &[(&str, &str, &str)] = &[
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

pub(crate) async fn dispatch_subagent(
    agent_id: &str,
    tool_name: &str,
    prompt: &str,
    _skill_filter: Option<&str>,
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
        skill_filter_override: None,
        category_filter_override: None,
        context: None,
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
            Ok(ToolResult::error(format!("{tool_name} failed: {message}")))
        }
    }
}

pub use archetype_delegation::ArchetypeDelegationTool;
pub use ask_clarification::AskClarificationTool;
pub use complete_onboarding::CompleteOnboardingTool;
pub use delegate::DelegateTool;
pub use spawn_subagent::SpawnSubagentTool;
