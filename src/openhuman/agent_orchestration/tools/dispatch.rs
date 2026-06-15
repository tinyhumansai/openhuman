//! Subagent dispatch logic shared by all agent delegation tools.

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::agent::harness::subagent_runner::{run_subagent, SubagentRunOptions};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::tools::traits::ToolResult;

pub(crate) async fn dispatch_subagent(
    agent_id: &str,
    tool_name: &str,
    prompt: &str,
    skill_filter: Option<&str>,
    model_override: Option<&str>,
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

    let parent_ctx = current_parent();
    if let Some(ctx) = &parent_ctx {
        if !ctx.allowed_subagent_ids.contains(&definition.id) {
            log::warn!(
                "[agent] blocked delegation via {}: parent={} requested={} allowed={:?}",
                tool_name,
                ctx.agent_definition_id,
                definition.id,
                ctx.allowed_subagent_ids
            );
            return Ok(ToolResult::error(format!(
                "{tool_name}: agent '{}' is not in parent agent '{}' subagents.allowlist",
                definition.id, ctx.agent_definition_id
            )));
        }
    }

    let parent_session = parent_ctx
        .as_ref()
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

    // Also send to the per-request progress sink so the web channel bridge
    // emits `subagent_spawned` to the frontend (same pattern as spawn_subagent.rs).
    if let Some(progress) = current_parent().and_then(|p| p.on_progress.clone()) {
        let _ = progress
            .send(AgentProgress::SubagentSpawned {
                agent_id: definition.id.clone(),
                task_id: task_id.clone(),
                mode: "typed".to_string(),
                dedicated_thread: false,
                prompt_chars: prompt.chars().count(),
                worker_thread_id: None,
                display_name: Some(definition.display_name().to_string()),
            })
            .await;
    }

    log::info!(
        "[agent] delegating to {} via {} (skill_filter={}) prompt_chars={}",
        agent_id,
        tool_name,
        skill_filter.unwrap_or("<none>"),
        prompt.chars().count()
    );

    // Propagate the per-call toolkit scope into the subagent runner so
    // that the collapsed `SkillDelegationTool` can narrow
    // `integrations_agent` to a single Composio toolkit (e.g.
    // `delegate_to_integrations_agent { toolkit: "gmail" }` →
    // integrations_agent + toolkit="gmail"). Earlier code plumbed this through
    // `skill_filter_override` (which matches `{skill}__` QuickJS-style
    // names), but Composio actions are named `GMAIL_*` / `NOTION_*` —
    // so the filter excluded every Composio tool instead of narrowing
    // them. `toolkit_override` applies the correct `{TOOLKIT}_` prefix
    // check, restricted to skill-category tools.
    let options = SubagentRunOptions {
        skill_filter_override: None,
        toolkit_override: skill_filter.map(str::to_string),
        context: None,
        model_override: model_override.map(str::to_string),
        task_id: Some(task_id.clone()),
        worker_thread_id: None,
        initial_history: None,
        checkpoint_dir: None,
        worktree_action_dir: None,
        run_queue: None,
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
            // Make the failure unmistakable to the orchestrator: the delegated
            // task did NOT run, so it must not be reported as success or have
            // its output fabricated. Without this guardrail a weak orchestrator
            // can narrate a plausible success from the bare error text — the
            // "hallucinated success" half of #3193 (e.g. claiming `run_code`
            // wrote a file when the coding model 404'd and nothing executed).
            Ok(ToolResult::error(format_subagent_failure(
                tool_name, &message,
            )))
        }
    }
}

/// Format a subagent-delegation failure so the orchestrator cannot mistake it
/// for success. Kept as a standalone, side-effect-free fn so the exact wording
/// is unit-testable without standing up a registry + failing model (#3193).
fn format_subagent_failure(tool_name: &str, message: &str) -> String {
    format!(
        "{tool_name} failed and did not complete — no work was performed and no \
         results were produced. Do NOT treat this as success or fabricate an \
         output; report the failure to the user. Error: {message}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::Tool;

    use crate::openhuman::agent::tools::AskClarificationTool;

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

    #[test]
    fn subagent_failure_envelope_forbids_fabricated_success() {
        // #3193: a hard delegation failure (e.g. run_code's coding model
        // 404ing) must be surfaced so the orchestrator cannot narrate a
        // plausible success. The envelope states the task did not run, tells
        // the model not to fabricate output, and preserves the root error.
        let msg = format_subagent_failure(
            "run_code",
            "openhuman API error (404): model 'davinci-002' does not support \
             the chat-completions API",
        );
        assert!(msg.contains("run_code failed"), "names the tool: {msg}");
        assert!(
            msg.contains("did not complete"),
            "states no completion: {msg}"
        );
        assert!(
            msg.to_lowercase().contains("do not treat this as success")
                && msg.contains("fabricate"),
            "warns against fabricated success: {msg}"
        );
        assert!(
            msg.contains("davinci-002") && msg.contains("404"),
            "preserves the root error: {msg}"
        );
    }
}
