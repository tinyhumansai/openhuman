use super::*;
use serde_json::json;

#[test]
fn raw_approval_security_registry_matches_its_independent_contract() {
    let contract = super::registry_test_support::BoundFamilyWireContract {
        namespaces: &["approval"],
        group: DomainGroup::Security,
        methods: &[
            ("approval.list_pending", "openhuman.approval_list_pending"),
            (
                "approval.list_recent_decisions",
                "openhuman.approval_list_recent_decisions",
            ),
            (
                "approval.list_tool_trust",
                "openhuman.approval_list_tool_trust",
            ),
            (
                "approval.revoke_tool_trust",
                "openhuman.approval_revoke_tool_trust",
            ),
            ("approval.decide", "openhuman.approval_decide"),
            (
                "approval.get_gate_state",
                "openhuman.approval_get_gate_state",
            ),
            (
                "approval.preauthorize_flow",
                "openhuman.approval_preauthorize_flow",
            ),
        ],
    };
    super::registry_test_support::verify_family_contract(
        registry(),
        internal_registry(),
        &contract,
        &crate::security::approval::approval_parameter_contracts(),
    )
    .expect("raw Approval entries retain their ordered wire contracts");
    let entries: Vec<_> = registry()
        .iter()
        .filter(|row| row.controller.schema.namespace == "approval")
        .collect();
    use TypeSchema::{Array, Bool, Option, Ref, String, U64};
    let fields = [
        (
            vec![("agent_id", Option(Box::new(String)), false)],
            vec![("pending", Array(Box::new(Ref("PendingApproval"))), true)],
        ),
        (
            vec![("limit", Option(Box::new(U64)), false)],
            vec![(
                "decisions",
                Array(Box::new(Ref("ApprovalAuditEntry"))),
                true,
            )],
        ),
        (
            vec![],
            vec![("grants", Array(Box::new(Ref("ToolTrustEntry"))), true)],
        ),
        (
            vec![("request_id", String, true)],
            vec![("revoked", Bool, true)],
        ),
        (
            vec![
                ("request_id", String, true),
                ("decision", String, true),
                ("agent_id", Option(Box::new(String)), false),
            ],
            vec![("decided", Ref("PendingApproval"), true)],
        ),
        (vec![], vec![("state", Ref("ApprovalGateBootState"), true)]),
        (
            vec![
                ("flow_id", String, true),
                ("tool_names", Array(Box::new(String)), true),
            ],
            vec![("result", Ref("FlowPreauthorizationResult"), true)],
        ),
    ];
    for (row, (inputs, outputs)) in entries.iter().zip(fields) {
        assert_eq!(
            crate::core::all::verify_wire_fields(&row.controller.schema.inputs, &inputs),
            Ok(()),
            "{} inputs",
            row.controller.schema.method_name()
        );
        assert_eq!(
            crate::core::all::verify_wire_fields(&row.controller.schema.outputs, &outputs),
            Ok(()),
            "{} outputs",
            row.controller.schema.method_name()
        );
    }
}

#[test]
fn raw_approval_decoders_keep_permissive_and_strict_wire_behavior() {
    let entries: Vec<_> = registry()
        .iter()
        .filter(|row| row.controller.schema.namespace == "approval")
        .map(|row| row.controller.clone())
        .collect();
    let methods = [
        "approval.list_pending",
        "approval.list_recent_decisions",
        "approval.list_tool_trust",
        "approval.revoke_tool_trust",
        "approval.decide",
        "approval.get_gate_state",
        "approval.preauthorize_flow",
    ];
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.schema.method_name())
            .collect::<Vec<_>>(),
        methods
    );
    for (entry, extra) in entries.iter().zip([
        json!({"agent_id": "  raw agent  ", "unknown": {"nested": [1, null]}}),
        json!({"limit": 0, "unknown": true}),
        json!({}),
        json!({"request_id": " grant-id "}),
        json!({"request_id": " raw id ", "decision": "deny", "extra": [false]}),
        json!({"anything": {"ignored": true}}),
        json!({"flow_id": " raw flow ", "tool_names": [" ", "tool"], "extra": null}),
    ]) {
        entry
            .decode_params(extra.as_object().unwrap().clone())
            .expect("actual bound decoder ignores undeclared parameter keys");
    }
    entries[0]
        .decode_params(Map::from_iter([("agent_id".into(), json!(false))]))
        .expect_err("bound schema decoder rejects wrong agent_id type");
    entries[1]
        .decode_params(Map::from_iter([("limit".into(), json!(-1))]))
        .expect_err("bound schema decoder rejects negative limit");
    entries[3]
        .decode_params(Map::new())
        .expect_err("bound schema decoder requires grant ID");
    entries[4]
        .decode_params(Map::from_iter([
            ("request_id".into(), json!("id")),
            ("decision".into(), json!(1)),
        ]))
        .expect_err("bound schema decoder enforces string wire types");
    entries[4]
        .decode_params(Map::from_iter([
            ("flow_id".into(), json!("flow")),
            ("tool_names".into(), json!([1])),
        ]))
        .expect_err("bound schema decoder enforces array item wire types");
}
