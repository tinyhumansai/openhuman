use super::*;

fn tool(base_url: String) -> SearxngSearchTool {
    SearxngSearchTool::new(base_url, 10, "en".into(), 5)
}

#[test]
fn normalizes_categories_and_maps_web_to_general() {
    let categories = normalize_categories(vec![
        "web".into(),
        "news".into(),
        "general".into(),
        " images ".into(),
    ])
    .expect("categories");
    assert_eq!(categories, vec!["general", "news", "images"]);
}

#[test]
fn rejects_unknown_category() {
    let err = normalize_categories(vec!["videos".into()]).expect_err("must reject");
    assert!(err.to_string().contains("unsupported SearXNG category"));
}

#[test]
fn normalize_results_falls_back_to_snippet_when_content_is_blank() {
    let results = normalize_results(
        RawSearxngResponse {
            results: vec![RawSearxngResult {
                title: Some("Result".into()),
                url: Some("https://example.com".into()),
                content: Some("   ".into()),
                snippet: Some("Useful fallback snippet".into()),
                engine: Some("engine".into()),
                engines: Vec::new(),
            }],
        },
        5,
    );

    assert_eq!(results[0].snippet, "Useful fallback snippet");
}

#[test]
fn parse_search_args_rejects_malformed_optional_values() {
    let language_err = parse_search_args(json!({
        "query": "privacy search",
        "language": 1
    }))
    .expect_err("language must reject wrong type");
    assert!(language_err
        .to_string()
        .contains("language must be a non-empty string"));

    let max_results_err = parse_search_args(json!({
        "query": "privacy search",
        "max_results": "10"
    }))
    .expect_err("max_results must reject wrong type");
    assert!(max_results_err
        .to_string()
        .contains("max_results must be a positive integer"));
}

#[test]
fn parameters_schema_includes_mcp_expected_fields() {
    let schema = tool("http://localhost:8080".into()).parameters_schema();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["query"].is_object());
    assert!(schema["properties"]["categories"].is_object());
    assert!(schema["properties"]["language"].is_object());
    assert!(schema["properties"]["max_results"].is_object());
}

#[tokio::test]
async fn search_calls_json_endpoint_and_normalizes_results() {
    use axum::{extract::Query, routing::get, Json, Router};
    use std::collections::HashMap;

    let app = Router::new().route(
        "/search",
        get(|Query(params): Query<HashMap<String, String>>| async move {
            assert_eq!(params.get("q").map(String::as_str), Some("test query"));
            assert_eq!(params.get("format").map(String::as_str), Some("json"));
            assert_eq!(
                params.get("categories").map(String::as_str),
                Some("general,news")
            );
            assert_eq!(params.get("language").map(String::as_str), Some("en"));
            Json(json!({
                "results": [
                    {
                        "title": "First result",
                        "url": "https://example.com/one",
                        "content": "A useful snippet.",
                        "engine": "duckduckgo"
                    },
                    {
                        "title": "Missing URL should be skipped",
                        "content": "No URL"
                    },
                    {
                        "url": "https://example.com/two",
                        "snippet": "Fallback snippet.",
                        "engines": ["brave"]
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

    let response = tool(format!("http://127.0.0.1:{}", addr.port()))
        .search(SearxngSearchArgs {
            query: " test query ".into(),
            categories: vec!["web".into(), "news".into()],
            language: None,
            max_results: Some(5),
        })
        .await
        .expect("search");

    assert_eq!(response.query, "test query");
    assert_eq!(response.results.len(), 2);
    assert_eq!(response.results[0].title, "First result");
    assert_eq!(response.results[0].source, "duckduckgo");
    assert_eq!(response.results[1].title, "https://example.com/two");
    assert_eq!(response.results[1].snippet, "Fallback snippet.");
    assert_eq!(response.results[1].source, "brave");
}
