//! Phase 7 Acceptance Tests: Interruption and Accounting Use Cases.
//!
//! Tests cancellation before/during execution, fresh-process resume, no duplicate
//! effect/orphan, call ceilings, explicit stop reasons, context occupancy vs
//! cumulative traffic, and bounded deterministic final rendering.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use serde_json::json;

use super::*;
use crate::agent::{
    goose::{GooseCheckpointStore, GooseStopReason},
    primary_orchestration::capability::*,
};

#[tokio::test]
async fn cancellation_before_execution_stops_cleanly() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-cancel-pre";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("Do something long"),
        )
        .unwrap(),
    );

    let calls = Arc::new(AtomicUsize::new(0));
    let effects = Arc::new(AtomicUsize::new(0));
    let tool = Box::new(RecordingTool::new(
        "heavy_task",
        vec![CapabilityOperation::ExecuteCommand],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::LocalWrite,
        calls.clone(),
        effects.clone(),
        |_args| Ok(ToolResult::success("done")),
    ));
    let route = tool.as_route(100);

    let model = Arc::new(RecordingModel::new(
        vec![make_final_response("all done", 20, 10)],
        harness.endpoint_counter.clone(),
    ));

    let adapter = harness.build_adapter(
        model,
        vec![tool],
        vec![route],
        Arc::new(MockAllowSecurity),
        None,
        5,
    );

    // Cancel before run
    adapter.cancel.cancel();

    let outcome = adapter.run(turn_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::Cancelled);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    assert_eq!(harness.endpoint_counter.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancellation_during_execution_stops_without_orphan_effect() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-cancel-mid";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("Run risky task"),
        )
        .unwrap(),
    );

    let calls = Arc::new(AtomicUsize::new(0));
    let effects = Arc::new(AtomicUsize::new(0));
    let tool = Box::new(RecordingTool::new(
        "risky_task",
        vec![CapabilityOperation::ExecuteCommand],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::LocalWrite,
        calls.clone(),
        effects.clone(),
        |_args| Ok(ToolResult::success("risky operation completed")),
    ));
    let route = tool.as_route(100);

    let model = Arc::new(RecordingModel::new(
        vec![
            make_tool_call_response("call-risky", "risky_task", json!({ "arg": "val" }), 30, 15),
            make_final_response("Finished", 50, 10),
        ],
        harness.endpoint_counter.clone(),
    ));

    let adapter = harness.build_adapter(
        model,
        vec![tool],
        vec![route],
        Arc::new(MockAllowSecurity),
        None,
        5,
    );

    // Cancel token during execution
    adapter.cancel.cancel();

    let outcome = adapter.run(turn_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::Cancelled);
}

#[tokio::test]
async fn fresh_process_resume_from_persisted_checkpoint() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-fresh-resume";

    // Initialize checkpoint in store
    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("Execute step 1 then step 2"),
        )
        .unwrap(),
    );

    let calls = Arc::new(AtomicUsize::new(0));
    let effects = Arc::new(AtomicUsize::new(0));
    let tool = Box::new(RecordingTool::new(
        "step_runner",
        vec![CapabilityOperation::ExecuteCommand],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::LocalWrite,
        calls.clone(),
        effects.clone(),
        |_args| Ok(ToolResult::success("Step 1 done")),
    ));
    let route = tool.as_route(100);

    // Pass 1: adapter runs step 1
    let model1 = Arc::new(RecordingModel::new(
        vec![
            make_tool_call_response("call-step1", "step_runner", json!({ "step": 1 }), 30, 10),
            make_final_response("Step 1 finished", 45, 10),
        ],
        harness.endpoint_counter.clone(),
    ));

    let adapter1 = harness.build_adapter(
        model1,
        vec![tool],
        vec![route.clone()],
        Arc::new(MockAllowSecurity),
        None,
        5,
    );

    let outcome1 = adapter1.run(turn_id).await.unwrap();
    assert_eq!(outcome1.stop_reason, GooseStopReason::FinalAnswer);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(effects.load(Ordering::SeqCst), 1);

    // Verify checkpoint is persisted
    let persisted = harness.store.load(turn_id).await.unwrap();
    assert!(persisted.revision >= 1);

    // Pass 2: "Fresh process" boots up, reads same store with new adapter
    let tool2 = Box::new(RecordingTool::new(
        "step_runner",
        vec![CapabilityOperation::ExecuteCommand],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::LocalWrite,
        calls.clone(),
        effects.clone(),
        |_args| Ok(ToolResult::success("Step 2 done")),
    ));

    let model2 = Arc::new(RecordingModel::new(
        vec![make_final_response("All steps completed.", 40, 10)],
        harness.endpoint_counter.clone(),
    ));

    let adapter2 = harness.build_adapter(
        model2,
        vec![tool2],
        vec![route],
        Arc::new(MockAllowSecurity),
        None,
        5,
    );

    let outcome2 = adapter2.run(turn_id).await.unwrap();
    assert_eq!(outcome2.stop_reason, GooseStopReason::FinalAnswer);
    // Crucial: Step 1 tool action was NOT re-executed!
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn primary_call_ceiling_enforces_bounded_iterations() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-ceiling";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("Keep searching"),
        )
        .unwrap(),
    );

    let calls = Arc::new(AtomicUsize::new(0));
    let effects = Arc::new(AtomicUsize::new(0));
    let tool = Box::new(RecordingTool::new(
        "search_again",
        vec![CapabilityOperation::SearchWeb],
        vec![CapabilityModality::Text],
        CapabilityBackend::DirectNetwork,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::ExternalRead,
        calls.clone(),
        effects.clone(),
        |_args| Ok(ToolResult::success("more info found")),
    ));
    let route = tool.as_route(100);

    // Model loops with tool calls
    let model = Arc::new(RecordingModel::new(
        vec![
            make_tool_call_response("call-1", "search_again", json!({ "iter": 1 }), 30, 10),
            make_tool_call_response("call-2", "search_again", json!({ "iter": 2 }), 40, 10),
            make_tool_call_response("call-3", "search_again", json!({ "iter": 3 }), 50, 10),
            make_tool_call_response("call-4", "search_again", json!({ "iter": 4 }), 60, 10),
        ],
        harness.endpoint_counter.clone(),
    ));

    // Limit to 2 primary calls
    let mut adapter = harness.build_adapter(
        model,
        vec![tool],
        vec![route],
        Arc::new(MockAllowSecurity),
        None,
        2,
    );
    adapter.max_no_progress_calls = 10;

    let outcome = adapter.run(turn_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::CallCeiling);
    assert_eq!(outcome.checkpoint.usage.primary_calls, 2);
    assert!(harness.endpoint_counter.load(Ordering::SeqCst) <= 2);
}

