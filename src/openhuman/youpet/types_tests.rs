//! Unit tests for YouPet workbench DTOs.

use serde_json::{json, Value};

use super::*;

#[test]
fn alert_status_and_severity_use_core_literals() {
    assert_eq!(
        serde_json::to_value(CoreAlertStatus::Acknowledged).unwrap(),
        json!("acknowledged")
    );
    assert_eq!(
        serde_json::to_value(CoreAlertSeverity::Critical).unwrap(),
        json!("critical")
    );
}

#[test]
fn alert_shape_tolerates_unknown_fields_but_requires_required_fields() {
    let alert: CoreWorkbenchAlert = serde_json::from_value(json!({
        "id": "alert-1",
        "alert_type": "missed_checkin",
        "severity": "high",
        "related_type": "task_instance",
        "related_id": "task-1",
        "status": "open",
        "created_at": "2026-06-01T00:00:00Z",
        "future_field": "tolerated"
    }))
    .unwrap();
    assert_eq!(alert.id, "alert-1");

    let err = serde_json::from_value::<CoreWorkbenchAlert>(json!({
        "alert_type": "missed_checkin",
        "severity": "high",
        "related_type": "task_instance",
        "related_id": "task-1",
        "status": "open",
        "created_at": "2026-06-01T00:00:00Z"
    }))
    .unwrap_err();
    assert!(err.to_string().contains("id"));
}

#[test]
fn alert_shape_accepts_optional_operational_context() {
    let alert: CoreWorkbenchAlert = serde_json::from_value(json!({
        "id": "alert-1",
        "alert_type": "missed_checkin",
        "severity": "high",
        "related_type": "task_instance",
        "related_id": "task-1",
        "status": "open",
        "created_at": "2026-06-01T00:00:00Z",
        "context": {
            "pet": {
                "id": "pet-1",
                "name": "Mochi",
                "species": "cat",
                "breed": null,
                "status": "active"
            },
            "owner": {
                "id": "owner-1",
                "name": "Owner A",
                "phone": null,
                "status": "active"
            },
            "health_plan": {
                "id": "plan-1",
                "title": "Daily check-in",
                "plan_type": "checkin",
                "status": "active",
                "openclaw_flow_id": "flow-plan-1"
            },
            "task": {
                "id": "task-1",
                "status": "missed",
                "due_at": "2026-06-01T10:01:00Z",
                "missed_count": 2,
                "openclaw_flow_id": null
            },
            "latest_checkin": {
                "id": "checkin-1",
                "submitted_at": "2026-06-01T10:10:00Z",
                "submitted_by": "owner-1",
                "text": "Looks normal.",
                "status_tags": ["normal"],
                "future_field": "tolerated"
            },
            "future_context_field": true
        }
    }))
    .unwrap();

    let context = alert.context.expect("context");
    assert_eq!(context.pet.name, "Mochi");
    assert_eq!(
        context.health_plan.openclaw_flow_id.as_deref(),
        Some("flow-plan-1")
    );
    assert_eq!(
        context.latest_checkin.expect("latest check-in").status_tags,
        vec!["normal".to_string()]
    );

    let unsupported: CoreWorkbenchAlert = serde_json::from_value(json!({
        "id": "alert-2",
        "alert_type": "outbox_dead_letter",
        "severity": "high",
        "related_type": "event_outbox",
        "related_id": "event-1",
        "status": "open",
        "created_at": "2026-06-01T00:00:00Z",
        "context": null
    }))
    .unwrap();
    assert!(unsupported.context.is_none());
}

#[test]
fn alert_list_requires_context_key_but_accepts_explicit_null() {
    let alert = json!({
        "id": "alert-1",
        "alert_type": "missed_checkin",
        "severity": "high",
        "related_type": "task_instance",
        "related_id": "task-1",
        "status": "open",
        "created_at": "2026-06-01T00:00:00Z"
    });

    let missing = serde_json::from_value::<CoreWorkbenchAlertsResponse>(json!({
        "items": [alert.clone()]
    }))
    .unwrap_err();
    assert!(missing.to_string().contains("must include context"));

    let mut with_null = alert;
    with_null
        .as_object_mut()
        .expect("alert object")
        .insert("context".to_string(), Value::Null);
    let response: CoreWorkbenchAlertsResponse =
        serde_json::from_value(json!({ "items": [with_null] })).unwrap();
    assert!(response.items[0].context.is_none());
}

