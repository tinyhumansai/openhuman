use super::*;
use crate::core::all::{
    assert_schema_controller_parity, ControllerHandler, ControllerParamContract,
};
use serde_json::json;

#[test]
fn legacy_handlers_keep_their_identity_and_order() {
    let expected: [(&str, ControllerHandler); 7] = [
        ("list_pending", handle_list_pending),
        ("list_recent_decisions", handle_list_recent_decisions),
        ("list_tool_trust", handle_list_tool_trust),
        ("revoke_tool_trust", handle_revoke_tool_trust),
        ("decide", handle_decide),
        ("get_gate_state", handle_get_gate_state),
        ("preauthorize_flow", handle_preauthorize_flow),
    ];
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), expected.len());
    for ((function, handler), controller) in expected.iter().zip(&controllers) {
        assert_eq!(controller.schema.namespace, "approval");
        assert_eq!(controller.schema.function, *function);
        assert!(std::ptr::fn_addr_eq(controller.handler, *handler));
    }
}

pub(super) fn parameter_contracts() -> Vec<ControllerParamContract> {
    vec![
        ControllerParamContract {
            method: "approval.list_pending",
            required: &[],
            optional: &["agent_id"],
            examples: vec![
                json!({}),
                json!({"agent_id": null}),
                json!({"agent_id": ""}),
                json!({"agent_id": " agent-1 "}),
            ],
        },
        ControllerParamContract {
            method: "approval.list_recent_decisions",
            required: &[],
            optional: &["limit"],
            examples: vec![
                json!({}),
                json!({"limit": null}),
                json!({"limit": 0}),
                json!({"limit": 500}),
            ],
        },
        ControllerParamContract {
            method: "approval.list_tool_trust",
            required: &[],
            optional: &[],
            examples: vec![json!({})],
        },
        ControllerParamContract {
            method: "approval.revoke_tool_trust",
            required: &["request_id"],
            optional: &[],
            examples: vec![json!({"request_id": "grant-1"})],
        },
        ControllerParamContract {
            method: "approval.decide",
            required: &["request_id", "decision"],
            optional: &["agent_id"],
            examples: vec![
                json!({"request_id": "request-1", "decision": "approve_once"}),
                json!({"request_id": " request-2 ", "decision": " deny ", "agent_id": null}),
            ],
        },
        ControllerParamContract {
            method: "approval.get_gate_state",
            required: &[],
            optional: &[],
            examples: vec![json!({})],
        },
        ControllerParamContract {
            method: "approval.preauthorize_flow",
            required: &["flow_id", "tool_names"],
            optional: &[],
            examples: vec![
                json!({"flow_id": "flow-1", "tool_names": []}),
                json!({"flow_id": " flow-2 ", "tool_names": ["", " tool ", "tool"]}),
            ],
        },
    ]
}

#[test]
fn revoke_tool_trust_parser_requires_a_string_id() {
    assert_eq!(
        parse_revoke_tool_trust(Map::from_iter([("request_id".into(), json!(" grant-1 "))]))
            .unwrap(),
        " grant-1 "
    );
    assert_eq!(
        parse_revoke_tool_trust(Map::new()).unwrap_err(),
        "missing required param 'request_id'"
    );
    assert_eq!(
        parse_revoke_tool_trust(Map::from_iter([("request_id".into(), json!(false))])).unwrap_err(),
        "invalid 'request_id': expected string, got bool"
    );
}

#[test]
fn approval_schema_catalog_matches_legacy_handler_order() {
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "list_pending",
    );
}

#[test]
fn list_pending_parser_trims_optional_agent_and_ignores_extras() {
    for (params, expected) in [
        (Map::new(), None),
        (Map::from_iter([("agent_id".into(), json!(null))]), None),
        (Map::from_iter([("agent_id".into(), json!("  "))]), None),
        (
            Map::from_iter([
                ("agent_id".into(), json!("  agent-a  ")),
                ("extra".into(), json!({"ignored": [true]})),
            ]),
            Some("agent-a".to_owned()),
        ),
    ] {
        assert_eq!(parse_list_pending(params).unwrap(), expected);
    }
    for value in [json!(false), json!(0), json!([]), json!({})] {
        assert_eq!(
            parse_list_pending(Map::from_iter([("agent_id".into(), value.clone())])).unwrap_err(),
            format!(
                "invalid 'agent_id': expected string, got {}",
                value_kind(&value)
            )
        );
    }
}

