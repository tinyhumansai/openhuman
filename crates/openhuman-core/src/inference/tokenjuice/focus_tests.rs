use super::*;
use serde_json::json;

#[test]
fn the_focus_is_taken_out_of_the_arguments() {
    let mut args = json!({ "url": "https://example.com", "summary_focus": "  the pricing  " });
    assert_eq!(
        take_summary_focus(&mut args).as_deref(),
        Some("the pricing")
    );
    assert_eq!(args, json!({ "url": "https://example.com" }));
}

#[test]
fn a_blank_or_non_string_focus_is_dropped_and_still_removed() {
    for value in [json!("   "), json!(42), json!(null)] {
        let mut args = json!({ "url": "u", "summary_focus": value });
        assert_eq!(take_summary_focus(&mut args), None);
        assert_eq!(args, json!({ "url": "u" }));
    }
}

#[test]
fn arguments_without_a_focus_are_untouched() {
    let mut args = json!({ "url": "u" });
    assert_eq!(take_summary_focus(&mut args), None);
    assert_eq!(args, json!({ "url": "u" }));
    let mut not_an_object = json!("raw");
    assert_eq!(take_summary_focus(&mut not_an_object), None);
}

#[test]
fn the_property_is_an_optional_string() {
    let property = summary_focus_property();
    assert_eq!(property["type"], "string");
    assert!(property["description"]
        .as_str()
        .unwrap()
        .contains("summarized"));
}

#[test]
fn only_the_shared_property_counts_as_declaring_the_focus() {
    let declared =
        json!({ "properties": { "url": {}, "summary_focus": summary_focus_property() } });
    assert!(declares_summary_focus(&declared));

    let own_parameter = json!({ "properties": { "summary_focus": { "type": "integer" } } });
    assert!(!declares_summary_focus(&own_parameter));
    assert!(!declares_summary_focus(
        &json!({ "properties": { "url": {} } })
    ));
    assert!(!declares_summary_focus(&json!({})));
}

#[test]
fn the_property_schema_is_pinned_literally() {
    assert_eq!(
        summary_focus_property(),
        json!({
            "type": "string",
            "description": "What you need from this result. A large result is summarized around this; the full output stays retrievable."
        })
    );
    assert_eq!(SUMMARY_FOCUS_ARG, "summary_focus");
}