#[test]
fn trace_shape_accepts_nullable_fields_warnings_metadata_and_unknowns() {
    let trace: WorkbenchAlertTrace = serde_json::from_value(json!({
        "alert_id": "alert-1",
        "workflow": {
            "type": "health_plan",
            "id": "plan-1",
            "task_id": "task-1",
            "openclaw_flow_id": "flow-plan-1"
        },
        "partial": true,
        "warnings": [{
            "code": "trace_truncated",
            "message": "Trace limited to 50 entries",
            "source": "event_outbox"
        }],
        "entries": [
            {
                "id": "event:event-1",
                "occurred_at": "2026-06-01T00:00:00Z",
                "kind": "outbox_event",
                "source": "event_outbox",
                "title": "Event emitted",
                "detail": null,
                "actor": { "type": "system", "id": null },
                "related_type": null,
                "related_id": null,
                "severity": null,
                "metadata": {
                    "event_type": "task.checkin_received",
                    "delivery_state": ["pending", "acked"]
                },
                "future_field": "tolerated"
            },
            {
                "id": "audit:ack-1",
                "occurred_at": "2026-06-01T00:01:00Z",
                "kind": "delivery_succeeded",
                "source": "audit_logs",
                "title": "Delivery succeeded",
                "metadata": {
                    "consumer": "openclaw",
                    "attempts": 0,
                    "recovered": false
                }
            }
        ]
    }))
    .unwrap();
    assert_eq!(
        trace.workflow.as_ref().map(|workflow| workflow.id.as_str()),
        Some("plan-1")
    );

    assert!(trace.partial);
    assert_eq!(
        trace.warnings[0].code,
        WorkbenchTraceWarningCode::TraceTruncated
    );
    assert_eq!(
        trace.warnings[0].source,
        Some(WorkbenchTraceSource::EventOutbox)
    );
    assert_eq!(trace.entries[0].kind, WorkbenchTraceEntryKind::OutboxEvent);
    assert_eq!(trace.entries[0].source, WorkbenchTraceSource::EventOutbox);
    assert_eq!(trace.entries[0].severity, None);
    assert_eq!(
        trace.entries[0].metadata["event_type"],
        json!("task.checkin_received")
    );
    assert_eq!(
        trace.entries[1].kind,
        WorkbenchTraceEntryKind::DeliverySucceeded
    );
    assert_eq!(trace.entries[1].source, WorkbenchTraceSource::AuditLogs);
    assert_eq!(trace.entries[1].metadata["attempts"], json!(0));
    assert_eq!(
        serde_json::to_value(&trace.entries[1].kind).unwrap(),
        json!("delivery_succeeded")
    );
}

#[test]
fn trace_shape_accepts_action_request_literals() {
    let trace: WorkbenchAlertTrace = serde_json::from_value(json!({
        "alert_id": "alert-1",
        "partial": true,
        "warnings": [{
            "code": "action_request_projection_truncated",
            "message": "ActionRequest projection limited to the latest request",
            "source": "action_requests"
        }, {
            "code": "invalid_action_request_projection",
            "message": "ActionRequest document could not be projected",
            "source": "action_requests"
        }, {
            "code": "missing_related_action_request",
            "message": "alert related action_request was not found",
            "source": "action_requests"
        }, {
            "code": "action_request_links_truncated",
            "message": "ActionRequest link identifiers limited to 3 values",
            "source": "action_requests"
        }],
        "entries": [
            {
                "id": "action-request:req-1:proposal",
                "occurred_at": "2026-06-01T00:01:00Z",
                "kind": "action_request_proposed",
                "source": "action_requests",
                "title": "ActionRequest proposed",
                "metadata": { "action_request_id": "req-1" }
            },
            {
                "id": "action-request:req-1:approved",
                "occurred_at": "2026-06-01T00:02:00Z",
                "kind": "action_request_approved",
                "source": "action_requests",
                "title": "ActionRequest approved",
                "metadata": { "approval_state": "approved" }
            },
            {
                "id": "action-request:req-2:rejected",
                "occurred_at": "2026-06-01T00:03:00Z",
                "kind": "action_request_rejected",
                "source": "action_requests",
                "title": "ActionRequest rejected",
                "metadata": { "approval_state": "rejected" }
            },
            {
                "id": "action-request:req-1:execution",
                "occurred_at": "2026-06-01T00:04:00Z",
                "kind": "action_request_execution",
                "source": "action_requests",
                "title": "ActionRequest execution succeeded",
                "metadata": { "execution_state": "succeeded" }
            }
        ]
    }))
    .unwrap();

    assert_eq!(
        trace.warnings[0].code,
        WorkbenchTraceWarningCode::ActionRequestProjectionTruncated
    );
    assert_eq!(
        trace.warnings[0].source,
        Some(WorkbenchTraceSource::ActionRequests)
    );
    assert_eq!(
        trace.warnings[1].code,
        WorkbenchTraceWarningCode::InvalidActionRequestProjection
    );
    assert_eq!(
        trace.warnings[2].code,
        WorkbenchTraceWarningCode::MissingRelatedActionRequest
    );
    assert_eq!(
        trace.warnings[3].code,
        WorkbenchTraceWarningCode::ActionRequestLinksTruncated
    );
    assert_eq!(
        trace.warnings[1].source,
        Some(WorkbenchTraceSource::ActionRequests)
    );
    assert_eq!(
        trace.entries[0].kind,
        WorkbenchTraceEntryKind::ActionRequestProposed
    );
    assert_eq!(
        trace.entries[1].kind,
        WorkbenchTraceEntryKind::ActionRequestApproved
    );
    assert_eq!(
        trace.entries[2].kind,
        WorkbenchTraceEntryKind::ActionRequestRejected
    );
    assert_eq!(
        trace.entries[3].kind,
        WorkbenchTraceEntryKind::ActionRequestExecution
    );
    assert_eq!(
        trace.entries[0].source,
        WorkbenchTraceSource::ActionRequests
    );
    assert_eq!(
        serde_json::to_value(&trace.entries[0].kind).unwrap(),
        json!("action_request_proposed")
    );
}

