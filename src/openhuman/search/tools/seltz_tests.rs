use super::*;

fn tool() -> SeltzSearchTool {
    SeltzSearchTool::new(None, None, 5, 15)
}

fn tool_with_key() -> SeltzSearchTool {
    SeltzSearchTool::new(Some("test-key".into()), None, 5, 15)
}

#[test]
fn test_tool_name() {
    assert_eq!(tool().name(), "seltz_search");
}

#[test]
fn test_tool_description() {
    assert!(tool().description().contains("Seltz"));
}

#[test]
fn test_parameters_schema() {
    let schema = tool().parameters_schema();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["query"].is_object());
    assert!(schema["properties"]["include_domains"].is_object());
    assert!(schema["properties"]["scope"].is_object());
}

#[test]
fn test_render_plain_empty() {
    let result = tool().render_results_plain(&[], "test query");
    assert!(result.contains("No results found"));
}

#[test]
fn test_render_plain_with_data() {
    let docs = vec![
        SeltzDocument {
            url: "https://example.com/a".into(),
            content: "First result content.".into(),
            title: Some("First Result".into()),
            published_date: Some("2026-01-15".into()),
        },
        SeltzDocument {
            url: "https://example.com/b".into(),
            content: "Second result content.".into(),
            title: None,
            published_date: None,
        },
    ];

    let result = tool().render_results_plain(&docs, "test");
    assert!(result.contains("via Seltz"));
    assert!(result.contains("First Result"));
    assert!(result.contains("https://example.com/a"));
    assert!(result.contains("Published: 2026-01-15"));
    assert!(result.contains("First result content."));
    assert!(result.contains("Untitled"));
}

#[test]
fn test_render_plain_respects_max_results() {
    let tool = SeltzSearchTool::new(None, None, 1, 15);
    let docs = vec![
        SeltzDocument {
            url: "https://a.com".into(),
            content: "A".into(),
            title: Some("A".into()),
            published_date: None,
        },
        SeltzDocument {
            url: "https://b.com".into(),
            content: "B".into(),
            title: Some("B".into()),
            published_date: None,
        },
    ];
    let result = tool.render_results_plain(&docs, "q");
    assert!(result.contains("https://a.com"));
    assert!(!result.contains("https://b.com"));
}

#[test]
fn test_render_plain_truncates_long_content() {
    let long_content = "x".repeat(600);
    let docs = vec![SeltzDocument {
        url: "https://t.com".into(),
        content: long_content,
        title: Some("T".into()),
        published_date: None,
    }];
    let result = tool().render_results_plain(&docs, "q");
    assert!(result.contains("..."));
    let content_line = result.lines().find(|l| l.trim().starts_with('x')).unwrap();
    assert!(content_line.trim().len() <= 503);
}

#[test]
fn test_render_markdown_empty() {
    let result = tool().render_results_markdown(&[], "test");
    assert!(result.contains("No results"));
    // A completed empty search still carries attribution, so the timeline
    // labels the row instead of leaving it as in-progress (#5136).
    assert!(result.trim_end().ends_with("(via Seltz)"));
}

#[test]
fn test_render_markdown_with_data() {
    let docs = vec![SeltzDocument {
        url: "https://example.com".into(),
        content: "Some content.".into(),
        title: Some("Example".into()),
        published_date: Some("2026-01-01".into()),
    }];
    let result = tool().render_results_markdown(&docs, "test");
    // The markdown renderer is what production shows, so it must carry the
    // shared `(via <Provider>)` marker on its heading line (#5136).
    assert!(result.lines().next().unwrap().ends_with("(via Seltz)"));
    assert!(result.contains("[Example](https://example.com)"));
    assert!(result.contains("Published: 2026-01-01"));
    assert!(result.contains("> Some content."));
}

#[tokio::test]
async fn test_execute_missing_query() {
    let result = tool_with_key().execute(json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_execute_empty_query() {
    let result = tool_with_key().execute(json!({"query": ""})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_execute_without_api_key() {
    let result = tool().execute(json!({"query": "test"})).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("no API key configured"));
}

#[tokio::test]
async fn test_execute_posts_to_seltz_and_renders_results() {
    use axum::{extract::Json, routing::post, Router};
    use serde_json::Value;

    let app = Router::new().route(
        "/search",
        post(|Json(body): Json<Value>| async move {
            assert_eq!(body["query"], "test query");
            Json(json!({
                "documents": [
                    {
                        "url": "https://example.com/result",
                        "title": "Seltz Result",
                        "content": "Content from Seltz search.",
                        "published_date": "2026-05-01"
                    }
                ]
            }))
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base_url = format!("http://127.0.0.1:{}", addr.port());

    let tool = SeltzSearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let result = tool
        .execute(json!({"query": "test query"}))
        .await
        .expect("execute() should succeed");

    assert!(result.output().contains("Seltz Result"));
    assert!(result.output().contains("https://example.com/result"));
    assert!(result.output().contains("Content from Seltz search."));
}

#[test]
fn test_max_results_clamped() {
    let tool = SeltzSearchTool::new(None, None, 100, 15);
    assert_eq!(tool.max_results, 20);
    let tool = SeltzSearchTool::new(None, None, 0, 15);
    assert_eq!(tool.max_results, 1);
}
