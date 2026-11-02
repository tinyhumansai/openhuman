use super::*;
use crate::core::all::ControllerParamContract;
use serde_json::{json, Map};

pub(crate) fn parameter_contracts() -> Vec<ControllerParamContract> {
    vec![
        ControllerParamContract {
            method: "keyring_consent.status",
            required: &[],
            optional: &[],
            examples: vec![json!({})],
        },
        ControllerParamContract {
            method: "keyring_consent.decide",
            required: &["mode"],
            optional: &[],
            examples: [
                "local_encrypted",
                "declined",
                "",
                "unknown",
                " local_encrypted ",
            ]
            .into_iter()
            .map(|mode| json!({ "mode": mode }))
            .collect(),
        },
        ControllerParamContract {
            method: "keyring_consent.retry_probe",
            required: &[],
            optional: &[],
            examples: vec![json!({})],
        },
        ControllerParamContract {
            method: "keyring_consent.respond_operation",
            required: &["request_id", "approved"],
            optional: &[],
            examples: vec![json!({ "request_id": "approval-1", "approved": true })],
        },
    ]
}

#[test]
fn schemas_and_controllers_match() {
    let s = all_keyring_consent_controller_schemas();
    let c = all_keyring_consent_registered_controllers();
    assert_eq!(s.len(), c.len());
    for (schema, ctrl) in s.iter().zip(c.iter()) {
        assert_eq!(schema.function, ctrl.schema.function);
        assert_eq!(schema.namespace, ctrl.schema.namespace);
    }
}

#[test]
fn all_schemas_use_keyring_consent_namespace() {
    for s in all_keyring_consent_controller_schemas() {
        assert_eq!(s.namespace, "keyring_consent");
        assert!(!s.description.is_empty());
    }
}

#[test]
fn decide_schema_requires_mode() {
    let s = keyring_consent_schema("decide");
    assert_eq!(s.inputs.len(), 1);
    assert!(s.inputs[0].required);
    assert_eq!(s.inputs[0].name, "mode");
    assert_eq!(s.inputs[0].ty, TypeSchema::String);
    assert_eq!(s.outputs.len(), 1);
    assert_eq!(s.outputs[0].name, "result");
    assert_eq!(s.outputs[0].ty, TypeSchema::Json);
    assert!(s.outputs[0].required);
}

#[test]
fn decide_parser_preserves_every_string_byte_for_byte() {
    for mode in [
        "local_encrypted",
        "declined",
        "",
        "unknown",
        " local_encrypted ",
    ] {
        let params = Map::from_iter([
            ("mode".into(), json!(mode)),
            ("ignored".into(), json!({"nested": [1, true]})),
        ]);
        assert_eq!(
            parse_decide(params).expect("all string values pass through unchanged"),
            mode,
        );
    }
}

#[test]
fn decide_parser_uses_exact_missing_error_for_absent_or_nonstring_mode() {
    let expected = "missing required param 'mode'";
    assert_eq!(parse_decide(Map::new()).unwrap_err(), expected);
    for value in [json!(null), json!(false), json!(0.5), json!([]), json!({})] {
        assert_eq!(
            parse_decide(Map::from_iter([("mode".into(), value)]))
                .unwrap_err()
                .as_str(),
            expected,
        );
    }
}

#[test]
fn status_and_retry_probe_parsers_ignore_the_entire_parameter_map() {
    let params = Map::from_iter([("ignored".into(), json!({"anything": [null, 3.5]}))]);
    assert_eq!(parse_status(params.clone()), Ok(()));
    assert_eq!(parse_retry_probe(params), Ok(()));
}

#[test]
fn operation_response_parser_requires_an_id_and_boolean_decision() {
    assert_eq!(
        parse_respond_operation(Map::from_iter([
            ("request_id".into(), json!("request-1")),
            ("approved".into(), json!(false)),
        ]))
        .unwrap(),
        ("request-1".into(), false),
    );
    assert_eq!(
        parse_respond_operation(Map::new()).unwrap_err(),
        "missing required param 'request_id'"
    );
    assert_eq!(
        parse_respond_operation(Map::from_iter([
            ("request_id".into(), json!("request-1")),
            ("approved".into(), json!("yes")),
        ]))
        .unwrap_err(),
        "missing required param 'approved'"
    );
}
