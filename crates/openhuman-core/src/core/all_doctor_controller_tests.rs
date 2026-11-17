use super::*;

#[test]
fn raw_doctor_platform_registry_matches_its_independent_contract() {
    let contract = super::registry_test_support::BoundFamilyWireContract {
        namespaces: &["doctor"],
        group: DomainGroup::Platform,
        methods: &[
            ("doctor.report", "openhuman.doctor_report"),
            ("doctor.models", "openhuman.doctor_models"),
        ],
    };
    super::registry_test_support::verify_family_contract(
        registry(),
        internal_registry(),
        &contract,
        &crate::doctor::doctor_parameter_contracts(),
    )
    .expect("raw Doctor entries retain their ordered wire contracts");
    let entries: Vec<_> = registry()
        .iter()
        .filter(|row| row.controller.schema.namespace == "doctor")
        .collect();
    let use_cache = TypeSchema::Option(Box::new(TypeSchema::Bool));
    let report_inputs = [
        (
            "tier",
            TypeSchema::Option(Box::new(TypeSchema::Enum {
                variants: vec!["static", "connect", "live"],
            })),
            false,
        ),
        (
            "only",
            TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
            false,
        ),
        (
            "skip",
            TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
            false,
        ),
        (
            "profile",
            TypeSchema::Option(Box::new(TypeSchema::String)),
            false,
        ),
    ];
    assert_eq!(
        crate::core::all::verify_wire_fields(&entries[0].controller.schema.inputs, &report_inputs,),
        Ok(())
    );
    assert_eq!(
        crate::core::all::verify_wire_fields(
            &entries[0].controller.schema.outputs,
            &[("report", TypeSchema::Ref("DoctorReport"), true)]
        ),
        Ok(())
    );
    assert_eq!(
        crate::core::all::verify_wire_fields(
            &entries[1].controller.schema.inputs,
            &[("use_cache", use_cache, false)]
        ),
        Ok(())
    );
    assert_eq!(
        crate::core::all::verify_wire_fields(
            &entries[1].controller.schema.outputs,
            &[("report", TypeSchema::Ref("ModelProbeReport"), true)]
        ),
        Ok(())
    );
}

#[test]
fn raw_doctor_decoders_ignore_unknown_fields_and_keep_boolean_types_strict() {
    use serde_json::json;

    let entries: Vec<_> = registry()
        .iter()
        .filter(|row| row.controller.schema.namespace == "doctor")
        .map(|row| row.controller.clone())
        .collect();
    let report = entries
        .iter()
        .find(|entry| entry.schema.method_name() == "doctor.report")
        .expect("raw doctor.report entry");
    report
        .decode_params(Map::from_iter([
            ("use_cache".into(), json!("not a boolean")),
            ("nested_extra".into(), json!({"ignored":[1, false]})),
        ]))
        .expect("doctor.report ignores unknown fields and parses declared fields");

    let models = entries
        .iter()
        .find(|entry| entry.schema.method_name() == "doctor.models")
        .expect("raw doctor.models entry");
    models
        .decode_params(Map::from_iter([
            ("use_cache".into(), json!(false)),
            ("extra_scalar".into(), json!(true)),
            ("nested_extra".into(), json!({"ignored":true})),
        ]))
        .expect("doctor.models ignores undeclared fields");

    for invalid in [
        json!("false"),
        json!(1),
        json!([]),
        json!({"enabled":false}),
    ] {
        let error = models
            .decode_params(Map::from_iter([("use_cache".into(), invalid)]))
            .expect_err("use_cache accepts only JSON booleans or null");
        assert!(
            error.starts_with("invalid 'use_cache':"),
            "invalid model flag keeps its parser prefix: {error}"
        );
    }
}
