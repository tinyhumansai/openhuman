//! Phase 7 Acceptance Tests: Memory, Repository, and Scheduling Use Cases.
//!
//! Tests chat with failed memory, explicit failed recall, healthy recall/store,
//! repository edits with verification, durable schedules, and denied approvals.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use serde_json::json;

use super::*;
use crate::agent::{
    goose::GooseStopReason,
    primary_orchestration::{capability::*, completion::CompletionContract},
};

#[tokio::test]
async fn unrelated_chat_with_failed_memory_completes_cleanly() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-chat-failed-mem";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("Hello, how are you today?"),
        )
        .unwrap(),
    );

    let mem_tool = Box::new(MemoryStubTool::failed("memory_tool"));
    let mem_route = mem_tool.as_route(false, 100);

    let model = Arc::new(RecordingModel::new(
        vec![make_final_response(
            "Hello! I am doing well, ready to help you.",
            25,
            12,
        )],
        harness.endpoint_counter.clone(),
    ));

    let adapter = harness.build_adapter(
        model,
        vec![mem_tool],
        vec![mem_route],
        Arc::new(MockAllowSecurity),
        Some(CompletionContract::Chat),
        5,
    );

    let outcome = adapter.run(turn_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::Completed);
    assert_eq!(harness.endpoint_counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn explicit_failed_recall_handles_tool_error_gracefully() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-explicit-failed-recall";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("What is my project code name from before?"),
        )
        .unwrap(),
    );

    let mem_tool = Box::new(MemoryStubTool::failed("recall_memory"));
    let mut mem_route = mem_tool.as_route(false, 100);
    mem_route.capability.availability = CapabilityAvailability::Available;

    let model = Arc::new(RecordingModel::new(
        vec![
            make_tool_call_response(
                "call-recall",
                "recall_memory",
                json!({ "query": "project code name" }),
                35,
                15,
            ),
            make_final_response(
                "I was unable to retrieve your project code name because the memory service is currently unavailable.",
                50,
                20,
            ),
        ],
        harness.endpoint_counter.clone(),
    ));

    let adapter = harness.build_adapter(
        model,
        vec![mem_tool],
        vec![mem_route],
        Arc::new(MockAllowSecurity),
        Some(CompletionContract::Chat),
        5,
    );

    let outcome = adapter.run(turn_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::Completed);
    assert_eq!(harness.endpoint_counter.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn healthy_recall_and_store_lifecycle() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-store-mem";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("Remember that my project is Bluebell."),
        )
        .unwrap(),
    );

    let mem_tool = Box::new(MemoryStubTool::healthy("store_memory", Vec::new()));
    let mem_store = mem_tool.memory_store.clone();
    let mem_route = mem_tool.as_route(true, 100);

    let model = Arc::new(RecordingModel::new(
        vec![
            make_tool_call_response(
                "call-store",
                "store_memory",
                json!({ "content": "Project name is Bluebell" }),
                30,
                15,
            ),
            make_final_response("I've noted that your project name is Bluebell.", 45, 15),
        ],
        harness.endpoint_counter.clone(),
    ));

    let adapter = harness.build_adapter(
        model,
        vec![mem_tool],
        vec![mem_route],
        Arc::new(MockAllowSecurity),
        Some(CompletionContract::Chat),
        5,
    );

    let outcome = adapter.run(turn_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::Completed);
    let stored = mem_store.lock().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0], "Project name is Bluebell");
}

