use super::*;
use serde_json::json;
use tinychannels_bus::web::MAX_WEB_TOOL_PAYLOAD_BYTES;

fn expect_subagent(event: ChannelOutputEvent) -> SubagentOutput {
    match event {
        ChannelOutputEvent::Subagent(out) => *out,
        other => panic!("expected a sub-agent output, got {other:?}"),
    }
}

#[test]
fn lifecycle_events_name_the_stage_and_the_round() {
    let ChannelOutputEvent::Lifecycle {
        stage, iteration, ..
    } = inference_start()
    else {
        panic!("lifecycle")
    };
    assert_eq!((stage.as_str(), iteration), ("inference_start", None));
    let ChannelOutputEvent::Lifecycle { stage, .. } = inference_heartbeat() else {
        panic!("lifecycle")
    };
    assert_eq!(stage, "inference_heartbeat");
    let ChannelOutputEvent::Lifecycle {
        stage,
        message,
        iteration,
        max_iterations,
        ..
    } = iteration_start(2, 9)
    else {
        panic!("lifecycle")
    };
    assert_eq!(stage, "iteration_start");
    assert_eq!(message.as_deref(), Some("Iteration 2/9"));
    assert_eq!((iteration, max_iterations), (Some(2), Some(9)));
}

#[test]
fn native_progress_mapping_leaves_subagent_lifecycle_to_its_surface() {
    let event = turn_event(AgentProgress::SubagentSpawned {
        agent_id: "researcher".into(),
        task_id: "task-native".into(),
        mode: "typed".into(),
        dedicated_thread: false,
        prompt_chars: 4,
        prompt: "help".into(),
        worker_thread_id: None,
        display_name: Some("Researcher".into()),
        parent_call_id: Some("parent-call".into()),
    });
    assert!(event.is_none(), "native delivery keeps its existing filter");
}

#[test]
fn shared_subagent_mapping_uses_the_callers_correlation_context() {
    let event = subagent_event(
        AgentProgress::SubagentSpawned {
            agent_id: "researcher".into(),
            task_id: "task-web".into(),
            mode: "typed".into(),
            dedicated_thread: false,
            prompt_chars: 4,
            prompt: "help".into(),
            worker_thread_id: None,
            display_name: Some("Researcher".into()),
            parent_call_id: Some("event-parent-is-not-authoritative".into()),
        },
        3,
        Some("ledger-parent".into()),
    )
    .expect("subagent event has a canonical output");

    let out = expect_subagent(event);
    assert_eq!(out.state, "spawned");
    assert_eq!(out.agent_id, "researcher");
    assert_eq!(out.iteration, Some(3));
    assert_eq!(out.parent_call_id.as_deref(), Some("ledger-parent"));
}

#[test]
fn shared_subagent_mapping_ignores_unrelated_progress() {
    assert!(subagent_event(
        AgentProgress::TextDelta {
            delta: "hello".into(),
            iteration: 1,
        },
        1,
        None,
    )
    .is_none());
}

#[test]
fn a_started_tool_call_keeps_small_arguments_and_nulls_empty_ones() {
    let ChannelOutputEvent::ToolCallStarted { arguments, .. } = tool_started(
        "c".into(),
        "shell".into(),
        json!({ "cmd": "ls" }),
        1,
        Some("Listing".into()),
        None,
    ) else {
        panic!("tool call")
    };
    assert_eq!(arguments, json!({ "cmd": "ls" }));
    let ChannelOutputEvent::ToolCallStarted { arguments, .. } =
        tool_started("c".into(), "shell".into(), Value::Null, 1, None, None)
    else {
        panic!("tool call")
    };
    assert_eq!(arguments, Value::Null);
}

