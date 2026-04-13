//! Unit tests for the Notion provider.

use super::sync::{extract_notion_cursor, extract_page_title, extract_results};
use super::NotionProvider;
use crate::openhuman::composio::providers::ComposioProvider;
use serde_json::json;

#[test]
fn extract_results_walks_common_shapes() {
    let v1 = json!({ "data": { "results": [{"id": "p1"}] } });
    let v2 = json!({ "results": [{"id": "p2"}, {"id": "p3"}] });
    let v3 = json!({ "data": {} });
    assert_eq!(extract_results(&v1).len(), 1);
    assert_eq!(extract_results(&v2).len(), 2);
    assert_eq!(extract_results(&v3).len(), 0);
}

#[test]
fn extract_notion_cursor_finds_nested() {
    let v = json!({ "data": { "next_cursor": "abc123" } });
    assert_eq!(extract_notion_cursor(&v), Some("abc123".to_string()));
}

#[test]
fn extract_notion_cursor_none_when_missing() {
    let v = json!({ "data": { "has_more": false } });
    assert_eq!(extract_notion_cursor(&v), None);
}

#[test]
fn extract_page_title_from_properties() {
    let page = json!({
        "id": "page-1",
        "properties": {
            "Name": {
                "type": "title",
                "title": [
                    { "plain_text": "My " },
                    { "plain_text": "Page Title" }
                ]
            }
        }
    });
    assert_eq!(extract_page_title(&page), Some("My Page Title".to_string()));
}

#[test]
fn extract_page_title_fallback_to_top_level() {
    let page = json!({ "title": "Fallback Title" });
    assert_eq!(
        extract_page_title(&page),
        Some("Fallback Title".to_string())
    );
}

#[test]
fn extract_page_title_returns_none_when_missing() {
    let page = json!({ "id": "p1" });
    assert_eq!(extract_page_title(&page), None);
}

#[test]
fn provider_metadata_is_stable() {
    let p = NotionProvider::new();
    assert_eq!(p.toolkit_slug(), "notion");
    assert_eq!(p.sync_interval_secs(), Some(30 * 60));
}
