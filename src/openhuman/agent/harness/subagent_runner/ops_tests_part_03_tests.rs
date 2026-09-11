use super::*;

// ── Guard: #3152 prefix tier must stay strictly unique ──────────────
//
// When a truncated slug prefix-matches MORE than one catalogued action,
// the resolver must refuse rather than guess — a mis-dispatched write
// could create/update the wrong resource (data-integrity). Also asserts
// the length gate: a too-short request never fans out.
#[test]
fn prefix_tier_refuses_ambiguous_and_short_slugs() {
    use crate::openhuman::agent::context::prompt::ConnectedIntegrationTool;
    let mk = |name: &str| ConnectedIntegrationTool {
        name: name.into(),
        description: "d".into(),
        parameters: None,
    };
    let resolver = LazyToolkitResolver {
        config: std::sync::Arc::new(crate::openhuman::config::Config::default()),
        actions: vec![
            mk("NOTION_SEARCH_NOTION_PAGE"),
            mk("NOTION_SEARCH_NOTION_DATABASE"),
            mk("NOTION_CREATE_NOTION_PAGE"),
        ],
        resolved: std::sync::Mutex::default(),
    };
    // `NOTION_SEARCH_NOTION` is a prefix of TWO actions → ambiguous → None.
    assert!(
        resolver.resolve("NOTION_SEARCH_NOTION").is_none(),
        "#3152: ambiguous prefix must not silently dispatch to a guess"
    );
    // Short slug below the length gate never engages the prefix tier.
    assert!(resolver.resolve("NOTION").is_none());
}

#[test]
fn tier_gate_skips_when_parent_unresolved() {
    use crate::openhuman::agent::harness::definition::AgentTier;
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
    use crate::openhuman::agent::harness::definition::AgentTier;
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
fn tier_gate_allows_worker_parent_for_collapsed_integration() {
    use crate::openhuman::agent::harness::definition::AgentTier;
    // A worker only reaches the runtime spawn chokepoint via the documented
    // collapsed `delegate_to_integrations_agent` path (→ `integrations_agent`,
    // itself a worker). The gate must NOT re-deny that — the worker-leaf rule
    // is a static boot-time authoring constraint, not a runtime one. Regression
    // for the wildcard-integration case (CodeRabbit P2 on PR #4102).
    let mut parent = make_def_named_tools(&[]);
    let child = make_def_named_tools(&[]); // worker by default
    parent.agent_tier = AgentTier::Worker;
    assert!(gate(Some(&parent), &child).is_ok());
}

#[test]
fn tier_gate_denies_chat_to_chat() {
    use crate::openhuman::agent::harness::definition::AgentTier;
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
    use crate::openhuman::agent::harness::definition::AgentTier;
    // Upward delegation is intentionally legal (subconscious reasoner →
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
// an ungated `run_subagent` returns `NoParentContext`: each refusal below is
// therefore evidence the gate ran *and* that it ran before anything was spent,
// and removing the gate turns every one of them into `NoParentContext`.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dispatch_is_refused_after_the_turn_requests_a_graceful_pause() {
    let definition = make_def_named_tools(&[]);
    let outcome = turn_dispatch_guard::with_dispatch_guard(
        Some(std::time::Duration::from_secs(600)),
        async {
            // Stands in for `CapPauser`, which writes through a clone of this
            // same `Arc` when the model-call cap is reached.
            turn_dispatch_guard::current()
                .expect("guard installed")
                .record_pause_requested(15, 15);
            run_subagent(&definition, "task", SubagentRunOptions::default()).await
        },
    )
    .await;

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
    let outcome = turn_dispatch_guard::with_dispatch_guard(
        Some(std::time::Duration::from_millis(1)),
        async {
            // One completed sub-agent took a minute; the turn's whole ceiling
            // is a millisecond and it has already elapsed.
            turn_dispatch_guard::record_subagent_elapsed(std::time::Duration::from_secs(60));
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            run_subagent(&definition, "task", SubagentRunOptions::default()).await
        },
    )
    .await;

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
    let outcome = turn_dispatch_guard::with_dispatch_guard(
        Some(std::time::Duration::from_millis(1)),
        async {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            run_subagent(&definition, "task", SubagentRunOptions::default()).await
        },
    )
    .await;

    assert!(
        matches!(outcome, Err(SubagentRunError::NoParentContext)),
        "an exhausted budget with no observed sub-agent is not evidence — the \
         dispatch must proceed: {outcome:?}"
    );
}