#[test]
fn a_finished_tool_call_caps_output_and_arguments_and_names_its_failure() {
    let failure = tinytools::status::classify("[not-found] no such file", false);
    let event = AgentProgress::ToolCallCompleted {
        call_id: "c".into(),
        tool_name: "file_read".into(),
        success: false,
        output_chars: 3,
        output: "x".repeat(MAX_WEB_TOOL_PAYLOAD_BYTES + 10),
        arguments: Some(json!({ "body": "y".repeat(MAX_WEB_TOOL_PAYLOAD_BYTES) })),
        elapsed_ms: 7,
        iteration: 2,
        failure: Some(failure.clone()),
        display_label: None,
        display_detail: Some("a.txt".into()),
        structured: Some(json!({ "kind": "probe" })),
    };
    let ChannelOutputEvent::ToolCallCompleted {
        output,
        arguments,
        failure: summary,
        elapsed_ms,
        iteration,
        structured,
        display_detail,
        success,
        ..
    } = tool_completed(event).expect("a completed call")
    else {
        panic!("tool result")
    };
    assert!(output.len() <= MAX_WEB_TOOL_PAYLOAD_BYTES && output.contains("truncated"));
    assert!(arguments
        .expect("arguments")
        .as_str()
        .is_some_and(|marker| marker.contains("truncated")));
    let summary = summary.expect("failure summary");
    assert_eq!(summary.cause_plain, failure.cause_plain);
    assert_eq!(summary.recoverable, failure.recoverable);
    assert!(!summary.class.is_empty() && !summary.category.is_empty());
    assert_eq!((elapsed_ms, iteration, success), (7, 2, false));
    assert_eq!(structured, Some(json!({ "kind": "probe" })));
    assert_eq!(display_detail.as_deref(), Some("a.txt"));

    assert!(tool_completed(AgentProgress::TurnStarted).is_none());
}

#[test]
fn a_cost_update_is_a_charged_usage_readout() {
    let ChannelOutputEvent::Usage {
        model,
        iteration,
        input_tokens,
        total_usd,
        ..
    } = cost_update("m".into(), 3, 10, 20, 5, 0.25)
    else {
        panic!("usage")
    };
    assert_eq!(model.as_deref(), Some("m"));
    assert_eq!(
        (iteration, input_tokens, total_usd),
        (Some(3), 10, Some(0.25))
    );
}

#[test]
fn a_spawned_subagent_is_attributed_to_the_parents_round_and_call() {
    let out = expect_subagent(sub_spawned(
        "researcher",
        "task-1",
        4,
        "typed".into(),
        true,
        120,
        Some("worker".into()),
        Some("Researcher".into()),
        Some("call-9".into()),
    ));
    assert_eq!(out.state, "spawned");
    assert_eq!(
        out.message.as_deref(),
        Some("Sub-agent 'Researcher' spawned")
    );
    assert_eq!(out.tool_name.as_deref(), Some("researcher"));
    assert_eq!((out.iteration, out.prompt_chars), (Some(4), Some(120)));
    assert_eq!(out.parent_call_id.as_deref(), Some("call-9"));
    assert_eq!(out.worker_thread_id.as_deref(), Some("worker"));
    assert_eq!(out.dedicated_thread, Some(true));
}

#[test]
fn a_finished_subagent_carries_its_iterations_result_spend_and_worktree() {
    let usage = tinyagents_harness::progress::SubagentUsage {
        input_tokens: 5,
        output_tokens: 6,
        cached_input_tokens: 1,
        charged_amount_usd: 0.5,
        cost_source: tinyagents_harness::cost::CostSource::Charged,
    };
    let out = expect_subagent(sub_completed(
        "researcher",
        "task-1",
        1_500,
        3,
        4,
        "done".into(),
        Some(&usage),
        Some("/w".into()),
        vec!["a.rs".into()],
        Some(true),
        Some("call-9".into()),
    ));
    assert_eq!(out.state, "completed");
    assert_eq!(out.iteration, Some(3), "the child's own iteration total");
    assert_eq!(
        out.message.as_deref(),
        Some("Sub-agent 'researcher' completed in 1500ms")
    );
    assert_eq!(out.output.as_deref(), Some("done"));
    assert_eq!(out.success, Some(true));
    let spend = out.usage.expect("spend");
    assert_eq!((spend.input_tokens, spend.charged_amount_usd), (5, 0.5));
    assert_eq!(out.worktree_path.as_deref(), Some("/w"));
    assert_eq!(
        (out.changed_files, out.dirty_status),
        (vec!["a.rs".to_string()], Some(true))
    );

    let none = expect_subagent(sub_completed(
        "r",
        "t",
        1,
        1,
        0,
        String::new(),
        None,
        None,
        Vec::new(),
        None,
        None,
    ));
    assert!(none.usage.is_none());
}

