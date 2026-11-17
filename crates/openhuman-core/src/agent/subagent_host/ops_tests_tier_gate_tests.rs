use super::*;

#[test]
fn tier_gate_skips_when_parent_unresolved() {
    use tinyagents_definition::AgentTier;
    // No resolvable parent definition (e.g. registry uninitialised, or a
    // dynamically-named model-council juror / custom agent absent from it) →
    // skip rather than mask. Even a would-be-illegal child tier passes, because
    // we have no parent tier to judge against.
    let mut child = make_def_named_tools(&[]);
    child.agent_tier = AgentTier::Chat;
    assert!(gate(None, &child).is_ok());
}

#[test]
fn tier_gate_allows_legal_descending_hops() {
    use tinyagents_definition::AgentTier;
    let mut parent = make_def_named_tools(&[]);
    let mut child = make_def_named_tools(&[]);

    // chat → worker
    parent.agent_tier = AgentTier::Chat;
    child.agent_tier = AgentTier::Worker;
    assert!(gate(Some(&parent), &child).is_ok());

    // chat → reasoning
    child.agent_tier = AgentTier::Reasoning;
    assert!(gate(Some(&parent), &child).is_ok());

    // reasoning → worker
    parent.agent_tier = AgentTier::Reasoning;
    child.agent_tier = AgentTier::Worker;
    assert!(gate(Some(&parent), &child).is_ok());
}

#[test]
fn tier_gate_allows_worker_parent() {
    use tinyagents_definition::AgentTier;
    // A worker's `subagents` list holds no agent id (the loader rejects
    // one), so any spawn it reaches at runtime is one the host dispatched for
    // it. The gate must NOT re-deny that — the worker-leaf rule is a static
    // boot-time authoring constraint, not a runtime one; the per-parent
    // allowlist gate blocks any other worker spawn. Regression for the
    // wildcard-integration case (CodeRabbit P2 on PR #4102).
    let mut parent = make_def_named_tools(&[]);
    let child = make_def_named_tools(&[]); // worker by default
    parent.agent_tier = AgentTier::Worker;
    assert!(gate(Some(&parent), &child).is_ok());
}

#[test]
fn tier_gate_denies_chat_to_chat() {
    use tinyagents_definition::AgentTier;
    let mut parent = make_def_named_tools(&[]);
    let mut child = make_def_named_tools(&[]);
    parent.agent_tier = AgentTier::Chat;
    child.agent_tier = AgentTier::Chat;

    let err =
        gate(Some(&parent), &child).expect_err("chat→chat must be denied at the runtime gate");
    match err {
        SubagentRunError::TierViolation {
            parent_tier,
            child_tier,
            reason,
        } => {
            assert_eq!(parent_tier, AgentTier::Chat);
            assert_eq!(child_tier, AgentTier::Chat);
            assert!(
                reason.contains("chat") && reason.contains("leaf"),
                "got: {reason}"
            );
        }
        other => panic!("expected TierViolation, got: {other:?}"),
    }
}

#[test]
fn tier_gate_allows_upward_reasoning_to_chat() {
    use tinyagents_definition::AgentTier;
    // Upward delegation is intentionally legal (reasoning agent →
    // orchestrator chat). The gate must not deny it.
    let mut parent = make_def_named_tools(&[]);
    let mut child = make_def_named_tools(&[]);
    parent.agent_tier = AgentTier::Reasoning;
    child.agent_tier = AgentTier::Chat;
    assert!(gate(Some(&parent), &child).is_ok());
}

// ---------------------------------------------------------------------------
// Turn-scoped dispatch gate (#5804)
//
// These exercise the gate at its real call site rather than only the policy it
// consults. The lever is that no `ParentExecutionContext` is installed here, so
// an ungated child execution returns `NoParentContext`: each refusal below is
// therefore evidence the gate ran *and* that it ran before anything was spent,
// and removing the gate turns every one of them into `NoParentContext`.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dispatch_is_refused_after_the_turn_requests_a_graceful_pause() {
    let definition = make_def_named_tools(&[]);
    let dispatch = std::sync::Arc::new(crate::agent::tinyagents::host::TurnDispatchState::new(
        Some(std::time::Duration::from_secs(600)),
    ));
    dispatch.record_pause_requested(15, 15);
    let mut options = SubagentRunOptions::default();
    options.run_context.dispatch = Some(dispatch);
    let outcome = run_host_from_test_options(&definition, "task", options).await;

    assert!(
        matches!(
            outcome,
            Err(SubagentRunError::PauseRequested {
                completed_model_calls: 15,
                cap: 15
            })
        ),
        "a dispatch after the cap-pause request must be refused, not run: {outcome:?}"
    );
}

#[tokio::test]
async fn dispatch_is_refused_when_the_remaining_budget_cannot_fit_an_observed_subagent() {
    let definition = make_def_named_tools(&[]);
    let dispatch = std::sync::Arc::new(crate::agent::tinyagents::host::TurnDispatchState::new(
        Some(std::time::Duration::ZERO),
    ));
    dispatch.record_subagent_elapsed(std::time::Duration::from_secs(60));
    let mut options = SubagentRunOptions::default();
    options.run_context.dispatch = Some(dispatch);
    let outcome = run_host_from_test_options(&definition, "task", options).await;

    assert!(
        matches!(
            outcome,
            Err(SubagentRunError::DispatchBudgetExhausted { .. })
        ),
        "a dispatch that cannot fit the remaining budget must be refused: {outcome:?}"
    );
}

#[tokio::test]
async fn dispatch_is_not_refused_while_the_guard_has_no_evidence() {
    // The other half of the contract, and the one that keeps this from being a
    // throughput regression: with no pause requested and no completed
    // sub-agent to learn from, the gate must let the dispatch through. Reaching
    // `NoParentContext` is exactly that — the gate declined to interfere and
    // the normal path ran.
    let definition = make_def_named_tools(&[]);
    let mut options = SubagentRunOptions::default();
    options.run_context.dispatch = Some(std::sync::Arc::new(
        crate::agent::tinyagents::host::TurnDispatchState::new(Some(std::time::Duration::ZERO)),
    ));
    let outcome = run_host_from_test_options(&definition, "task", options).await;

    assert!(
        matches!(outcome, Err(SubagentRunError::NoParentContext)),
        "an exhausted budget with no observed sub-agent is not evidence — the \
         dispatch must proceed: {outcome:?}"
    );
}
