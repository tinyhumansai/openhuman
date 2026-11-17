use super::*;

#[test]
fn parse_embedded_returns_the_parsed_value() {
    let value = parse_embedded("test", r#"{"type":"object","properties":{}}"#);
    assert_eq!(value["type"], "object");
}

#[test]
#[should_panic(expected = "not valid JSON")]
fn parse_embedded_panics_with_site_on_malformed_json() {
    parse_embedded("test:1", "{ nope");
}

fn cached() -> serde_json::Value {
    static_schema!(r#"{"a":1}"#)
}

#[test]
fn static_schema_clones_independently() {
    let mut first = cached();
    first["a"] = serde_json::json!(2);
    assert_eq!(cached()["a"], 1);
}
