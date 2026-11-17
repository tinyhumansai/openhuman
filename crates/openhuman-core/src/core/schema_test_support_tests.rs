use super::*;
use crate::core::runtime::DispatchCtx;

fn parse_fixture(params: Map<String, Value>) -> Result<(), String> {
    match params.get("id").and_then(Value::as_str) {
        Some("example") => Ok(()),
        _ => Err("fixture id must be example".into()),
    }
}

fn panic_runner(_ctx: DispatchCtx, _parsed: ()) -> ControllerFuture {
    panic!("the pure contract verifier must never execute a business runner");
}

fn legacy_handler(_ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    panic!("legacy rejection must not execute a handler");
}

fn fixture() -> (ControllerEntry, ControllerParamContract) {
    let schema = ControllerSchema {
        namespace: "fixture",
        function: "read",
        description: "Read a fixture.",
        inputs: vec![
            crate::core::FieldSchema {
                name: "id",
                ty: crate::core::TypeSchema::String,
                comment: "Fixture identifier.",
                required: true,
            },
            crate::core::FieldSchema {
                name: "label",
                ty: crate::core::TypeSchema::String,
                comment: "Optional fixture label.",
                required: false,
            },
        ],
        outputs: vec![],
    };
    let controller = ControllerEntry::bound(BoundRegisteredController::typed(
        schema,
        parse_fixture,
        panic_runner,
    ));
    let contract = ControllerParamContract {
        method: "fixture.read",
        required: &["id"],
        optional: &["label"],
        examples: vec![
            serde_json::json!({"id":"example"}),
            serde_json::json!({"id":"example","label":"fixture"}),
        ],
    };
    (controller, contract)
}

#[test]
fn explicit_example_reaches_actual_bound_decoder_without_running_business_handler() {
    let (controller, contract) = fixture();
    verify_parameter_contracts(&[controller], &[contract]).unwrap();
}

#[test]
fn contract_inventory_rejects_missing_extra_duplicate_and_empty_rows() {
    let (controller, contract) = fixture();
    assert!(
        verify_parameter_contracts(std::slice::from_ref(&controller), &[])
            .unwrap_err()
            .contains("missing contract")
    );
    assert!(
        verify_parameter_contracts(&[], std::slice::from_ref(&contract))
            .unwrap_err()
            .contains("extra contract")
    );
    assert!(
        verify_parameter_contracts(&[controller], &[contract.clone(), contract])
            .unwrap_err()
            .contains("duplicate contract")
    );
    assert!(verify_parameter_contracts(&[], &[])
        .unwrap_err()
        .contains("empty"));
}

#[test]
fn contract_rejects_each_removed_or_optional_required_declaration() {
    let (controller, contract) = fixture();
    for remove in [true, false] {
        let mut broken = controller.clone();
        if remove {
            broken.schema.inputs.clear();
        } else {
            broken.schema.inputs[0].required = false;
        }
        assert!(
            verify_parameter_contracts(&[broken], std::slice::from_ref(&contract))
                .unwrap_err()
                .contains("required inputs mismatch")
        );
    }
}

#[test]
fn contract_rejects_removed_or_required_optional_declarations() {
    let (controller, contract) = fixture();
    for remove in [true, false] {
        let mut broken = controller.clone();
        if remove {
            broken.schema.inputs.retain(|field| field.name != "label");
        } else {
            broken
                .schema
                .inputs
                .iter_mut()
                .find(|field| field.name == "label")
                .expect("independent optional field is present")
                .required = true;
        }
        let expected_error = if remove {
            "optional inputs mismatch"
        } else {
            "required inputs mismatch"
        };
        assert!(
            verify_parameter_contracts(&[broken], std::slice::from_ref(&contract))
                .unwrap_err()
                .contains(expected_error)
        );
    }
}

#[test]
fn missing_examples_and_actual_parser_rejection_are_not_hidden() {
    let (controller, contract) = fixture();
    let mut broken = contract.clone();
    broken.examples.clear();
    assert!(
        verify_parameter_contracts(std::slice::from_ref(&controller), &[broken])
            .unwrap_err()
            .contains("no examples")
    );
    let mut broken = contract.clone();
    broken.examples = vec![serde_json::json!({"id":"wrong"})];
    assert!(
        verify_parameter_contracts(std::slice::from_ref(&controller), &[broken])
            .unwrap_err()
            .contains("bound decoder rejects example")
    );
    let mut broken = contract;
    broken.examples = vec![serde_json::json!({"id":7})];
    assert!(verify_parameter_contracts(&[controller], &[broken])
        .unwrap_err()
        .contains("schema rejects example"));
}

