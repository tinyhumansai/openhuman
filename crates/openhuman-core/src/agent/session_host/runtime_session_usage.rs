//! Committed-turn goal accounting and billing projection.

fn rollup_usage(
    sidecar: &crate::agent::tinyagents::host::run_context::SessionTurnSidecar,
) -> tinyagents_harness::cost::UsageContribution {
    let contribution = |input_tokens, output_tokens, cached_input_tokens, cost| {
        tinyagents_harness::cost::UsageContribution {
            input_tokens,
            output_tokens,
            cached_input_tokens,
            cost,
        }
    };
    tinyagents_harness::cost::roll_up_inclusive_subagent_usage(
        contribution(
            sidecar.input_tokens,
            sidecar.output_tokens,
            sidecar.cached_input_tokens,
            sidecar.cost,
        ),
        sidecar.subagents.iter().map(|entry| {
            let usage = &entry.usage;
            contribution(
                usage.input_tokens,
                usage.output_tokens,
                usage.cached_input_tokens,
                usage.cost(),
            )
        }),
    )
}

/// Account only a receipt-backed turn, using the same direct-plus-completed
/// child total the codec attached to the atomic transcript append.
pub(in crate::agent::session_host) async fn account_committed_turn_against_goal(
    workspace_dir: &std::path::Path,
    context: &crate::core::runtime::DispatchCtx,
    thread_id: Option<&str>,
    sidecar: &crate::agent::tinyagents::host::run_context::SessionTurnSidecar,
) {
    let usage = rollup_usage(sidecar);
    crate::agent::goals::account_turn(
        workspace_dir,
        context,
        thread_id,
        usage.input_tokens,
        usage.output_tokens,
        sidecar.duration.unwrap_or_default(),
    )
    .await;
}

/// UI billing projection for a committed turn. It keeps child records for a
/// detailed display while reporting the same all-in totals the codec persists.
pub(in crate::agent::session_host) fn holistic_last_turn_usage(
    sidecar: &crate::agent::tinyagents::host::run_context::SessionTurnSidecar,
) -> crate::agent::tinyagents::host::LastTurnUsage {
    let usage = rollup_usage(sidecar);
    crate::agent::tinyagents::host::LastTurnUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        cost_usd: usage.cost.usd(),
        cost_source: usage.cost.source,
        context_window: sidecar.context_window,
        context_tokens: sidecar
            .last_call_input_tokens
            .saturating_add(sidecar.last_call_output_tokens),
        subagents: sidecar.subagents.clone(),
        // The session does not count these; a library turn that asked for a
        // final-response report fills them in (`response_shape`).
        reasoning_tokens: 0,
    }
}
