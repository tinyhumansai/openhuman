use super::*;

use crate::core::all::{
    assert_schema_controller_parity, expect_schema_fields, expect_schema_identity,
    expect_schema_output_fields,
};
use crate::tools::schemas::registry::tools_schemas;
use serde_json::{json, Map};
const BASE_CONTROLLERS: usize = 5;

type InputContract = (&'static str, Option<bool>);
type ToolSchemaContract = (&'static str, &'static [InputContract]);

#[test]
fn all_schemas_covers_the_base_surface() {
    assert_eq!(all_controller_schemas().len(), BASE_CONTROLLERS);
}

#[test]
fn all_controllers_covers_the_base_surface() {
    assert_eq!(all_registered_controllers().len(), BASE_CONTROLLERS);
}

#[test]
fn apify_linkedin_scrape_controller_is_retired() {
    // The backend retired its Apify routes; the controller must not be
    // registered or resolvable any more.
    assert!(!all_controller_schemas()
        .iter()
        .any(|s| s.function == "apify_linkedin_scrape"));
    assert_eq!(
        tools_schemas("tools_apify_linkedin_scrape").function,
        "unknown"
    );
}

#[test]
fn tools_schema_wire_contracts_are_preserved() {
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "web_search",
    );
    let contracts: &[ToolSchemaContract] = &[
        ("composio_execute", &[("action", Some(true))]),
        ("web_answer", &[("query", Some(true)), ("depth", None)]),
        ("web_contents", &[("urls", Some(true))]),
        (
            "searxng_search",
            &[("query", Some(true)), ("max_results", None)],
        ),
        ("web_search", &[("query", Some(true))]),
    ];
    for (function, inputs) in contracts {
        let schema = tools_schemas(&format!("tools_{function}"));
        expect_schema_identity(&schema, "tools", function);
        expect_schema_fields(&schema, inputs);
    }
    expect_schema_output_fields(&tools_schemas("tools_web_answer"), &["citations"]);
    expect_schema_output_fields(&tools_schemas("tools_web_search"), &["provider"]);
}

#[test]
fn optional_string_array_trims_and_drops_blank_entries() {
    let params = Map::from_iter([("categories".to_string(), json!([" web ", "", "  ", "news"]))]);

    let values = optional_string_array(&params, "categories").expect("string array");

    assert_eq!(values, vec!["web", "news"]);
}
