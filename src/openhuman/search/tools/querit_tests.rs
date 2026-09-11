use super::*;

fn tool() -> QueritSearchTool {
    QueritSearchTool::new(None, None, 5, 15)
}

fn tool_with_key() -> QueritSearchTool {
    QueritSearchTool::new(Some("test-key".into()), None, 5, 15)
}

#[test]
fn test_tool_name() {
    assert_eq!(tool().name(), "querit_search");
    assert_eq!(
        QueritSearchTool::new_web_search_tool(None, None, 5, 15).name(),
        "web_search_tool"
    );
}

#[test]
fn test_parameters_schema() {
    let schema = tool().parameters_schema();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["query"].is_object());
    assert!(schema["properties"]["time_range"].is_object());
    assert!(schema["properties"]["countries"].is_object());
}

#[test]
fn test_render_plain_with_data() {
    let results = vec![QueritResultItem {
        url: "https://example.com/a".into(),
        page_age: Some("2026-05-01 00:00:00".into()),
        title: Some("First Result".into()),
        snippet: Some("First result snippet.".into()),
        site_name: Some("Example".into()),
        site_icon: None,
        sentence: vec![],
    }];

    let result = tool().render_results_plain(&results, "test");
    assert!(result.contains("via Querit"));
    assert!(result.contains("First Result"));
    assert!(result.contains("https://example.com/a"));
    assert!(result.contains("Page age: 2026-05-01"));
    assert!(result.contains("Site: Example"));
    assert!(result.contains("First result snippet."));
}

#[test]
fn test_render_markdown_carries_provider_marker() {
    // The markdown renderer is what production shows
    // (`output_for_llm(true)`), so it must carry the same shared
    // `(via <Provider>)` marker the plain-text renderer emits — that is
    // what the tool timeline reads back to label the row (#5136).
    let results = vec![QueritResultItem {
        url: "https://example.com/a".into(),
        page_age: None,
        title: Some("First Result".into()),
        snippet: Some("Snippet.".into()),
        site_name: None,
        site_icon: None,
        sentence: vec![],
    }];
    let result = tool().render_results_markdown(&results, "test");
    assert!(result.lines().next().unwrap().ends_with("(via Querit)"));

    // A completed empty search is attributed too, so the timeline does not
    // keep showing it as in-progress.
    let empty = tool().render_results_markdown(&[], "test");
    assert!(empty.trim_end().ends_with("(via Querit)"));
}

#[test]
fn test_build_filters_maps_supported_fields() {
    let filters = QueritSearchTool::build_filters(&json!({
        "include_domains": ["example.com"],
        "exclude_domains": ["spam.test"],
        "time_range": "d7",
        "countries": ["united states"],
        "languages": ["english"]
    }))
    .expect("filters");

    assert_eq!(filters["sites"]["include"][0], "example.com");
    assert_eq!(filters["sites"]["exclude"][0], "spam.test");
    assert_eq!(filters["timeRange"]["date"], "d7");
    assert_eq!(filters["geo"]["countries"]["include"][0], "united states");
    assert_eq!(filters["languages"]["include"][0], "english");
}

#[test]
fn test_build_filters_preserves_native_filters_payload() {
    let filters = QueritSearchTool::build_filters(&json!({
        "filters": {
            "sites": {
                "include": ["techcrunch.com"]
            },
            "timeRange": {
                "date": "m3"
            },
            "geo": {
                "countries": {
                    "include": ["united states"]
                }
            },
            "languages": {
                "include": ["english"]
            }
        }
    }))
    .expect("filters");

    assert_eq!(filters["sites"]["include"][0], "techcrunch.com");
    assert_eq!(filters["timeRange"]["date"], "m3");
    assert_eq!(filters["geo"]["countries"]["include"][0], "united states");
    assert_eq!(filters["languages"]["include"][0], "english");
}

#[test]
fn test_build_filters_normalizes_native_shorthand_values() {
    let filters = QueritSearchTool::build_filters(&json!({
        "filters": {
            "sites": ["example.com"],
            "time_range": "m3",
            "geo": {
                "countries": ["united states"]
            },
            "languages": ["english"]
        }
    }))
    .expect("filters");

    assert_eq!(filters["sites"]["include"][0], "example.com");
    assert_eq!(filters["timeRange"]["date"], "m3");
    assert_eq!(filters["geo"]["countries"]["include"][0], "united states");
    assert_eq!(filters["languages"]["include"][0], "english");
}

