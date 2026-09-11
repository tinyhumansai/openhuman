use super::*;

const BASE_CONTROLLERS: usize = 6;

#[test]
fn all_schemas_covers_the_base_surface() {
    assert_eq!(all_controller_schemas().len(), BASE_CONTROLLERS);
}

#[test]
fn all_controllers_covers_the_base_surface() {
    assert_eq!(all_registered_controllers().len(), BASE_CONTROLLERS);
}

#[test]
fn apify_linkedin_scrape_schema_shape() {
    let s = tools_schemas("tools_apify_linkedin_scrape");
    assert_eq!(s.namespace, "tools");
    assert_eq!(s.function, "apify_linkedin_scrape");
    assert!(s
        .inputs
        .iter()
        .any(|f| f.name == "profile_url" && f.required));
}

#[test]
fn composio_execute_schema_shape() {
    let s = tools_schemas("tools_composio_execute");
    assert_eq!(s.namespace, "tools");
    assert_eq!(s.function, "composio_execute");
    assert!(s.inputs.iter().any(|f| f.name == "action" && f.required));
}

#[test]
fn seltz_search_schema_shape() {
    let s = tools_schemas("tools_seltz_search");
    assert_eq!(s.namespace, "tools");
    assert_eq!(s.function, "seltz_search");
    assert!(s.inputs.iter().any(|f| f.name == "query" && f.required));
    assert!(s.inputs.iter().any(|f| f.name == "include_domains"));
    assert!(s.inputs.iter().any(|f| f.name == "scope"));
}

#[test]
fn querit_search_schema_shape() {
    let s = tools_schemas("tools_querit_search");
    assert_eq!(s.namespace, "tools");
    assert_eq!(s.function, "querit_search");
    assert!(s.inputs.iter().any(|f| f.name == "query" && f.required));
    assert!(s.inputs.iter().any(|f| f.name == "filters"));
    assert!(s.inputs.iter().any(|f| f.name == "count"));
    assert!(s.inputs.iter().any(|f| f.name == "include_domains"));
    assert!(s.inputs.iter().any(|f| f.name == "time_range"));
    assert!(s.inputs.iter().any(|f| f.name == "from_date"));
    assert!(s.inputs.iter().any(|f| f.name == "to_date"));
    assert!(s.inputs.iter().any(|f| f.name == "countries"));
    assert!(s.inputs.iter().any(|f| f.name == "languages"));
}

#[test]
fn searxng_search_schema_shape() {
    let s = tools_schemas("tools_searxng_search");
    assert_eq!(s.namespace, "tools");
    assert_eq!(s.function, "searxng_search");
    assert!(s.inputs.iter().any(|f| f.name == "query" && f.required));
    assert!(s.inputs.iter().any(|f| f.name == "categories"));
    assert!(s.inputs.iter().any(|f| f.name == "language"));
}

#[test]
fn optional_string_array_trims_and_drops_blank_entries() {
    let params = Map::from_iter([("categories".to_string(), json!([" web ", "", "  ", "news"]))]);

    let values = optional_string_array(&params, "categories").expect("string array");

    assert_eq!(values, vec!["web", "news"]);
}

#[test]
fn web_search_schema_shape() {
    let s = tools_schemas("tools_web_search");
    assert_eq!(s.namespace, "tools");
    assert_eq!(s.function, "web_search");
    assert!(s.inputs.iter().any(|f| f.name == "query" && f.required));
    // The resolved search provider is part of the documented output so
    // callers can attribute a managed search (#5136).
    assert!(s.outputs.iter().any(|f| f.name == "provider"));
}

#[test]
fn unknown_function_returns_unknown() {
    let s = tools_schemas("nonexistent");
    assert_eq!(s.function, "unknown");
}
