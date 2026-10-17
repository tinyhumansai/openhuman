use super::*;
use serde_json::json;

use crate::mcp::ui::types::MCP_UI_KIND;

fn view() -> UiCallView {
    UiCallView {
        server_id: "srv".into(),
        tool: "order".into(),
        tool_input: json!({"item": "dosa"}),
        ..UiCallView::default()
    }
}

#[test]
fn embedded_html_resource_is_cached_inline() {
    let mut v = view();
    v.resources = vec![
        json!({"uri": "ui://w/card", "mimeType": "text/html;profile=mcp-app", "text": "<p>hi</p>"}),
    ];
    let presentation = presentation_from_view(&v).expect("presentation");
    assert_eq!(presentation.kind, MCP_UI_KIND);
    assert!(presentation.resource_uri.is_none());
    let id = presentation.inline_id.expect("inline id");
    let entry = cache::get_inline(&id).expect("cached");
    assert_eq!(entry.resource.html, "<p>hi</p>");
    assert_eq!(entry.server_id.as_deref(), Some("srv"));
}

#[test]
fn embedded_template_document_is_cached_for_reads() {
    let mut v = view();
    v.server_id = "srv-prefetch".into();
    v.tool_meta = Some(json!({"ui": {"resourceUri": "ui://w/prefetched"}}));
    v.resources =
        vec![json!({"uri": "ui://w/prefetched", "mimeType": "text/html", "text": "<p>card</p>"})];
    let presentation = presentation_from_view(&v).expect("presentation");
    assert_eq!(
        presentation.resource_uri.as_deref(),
        Some("ui://w/prefetched")
    );
    assert!(presentation.inline_id.is_none());
    let cached = cache::get_read("srv-prefetch", "ui://w/prefetched").expect("cached");
    assert_eq!(cached.html, "<p>card</p>");
}

#[test]
fn no_ui_and_no_links_yields_nothing() {
    let mut v = view();
    v.text = "Your order is placed.".into();
    assert!(presentation_from_view(&v).is_none());
}
