use super::*;
use serde_json::json;

#[test]
fn schemas_and_controllers_match() {
    let controllers = all_internal_controllers();
    assert_eq!(controllers.len(), 18);
    for controller in controllers {
        let expected = if controller.schema.function.starts_with("registry_") {
            crate::openhuman::youpet::registry::registry_schemas(controller.schema.function)
        } else {
            youpet_schemas(controller.schema.function)
        };
        assert_eq!(controller.schema, expected);
    }
}

#[test]
fn action_params_parse_camel_case() {
    let payload: AlertActionRpcParams = serde_json::from_value(json!({
        "alertId": "11111111-1111-4111-8111-111111111111",
        "idempotencyKey": "idem-1",
        "note": "checking"
    }))
    .unwrap();
    assert_eq!(payload.alert_id, "11111111-1111-4111-8111-111111111111");
    assert_eq!(payload.idempotency_key.as_deref(), Some("idem-1"));
}

#[test]
fn action_schema_does_not_request_renderer_actor_user_id() {
    for function in ["ack_alert", "resolve_alert"] {
        let schema = youpet_schemas(function);
        let actor_field = schema
            .inputs
            .iter()
            .find(|field| field.name == "actorUserId");
        assert!(
            actor_field.is_none(),
            "{function} must not expose actorUserId to renderer callers"
        );
        assert!(
            schema
                .inputs
                .iter()
                .any(|field| field.name == "alertId" && field.required),
            "{function} must still require alertId"
        );
    }
}

#[test]
fn list_params_preserve_null_status() {
    let payload: ListAlertsRpcParams = serde_json::from_value(json!({
        "status": null,
        "severity": "critical"
    }))
    .unwrap();
    assert_eq!(
        payload.status,
        crate::openhuman::youpet::types::CoreAlertStatusFilter::All
    );
}

#[test]
fn trace_schema_requires_only_alert_id() {
    let schema = youpet_schemas("trace_alert");
    assert_eq!(schema.function, "trace_alert");
    assert_eq!(schema.inputs.len(), 1);
    assert_eq!(schema.inputs[0].name, "alertId");
    assert!(schema.inputs[0].required);
    assert_eq!(schema.inputs[0].ty, TypeSchema::String);
}

#[test]
fn trace_params_parse_camel_case() {
    let payload: TraceAlertRpcParams = serde_json::from_value(json!({
        "alertId": "11111111-1111-4111-8111-111111111111"
    }))
    .unwrap();
    assert_eq!(payload.alert_id, "11111111-1111-4111-8111-111111111111");
}
