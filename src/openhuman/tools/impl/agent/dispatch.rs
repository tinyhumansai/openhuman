//! Subagent dispatch logic shared by all agent delegation tools.

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{run_subagent, SubagentRunOptions};
use crate::openhuman::tools::traits::ToolResult;

pub(crate) async fn dispatch_subagent(
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
        "[agent] delegating to {} via {} (skill_filter={}) prompt_chars={}",
        agent_id,
        tool_name,
        skill_filter.unwrap_or("<none>"),
        prompt.chars().count()
    );

    // Propagate the per-call toolkit scope into the subagent runner so
    // that `SkillDelegationTool`s can narrow `integrations_agent` to a single
    // Composio toolkit (e.g. `delegate_gmail` → integrations_agent +
    // toolkit="gmail"). Earlier code plumbed this through
    // `skill_filter_override` (which matches `{skill}__` QuickJS-style
    // names), but Composio actions are named `GMAIL_*` / `NOTION_*` —
    // so the filter excluded every Composio tool instead of narrowing
    // them. `toolkit_override` applies the correct `{TOOLKIT}_` prefix
    // check, restricted to skill-category tools.
    let options = SubagentRunOptions {
        skill_filter_override: None,
        toolkit_override: skill_filter.map(str::to_string),
        context: None,
        task_id: Some(task_id.clone()),
        worker_thread_id: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::Tool;

    use super::super::AskClarificationTool;

    #[test]
    fn ask_clarification_tool_re_exported() {
        let tool = AskClarificationTool::new();
        assert_eq!(tool.name(), "ask_user_clarification");
    }

    #[tokio::test]
    async fn dispatch_subagent_returns_tool_error_when_agent_unknown() {
        // Exercises the graceful-failure paths of `dispatch_subagent`:
        // without a global registry we get the "registry not initialised"
        // branch, and with one (set by another test in the same binary)
        // a bogus agent id hits the "agent not found" branch. Either way
        // the function must return `Ok(ToolResult::error(..))` rather than
        // panicking or returning `Err`.
        let res = dispatch_subagent(
            "__definitely_not_a_real_agent__",
            "test_tool",
            "irrelevant prompt",
            None,
        )
        .await
        .expect("dispatch_subagent should not return Err on these inputs");

        assert!(res.is_error, "expected a tool-error ToolResult");
        let out = res.output();
        assert!(
            out.contains("registry not initialised") || out.contains("not found in registry"),
            "unexpected graceful-failure message: {out}"
        );
    }
}
