use super::*;
use crate::core::all::{
    assert_schema_controller_parity, expect_schema_fields, expect_schema_identity,
};

#[test]
fn notification_schema_contracts_are_preserved() {
    const FUNCTIONS: &[&str] = &[
        "ingest",
        "list",
        "mark_read",
        "settings_get",
        "settings_set",
        "dismiss",
        "mark_acted",
        "stats",
        "core_list",
        "core_mark_read",
    ];
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(names, FUNCTIONS);
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "ingest",
    );

    for function in ["dismiss", "mark_acted"] {
        let schema = schemas(function);
        expect_schema_identity(&schema, "notification", function);
        assert_eq!(schema.inputs.len(), 1);
        expect_schema_fields(&schema, &[("id", Some(true))]);
        assert_eq!(schema.inputs[0].ty, TypeSchema::String);
        assert_eq!(schema.outputs.len(), 1);
        assert_eq!(schema.outputs[0].name, "ok");
        assert_eq!(schema.outputs[0].ty, TypeSchema::Bool);
        assert!(schema.outputs[0].required);
    }

    let ingest = schemas("ingest");
    expect_schema_fields(
        &ingest,
        &[
            ("provider", Some(true)),
            ("title", Some(true)),
            ("body", Some(true)),
            ("raw_payload", Some(true)),
        ],
    );
    assert_eq!(ingest.namespace, "notification");
    assert!(schemas("list").inputs.iter().all(|field| !field.required));
    expect_schema_fields(&schemas("mark_read"), &[("id", Some(true))]);

    let stats = schemas("stats");
    assert!(stats.inputs.is_empty());
    assert_eq!(stats.outputs.len(), 5);
    for (name, ty) in [
        ("total", TypeSchema::I64),
        ("unread", TypeSchema::I64),
        ("unscored", TypeSchema::I64),
        ("by_provider", TypeSchema::Map(Box::new(TypeSchema::I64))),
        ("by_action", TypeSchema::Map(Box::new(TypeSchema::I64))),
    ] {
        let field = stats
            .outputs
            .iter()
            .find(|field| field.name == name)
            .unwrap();
        assert_eq!(field.ty, ty, "stats.{name}");
        assert!(field.required, "stats.{name}");
    }
}
