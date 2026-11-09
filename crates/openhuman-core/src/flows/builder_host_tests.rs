use super::*;

use serde_json::json;

fn required_args_contract() -> crate::integrations::composio::catalog::ToolContract {
    crate::integrations::composio::catalog::ToolContract {
        slug: "PREFLIGHTCAPSHOST_SEND_EMAIL".into(),
        toolkit: "preflightcapshost".into(),
        description: None,
        required_args: vec!["to".into()],
        input_schema: None,
        output_fields: Vec::new(),
        output_schema: None,
        primary_array_path: None,
        is_curated: false,
    }
}

#[tokio::test]
async fn mock_tool_preflight_rejects_missing_required_args_without_invoking_backend() {
    crate::integrations::composio::catalog::seed_live_catalog_cache(
        "preflightcapshost",
        vec![required_args_contract()],
    );
    let config = Config::default();

    let error = preflight_mock_tool(
        &config,
        "PREFLIGHTCAPSHOST_SEND_EMAIL",
        &json!({"to": null}),
    )
    .await
    .expect_err("mock calls must not accept unwired required arguments");
    assert!(error.contains("`to`"), "{error}");
}

#[tokio::test]
async fn mock_tool_preflight_accepts_wired_composio_and_native_arguments() {
    crate::integrations::composio::catalog::seed_live_catalog_cache(
        "preflightcapshost",
        vec![required_args_contract()],
    );
    let config = Config::default();

    preflight_mock_tool(
        &config,
        "PREFLIGHTCAPSHOST_SEND_EMAIL",
        &json!({"to": "person@example.com"}),
    )
    .await
    .unwrap();
    preflight_mock_tool(&config, "oh:web_search", &json!({}))
        .await
        .unwrap();
}