#[tokio::test]
async fn approved_repository_edit_plus_verification() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-repo-edit";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("Update error handling in src/lib.rs"),
        )
        .unwrap(),
    );

    let patch_calls = Arc::new(AtomicUsize::new(0));
    let patch_effects = Arc::new(AtomicUsize::new(0));
    let patch_tool = Box::new(RecordingTool::new(
        "apply_patch",
        vec![CapabilityOperation::WriteWorkspace],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::LocalWrite,
        patch_calls.clone(),
        patch_effects.clone(),
        |_args| Ok(ToolResult::success("Hunk applied cleanly to src/lib.rs")),
    ));
    let patch_route = patch_tool.as_route(100);

    let verify_calls = Arc::new(AtomicUsize::new(0));
    let verify_effects = Arc::new(AtomicUsize::new(0));
    let verify_tool = Box::new(RecordingTool::new(
        "verify_build",
        vec![CapabilityOperation::ExecuteCommand],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::LocalRead,
        verify_calls.clone(),
        verify_effects.clone(),
        |_args| {
            Ok(ToolResult::success(
                json!({
                    "file_paths": ["src/lib.rs"],
                    "verification": "verified",
                    "summary": "cargo check and tests passed cleanly"
                })
                .to_string(),
            ))
        },
    ));
    let verify_route = verify_tool.as_route(90);

    let model = Arc::new(RecordingModel::new(
        vec![
            // 1. Model invokes patch
            make_tool_call_response(
                "call-patch",
                "apply_patch",
                json!({ "path": "src/lib.rs", "content": "// updated" }),
                50,
                20,
            ),
            // 2. Model invokes verify
            make_tool_call_response(
                "call-verify",
                "verify_build",
                json!({ "command": "cargo check" }),
                70,
                15,
            ),
            // 3. Model delivers final explanation
            make_final_response(
                "Refactored error handling in src/lib.rs and verified compilation.",
                85,
                18,
            ),
        ],
        harness.endpoint_counter.clone(),
    ));

    let contract = CompletionContract::RepositoryMutation {
        require_verification: true,
    };

    let adapter = harness.build_adapter(
        model,
        vec![patch_tool, verify_tool],
        vec![patch_route, verify_route],
        Arc::new(MockAllowSecurity),
        Some(contract),
        5,
    );

    let outcome = adapter.run(turn_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::Completed);
    assert_eq!(patch_calls.load(Ordering::SeqCst), 1);
    assert_eq!(patch_effects.load(Ordering::SeqCst), 1);
    assert_eq!(verify_calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.endpoint_counter.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn durable_schedule_id_and_status() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-schedule-sync";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("Schedule a recurring sync at 9 AM"),
        )
        .unwrap(),
    );

    let calls = Arc::new(AtomicUsize::new(0));
    let effects = Arc::new(AtomicUsize::new(0));
    let schedule_tool = Box::new(RecordingTool::new(
        "schedule_job",
        vec![CapabilityOperation::Schedule],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::LocalWrite,
        calls.clone(),
        effects.clone(),
        |_args| {
            Ok(ToolResult::success(
                json!({
                    "schedule_id": "cron-daily-sync-9am",
                    "state": "active",
                    "cron_expression": "0 9 * * *"
                })
                .to_string(),
            ))
        },
    ));
    let schedule_route = schedule_tool.as_route(100);

    let model = Arc::new(RecordingModel::new(
        vec![
            make_tool_call_response(
                "call-sched",
                "schedule_job",
                json!({ "name": "daily sync", "time": "09:00" }),
                40,
                15,
            ),
            make_final_response(
                "Scheduled your daily sync job for 9:00 AM (ID: cron-daily-sync-9am).",
                55,
                18,
            ),
        ],
        harness.endpoint_counter.clone(),
    ));

    let contract = CompletionContract::Scheduling;

    let adapter = harness.build_adapter(
        model,
        vec![schedule_tool],
        vec![schedule_route],
        Arc::new(MockAllowSecurity),
        Some(contract),
        5,
    );

    let outcome = adapter.run(turn_id).await.unwrap();

    assert_eq!(outcome.stop_reason, GooseStopReason::Completed);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn denied_approval_enforces_security_authority() {
    let harness = AcceptanceHarness::new();
    let turn_id = "turn-denied-delete";

    harness.store.insert(
        turn_id,
        crate::agent::goose::GooseTurnAdapter::checkpoint_from_openhuman(
            &AcceptanceHarness::initial_messages("Drop table users"),
        )
        .unwrap(),
    );

    let calls = Arc::new(AtomicUsize::new(0));
    let effects = Arc::new(AtomicUsize::new(0));
    let delete_tool = Box::new(RecordingTool::new(
        "drop_table",
        vec![CapabilityOperation::ExecuteCommand],
        vec![CapabilityModality::Text],
        CapabilityBackend::Local,
        MonetaryBoundary::NonMetered,
        CapabilitySideEffect::LocalWrite,
        calls.clone(),
        effects.clone(),
        |_args| Ok(ToolResult::success("table dropped")),
    ));
    let delete_route = delete_tool.as_route(100);

    let model = Arc::new(RecordingModel::new(
        vec![
            make_tool_call_response(
                "call-drop",
                "drop_table",
                json!({ "table": "users" }),
                30,
                10,
            ),
            make_final_response(
                "Action was denied by security policy: destructive database operations are forbidden.",
                45,
                15,
            ),
        ],
        harness.endpoint_counter.clone(),
    ));

    // Security gate denies the call
    let deny_security = Arc::new(MockDenySecurity::new(
        "Destructive database operations are forbidden by security policy",
    ));

    let adapter = harness.build_adapter(
        model,
        vec![delete_tool],
        vec![delete_route],
        deny_security,
        None,
        5,
    );

    let outcome = adapter.run(turn_id).await.unwrap();

    // Security policy remained the execution authority:
    // The tool was NEVER called, effect counter remains 0
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    assert_eq!(outcome.stop_reason, GooseStopReason::FinalAnswer);
}
