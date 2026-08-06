//! Tests for the Exa BYOK search tool family.

use super::*;
use axum::{extract::Json, http::StatusCode, routing::post, Router};

fn search_tool() -> ExaSearchTool {
    ExaSearchTool::new(None, None, 5, 15)
}

fn search_tool_with_key() -> ExaSearchTool {
    ExaSearchTool::new(Some("test-key".into()), None, 5, 15)
}

/// Spawn a local stand-in for `api.exa.ai` and return its base URL.
async fn spawn(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn one_result_payload() -> Value {
    json!({
        "requestId": "req-1",
        "resolvedSearchType": "neural",
        "results": [
            {
                "id": "https://example.com/a",
                "url": "https://example.com/a",
                "title": "Exa Result",
                "author": "Jane Doe",
                "publishedDate": "2026-05-01T00:00:00.000Z",
                "score": 0.42,
                "text": "Body text from Exa."
            }
        ]
    })
}

#[test]
fn tool_names_match_the_documented_byok_surface() {
    assert_eq!(search_tool().name(), "exa_search");
    assert_eq!(
        ExaSearchTool::new_web_search_tool(None, None, 5, 15).name(),
        "web_search_tool"
    );
    assert_eq!(
        ExaFindSimilarTool::new(None, None, 5, 15).name(),
        "exa_find_similar"
    );
    assert_eq!(
        ExaGetContentsTool::new(None, None, 5, 15).name(),
        "exa_get_contents"
    );
}

#[test]
fn search_schema_exposes_the_exa_filters() {
    let schema = search_tool().parameters_schema();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["query"].is_object());
    assert!(schema["properties"]["type"].is_object());
    assert!(schema["properties"]["include_domains"].is_object());
    assert!(schema["properties"]["start_published_date"].is_object());
    assert_eq!(schema["required"][0], "query");
}

#[test]
fn search_type_enum_matches_exa_current_search_modes() {
    // Exa's documented modes, fastest to most thorough. `neural` / `keyword`
    // are legacy spellings and must not be advertised to the agent.
    let schema = search_tool().parameters_schema();
    let modes = schema["properties"]["type"]["enum"]
        .as_array()
        .expect("type enum")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect::<Vec<_>>();

    assert_eq!(
        modes,
        vec![
            "auto",
            "instant",
            "fast",
            "deep-lite",
            "deep",
            "deep-reasoning"
        ]
    );
}

#[test]
fn search_body_maps_snake_case_args_to_exa_camel_case() {
    let tool = search_tool_with_key();
    let body = tool.build_body(
        &json!({
            "max_results": 3,
            "type": "neural",
            "category": "research paper",
            "include_domains": ["arxiv.org"],
            "exclude_domains": ["spam.test"],
            "start_published_date": "2026-01-01",
            "end_published_date": "2026-06-30",
            "include_text": true,
            "include_highlights": true
        }),
        "quantum error correction",
    );

    assert_eq!(body["query"], "quantum error correction");
    assert_eq!(body["numResults"], 3);
    assert_eq!(body["type"], "neural");
    assert_eq!(body["category"], "research paper");
    assert_eq!(body["includeDomains"][0], "arxiv.org");
    assert_eq!(body["excludeDomains"][0], "spam.test");
    assert_eq!(body["startPublishedDate"], "2026-01-01");
    assert_eq!(body["endPublishedDate"], "2026-06-30");
    assert_eq!(body["contents"]["text"], true);
    assert_eq!(body["contents"]["highlights"], true);
}

#[test]
fn search_body_omits_contents_unless_requested() {
    let body = search_tool_with_key().build_body(&json!({}), "plain query");
    assert_eq!(body["numResults"], 5);
    assert!(body.get("contents").is_none());
    assert!(body.get("type").is_none());
}

#[test]
fn excerpt_prefers_summary_then_highlights_then_text() {
    let mut item = ExaResultItem {
        text: Some("full text".into()),
        ..Default::default()
    };
    assert_eq!(item.excerpt().as_deref(), Some("full text"));

    item.highlights = vec!["  ".into(), "a highlight".into()];
    assert_eq!(item.excerpt().as_deref(), Some("a highlight"));

    item.summary = Some("the summary".into());
    assert_eq!(item.excerpt().as_deref(), Some("the summary"));
}

