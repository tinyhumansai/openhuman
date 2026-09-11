use super::*;

#[test]
fn require_key_rejects_blank() {
    let cfg = BraveConfig {
        api_key: Some("   ".into()),
        max_results: 5,
        timeout_secs: 5,
    };
    assert!(cfg.require_key().is_err());
}

#[test]
fn require_key_accepts_trimmed() {
    let cfg = BraveConfig {
        api_key: Some("  abc  ".into()),
        max_results: 5,
        timeout_secs: 5,
    };
    assert_eq!(cfg.require_key().unwrap(), "abc");
}

#[test]
fn web_tool_advertises_unified_name() {
    let t = BraveWebSearchTool::new(Some("k".into()), 5, 5);
    assert_eq!(t.name(), "web_search_tool");
}

#[test]
fn web_markdown_carries_provider_marker() {
    // The markdown renderer is what production shows
    // (`output_for_llm(true)`), so its heading must end with the shared
    // `(via <Provider>)` marker the plain-text renderer already emits —
    // that is what the tool timeline reads back to label the row (#5136).
    // It previously read `(Brave)`, which no marker parser matched.
    let results = vec![WebResult {
        title: "Example".into(),
        url: "https://example.com".into(),
        description: "Desc.".into(),
        age: None,
    }];
    let out = render_web_markdown(&results, "test", 5);
    assert!(out.lines().next().unwrap().ends_with("(via Brave)"));
    assert!(out.contains("[Example](https://example.com)"));

    // A completed empty search is attributed too, so the timeline does not
    // keep showing it as in-progress.
    let empty = render_web_markdown(&[], "test", 5);
    assert!(empty.trim_end().ends_with("(via Brave)"));
}

#[test]
fn news_tool_name() {
    let t = BraveNewsSearchTool::new(Some("k".into()), 5, 5);
    assert_eq!(t.name(), "brave_news_search");
}

#[test]
fn image_tool_name() {
    let t = BraveImageSearchTool::new(Some("k".into()), 5, 5);
    assert_eq!(t.name(), "brave_image_search");
}

#[test]
fn video_tool_name() {
    let t = BraveVideoSearchTool::new(Some("k".into()), 5, 5);
    assert_eq!(t.name(), "brave_video_search");
}

#[tokio::test]
async fn execute_without_key_returns_error() {
    let t = BraveWebSearchTool::new(None, 5, 5);
    let err = t
        .execute(json!({ "query": "test" }))
        .await
        .expect_err("should error without key");
    assert!(err.to_string().contains("no API key"));
}

#[test]
fn clamped_count_respects_max() {
    assert_eq!(clamped_count(&json!({"count": 99}), 5, 20), 20);
    assert_eq!(clamped_count(&json!({"count": 0}), 5, 20), 1);
    assert_eq!(clamped_count(&json!({}), 5, 20), 5);
}