#[test]
fn trace_shape_tolerates_future_literals_without_dropping_the_trace() {
    let trace: WorkbenchAlertTrace = serde_json::from_value(json!({
        "alert_id": "alert-1",
        "partial": true,
        "warnings": [{
            "code": "future_warning",
            "message": "Future warning",
            "source": "future_source"
        }],
        "entries": [{
            "id": "future:1",
            "occurred_at": "2026-06-01T00:00:00Z",
            "kind": "future_kind",
            "source": "future_source",
            "title": "Future trace entry",
            "severity": "future_severity",
            "metadata": {}
        }]
    }))
    .unwrap();

    assert_eq!(
        trace.entries[0].kind,
        WorkbenchTraceEntryKind::Unknown("future_kind".to_string())
    );
    assert_eq!(
        trace.entries[0].source,
        WorkbenchTraceSource::Unknown("future_source".to_string())
    );
    assert_eq!(
        trace.warnings[0].code,
        WorkbenchTraceWarningCode::Unknown("future_warning".to_string())
    );
    assert_eq!(
        trace.warnings[0].source,
        Some(WorkbenchTraceSource::Unknown("future_source".to_string()))
    );
    assert_eq!(
        trace.entries[0].severity,
        Some(WorkbenchTraceSeverity::Unknown(
            "future_severity".to_string()
        ))
    );
    assert_eq!(
        serde_json::to_value(&trace.entries[0].kind).unwrap(),
        json!("future_kind")
    );
    assert_eq!(
        serde_json::to_value(trace.entries[0].severity.as_ref().unwrap()).unwrap(),
        json!("future_severity")
    );
}

#[test]
fn trace_params_parse_camel_case_alert_id() {
    let payload: TraceAlertRpcParams = serde_json::from_value(json!({
        "alertId": "alert-1"
    }))
    .unwrap();
    assert_eq!(payload.alert_id, "alert-1");
}

#[test]
fn status_filter_preserves_omitted_null_empty_and_specific() {
    let omitted: ListAlertsRpcParams = serde_json::from_value(json!({})).unwrap();
    assert_eq!(omitted.status, CoreAlertStatusFilter::Omitted);

    let all_from_null: ListAlertsRpcParams =
        serde_json::from_value(json!({ "status": null })).unwrap();
    assert_eq!(all_from_null.status, CoreAlertStatusFilter::All);

    let all_from_empty: ListAlertsRpcParams =
        serde_json::from_value(json!({ "status": " " })).unwrap();
    assert_eq!(all_from_empty.status, CoreAlertStatusFilter::All);

    let open: ListAlertsRpcParams = serde_json::from_value(json!({ "status": "open" })).unwrap();
    assert_eq!(
        open.status,
        CoreAlertStatusFilter::Status(CoreAlertStatus::Open)
    );
}
