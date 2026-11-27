use super::*;

use crate::core::all::{
    assert_schema_controller_parity, expect_schema_fields, expect_schema_identity,
    expect_schema_output_fields,
};
const BASE_CONTROLLERS: usize = 4;

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
    assert_eq!(schemas("tools_apify_linkedin_scrape").function, "unknown");
}

#[test]
fn search_schema_wire_contracts_are_preserved() {
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "web_search",
    );
    let contracts: &[ToolSchemaContract] = &[
        ("web_answer", &[("query", Some(true)), ("depth", None)]),
        ("web_contents", &[("urls", Some(true))]),
        (
            "searxng_search",
            &[("query", Some(true)), ("max_results", None)],
        ),
        ("web_search", &[("query", Some(true))]),
    ];
    for (function, inputs) in contracts {
        let schema = schemas(&format!("tools_{function}"));
        expect_schema_identity(&schema, "tools", function);
        expect_schema_fields(&schema, inputs);
    }
    expect_schema_output_fields(&schemas("tools_web_answer"), &["citations"]);
    expect_schema_output_fields(&schemas("tools_web_search"), &["provider"]);
}