#[test]
fn contents_accepts_a_urls_array_a_bare_string_or_a_single_url() {
    assert_eq!(
        ExaGetContentsTool::collect_urls(
            &json!({"urls": ["https://a.test", "  ", "https://b.test"]})
        )
        .unwrap(),
        vec!["https://a.test".to_string(), "https://b.test".to_string()]
    );
    assert_eq!(
        ExaGetContentsTool::collect_urls(&json!({"urls": "https://a.test"})).unwrap(),
        vec!["https://a.test".to_string()]
    );
    assert_eq!(
        ExaGetContentsTool::collect_urls(&json!({"url": "https://a.test"})).unwrap(),
        vec!["https://a.test".to_string()]
    );
    assert!(ExaGetContentsTool::collect_urls(&json!({"urls": []})).is_err());
}

#[tokio::test]
async fn search_without_a_key_reports_where_to_set_one() {
    let err = search_tool()
        .execute(json!({"query": "test"}))
        .await
        .expect_err("missing key must fail");
    let message = err.to_string();

    assert!(message.contains("no API key configured"));
    assert!(message.contains("Connections > Search engine"));
}

#[tokio::test]
async fn search_requires_a_query() {
    assert!(search_tool_with_key().execute(json!({})).await.is_err());
}

#[tokio::test]
async fn search_posts_directly_to_exa_and_renders_results() {
    let app = Router::new().route(
        "/search",
        post(
            |headers: axum::http::HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(headers.get("x-api-key").unwrap(), "test-key");
                assert_eq!(body["query"], "exa byok");
                assert_eq!(body["numResults"], 3);
                assert_eq!(body["includeDomains"][0], "example.com");
                Json(one_result_payload())
            },
        ),
    );
    let base_url = spawn(app).await;

    let tool = ExaSearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let result = tool
        .execute(json!({
            "query": "exa byok",
            "max_results": 3,
            "include_domains": ["example.com"]
        }))
        .await
        .expect("execute() should succeed");

    assert!(result.output().contains("via Exa"));
    assert!(result.output().contains("Exa Result"));
    assert!(result.output().contains("https://example.com/a"));
    assert!(result.output().contains("Author: Jane Doe"));
    assert!(result.output().contains("Body text from Exa."));
}

#[tokio::test]
async fn find_similar_posts_the_source_url_and_renders_results() {
    let app = Router::new().route(
        "/findSimilar",
        post(|Json(body): Json<Value>| async move {
            assert_eq!(body["url"], "https://example.com/seed");
            assert_eq!(body["numResults"], 2);
            assert_eq!(body["excludeSourceDomain"], true);
            Json(one_result_payload())
        }),
    );
    let base_url = spawn(app).await;

    let tool = ExaFindSimilarTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let result = tool
        .execute(json!({
            "url": "https://example.com/seed",
            "max_results": 2,
            "exclude_source_domain": true
        }))
        .await
        .expect("execute() should succeed");

    assert!(result
        .output()
        .contains("pages similar to https://example.com/seed"));
    assert!(result.output().contains("Exa Result"));
}

#[tokio::test]
async fn get_contents_requests_text_for_every_url() {
    let app = Router::new().route(
        "/contents",
        post(|Json(body): Json<Value>| async move {
            assert_eq!(body["urls"][0], "https://example.com/a");
            assert_eq!(body["urls"][1], "https://example.com/b");
            assert_eq!(body["text"], true);
            assert_eq!(body["summary"], true);
            Json(one_result_payload())
        }),
    );
    let base_url = spawn(app).await;

    let tool = ExaGetContentsTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let result = tool
        .execute(json!({
            "urls": ["https://example.com/a", "https://example.com/b"],
            "include_summary": true
        }))
        .await
        .expect("execute() should succeed");

    assert!(result.output().contains("2 URL(s)"));
    assert!(result.output().contains("Body text from Exa."));
}

#[tokio::test]
async fn get_contents_renders_more_documents_than_the_configured_max_results() {
    // The result cap belongs to search, not to a caller-supplied URL batch.
    let app = Router::new().route(
        "/contents",
        post(|Json(_): Json<Value>| async move {
            Json(json!({
                "results": (0..4)
                    .map(|i| json!({
                        "url": format!("https://example.com/{i}"),
                        "title": format!("Doc {i}"),
                    }))
                    .collect::<Vec<_>>()
            }))
        }),
    );
    let base_url = spawn(app).await;

    let tool = ExaGetContentsTool::new(Some("test-key".into()), Some(base_url), 2, 15);
    let result = tool
        .execute(json!({
            "urls": [
                "https://example.com/0",
                "https://example.com/1",
                "https://example.com/2",
                "https://example.com/3"
            ]
        }))
        .await
        .expect("execute() should succeed");

    assert!(result.output().contains("Doc 3"));
}

