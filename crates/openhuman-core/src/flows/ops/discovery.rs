use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// Flow Scout — workflow discovery + suggestion lifecycle
// ─────────────────────────────────────────────────────────────────────────────

/// Overall safety bound on one `flows_discover` run. The `flow_discovery` agent
/// reasons read-only over the user's data and ends by emitting
/// `suggest_workflows`; its own `max_iterations` caps the loop, but a hung
/// LLM/tool call must never let the RPC block indefinitely.
///
/// Matches [`FLOW_BUILD_TIMEOUT_SECS`] (600s): the session builder applies the
/// `flow_discovery` definition's `effective_max_iterations()` (50, not the
/// global default of 10) to this path (issue #4868), so a worst-case run at
/// ~10s/iteration can take up to ~500s — the old 300s bound could clip a
/// legitimate long discovery run before the iteration cap ever got a chance
/// to (post-merge Codex P2 finding).
const FLOW_DISCOVER_TIMEOUT_SECS: u64 = 600;

/// The canned brief handed to the `flow_discovery` agent. The agent's own
/// archetype prompt teaches the read → correlate → ground → emit loop; this is
/// just the kick-off instruction for the on-demand "Discover" action.
const FLOW_DISCOVER_PROMPT: &str = "Discover the most useful automations you could set up for me. \
     Read what you can about how I work — my goals, recurring conversations, the people and apps I \
     deal with, and the flows I already have — then propose a few concrete, buildable workflows. \
     Ground each in something you actually observed about me, and end by calling suggest_workflows.";

/// Runs the read-only `flow_discovery` agent ("Flow Scout") on demand: it reads
/// the user's memory/threads/people/connections/existing flows, grounds a few
/// automation ideas, and records them via the `suggest_workflows` tool (which
/// persists to the `flow_suggestions` table). Returns the current set of active
/// (`New`) suggestions after the run.
///
/// The agent is strictly read-only — its only write is `suggest_workflows`
/// (`PermissionLevel::None`) — so this never persists, enables, or runs a flow.
/// Turning a suggestion into a real flow is the user's separate "Build this"
/// action, which routes to `workflow_builder`.
pub async fn flows_discover(
    config: &Config,
    stream: Option<FlowStreamTarget>,
) -> Result<RpcOutcome<Vec<FlowSuggestion>>, String> {
    use crate::agent::turn_origin::{with_origin, AgentTurnOrigin};
    use crate::agent::OpenHumanSessionHost;

    tracing::info!(
        target: "flows",
        streaming = stream.is_some(),
        "[flows] flows_discover: starting Flow Scout discovery run"
    );

    // The registry must be initialised before building a named builtin agent
    // (mirrors `agent_registry::ops::available_tools`); it is idempotent, so a
    // second call from an already-booted core is a cheap no-op.
    crate::agent::harness::AgentDefinitionRegistry::init_global(&config.workspace_dir)
        .map_err(|e| format!("failed to initialise agent registry: {e}"))?;

    let mut agent = OpenHumanSessionHost::from_config_for_agent(config, "flow_discovery")
        .map_err(|e| format!("failed to build flow_discovery agent: {e:#}"))?;
    agent.set_agent_definition_name("flow_discovery".to_string());

    // When a chat thread is attached, stream the scout turn into it exactly like
    // an interactive turn (see `FlowStreamTarget`). Best-effort — with no target
    // the run stays headless, exactly as before.
    if let Some(target) = &stream {
        attach_flow_progress_bridge(&mut agent, target, "flows_discover", config);
        agent.set_thread_id(Some(target.thread_id.as_str()));
    }

    // Run to completion under a CLI origin (an internal, user-initiated action —
    // the approval gate must not fail-closed on it), bounded by a wall-clock
    // timeout so a hung provider call can't wedge the RPC. When streaming, the
    // run is wrapped in the thread-id scope so descendant turns tag their trace
    // and socket events with this thread.
    let run = with_origin(AgentTurnOrigin::Cli, agent.run_single(FLOW_DISCOVER_PROMPT));
    let run = tokio::time::timeout(
        std::time::Duration::from_secs(FLOW_DISCOVER_TIMEOUT_SECS),
        run,
    );
    let timed = run.await;
    // Reduce the (timeout, run) result to a single `Result<summary, error>` so
    // the terminal chat event can be emitted uniformly for the streamed case.
    let outcome: Result<String, String> = match timed {
        Ok(Ok(summary)) => {
            tracing::debug!(target: "flows", "[flows] flows_discover: agent run completed");
            Ok(summary)
        }
        Ok(Err(e)) => {
            // The agent errored. Surface it, but still return whatever
            // suggestions may already be persisted (a prior run's active set)
            // rather than hard-failing the UI.
            tracing::warn!(target: "flows", error = %e, "[flows] flows_discover: agent run failed");
            Err(format!("flow_discovery run failed: {e:#}"))
        }
        Err(_) => {
            tracing::warn!(
                target: "flows",
                timeout_secs = FLOW_DISCOVER_TIMEOUT_SECS,
                "[flows] flows_discover: agent run timed out"
            );
            Err(format!(
                "flow_discovery run timed out after {FLOW_DISCOVER_TIMEOUT_SECS}s"
            ))
        }
    };

    // Emit the terminal chat event so a client viewing the thread finalizes the
    // assistant bubble instead of spinning (the bridge only streams deltas).
    if let Some(target) = &stream {
        finalize_flow_stream(target, &outcome, FLOW_DISCOVER_PROMPT).await;
    }

    let suggestions = store::list_suggestions(config, Some(SuggestionStatus::New), 50)
        .map_err(|e| e.to_string())?;
    tracing::info!(
        target: "flows",
        count = suggestions.len(),
        "[flows] flows_discover: returning active suggestions"
    );
    Ok(RpcOutcome::single_log(
        suggestions,
        "flow discovery complete",
    ))
}

/// Lists persisted workflow suggestions. `status` filters to one lifecycle
/// state (the UI passes `New` for the active "Suggested for you" cards); `None`
/// returns every status.
pub async fn flows_list_suggestions(
    config: &Config,
    status: Option<SuggestionStatus>,
) -> Result<RpcOutcome<Vec<FlowSuggestion>>, String> {
    let suggestions = store::list_suggestions(config, status, 100).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(suggestions, "suggestions listed"))
}

/// Marks a suggestion `dismissed` (the user rejected the card). The row is kept
/// so a later discovery run dedupes against it and won't re-surface the idea.
pub async fn flows_dismiss_suggestion(
    config: &Config,
    id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let found = store::set_suggestion_status(config, id, SuggestionStatus::Dismissed)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        json!({ "id": id, "dismissed": found }),
        "suggestion dismissed",
    ))
}

/// Marks a suggestion `built` — called by the frontend after the user saves a
/// flow authored from this suggestion, so it drops out of the active cards.
pub async fn flows_mark_suggestion_built(
    config: &Config,
    id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let found = store::set_suggestion_status(config, id, SuggestionStatus::Built)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        json!({ "id": id, "built": found }),
        "suggestion marked built",
    ))
}
