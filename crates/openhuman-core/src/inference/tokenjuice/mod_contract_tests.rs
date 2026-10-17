use tinyjuice_bus::types::ContentKind;
use tinyjuice_bus::types::{CompressOptions, ContentHint};

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
            "explicit": "plainText",
            // Added to the contract alongside the source-stub work. `Exact`
            // is the default and the shape the module expects; pinning it
            // here keeps this test doing its job — catching a wire change
            // rather than absorbing one.
            "readIntent": "Exact"
        })
    );
}
