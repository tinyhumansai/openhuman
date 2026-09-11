use super::*;

fn usage(input: u64, output: u64, usd: f64) -> SubagentUsage {
    SubagentUsage {
        input_tokens: input,
        output_tokens: output,
        cached_input_tokens: 0,
        charged_amount_usd: usd,
    }
}

#[tokio::test]
async fn no_collector_outside_scope() {
    assert!(current_collector().is_none());
    // Must not panic when there is nothing to record into.
    record_subagent_usage("t1", "researcher", usage(10, 5, 0.01));
}

#[tokio::test]
async fn collects_entries_within_scope() {
    let ((), entries) = with_turn_collector(async {
        record_subagent_usage("t1", "researcher", usage(10, 5, 0.01));
        record_subagent_usage("t2", "coder", usage(20, 8, 0.02));
    })
    .await;
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].task_id, "t1");
    assert_eq!(entries[0].usage.input_tokens, 10);
    assert_eq!(entries[1].agent_id, "coder");
    assert_eq!(entries[1].usage.charged_amount_usd, 0.02);
}

#[tokio::test]
async fn map_reduce_fanout_preserves_scope() {
    use tinyagents_graph::parallel::{map_reduce, FailurePolicy, ParallelOptions};

    let (result, entries) = with_turn_collector(async {
        map_reduce(
            vec![
                ("t1", "researcher", usage(10, 5, 0.01)),
                ("t2", "coder", usage(20, 8, 0.02)),
            ],
            ParallelOptions::default()
                .with_max_concurrency(2)
                .with_failure_policy(FailurePolicy::CollectAll),
            |_index, (task_id, agent_id, usage)| async move {
                record_subagent_usage(task_id, agent_id, usage);
                Ok::<_, tinyagents_harness::TinyAgentsError>(task_id)
            },
        )
        .await
    })
    .await;

    let outcome = result.expect("map_reduce should complete");
    assert_eq!(outcome.outcomes.len(), 2);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].task_id, "t1");
    assert_eq!(entries[1].agent_id, "coder");
}

#[tokio::test]
async fn scope_does_not_leak() {
    let _ = with_turn_collector(async {
        record_subagent_usage("t1", "researcher", usage(1, 1, 0.0));
    })
    .await;
    assert!(current_collector().is_none());
}