#[tokio::test]
async fn unauthorized_status_names_the_key_without_leaking_the_body() {
    let app = Router::new().route(
        "/search",
        post(|| async {
            (
                StatusCode::UNAUTHORIZED,
                "invalid api key for query: private search",
            )
        }),
    );
    let base_url = spawn(app).await;

    let tool = ExaSearchTool::new(Some("bad-key".into()), Some(base_url), 5, 15);
    let err = tool
        .execute(json!({"query": "private search"}))
        .await
        .expect_err("401 must fail");
    let message = err.to_string();

    assert!(message.contains("Exa rejected the configured API key"));
    assert!(message.contains("Connections > Search engine"));
    assert!(!message.contains("invalid api key for query"));
}

#[tokio::test]
async fn non_success_status_does_not_expose_the_response_body() {
    let app = Router::new().route(
        "/search",
        post(|| async {
            (
                StatusCode::BAD_REQUEST,
                "sensitive query context should stay private",
            )
        }),
    );
    let base_url = spawn(app).await;

    let tool = ExaSearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let err = tool
        .execute(json!({"query": "private search"}))
        .await
        .expect_err("non-2xx responses should fail");
    let message = err.to_string();

    assert!(message.contains("Exa returned non-2xx status 400 Bad Request"));
    assert!(!message.contains("sensitive query context"));
}

/// Privacy epic S7 (#4441): `LocalOnly` must refuse every Exa call before any
/// query or URL reaches api.exa.ai. The base URL points at a port nothing is
/// listening on, so a request escaping the guard fails the test rather than
/// silently passing.
fn local_only() -> crate::openhuman::security::live_policy::TestPrivacyGuard {
    crate::openhuman::security::live_policy::test_privacy_scope(
        crate::openhuman::config::PrivacyMode::LocalOnly,
    )
}

#[tokio::test]
async fn search_is_refused_under_local_only_privacy_mode() {
    let _mode = local_only();
    let tool = ExaSearchTool::new(
        Some("test-key".into()),
        Some("http://127.0.0.1:1".into()),
        5,
        15,
    );

    let result = tool
        .execute(json!({"query": "private search"}))
        .await
        .expect("the block is a tool error, not a transport failure");

    assert!(result.is_error);
    assert!(
        result.output().contains("[policy-blocked]"),
        "got: {}",
        result.output()
    );
    assert!(
        !result.output().contains("private search"),
        "the block message must not echo the query: {}",
        result.output()
    );
}

#[tokio::test]
async fn find_similar_and_get_contents_are_refused_under_local_only() {
    let _mode = local_only();
    let base = || Some("http://127.0.0.1:1".to_string());

    let similar = ExaFindSimilarTool::new(Some("k".into()), base(), 5, 15)
        .execute(json!({"url": "https://example.com/seed"}))
        .await
        .expect("blocked, not failed");
    assert!(similar.is_error);
    assert!(similar.output().contains("[policy-blocked]"));

    let contents = ExaGetContentsTool::new(Some("k".into()), base(), 5, 15)
        .execute(json!({"urls": ["https://example.com/a"]}))
        .await
        .expect("blocked, not failed");
    assert!(contents.is_error);
    assert!(contents.output().contains("[policy-blocked]"));
}

#[test]
fn markdown_rendering_neutralizes_hostile_titles_and_urls() {
    // Title and URL are remote, attacker-controlled. An unescaped `]` would
    // close the link label and let a crafted page inject markdown into the
    // agent transcript.
    let client = ExaClient::new(Some("k".into()), None, 5, 15);
    let results = vec![ExaResultItem {
        url: "https://example.com/a_(b)".into(),
        title: Some("Pwned](https://evil.test) [click".into()),
        ..Default::default()
    }];

    let out = client.render_markdown(&results, "q", 5);

    // The hostile `]` is escaped, so the label never closes early and the
    // injected `(https://evil.test)` cannot become a link destination. Bare
    // parens need no escaping inside a label.
    assert!(!out.contains("[Pwned](https://evil.test)"));
    assert!(out.contains(r"[Pwned\](https://evil.test) \[click]"));
    // Parenthesised URLs survive intact inside an angle-bracket destination.
    assert!(out.contains("(<https://example.com/a_(b)>)"));
}

#[tokio::test]
async fn empty_results_render_a_plain_no_results_line() {
    let app = Router::new().route(
        "/search",
        post(|Json(_): Json<Value>| async move { Json(json!({ "results": [] })) }),
    );
    let base_url = spawn(app).await;

    let tool = ExaSearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let result = tool
        .execute(json!({"query": "nothing here"}))
        .await
        .expect("execute() should succeed");

    assert!(result.output().contains("No Exa results for: nothing here"));
}
