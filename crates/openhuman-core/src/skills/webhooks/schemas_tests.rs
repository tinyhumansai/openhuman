use super::*;
use crate::core::all::{
    assert_schema_controller_parity, expect_schema_fields, expect_schema_identity,
    expect_schema_input_order, expect_schema_output_names, expect_schema_required_inputs,
};

#[test]
fn webhook_schema_contracts_are_preserved() {
    crate::core::all::expect_schema_namespace(&all_controller_schemas(), "webhooks");
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "list_registrations",
    );
    let contracts = [
        ("list_registrations", &[][..], &[][..]),
        ("list_logs", &[][..], &["limit"][..]),
        ("clear_logs", &[][..], &[][..]),
        (
            "register_echo",
            &["tunnel_uuid"][..],
            &["tunnel_name", "backend_tunnel_id"][..],
        ),
        ("unregister_echo", &["tunnel_uuid"][..], &[][..]),
        (
            "register_agent",
            &["tunnel_uuid"][..],
            &["agent_id", "tunnel_name", "backend_tunnel_id"][..],
        ),
        (
            "trigger_agent",
            &["caller_id"][..],
            &["source", "reason", "payload"][..],
        ),
    ];
    for (function, required, optional) in contracts {
        let schema = schemas(function);
        expect_schema_identity(&schema, "webhooks", function);
        expect_schema_required_inputs(&schema, required);
        expect_schema_fields(
            &schema,
            &optional
                .iter()
                .map(|name| (*name, Some(false)))
                .collect::<Vec<_>>(),
        );
    }
    expect_schema_input_order(&schemas("list_logs"), &["limit"]);
    expect_schema_output_names(&schemas("list_registrations"), &["result"]);
    assert!(matches!(
        schemas("list_registrations").outputs[0].ty,
        TypeSchema::Json
    ));
    for function in ["register_echo", "register_agent"] {
        let schema = schemas(function);
        for name in if function == "register_echo" {
            &["tunnel_name", "backend_tunnel_id"][..]
        } else {
            &["agent_id", "tunnel_name", "backend_tunnel_id"][..]
        } {
            let field = schema
                .inputs
                .iter()
                .find(|field| field.name == *name)
                .unwrap();
            assert!(
                matches!(&field.ty, TypeSchema::Option(inner) if matches!(**inner, TypeSchema::String))
            );
        }
    }
    let limit = &schemas("list_logs").inputs[0].ty;
    assert!(matches!(limit, TypeSchema::Option(inner) if matches!(**inner, TypeSchema::U64)));
}

// ── deserialize_params ────────────────────────────────────────

#[test]
fn deserialize_params_returns_typed_struct_for_valid_input() {
    let mut params = Map::new();
    params.insert("tunnel_uuid".to_string(), Value::String("u-1".into()));
    params.insert("tunnel_name".to_string(), Value::String("n".into()));
    params.insert("backend_tunnel_id".to_string(), Value::Null);
    let parsed = deserialize_params::<WebhookRegisterEchoParams>(params).unwrap();
    assert_eq!(parsed.tunnel_uuid, "u-1");
    assert_eq!(parsed.tunnel_name.as_deref(), Some("n"));
    assert!(parsed.backend_tunnel_id.is_none());
}

#[test]
fn deserialize_params_reports_invalid_params_errors() {
    // Missing required `tunnel_uuid` for WebhookUnregisterEchoParams.
    let err = deserialize_params::<WebhookUnregisterEchoParams>(Map::new()).unwrap_err();
    assert!(
        err.contains("invalid params"),
        "expected 'invalid params' prefix, got: {err}"
    );
}
