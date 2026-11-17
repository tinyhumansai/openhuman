use super::*;

pub(super) fn driver_error(error: impl std::fmt::Display) -> DriverFailure {
    driver_failure(error)
}

pub(super) fn driver_failure(error: impl std::fmt::Display) -> DriverFailure {
    DriverFailure {
        error: RuntimeError::Driver(error.to_string()),
        partial: None,
        outcome: None,
    }
}

/// Refuse fresh declarations without a request-scoped executable source.
/// Historical declarations may outlive their executors. The harness handles
/// calls to those missing tools as recoverable errors; history never supplies
/// an executor or bypasses the current tool policy.
pub(super) fn ensure_snapshot_tools_are_executable(
    visible_tool_names: &HashSet<String>,
    tools: &Arc<Vec<Box<dyn tinytools::Tool>>>,
    synthesized_tools: &Arc<Vec<Box<dyn tinytools::Tool>>>,
    recorded_tool_names: &HashSet<String>,
) -> Result<(), DriverFailure> {
    let executable_tool_names = tools
        .iter()
        .chain(synthesized_tools.iter())
        .map(|tool| tool.name().to_string())
        .collect::<HashSet<_>>();
    let missing = visible_tool_names
        .difference(&executable_tool_names)
        .filter(|name| !recorded_tool_names.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(driver_failure(format!(
            "session tool snapshot declares non-executable tools: {}",
            missing.join(", ")
        )))
    }
}

pub(super) fn driver_error_with_snapshot(
    error: anyhow::Error,
    snapshot: &crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshotSink,
    sidecar: &std::sync::Arc<
        std::sync::Mutex<crate::agent::tinyagents::host::run_context::SessionTurnSidecar>,
    >,
    elapsed: std::time::Duration,
) -> DriverFailure {
    // Classify from the typed harness error when the chain carries one, rather
    // than matching on its rendered text.
    let dispatch_refusal = error.chain().find_map(|cause| {
        cause.downcast_ref::<crate::agent::tinyagents::budget::DispatchRefusal>()
    });
    sidecar
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .dispatch_refusal = dispatch_refusal.copied();
    let typed = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<tinyagents_harness::TinyAgentsError>());
    let stalled = matches!(
        typed,
        Some(tinyagents_harness::TinyAgentsError::GenerationStalled)
    );
    let terminal = typed.map(|typed| {
        tinyagents_harness::terminal::TerminalOutcome::from_error(
            typed,
            tinyagents_harness::terminal::TimeoutPhase::AfterTurn,
        )
    });
    let error = error.to_string();
    let guard = snapshot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    record_snapshot_usage(&guard, sidecar, elapsed);
    if guard.messages.is_empty() {
        let mut failure = driver_failure(error);
        failure.outcome = terminal;
        return failure;
    }
    let accepted_end = guard.accepted_end();
    let history = guard.messages[..accepted_end].to_vec();
    let unanswered = crate::agent::tinyagents::middleware::turn_context::render_unanswered_steps(
        &guard.messages[accepted_end..],
    );
    let display = if stalled {
        // The model's streamed narration was stopped before it could repeat
        // indefinitely. Preserve the completed tools as a useful, bounded
        // partial rather than showing only the failed model's process text.
        let results = crate::agent::session_host::product_copy::close_results(&guard.tool_outcomes);
        let evidence =
            crate::agent::session_host::product_copy::render_close_results(&results, 4_000);
        format!(
            "I stopped a repetitive model response before it could finish. Here are the completed tool results I can report:\n{evidence}"
        )
    } else {
        match unanswered {
            Some(steps) => format!("The turn stopped before completion: {error}.\n\n{steps}"),
            None => format!("The turn stopped before completion: {error}."),
        }
    };
    DriverFailure {
        error: RuntimeError::Driver(error),
        partial: Some(DriverOutcome {
            history,
            output: None,
            partial: Some(TranscriptPartial::new(display)),
            interrupted: true,
            outcome: terminal.clone(),
            tool_outcomes: guard.tool_outcomes.clone(),
        }),
        outcome: terminal,
    }
}

/// A graph error or a Stop drops the harness's `AgentRun`, but its snapshot
/// has already observed provider-accepted usage and completed tool outcomes.
/// Copy those into the per-turn sidecar *before* producing the partial so
/// runtime's single atomic append carries the same usage/failure truth as a
/// successful turn. The partial history itself remains accepted-only.
pub(super) fn record_snapshot_usage(
    guard: &crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshot,
    sidecar: &std::sync::Arc<
        std::sync::Mutex<crate::agent::tinyagents::host::run_context::SessionTurnSidecar>,
    >,
    elapsed: std::time::Duration,
) {
    let mut observed = sidecar
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    observed.model_calls = guard.model_calls as usize;
    observed.tool_calls = guard.tool_outcomes.len();
    observed.input_tokens = guard.input_tokens;
    observed.output_tokens = guard.output_tokens;
    observed.cached_input_tokens = guard.cached_input_tokens;
    observed.last_call_input_tokens = guard.last_call_input_tokens;
    observed.last_call_output_tokens = guard.last_call_output_tokens;
    // Each answered call was priced as it arrived (reported charge, else
    // catalog estimate, else unknown); nothing is re-estimated here.
    observed.cost = guard.cost;
    observed.duration = Some(elapsed);
    observed.tool_outcomes = guard.tool_outcomes.clone();
    observed.resolved_route = guard.resolved_route.clone();
}

/// A stopped turn's partial (#7715): the prompt and the accepted rounds, or
/// just the request before the first model call. No display partial: the
/// turn_state mirror already records the streamed text.
pub(super) fn driver_cancelled_with_snapshot(
    snapshot: &crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshotSink,
    sidecar: &std::sync::Arc<
        std::sync::Mutex<crate::agent::tinyagents::host::run_context::SessionTurnSidecar>,
    >,
    elapsed: std::time::Duration,
    request_history: Vec<Message>,
) -> DriverFailure {
    let guard = snapshot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    record_snapshot_usage(&guard, sidecar, elapsed);
    let history = if guard.messages.is_empty() {
        request_history
    } else {
        guard.messages[..guard.accepted_end()].to_vec()
    };
    DriverFailure {
        error: RuntimeError::Cancelled,
        partial: Some(DriverOutcome {
            history,
            output: None,
            partial: None,
            interrupted: true,
            outcome: None,
            tool_outcomes: guard.tool_outcomes.clone(),
        }),
        outcome: None,
    }
}

/// Append the deterministic required-output block when `output` lacks it, and
/// make that the durable final assistant row. Used when no model repair may
/// run: a classified halt, or a turn out of time before the backstop.
pub(super) fn append_synthesized_block(
    output: &mut String,
    history: &mut Vec<Message>,
    contract: &tinyagents_harness::config::RequiredOutput,
) {
    if tinyagents_harness::config::output_satisfies_contract(output, contract) {
        return;
    }
    output.push_str("\n\n");
    output.push_str(&tinyagents_harness::config::synthesize_block(contract));
    if history
        .last()
        .is_some_and(|message| matches!(message, Message::Assistant(_)))
    {
        history.pop();
    }
    history.push(Message::assistant(output.clone()));
}
