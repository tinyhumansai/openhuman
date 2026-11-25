use super::*;

#[test]
fn inclusive_child_usage_counts_each_public_entry_once_and_keeps_unknown_cost() {
    let charged = tinyagents_harness::cost::CostTally {
        known_usd: 0.002,
        source: tinyagents_harness::cost::CostSource::Charged,
    };
    let mut sidecar = crate::agent::tinyagents::host::run_context::SessionTurnSidecar {
        input_tokens: 10,
        output_tokens: 4,
        cost: charged,
        ..Default::default()
    };
    sidecar.subagents = vec![
        crate::agent::tinyagents::host::SubagentUsageEntry {
            task_id: "reused-task".into(),
            agent_id: "researcher".into(),
            usage: crate::agent::subagent_host::SubagentUsage {
                // Includes this run's grandchild totals; that grandchild is
                // intentionally absent as a separate public ledger entry.
                input_tokens: 20,
                output_tokens: 8,
                cached_input_tokens: 3,
                charged_amount_usd: charged.known_usd,
                cost_source: charged.source,
            },
        },
        crate::agent::tinyagents::host::SubagentUsageEntry {
            // A second entry for the same task remains a distinct legacy
            // contribution; this public ledger carries no run identity.
            task_id: "reused-task".into(),
            agent_id: "researcher".into(),
            usage: crate::agent::subagent_host::SubagentUsage {
                input_tokens: 7,
                output_tokens: 2,
                cost_source: tinyagents_harness::cost::CostSource::Unknown,
                ..Default::default()
            },
        },
    ];

    let usage = runtime_session::holistic_last_turn_usage(&sidecar);
    assert_eq!(usage.input_tokens, 37);
    assert_eq!(usage.output_tokens, 14);
    assert_eq!(usage.cached_input_tokens, 3);
    assert_eq!(usage.subagents.len(), 2);
    assert_eq!(usage.cost_usd, None, "unknown child cost stays unknown");
    assert_eq!(
        usage.cost_source,
        tinyagents_harness::cost::CostSource::Unknown
    );
}

#[tokio::test]
async fn committed_goal_accounting_saturates_direct_plus_inclusive_child_tokens() {
    let workspace = tempfile::tempdir().unwrap();
    let core = crate::core::runtime::CoreContext::for_test(
        crate::core::runtime::DomainSet::full(),
        Some(workspace.path().to_path_buf()),
    );
    crate::core::runtime::CoreContext::scope(core, async {
        let goal_store = crate::storage::sessions::stores_or_ephemeral(workspace.path()).kv;
        tinyagents_harness_tools::goals::store::set(
            &goal_store,
            "saturated-goal",
            "complete safely",
            Some(u64::MAX),
        )
        .await
        .unwrap();
        assert!(
            tinyagents_harness_tools::goals::store::get(&goal_store, "saturated-goal")
                .await
                .unwrap()
                .is_some(),
            "the scoped goal store must contain the fixture before accounting"
        );

        let mut sidecar = crate::agent::tinyagents::host::run_context::SessionTurnSidecar {
            input_tokens: 1,
            output_tokens: 2,
            ..Default::default()
        };
        sidecar.subagents = vec![
            crate::agent::tinyagents::host::SubagentUsageEntry {
                task_id: "large-child-a".into(),
                agent_id: "researcher".into(),
                usage: crate::agent::subagent_host::SubagentUsage {
                    input_tokens: u64::MAX - 2,
                    ..Default::default()
                },
            },
            crate::agent::tinyagents::host::SubagentUsageEntry {
                task_id: "large-child-b".into(),
                agent_id: "researcher".into(),
                usage: crate::agent::subagent_host::SubagentUsage {
                    input_tokens: 10,
                    ..Default::default()
                },
            },
        ];

        runtime_session::account_committed_turn_against_goal(
            workspace.path(),
            &crate::core::runtime::DispatchCtx::ambient(),
            Some("saturated-goal"),
            &sidecar,
        )
        .await;

        let goal = tinyagents_harness_tools::goals::store::get(&goal_store, "saturated-goal")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(goal.tokens_used, u64::MAX);
    })
    .await;
}
