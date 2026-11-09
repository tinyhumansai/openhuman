use super::*;

fn finished(data: Value) -> runs::RunFinished {
    serde_json::from_value(data).expect("valid RunFinished event")
}

#[test]
fn failed_graph_with_engine_output_keeps_the_legacy_success_payload() {
    let output = json!({"partial": [1, 2], "nodes": {"node": "result"}});
    let outcome = finished_outcome(
        finished(json!({
            "status": "failed",
            "successful": false,
            "error": "a graph step failed",
            "output": output,
            "timed_out": false
        })),
        "flow:example:run",
    )
    .expect("an engine RunOutcome remains the legacy RPC payload, even for failed status");

    assert_eq!(outcome.run_id, "flow:example:run");
    assert_eq!(
        outcome.output,
        json!({"partial": [1, 2], "nodes": {"node": "result"}})
    );
    assert!(outcome.pending_approvals.is_empty());
    assert_eq!(outcome.status, "failed");
}

#[test]
fn explicit_cancel_maps_to_the_legacy_null_output_without_changing_module_data() {
    let module_output = json!({"partial": ["kept in module event"]});
    let outcome = finished_outcome(
        finished(json!({
            "status": "cancelled",
            "successful": false,
            "error": "run cancelled",
            "output": module_output,
            "timed_out": false
        })),
        "flow:example:cancelled",
    )
    .expect("explicit cancellation preserves the old null result");

    assert_eq!(outcome.output, Value::Null);
    assert_eq!(outcome.status, "cancelled");
}

#[test]
fn timeout_metadata_maps_to_legacy_error_without_parsing_error_text() {
    let error = finished_outcome(
        finished(json!({
            "status": "cancelled",
            "successful": false,
            "error": "run exceeded its 600000 ms timeout",
            "output": {"partial": true},
            "timed_out": true
        })),
        "flow:example:timeout",
    )
    .expect_err("timeouts remain errors in the synchronous legacy RPC");

    assert_eq!(error, "run exceeded its 600000 ms timeout");
}

#[test]
fn missing_engine_outcome_is_not_reconstructed_as_an_empty_object() {
    let error = finished_outcome(
        finished(json!({
            "status": "failed",
            "successful": false,
            "error": "capability setup failed",
            "timed_out": false
        })),
        "flow:example:no-output",
    )
    .expect_err("absence of an engine outcome must keep the legacy error path");

    assert_eq!(error, "capability setup failed");
}

#[test]
fn pre_output_wire_payload_deserializes_with_safe_defaults() {
    let finished = finished(json!({
        "status": "failed",
        "successful": false,
        "error": "legacy module event",
        "graph_run_id": null
    }));

    assert_eq!(finished.output, None);
    assert!(!finished.timed_out);
}
