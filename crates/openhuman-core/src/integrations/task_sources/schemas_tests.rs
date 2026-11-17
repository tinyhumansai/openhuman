use super::*;
use crate::core::all::{
    assert_schema_controller_parity, expect_schema_fields, expect_schema_identity,
};
use serde_json::json;

#[test]
fn task_source_schema_contracts_and_registration_are_preserved() {
    crate::core::all::expect_schema_namespace(&all_controller_schemas(), "task_sources");
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(
        names,
        [
            "list",
            "get",
            "add",
            "update",
            "remove",
            "fetch",
            "sync",
            "list_tasks",
            "preview_filter",
            "list_databases",
            "status"
        ]
    );
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "list",
    );
    for function in names {
        expect_schema_identity(&schemas(function), "task_sources", function);
    }
}

#[test]
fn schemas_add_requires_provider_and_filter() {
    let s = schemas("add");
    expect_schema_fields(&s, &[("provider", Some(true)), ("filter", None)]);
}

#[test]
fn read_provider_parses_known_and_rejects_unknown() {
    let mut params = Map::new();
    params.insert("provider".into(), json!("notion"));
    assert_eq!(read_provider(&params).unwrap(), ProviderSlug::Notion);

    params.insert("provider".into(), json!("jira"));
    assert!(read_provider(&params).is_err());
}

#[test]
fn read_optional_handles_absent_null_and_value() {
    let mut params = Map::new();
    assert_eq!(read_optional::<u64>(&params, "limit").unwrap(), None);
    params.insert("limit".into(), Value::Null);
    assert_eq!(read_optional::<u64>(&params, "limit").unwrap(), None);
    params.insert("limit".into(), json!(7));
    assert_eq!(read_optional::<u64>(&params, "limit").unwrap(), Some(7));
}

// ── the unavailability marker (issue #6118) ─────────────────────────────────

/// The four controllers whose fetch path was removed must SAY so in the
/// description, because that string is what the source picker, the source list
/// and the agent tool surface read. A schema that still advertises the
/// capability keeps offering flows that cannot complete.
///
/// Paired with its opposite below: marking everything unavailable would satisfy
/// this test alone.
#[test]
fn the_four_unfetchable_controllers_declare_themselves_unavailable() {
    for function in ["fetch", "sync", "preview_filter", "list_databases"] {
        let description = schemas(function).description;
        assert!(
            description.starts_with("UNAVAILABLE:"),
            "{function} cannot succeed, so its description must lead with the marker: {description}"
        );
        assert!(
            description.contains("tinymemory v1.13.4"),
            "{function} must name why, so a reader can find the removal: {description}"
        );
    }
}

/// The seven working controllers must NOT carry the marker. This is what makes
/// the change above a scoped statement about four controllers rather than a
/// blanket "this domain is broken".
#[test]
fn the_working_controllers_are_not_marked_unavailable() {
    for function in [
        "list",
        "get",
        "add",
        "update",
        "remove",
        "list_tasks",
        "status",
    ] {
        let description = schemas(function).description;
        assert!(
            !description.contains("UNAVAILABLE"),
            "{function} works and must not be marked unavailable: {description}"
        );
    }
}
