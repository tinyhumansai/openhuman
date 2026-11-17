use super::*;
use serde_json::{json, Map};

fn tool_registry_entries() -> Vec<ControllerEntry> {
    registry()
        .iter()
        .filter(|row| row.controller.schema.namespace == "tool_registry")
        .map(|row| row.controller.clone())
        .collect()
}

#[test]
fn raw_tool_registry_matches_its_independent_contract() {
    let contract = registry_test_support::BoundFamilyWireContract {
        namespaces: &["tool_registry"],
        group: DomainGroup::Platform,
        methods: &[
            ("tool_registry.list", "openhuman.tool_registry_list"),
            ("tool_registry.get", "openhuman.tool_registry_get"),
            (
                "tool_registry.diagnostics",
                "openhuman.tool_registry_diagnostics",
            ),
        ],
    };
    let fixtures = [
        ControllerParamContract {
            method: "tool_registry.list",
            required: &[],
            optional: &[],
            examples: vec![json!({})],
        },
        ControllerParamContract {
            method: "tool_registry.get",
            required: &["tool_id"],
            optional: &[],
            examples: vec![json!({"tool_id":" tools.web_search "})],
        },
        ControllerParamContract {
            method: "tool_registry.diagnostics",
            required: &[],
            optional: &[],
            examples: vec![json!({})],
        },
    ];
    registry_test_support::verify_family_contract(
        registry(),
        internal_registry(),
        &contract,
        &fixtures,
    )
    .expect("raw Tool Registry entries retain ordered bound wire contracts");

    let entries = tool_registry_entries();
    use TypeSchema::{Array, Json, String};
    let expected = [
        (vec![], vec![("tools", Array(Box::new(Json)), true)]),
        (vec![("tool_id", String, true)], vec![("tool", Json, true)]),
        (vec![], vec![("diagnostics", Json, true)]),
    ];
    assert_eq!(entries.len(), expected.len());
    for (entry, (inputs, outputs)) in entries.iter().zip(expected) {
        assert_eq!(
            crate::core::all::verify_wire_fields(&entry.schema.inputs, &inputs),
            Ok(()),
            "{} inputs",
            entry.schema.method_name()
        );
        assert_eq!(
            crate::core::all::verify_wire_fields(&entry.schema.outputs, &outputs),
            Ok(()),
            "{} outputs",
            entry.schema.method_name()
        );
    }
}

#[test]
fn raw_tool_registry_decoders_keep_ignored_maps_and_tool_id_errors() {
    let entries = tool_registry_entries();
    let list = entries
        .iter()
        .find(|entry| entry.schema.function == "list")
        .expect("raw list entry");
    let diagnostics = entries
        .iter()
        .find(|entry| entry.schema.function == "diagnostics")
        .expect("raw diagnostics entry");
    let ignored = Map::from_iter([("ignored".into(), json!({"nested": true}))]);
    list.decode_params(ignored.clone())
        .expect("list ignores its entire parameter map");
    diagnostics
        .decode_params(ignored)
        .expect("diagnostics ignores its entire parameter map");

    let get = entries
        .iter()
        .find(|entry| entry.schema.function == "get")
        .expect("raw get entry");
    get.decode_params(Map::from_iter([
        ("tool_id".into(), json!(" tools.web_search ")),
        ("ignored".into(), json!(true)),
    ]))
    .expect("get accepts a padded id and ignores extra fields");
    for params in [
        Map::new(),
        Map::from_iter([("tool_id".into(), json!(null))]),
        Map::from_iter([("tool_id".into(), json!(false))]),
        Map::from_iter([("tool_id".into(), json!(10))]),
        Map::from_iter([("tool_id".into(), json!([]))]),
        Map::from_iter([("tool_id".into(), json!({}))]),
        Map::from_iter([("tool_id".into(), json!("  "))]),
    ] {
        assert_eq!(
            get.decode_params(params).unwrap_err(),
            "tool_id must be a non-empty string"
        );
    }
}

#[test]
fn raw_tool_registry_get_rejects_invalid_id_before_lookup() {
    let get = tool_registry_entries()
        .into_iter()
        .find(|entry| entry.schema.function == "get")
        .expect("raw get entry");
    for params in [
        Map::new(),
        Map::from_iter([("tool_id".into(), json!(false))]),
        Map::from_iter([("tool_id".into(), json!("  "))]),
    ] {
        let error = futures::executor::block_on(get.invoke(DispatchCtx::unbound(), params))
            .expect_err("invalid id must stop before tool lookup");
        assert_eq!(error, "tool_id must be a non-empty string");
    }
}
