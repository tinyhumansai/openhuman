use super::*;

struct NeverCalledHooks;

#[async_trait]
impl FlowPolicyHooks for NeverCalledHooks {
    async fn resolve_policy(&self, _: &Config) -> Result<Arc<SecurityPolicy>, String> {
        panic!("empty native slugs must be rejected before policy resolution")
    }

    async fn authorize(
        &self,
        _: &SecurityPolicy,
        _: CommandClass,
        _: &str,
        _: &Value,
    ) -> Result<Option<String>, String> {
        panic!("empty native slugs must be rejected before authorization")
    }

    fn record_execution(&self, _: Option<&str>, _: &Result<Value, String>) {
        panic!("empty native slugs must not record execution")
    }
}

#[tokio::test]
async fn empty_native_slug_is_rejected_before_policy_hooks() {
    let error = invoke(&Config::default(), "oh:   ", json!({}), &NeverCalledHooks)
        .await
        .expect_err("empty tool names must fail");
    assert_eq!(
        error,
        "tool_call node: native tool slug is empty (expected `oh:<tool_name>`)"
    );
}

#[test]
fn native_tool_payload_unwraps_a_single_json_block() {
    let result = ToolResult::json(json!({
        "url": "https://example.test/presigned",
        "expires_at": "2026-01-01T00:00:00Z",
    }));
    let payload = native_tool_payload(&result);
    assert_eq!(payload["url"], "https://example.test/presigned");
    assert_eq!(payload["expires_at"], "2026-01-01T00:00:00Z");
    assert!(
        payload.get("content").is_none() && payload.get("is_error").is_none(),
        "the ToolResult envelope must not leak into item.json: {payload}"
    );
}

#[test]
fn native_tool_payload_collapses_text_to_a_bindable_field() {
    let payload = native_tool_payload(&ToolResult::success("done"));
    assert_eq!(payload["text"], "done");
}

#[test]
fn native_tool_payload_collapses_mixed_blocks_to_text() {
    let result = ToolResult {
        content: vec![
            tinytools::ToolContent::Text {
                text: "line".into(),
            },
            tinytools::ToolContent::Json {
                data: json!({"k": 1}),
            },
        ],
        is_error: false,
        markdown_formatted: None,
        ..ToolResult::default()
    };
    let payload = native_tool_payload(&result);
    let text = payload["text"].as_str().expect("text field");
    assert!(text.contains("line") && text.contains('k'), "got {text}");
}

#[test]
fn native_tool_failure_fails_the_step_instead_of_recording_success() {
    let result = ToolResult::error("storage quota exceeded");
    let err = reject_failed_native_tool_result("oh:storage_upload_file", &result)
        .expect_err("an is_error ToolResult must fail the step");
    assert!(
        err.contains("storage_upload_file") && err.contains("storage quota exceeded"),
        "error must name the tool and the provider detail: {err}"
    );
}

#[test]
fn native_tool_success_passes_through() {
    let result = ToolResult::json(json!({"file_id": "f_1"}));
    assert!(reject_failed_native_tool_result("oh:storage_upload_file", &result).is_ok());
}
