use super::*;

#[test]
fn redact_endpoint_keeps_only_origin() {
    assert_eq!(
        redact_endpoint("https://tinyhumans.gitbook.io/openhuman/~gitbook/mcp"),
        "https://tinyhumans.gitbook.io"
    );
    assert_eq!(
        redact_endpoint("http://example.com:8080/path?token=secret"),
        "http://example.com:8080"
    );
}

#[tokio::test]
async fn search_rejects_empty_query() {
    let t = GitbooksSearchTool::new("https://example.com/mcp".into(), 5).expect("a client builds");
    let result = t.execute(json!({"query": "   "})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("empty"));
}

#[tokio::test]
async fn get_page_rejects_empty_url() {
    let t = GitbooksGetPageTool::new("https://example.com/mcp".into(), 5).expect("a client builds");
    let result = t.execute(json!({"url": ""})).await.unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn live_search_smoke() {
    if std::env::var("OPENHUMAN_GITBOOKS_LIVE_TEST")
        .ok()
        .as_deref()
        != Some("1")
    {
        return;
    }
    let t = GitbooksSearchTool::new(
        "https://tinyhumans.gitbook.io/openhuman/~gitbook/mcp".into(),
        30,
    )
    .expect("a client builds");
    let result = t
        .execute(json!({"query": "what is openhuman"}))
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "live search returned error: {}",
        result.output()
    );
    assert!(!result.output().is_empty());
}
