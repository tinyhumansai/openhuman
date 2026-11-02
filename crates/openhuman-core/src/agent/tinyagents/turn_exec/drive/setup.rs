use super::*;

pub(super) fn run_config(
    run_context: &mut OpenHumanRunContext,
    native_subagent: Option<&NativeSubagentRunContext>,
    is_subagent: bool,
    observed: bool,
    max_iterations: usize,
    max_output_tokens: Option<u32>,
    model: &str,
) -> (
    tinyagents_harness::context::RunConfig,
    tinyagents_harness::ids::RunId,
) {
    let mut config = native_subagent
        .map(|native| native.config.clone())
        .unwrap_or_else(|| {
            run_context.root_run_config(if is_subagent {
                "openhuman-subagent"
            } else {
                "openhuman-agent-turn"
            })
        })
        .with_max_model_calls(max_iterations)
        .with_max_tool_calls(run_context.tool_call_limit_for(max_iterations))
        .with_max_depth(super::super::super::budget::depth(run_context))
        .with_tag("openhuman")
        .with_tag(if observed { "observed" } else { "unobserved" });
    // The loop stamps this cap onto every model request; it does not bake into
    // the primary or routed models.
    if let Some(cap) = max_output_tokens {
        config = config.with_max_turn_output_tokens(cap);
    }
    let journal_run_id = config.run_id.clone();
    tracing::info!(
        model,
        max_iterations,
        observed,
        "[tinyagents] routing turn through the runtime turn driver (shared tools)"
    );
    (config, journal_run_id)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn event_bridge(
    sink: &EventSink,
    on_progress: Option<tokio::sync::mpsc::Sender<crate::progress::AgentProgress>>,
    model: &str,
    provider_id: String,
    max_iterations: usize,
    subagent_scope: Option<SubagentScope>,
    cursor: &tinyagents_harness::events::IterationCursor,
    tool_names: &crate::agent::tinyagents::observability::ToolNameMap,
    failure_map: &crate::agent::tinyagents::observability::ToolFailureMap,
    provider_usage_carry: &crate::agent::tinyagents::observability::ProviderUsageCarry,
    bridge_tool_sets: Vec<Arc<Vec<Box<dyn tinytools::Tool>>>>,
    run_context: &OpenHumanRunContext,
) -> Arc<OpenhumanEventBridge> {
    // Attach for observed and background turns alike: this bridge records
    // usage even when progress streaming is disabled.
    let bridge = OpenhumanEventBridge::with_scope(
        on_progress,
        model,
        provider_id,
        max_iterations,
        subagent_scope,
        cursor.clone(),
        tool_names.clone(),
        failure_map.clone(),
        provider_usage_carry.clone(),
        bridge_tool_sets,
        run_context.dispatch_ctx(),
    );
    bridge.set_definition(
        run_context
            .memory_identity
            .as_ref()
            .and_then(|identity| identity.agent_id.as_deref()),
    );
    sink.subscribe(bridge.clone());
    bridge
}

pub(super) fn turn_completed_sink(
    on_progress: &Option<tokio::sync::mpsc::Sender<crate::progress::AgentProgress>>,
    is_subagent: bool,
    defer_to_caller: bool,
) -> Option<tokio::sync::mpsc::Sender<crate::progress::AgentProgress>> {
    // Only the caller emits a top-level completion; session callers may defer
    // that event until post-run wrap-up is finished.
    (!is_subagent && !defer_to_caller)
        .then(|| on_progress.clone())
        .flatten()
}

pub(super) fn event_sink(
    journal_run_id: &tinyagents_harness::ids::RunId,
    native_subagent: Option<&NativeSubagentRunContext>,
) -> EventSink {
    // The stream prefix makes replayed event ids stable across process restart.
    let sink = EventSink::with_stream_id(journal_run_id.as_str());
    if let Some(native) = native_subagent {
        *native
            .child_events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(sink.clone());
        native
            .parent_events
            .emit(tinyagents_harness::events::AgentEvent::SubAgentRunStarted {
                name: native.name.clone(),
                depth: native.depth,
                run_id: journal_run_id.clone(),
            });
    }
    sink
}