#[test]
fn an_unknown_cost_reads_as_zero_spend_not_missing_tokens() {
    let usage = tinyagents_harness::progress::SubagentUsage {
        input_tokens: 9,
        ..Default::default()
    };
    let out = expect_subagent(sub_completed(
        "r",
        "t",
        1,
        1,
        0,
        String::new(),
        Some(&usage),
        None,
        Vec::new(),
        None,
        None,
    ));
    let spend = out.usage.expect("tokens survive an unknown cost");
    assert_eq!((spend.input_tokens, spend.charged_amount_usd), (9, 0.0));
}

#[test]
fn failed_and_waiting_subagents_say_why() {
    let failed = expect_subagent(sub_failed("r", "t", 2, "boom".into(), Some("p".into())));
    assert_eq!(failed.state, "failed");
    assert_eq!(
        (failed.success, failed.message.as_deref()),
        (Some(false), Some("boom"))
    );
    assert_eq!(failed.parent_call_id.as_deref(), Some("p"));

    let waiting = expect_subagent(sub_awaiting_user(
        "r",
        "t",
        2,
        "which one?".into(),
        Some("w".into()),
        None,
    ));
    assert_eq!(waiting.state, "awaiting_user");
    assert_eq!(waiting.message.as_deref(), Some("which one?"));
    assert_eq!(waiting.worker_thread_id.as_deref(), Some("w"));
}

#[test]
fn a_subagent_iteration_hides_its_cap_under_the_extended_policy() {
    let capped = expect_subagent(sub_iteration("r", "t", 2, 8, false));
    assert_eq!(
        capped.message.as_deref(),
        Some("Sub-agent 'r' iteration 2/8")
    );
    assert_eq!(capped.max_iterations, Some(8));
    let extended = expect_subagent(sub_iteration("r", "t", 2, 8, true));
    assert_eq!(extended.message.as_deref(), Some("Sub-agent 'r' step 2"));
    assert_eq!(extended.max_iterations, None);
}

#[test]
fn subagent_tool_calls_carry_their_arguments_and_capped_results() {
    let started = expect_subagent(sub_tool_started(
        "r",
        "t",
        "call".into(),
        "web_search".into(),
        json!({ "q": 1 }),
        2,
        Some("Search".into()),
        None,
    ));
    assert_eq!(started.state, "tool_call");
    assert_eq!(started.arguments, Some(json!({ "q": 1 })));
    assert_eq!(started.call_id.as_deref(), Some("call"));

    let completed = expect_subagent(
        sub_tool_completed(AgentProgress::SubagentToolCallCompleted {
            agent_id: "r".into(),
            task_id: "t".into(),
            call_id: "call".into(),
            tool_name: "web_search".into(),
            success: true,
            output_chars: 4,
            output: "z".repeat(MAX_WEB_TOOL_PAYLOAD_BYTES + 1),
            arguments: Some(json!({ "q": 1 })),
            elapsed_ms: 3,
            iteration: 2,
            failure: None,
            display_label: None,
            display_detail: None,
            structured: Some(json!({ "kind": "web_search" })),
        })
        .expect("a completed sub-agent call"),
    );
    assert_eq!(completed.state, "tool_result");
    assert!(completed
        .output
        .is_some_and(|out| out.contains("truncated")));
    assert_eq!(completed.structured, Some(json!({ "kind": "web_search" })));
    assert!(sub_tool_completed(AgentProgress::TurnStarted).is_none());
}

#[test]
fn subagent_deltas_are_text_or_thinking() {
    let text = expect_subagent(sub_delta("r", "t", false, "hi".into(), 2));
    assert_eq!(
        (text.state.as_str(), text.message.as_deref()),
        ("text_delta", Some("hi"))
    );
    let thinking = expect_subagent(sub_delta("r", "t", true, "hm".into(), 2));
    assert_eq!(thinking.state, "thinking_delta");
    assert_eq!(thinking.iteration, Some(2));
}