#[test]
fn duplicate_controller_and_schema_identity_failures_are_rejected() {
    let (controller, contract) = fixture();
    assert!(verify_parameter_contracts(
        &[controller.clone(), controller.clone()],
        std::slice::from_ref(&contract)
    )
    .unwrap_err()
    .contains("duplicate controller"));
    let mut broken = controller.schema.clone();
    broken.function = "other";
    assert!(verify_parameter_schema(&broken, &contract)
        .unwrap_err()
        .contains("method identity"));
    let mut broken = controller.schema;
    broken.description = " ";
    assert!(verify_parameter_schema(&broken, &contract)
        .unwrap_err()
        .contains("no description"));
}

#[test]
fn legacy_entries_are_rejected_without_running_the_legacy_handler() {
    let (bound, contract) = fixture();
    let legacy = ControllerEntry::legacy(RegisteredController {
        schema: bound.schema,
        handler: legacy_handler,
    });
    assert!(verify_parameter_contracts(&[legacy], &[contract])
        .unwrap_err()
        .contains("registry entry is legacy"));
}

#[test]
fn examples_must_be_objects_and_satisfy_schema_before_decoding() {
    let (controller, mut contract) = fixture();
    contract.examples = vec![Value::Null];
    assert!(verify_parameter_schema(&controller.schema, &contract)
        .unwrap_err()
        .contains("not an object"));
    contract.examples = vec![serde_json::json!({})];
    assert!(verify_parameter_schema(&controller.schema, &contract)
        .unwrap_err()
        .contains("schema rejects example"));
}

#[test]
fn real_owner_namespaces_reject_each_controller_namespace_typo() {
    let families = vec![(
        "inference",
        crate::inference::all_inference_controller_schemas(),
    )];
    #[cfg(feature = "mcp")]
    let families = families
        .into_iter()
        .chain([(
            "mcp_clients",
            crate::mcp::registry::all_mcp_registry_controller_schemas(),
        )])
        .collect::<Vec<_>>();
    for (expected, schemas) in families {
        assert!(!schemas.is_empty(), "{expected} owner fixture must exist");
        expect_schema_namespace(&schemas, expected);
        for mut schema in schemas {
            let function = schema.function;
            schema.namespace = "misspelled_owner";
            assert!(
                std::panic::catch_unwind(|| expect_schema_namespace(&[schema], expected)).is_err(),
                "{expected}.{function} namespace mutation escaped"
            );
        }
    }
}

#[test]
fn wire_field_contract_pins_count_order_type_and_requiredness() {
    let field = crate::core::FieldSchema {
        name: "items",
        ty: crate::core::TypeSchema::Option(Box::new(crate::core::TypeSchema::Array(Box::new(
            crate::core::TypeSchema::Ref("WireItem"),
        )))),
        comment: "Items in wire order.",
        required: false,
    };
    let expected = [(
        "items",
        crate::core::TypeSchema::Option(Box::new(crate::core::TypeSchema::Array(Box::new(
            crate::core::TypeSchema::Ref("WireItem"),
        )))),
        false,
    )];
    assert_eq!(
        verify_wire_fields(std::slice::from_ref(&field), &expected),
        Ok(())
    );

    let wrong_name = [("renamed", expected[0].1.clone(), false)];
    assert!(
        verify_wire_fields(std::slice::from_ref(&field), &wrong_name)
            .unwrap_err()
            .contains("name mismatch")
    );
    assert!(verify_wire_fields(std::slice::from_ref(&field), &[])
        .unwrap_err()
        .contains("count mismatch"));
    let wrong_required = [("items", expected[0].1.clone(), true)];
    assert!(
        verify_wire_fields(std::slice::from_ref(&field), &wrong_required)
            .unwrap_err()
            .contains("requiredness mismatch")
    );
    let wrong_nested_type = [(
        "items",
        crate::core::TypeSchema::Option(Box::new(crate::core::TypeSchema::Array(Box::new(
            crate::core::TypeSchema::Ref("OtherWireItem"),
        )))),
        false,
    )];
    assert!(
        verify_wire_fields(std::slice::from_ref(&field), &wrong_nested_type)
            .unwrap_err()
            .contains("type mismatch")
    );

    let reversed = [
        crate::core::FieldSchema {
            name: "second",
            ty: crate::core::TypeSchema::Bool,
            comment: "Second.",
            required: true,
        },
        crate::core::FieldSchema {
            name: "first",
            ty: crate::core::TypeSchema::String,
            comment: "First.",
            required: true,
        },
    ];
    let ordered = [
        ("first", crate::core::TypeSchema::String, true),
        ("second", crate::core::TypeSchema::Bool, true),
    ];
    assert!(verify_wire_fields(&reversed, &ordered)
        .unwrap_err()
        .contains("name mismatch"));
}
