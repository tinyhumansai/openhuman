//! Provider-root overlays retain session metadata and child spend.
use super::overlay_response_usage;
use openhuman_core::agent::subagent_host::SubagentUsage;
use openhuman_core::agent::tinyagents::host::{LastTurnUsage, SubagentUsageEntry};
use tinyagents_harness::middleware::ResponseUsage;

#[test]
fn reported_root_usage_preserves_children_and_session_context_without_double_counting() {
    let report = ResponseUsage {
        input_tokens: 10,
        output_tokens: 5,
        cached_tokens: 3,
        reasoning_tokens: 2,
        cost_usd: Some(0.007),
        has_cost_receipt: true,
    };
    let mut captured = Some(LastTurnUsage {
        input_tokens: 99,
        context_window: 8000,
        context_tokens: 15,
        subagents: vec![SubagentUsageEntry {
            task_id: "child".into(),
            agent_id: "reader".into(),
            usage: SubagentUsage {
                input_tokens: 20,
                output_tokens: 8,
                cached_input_tokens: 4,
                charged_amount_usd: 0.003,
                cost_source: openhuman_core::agent::turn_projection::failure_usage(&report)
                    .cost_source,
            },
        }],
        ..Default::default()
    });
    for _ in 0..2 {
        overlay_response_usage(&mut captured, &report);
        let usage = captured.as_ref().unwrap();
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cached_input_tokens
            ),
            (30, 13, 7)
        );
        assert_eq!(usage.reasoning_tokens, 2);
        assert_eq!(usage.cost_usd, Some(0.010));
        assert_eq!((usage.context_window, usage.context_tokens), (8000, 15));
        assert_eq!(usage.subagents.len(), 1);
    }
    captured.as_mut().unwrap().subagents[0].usage.cost_source =
        openhuman_core::agent::turn_projection::failure_usage(&ResponseUsage::default())
            .cost_source;
    overlay_response_usage(&mut captured, &report);
    assert_eq!(captured.as_ref().unwrap().cost_usd, None);
    assert_eq!(captured.as_ref().unwrap().input_tokens, 30);
}

#[test]
fn absent_receipts_preserve_host_cost_but_invalid_receipts_remain_unknown() {
    let mut report = ResponseUsage {
        input_tokens: 10,
        ..Default::default()
    };
    let mut captured = Some(LastTurnUsage {
        cost_usd: Some(5.0),
        ..Default::default()
    });
    overlay_response_usage(&mut captured, &report);
    assert_eq!(captured.as_ref().unwrap().cost_usd, Some(5.0));
    report.has_cost_receipt = true;
    overlay_response_usage(&mut captured, &report);
    assert_eq!(captured.as_ref().unwrap().cost_usd, None);
}
