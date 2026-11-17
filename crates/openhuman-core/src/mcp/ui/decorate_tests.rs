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
    MetaLookup::Absent
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

#[tokio::test]
async fn wrapper_forwards_every_inner_tool_property() {
    let tool = UiAwareTool::new(
        Box::new(Fixed(ToolResult::success("plain"))),
        no_lookup(),
        ToolInput::Direct,
    );
    let args = json!({});
    assert_eq!(tool.name(), "mcp_shop_order");
    assert_eq!(tool.description(), "order");
    assert_eq!(tool.parameters_schema(), json!({"type": "object"}));
    let inner = Fixed(ToolResult::success("plain"));
    assert_eq!(tool.permission_level(), inner.permission_level());
    assert_eq!(
        tool.permission_level_with_args(&args),
        inner.permission_level_with_args(&args)
    );
    assert_eq!(tool.scope(), inner.scope());
    assert_eq!(tool.category(), inner.category());
    assert_eq!(tool.family(), inner.family());
    assert_eq!(tool.policy(), inner.policy());
    assert_eq!(
        tool.injected_arguments().len(),
        inner.injected_arguments().len()
    );
    assert_eq!(tool.supports_markdown(), inner.supports_markdown());
    assert_eq!(
        tool.is_concurrency_safe(&args),
        inner.is_concurrency_safe(&args)
    );
    assert_eq!(tool.external_effect(), inner.external_effect());
    assert_eq!(
        tool.external_effect_with_args(&args),
        inner.external_effect_with_args(&args)
    );
    assert_eq!(tool.max_result_size_chars(), inner.max_result_size_chars());
    assert_eq!(tool.timeout_policy(&args), inner.timeout_policy(&args));
    assert_eq!(
        tool.host_extension().is_some(),
        inner.host_extension().is_some()
    );
    assert_eq!(
        tool.host_call_extension(&args).is_some(),
        inner.host_call_extension(&args).is_some()
    );
    assert_eq!(tool.spec().name, inner.spec().name);
    assert_eq!(tool.display_label(&args), inner.display_label(&args));
    assert_eq!(tool.display_detail(&args), inner.display_detail(&args));
    assert_eq!(tool.return_direct(), inner.return_direct());
}

#[tokio::test]
async fn option_and_context_entry_points_present_the_widget() {
    let build = || {
        UiAwareTool::new(
            Box::new(Fixed(result_with(
                envelope(json!({"meta": {"ui": {"resourceUri": "ui://shop/cart"}}})),
                "Order placed",
            ))),
            no_lookup(),
            ToolInput::Direct,
        )
    };
    let with_options = build()
        .execute_with_options(json!({}), ToolCallOptions::default())
        .await
        .unwrap();
    assert_eq!(with_options.metadata.unwrap()["kind"], "mcp_ui");
    let with_context = build()
        .execute_with_context(json!({}), ToolCallOptions::default(), None)
        .await
        .unwrap();
    assert_eq!(with_context.metadata.unwrap()["kind"], "mcp_ui");
}

#[tokio::test]
async fn configured_lookup_without_a_module_has_no_meta() {
    let workspace = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    let lookup = MetaLookup::Configured(Arc::new(config));
    assert!(lookup.tool_meta("shop", "order").await.is_none());
}
