use super::*;

#[test]
fn compress_options_accept_omitted_fields() {
    let options: CompressOptions = serde_json::from_value(serde_json::json!({
        "routerEnabled": false
    }))
    .expect("partial options remain forward-compatible");
    assert!(!options.router_enabled);
    assert!(options.ccr_enabled);
    assert_eq!(options.min_bytes_to_compress, 2048);
}

#[test]
fn content_hint_matches_the_module_wire_shape() {
    let hint = ContentHint {
        source_tool: Some("shell".to_string()),
        explicit: Some(ContentKind::PlainText),
        ..ContentHint::default()
    };
    assert_eq!(
        serde_json::to_value(hint).expect("serialize hint"),
        serde_json::json!({
            "mime": null,
            "extension": null,
            "sourceTool": "shell",
            "query": null,
            "explicit": "plainText"
        })
    );
}

#[test]
fn retrieve_range_uses_camel_case_wire_values() {
    let range = RetrieveRange {
        start: 2,
        end: 5,
        unit: RangeUnit::Lines,
    };
    assert_eq!(
        serde_json::to_value(range).expect("serialize range"),
        serde_json::json!({ "start": 2, "end": 5, "unit": "lines" })
    );
}