#[test]
fn recent_decisions_parser_keeps_optional_unsigned_limit_semantics() {
    assert_eq!(parse_list_recent_decisions(Map::new()).unwrap(), None);
    assert_eq!(
        parse_list_recent_decisions(Map::from_iter([("limit".into(), json!(null))])).unwrap(),
        None
    );
    for (limit, expected) in [(0_u64, 0_usize), (25, 25), (u64::MAX, usize::MAX)] {
        assert_eq!(
            parse_list_recent_decisions(Map::from_iter([("limit".into(), json!(limit))])).unwrap(),
            Some(expected),
            "the decoder preserves the existing cast without defaulting or clamping"
        );
    }
    for value in [json!(-1), json!(1.5)] {
        assert_eq!(
            parse_list_recent_decisions(Map::from_iter([("limit".into(), value)])).unwrap_err(),
            "invalid 'limit': expected unsigned integer"
        );
    }
    for (value, kind) in [
        (json!("10"), "string"),
        (json!(true), "bool"),
        (json!([]), "array"),
        (json!({}), "object"),
    ] {
        assert_eq!(
            parse_list_recent_decisions(Map::from_iter([("limit".into(), value)])).unwrap_err(),
            format!("invalid 'limit': expected unsigned integer, got {kind}")
        );
    }
}

#[test]
fn decide_parser_preserves_parse_order_raw_values_and_four_decisions() {
    for (spelling, expected) in [
        ("approve_once", ApprovalDecision::ApproveOnce),
        (
            "approve_always_for_tool",
            ApprovalDecision::ApproveAlwaysForTool,
        ),
        (
            "approve_always_for_flow",
            ApprovalDecision::ApproveAlwaysForFlow,
        ),
        ("deny", ApprovalDecision::Deny),
    ] {
        let (request_id, decision, agent_id) = parse_decide(Map::from_iter([
            ("request_id".into(), json!(" request-raw ")),
            ("decision".into(), json!(format!(" {spelling} "))),
            ("agent_id".into(), json!(" agent-raw ")),
            ("ignored".into(), json!({"nested": true})),
        ]))
        .unwrap();
        assert_eq!(request_id, " request-raw ");
        assert_eq!(decision, expected);
        assert_eq!(agent_id.as_deref(), Some("agent-raw"));
    }
    let (_, _, agent_id) = parse_decide(Map::from_iter([
        ("request_id".into(), json!("id")),
        ("decision".into(), json!("deny")),
        ("agent_id".into(), json!("   ")),
    ]))
    .unwrap();
    assert_eq!(agent_id, None);

    assert_eq!(
        parse_decide(Map::new()).unwrap_err(),
        "missing required param 'request_id'"
    );
    assert_eq!(
        parse_decide(Map::from_iter([("request_id".into(), json!("id"))])).unwrap_err(),
        "missing required param 'decision'"
    );
    assert_eq!(
        parse_decide(Map::from_iter([
            ("request_id".into(), json!("id")),
            ("decision".into(), json!("  nope  ")),
            ("agent_id".into(), json!(false)),
        ]))
        .unwrap_err(),
        "invalid 'decision': expected approve_once|approve_always_for_tool|approve_always_for_flow|deny, got '  nope  '"
    );
    assert_eq!(
        parse_decide(Map::from_iter([
            ("request_id".into(), json!("id")),
            ("decision".into(), json!("deny")),
            ("agent_id".into(), json!(false)),
        ]))
        .unwrap_err(),
        "invalid 'agent_id': expected string, got bool"
    );
    assert_eq!(
        parse_decide(Map::from_iter([("request_id".into(), json!(null))])).unwrap_err(),
        "invalid 'request_id': expected string, got null"
    );
    for (value, kind) in [
        (json!(null), "null"),
        (json!(false), "bool"),
        (json!(7), "number"),
        (json!([]), "array"),
        (json!({}), "object"),
    ] {
        assert_eq!(
            parse_decide(Map::from_iter([
                ("request_id".into(), value),
                ("decision".into(), json!(false)),
                ("agent_id".into(), json!(false)),
            ]))
            .unwrap_err(),
            format!("invalid 'request_id': expected string, got {kind}"),
            "request_id parsing precedes decision and agent_id errors"
        );
    }
    for (value, kind) in [
        (json!(null), "null"),
        (json!(false), "bool"),
        (json!(7), "number"),
        (json!([]), "array"),
        (json!({}), "object"),
    ] {
        assert_eq!(
            parse_decide(Map::from_iter([
                ("request_id".into(), json!("id")),
                ("decision".into(), value),
                ("agent_id".into(), json!(false)),
            ]))
            .unwrap_err(),
            format!("invalid 'decision': expected string, got {kind}"),
            "decision type validation precedes the invalid agent_id"
        );
    }
}

