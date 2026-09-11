use super::*;
use serde_json::json;

#[test]
fn an_mcp_result_maps_across_with_its_error_flag_intact() {
    let ok = tool_result_from_mcp(tinymcp_bus::McpToolResult {
        content: vec![tinymcp_bus::McpToolContent::Text {
            text: "fine".into(),
        }],
        is_error: false,
        markdown_formatted: None,
    });
    assert!(!ok.is_error);
    assert_eq!(ok.text(), "fine");

    let failed = tool_result_from_mcp(tinymcp_bus::McpToolResult {
        content: vec![tinymcp_bus::McpToolContent::Text {
            text: "boom".into(),
        }],
        is_error: true,
        markdown_formatted: Some("**boom**".into()),
    });
    assert!(failed.is_error);
    assert_eq!(failed.markdown_formatted.as_deref(), Some("**boom**"));
}

#[test]
fn an_oversized_pass_through_block_is_elided_but_keeps_its_type() {
    // A base64 image or audio block can be megabytes; a model should see
    // what kind of block it was, not the bytes.
    let block = tinymcp_bus::McpToolContent::Json {
        data: json!({"base64": "x".repeat(70 * 1024)}),
    };
    let value = elide_oversized_block(&block);
    assert_eq!(value["type"], "json");
    let marker = value["data"].as_str().expect("an elided marker");
    assert!(marker.contains("bytes elided"), "{marker}");
    assert!(!marker.contains("xxxxx"), "the payload must not survive");
}

#[test]
fn a_small_pass_through_block_is_carried_whole() {
    let block = tinymcp_bus::McpToolContent::Json {
        data: json!({"n": 42}),
    };
    let value = elide_oversized_block(&block);
    assert_eq!(value, json!({"type": "json", "data": {"n": 42}}));
}