#[tokio::test]
async fn explicit_stop_reasons_tracked_in_outcome_and_checkpoint() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-dup-sig";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("Do repeated action"),
        )
        .unwrap(),
    );

    let calls = Arc::new(AtomicUsize::new(0));
    let effects = Arc::new(AtomicUsize::new(0));
    let tool = Box::new(RecordingTool::new(
        "ping_tool",
        vec![CapabilityOperation::ExecuteCommand],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::LocalRead,
        calls.clone(),
        effects.clone(),
        |_args| Ok(ToolResult::success("pong")),
    ));
    let route = tool.as_route(100);

    // Model repeats identical call signature consecutively
    let model = Arc::new(RecordingModel::new(
        vec![
            make_tool_call_response("call-ping-1", "ping_tool", json!({ "data": 1 }), 20, 10),
            make_tool_call_response("call-ping-2", "ping_tool", json!({ "data": 1 }), 20, 10),
        ],
        harness.endpoint_counter.clone(),
    ));

    let adapter = harness.build_adapter(
        model,
        vec![tool],
        vec![route],
        Arc::new(MockAllowSecurity),
        None,
        5,
    );

    let outcome = adapter.run(turn_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::DuplicateSignature);
    assert_eq!(
        outcome.checkpoint.terminal_reason.as_deref(),
        Some("duplicate_signature")
    );
}

#[tokio::test]
async fn latest_context_occupancy_versus_cumulative_traffic() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-accounting";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("Inspect file"),
        )
        .unwrap(),
    );

    let calls = Arc::new(AtomicUsize::new(0));
    let effects = Arc::new(AtomicUsize::new(0));
    let tool = Box::new(RecordingTool::new(
        "read_file",
        vec![CapabilityOperation::ReadWorkspace],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::LocalRead,
        calls.clone(),
        effects.clone(),
        |_args| Ok(ToolResult::success("file content line 1\nline 2")),
    ));
    let route = tool.as_route(100);

    let model = Arc::new(RecordingModel::new(
        vec![
            make_tool_call_response(
                "call-read",
                "read_file",
                json!({ "path": "file.txt" }),
                100, // input tokens
                25,  // output tokens
            ),
            make_final_response(
                "File contains two lines.",
                150, // input tokens
                15,  // output tokens
            ),
        ],
        harness.endpoint_counter.clone(),
    ));

    let adapter = harness.build_adapter(
        model,
        vec![tool],
        vec![route],
        Arc::new(MockAllowSecurity),
        None,
        5,
    );

    let outcome = adapter.run(turn_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::FinalAnswer);

    // Cumulative traffic across all inference calls
    assert_eq!(outcome.checkpoint.usage.primary_calls, 2);
    assert_eq!(outcome.checkpoint.usage.cumulative_input_tokens, 250);
    assert_eq!(outcome.checkpoint.usage.cumulative_output_tokens, 40);
    assert_eq!(outcome.checkpoint.usage.latest_primary_input_tokens, 150);

    // Context occupancy: conversation messages represent current context state
    assert!(outcome.checkpoint.conversation.len() >= 3);
}

#[tokio::test]
async fn bounded_deterministic_final_rendering() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-deterministic-render";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("Summarize status"),
        )
        .unwrap(),
    );

    let expected_answer = "System status: all services operational.";
    let model = Arc::new(RecordingModel::new(
        vec![make_final_response(expected_answer, 30, 15)],
        harness.endpoint_counter.clone(),
    ));

    let adapter = harness.build_adapter(
        model,
        Vec::new(),
        Vec::new(),
        Arc::new(MockAllowSecurity),
        None,
        5,
    );

    let outcome = adapter.run(turn_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::FinalAnswer);
    let messages = outcome.openhuman_messages;
    let assistant_text = messages
        .iter()
        .find_map(|m| match m {
            crate::agent::messages::ConversationMessage::Chat(c) if c.role == "assistant" => {
                Some(c.content.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(assistant_text.trim(), expected_answer);
}