#[test]
fn test_build_filters_combines_date_range() {
    let filters = QueritSearchTool::build_filters(&json!({
        "from_date": "2026-01-01",
        "to_date": "2026-01-31"
    }))
    .expect("filters");
    assert_eq!(filters["timeRange"]["date"], "2026-01-01to2026-01-31");
}

#[test]
fn test_decode_response_accepts_wrapped_aiapi_shape_and_sentence() {
    let parsed = QueritSearchTool::decode_response(json!({
        "response_data": {
            "aiapi_res": {
                "error_code": 0,
                "search_id": 42,
                "results": {
                    "result": [
                        {
                            "url": "https://example.com",
                            "title": "Wrapped",
                            "sentence": ["Sentence excerpt."]
                        }
                    ]
                }
            }
        }
    }))
    .expect("wrapped response");

    assert_eq!(parsed.results.result[0].title.as_deref(), Some("Wrapped"));
    assert_eq!(
        parsed.results.result[0].snippet_text().as_deref(),
        Some("Sentence excerpt.")
    );
}

#[tokio::test]
async fn test_execute_missing_query() {
    let result = tool_with_key().execute(json!({})).await;
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
async fn test_execute_posts_to_querit_and_renders_results() {
    use axum::{extract::Json, routing::post, Router};
    use serde_json::Value;

    let app = Router::new().route(
        "/search",
        post(|Json(body): Json<Value>| async move {
            assert_eq!(body["query"], "test query");
            assert_eq!(body["count"], 3);
            assert_eq!(body["filters"]["sites"]["include"][0], "example.com");
            Json(json!({
                "took": "12ms",
                "error_code": 200,
                "error_msg": "",
                "search_id": 42,
                "query_context": { "query": "test query" },
                "results": {
                    "result": [
                        {
                            "url": "https://example.com/result",
                            "title": "Querit Result",
                            "snippet": "Content from Querit search.",
                            "page_age": "2026-05-01 00:00:00",
                            "site_name": "Example"
                        }
                    ]
                }
            }))
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base_url = format!("http://127.0.0.1:{}", addr.port());

    let tool = QueritSearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let result = tool
        .execute(json!({
            "query": "test query",
            "max_results": 3,
            "include_domains": ["example.com"]
        }))
        .await
        .expect("execute() should succeed");

    assert!(result.output().contains("Querit Result"));
    assert!(result.output().contains("https://example.com/result"));
    assert!(result.output().contains("Content from Querit search."));
}

#[tokio::test]
async fn test_execute_non_success_status_does_not_expose_response_body() {
    use axum::{http::StatusCode, routing::post, Router};

    let app = Router::new().route(
        "/search",
        post(|| async {
            (
                StatusCode::BAD_REQUEST,
                "sensitive query context should stay private",
            )
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base_url = format!("http://127.0.0.1:{}", addr.port());

    let tool = QueritSearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let err = tool
        .execute(json!({
            "query": "private search",
            "max_results": 3
        }))
        .await
        .expect_err("non-2xx responses should fail");
    let message = err.to_string();

    assert!(message.contains("Querit returned non-2xx status 400 Bad Request"));
    assert!(!message.contains("sensitive query context"));
}

#[tokio::test]
async fn test_execute_app_error_does_not_expose_error_msg() {
    use axum::{extract::Json, routing::post, Router};
    use serde_json::Value;

    let app = Router::new().route(
        "/search",
        post(|Json(_body): Json<Value>| async move {
            Json(json!({
                "took": "3ms",
                "error_code": 400,
                "error_msg": "validation failed for sensitive query context",
                "search_id": 42,
                "query_context": { "query": "sensitive query context" },
                "results": { "result": [] }
            }))
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base_url = format!("http://127.0.0.1:{}", addr.port());

    let tool = QueritSearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let err = tool
        .execute(json!({
            "query": "sensitive query context",
            "max_results": 3
        }))
        .await
        .expect_err("application-level errors should fail");
    let message = err.to_string();

    assert_eq!(message, "Querit returned error_code 400");
    assert!(!message.contains("validation failed"));
    assert!(!message.contains("sensitive query context"));
}
