//! Persisted session records are available through the public embed facade.

use openhuman_embed::session_store::{
    AgentEvent, AgentObservation, ExecutionStatus, HarnessRunStatus, JournalMessage,
    SessionDescriptor, TodoList,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};

fn roundtrip<T: DeserializeOwned + Serialize>(wire: Value) -> T {
    let decoded: T = serde_json::from_value(wire.clone()).expect("persisted record decodes");
    assert_eq!(
        serde_json::to_value(&decoded).expect("record encodes"),
        wire
    );
    decoded
}

#[test]
fn public_session_codecs_preserve_todo_items_and_message_metadata() {
    let todos = roundtrip::<TodoList>(json!({
        "threadId": "thread-1",
        "items": [{"content": "Read the report", "status": "in_progress"}],
        "updatedAt": "1720000000000"
    }));
    assert_eq!(todos.items[0].content, "Read the report");
    let message = roundtrip::<JournalMessage>(json!({
        "id": "message-1", "role": "assistant", "content": "Report ready",
        "extra_metadata": {"openhuman_turn_usage": {"input_tokens": 27}, "host": "cloud"}
    }));
    assert_eq!(message.id.as_deref(), Some("message-1"));
}

#[test]
fn public_session_codecs_preserve_event_lineage_and_execution_status() {
    let event = json!({"kind": "run_started", "run_id": "run-2", "thread_id": "thread-1"});
    let decoded = roundtrip::<AgentEvent>(event.clone());
    assert!(matches!(decoded, AgentEvent::RunStarted { .. }));
    let observation = roundtrip::<AgentObservation>(json!({
        "event_id": "event-1", "run_id": "run-2", "parent_run_id": "run-1",
        "root_run_id": "run-1", "offset": 4, "ts_ms": 1720000000000_u64,
        "event": event
    }));
    assert_eq!(observation.offset, 4);
    for label in [
        "pending",
        "running",
        "interrupted",
        "completed",
        "failed",
        "cancelled",
        "drained",
    ] {
        roundtrip::<ExecutionStatus>(json!(label));
    }
}

#[test]
fn public_session_codecs_preserve_run_status_counters_and_provenance() {
    let status = roundtrip::<HarnessRunStatus>(json!({
        "run_id": "run-2", "parent_run_id": "run-1", "root_run_id": "run-1",
        "thread_id": "thread-1", "component": "model-1", "status": "completed",
        "current_phase": "idle", "model_calls": 2, "tool_calls": 1,
        "active_model_call": "call-2", "active_tool_calls": ["tool-1"], "last_event_id": "event-9",
        "usage": {"calls": 2, "usage": {
            "input_tokens": 27, "output_tokens": 9, "total_tokens": 36,
            "cache_read_tokens": 12, "cache_creation_tokens": 8, "reasoning_tokens": 3,
            "charged_amount": {"micros": 1250}, "context_window_tokens": 32000
        }},
        "cost": {"input_cost": 0.1, "output_cost": 0.2, "cache_cost": 0.03,
            "reasoning_cost": 0.04, "total_cost": 0.37},
        "started_at": 1720000000, "updated_at": 1720000009, "ended_at": 1720000009,
        "error": "provider interruption", "metadata": {"host": "cloud"}
    }));
    assert_eq!(status.status, ExecutionStatus::Completed);
    assert_eq!(status.usage.usage.input_tokens, 27);
}

#[test]
fn public_session_codecs_preserve_descriptor_generation_and_import_details() {
    let descriptor = roundtrip::<SessionDescriptor>(json!({
        "session_key": "session-2", "parent_session_key": "session-1",
        "thread_id": "thread-1", "thread_id_synthesized": false, "task_id": "task-1",
        "run_ids": ["run-1", "run-2"], "stream": "session.session-2",
        "dispatcher": "agent", "agent_name": "assistant", "agent_id": "agent-1",
        "agent_type": "assistant", "provider": "openai", "model": "model-1",
        "created": "2024-07-03T09:00:00Z", "updated": "2024-07-03T09:05:00Z", "turn_count": 2,
        "usage": {"input": 27, "output": 9, "cached_input": 12, "cost_usd": 0.37},
        "source": {"jsonl": "session_raw/session-2.jsonl", "md": "session_raw/session-2.md"},
        "import": {"version": 1, "imported_at": "2024-07-03T10:00:00Z", "warnings": 1}
    }));
    assert_eq!(descriptor.parent_session_key.as_deref(), Some("session-1"));
    assert_eq!(descriptor.run_ids, ["run-1", "run-2"]);
}

#[test]
fn public_session_codecs_reject_incomplete_or_unknown_persisted_records() {
    assert!(serde_json::from_value::<ExecutionStatus>(json!("unknown")).is_err());
    assert!(serde_json::from_value::<JournalMessage>(json!({"role": "assistant"})).is_err());
    assert!(serde_json::from_value::<AgentEvent>(json!({"kind": "run_started"})).is_err());
}
