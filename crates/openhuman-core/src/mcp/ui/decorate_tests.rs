use super::*;
use serde_json::json;

struct Fixed(ToolResult);

#[async_trait]
impl Tool for Fixed {
    fn name(&self) -> &str {
        "mcp_shop_order"
    }
    fn description(&self) -> &str {
        "order"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }
    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        Ok(self.0.clone())
    }
}

fn no_lookup() -> MetaLookup {
    MetaLookup::Configured(Arc::new(McpServerRegistry::default()))
}

fn envelope(extra: Value) -> Value {
    let mut base = json!({
        "kind": "mcp_result",
        "server": "shop",
        "tool": "order",
        "structured_content": null,
        "meta": null,
        "resources": [],
        "outcome": {"kind": "mcp_call", "server": "shop", "tool": "order", "ok": true}
    });
    base.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    base
}

fn result_with(metadata: Value, text: &str) -> ToolResult {
    let mut result = ToolResult::success(text);
    result.metadata = Some(metadata);
    result
}

#[tokio::test]
async fn widget_tool_emits_mcp_ui_and_keeps_text() {
    let tool = UiAwareTool::new(
        Box::new(Fixed(result_with(
            envelope(json!({"meta": {"ui": {"resourceUri": "ui://shop/cart"}}})),
            "Order placed",
        ))),
        no_lookup(),
        ToolInput::Direct,
    );
    assert_eq!(tool.exposure(), ToolExposure::Deferred);
    let result = tool.execute(json!({"item": "dosa"})).await.unwrap();
    assert_eq!(result.text(), "Order placed");
    let metadata = result.metadata.expect("presentation");
    assert_eq!(metadata["kind"], "mcp_ui");
    assert_eq!(metadata["resource_uri"], "ui://shop/cart");
    assert_eq!(metadata["tool_input"], json!({"item": "dosa"}));
    assert_eq!(metadata["server_id"], "shop");
}

#[tokio::test]
async fn envelope_without_ui_or_links_is_dropped() {
    let tool = UiAwareTool::new(
        Box::new(Fixed(result_with(envelope(json!({})), "Nothing to show"))),
        no_lookup(),
        ToolInput::Direct,
    );
    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.metadata.is_none());
}

#[tokio::test]
async fn link_in_structured_content_is_surfaced() {
    let tool = UiAwareTool::new(
        Box::new(Fixed(result_with(
            envelope(json!({"structured_content": {"pay": "phonepe://pay?o=1"}})),
            "ok",
        ))),
        no_lookup(),
        ToolInput::Nested("arguments"),
    );
    let metadata = tool.execute(json!({})).await.unwrap().metadata.unwrap();
    assert_eq!(metadata["links"][0]["url"], "phonepe://pay?o=1");
    assert_eq!(metadata["links"][0]["kind"], "handoff");
}

#[test]
fn other_metadata_is_left_alone() {
    let mut result = result_with(json!({"kind": "web_search", "hits": []}), "x");
    decorate_result(None, &json!({}), &mut result);
    assert_eq!(result.metadata.unwrap()["kind"], "web_search");
}

#[test]
fn malformed_envelope_is_removed() {
    let mut result = result_with(
        json!({"kind": "mcp_result", "resources": [{"text": "<html>"}]}),
        "x",
    );
    decorate_result(None, &json!({}), &mut result);
    assert!(result.metadata.is_none());
}

#[test]
fn scrub_strings_rewrites_nested_values() {
    let mut value = json!({"a": ["tok-SECRET"], "b": {"c": "SECRET"}, "n": 1});
    scrub_strings(&mut value, &|text| text.replace("SECRET", "[redacted]"));
    assert_eq!(
        value,
        json!({"a": ["tok-[redacted]"], "b": {"c": "[redacted]"}, "n": 1})
    );
}