#[test]
fn gate_state_parser_ignores_the_entire_parameter_map() {
    assert_eq!(
        parse_get_gate_state(Map::from_iter([
            ("anything".into(), json!({"nested": [1, false]})),
            ("limit".into(), json!("wrong type")),
        ])),
        Ok(())
    );
}

#[test]
fn preauthorize_parser_preserves_flow_and_tool_strings_without_normalizing() {
    let (flow, tools) = parse_preauthorize_flow(Map::from_iter([
        ("flow_id".into(), json!(" flow raw ")),
        ("tool_names".into(), json!(["", " tool ", "tool", "tool"])),
        ("extra".into(), json!(true)),
    ]))
    .unwrap();
    assert_eq!(flow, " flow raw ");
    assert_eq!(
        tools,
        vec![
            String::new(),
            " tool ".to_owned(),
            "tool".to_owned(),
            "tool".to_owned()
        ]
    );
    assert_eq!(
        parse_preauthorize_flow(Map::from_iter([
            ("flow_id".into(), json!("")),
            ("tool_names".into(), json!([])),
        ]))
        .unwrap(),
        (String::new(), Vec::new()),
        "blank flow ids and empty batches are passed to the business layer unchanged"
    );
    let long_batch: Vec<_> = (0..101).map(|index| format!("tool-{index}")).collect();
    let (flow, parsed_batch) = parse_preauthorize_flow(Map::from_iter([
        ("flow_id".into(), json!("flow")),
        ("tool_names".into(), json!(long_batch)),
    ]))
    .unwrap();
    assert_eq!(flow, "flow");
    assert_eq!(parsed_batch.len(), 101);
    assert_eq!(parsed_batch[100], "tool-100");
    assert_eq!(
        parse_preauthorize_flow(Map::from_iter([("flow_id".into(), json!("flow"))])).unwrap_err(),
        "missing required param 'tool_names'"
    );
    assert_eq!(
        parse_preauthorize_flow(Map::from_iter([("tool_names".into(), json!([]))])).unwrap_err(),
        "missing required param 'flow_id'"
    );
    for value in [
        json!(null),
        json!(true),
        json!(7),
        json!("tools"),
        json!({}),
    ] {
        let kind = value_kind(&value);
        assert_eq!(
            parse_preauthorize_flow(Map::from_iter([
                ("flow_id".into(), json!("flow")),
                ("tool_names".into(), value),
            ]))
            .unwrap_err(),
            format!("invalid 'tool_names': expected array of strings, got {kind}")
        );
    }
    for value in [json!(null), json!(false), json!(0), json!([]), json!({})] {
        let kind = value_kind(&value);
        assert_eq!(
            parse_preauthorize_flow(Map::from_iter([
                ("flow_id".into(), json!("flow")),
                ("tool_names".into(), json!([value])),
            ]))
            .unwrap_err(),
            format!("invalid 'tool_names' entry: expected string, got {kind}")
        );
    }
    assert_eq!(
        parse_preauthorize_flow(Map::from_iter([
            ("flow_id".into(), json!(null)),
            ("tool_names".into(), json!([])),
        ]))
        .unwrap_err(),
        "invalid 'flow_id': expected string, got null"
    );
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
