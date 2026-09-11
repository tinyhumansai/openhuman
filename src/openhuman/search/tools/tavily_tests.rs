//! Tests for the Tavily BYOK search + extract tool family.

use super::*;
use axum::{extract::Json, http::StatusCode, routing::post, Router};

fn search_tool() -> TavilySearchTool {
    TavilySearchTool::new(None, None, 5, 15)
}

fn search_tool_with_key() -> TavilySearchTool {
    TavilySearchTool::new(Some("test-key".into()), None, 5, 15)
}

/// Spawn a local stand-in for `api.tavily.com` and return its base URL.
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
        "query": "tavily byok",
        "answer": "A concise answer to the query.",
        "results": [
            {
                "title": "Tavily Result",
                "url": "https://example.com/a",
                "content": "Snippet content from Tavily.",
                "score": 0.42,
                "raw_content": null
            }
        ],
        "response_time": "0.7"
    })
}

#[test]
fn tool_names_match_the_documented_byok_surface() {
    assert_eq!(search_tool().name(), "tavily_search");
    assert_eq!(
        TavilySearchTool::new_web_search_tool(None, None, 5, 15).name(),
        "web_search_tool"
    );
    assert_eq!(
        TavilyExtractTool::new(None, None, 5, 15).name(),
        "tavily_extract"
    );
}

#[test]
fn search_schema_exposes_the_tavily_filters() {
    let schema = search_tool().parameters_schema();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["query"].is_object());
    assert!(schema["properties"]["search_depth"].is_object());
    assert!(schema["properties"]["topic"].is_object());
    assert!(schema["properties"]["time_range"].is_object());
    assert!(schema["properties"]["include_answer"].is_object());
    assert!(schema["properties"]["include_domains"].is_object());
    assert!(schema["properties"]["exclude_domains"].is_object());
    assert_eq!(schema["required"][0], "query");
}

#[test]
fn search_depth_enum_matches_tavily_documented_levels() {
    let schema = search_tool().parameters_schema();
    let levels = schema["properties"]["search_depth"]["enum"]
        .as_array()
        .expect("search_depth enum")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect::<Vec<_>>();

    assert_eq!(levels, vec!["basic", "advanced", "fast", "ultra-fast"]);
}

#[test]
fn topic_enum_matches_tavily_documented_topics() {
    let schema = search_tool().parameters_schema();
    let topics = schema["properties"]["topic"]["enum"]
        .as_array()
        .expect("topic enum")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect::<Vec<_>>();

    assert_eq!(topics, vec!["general", "news", "finance"]);
}

#[test]
fn search_body_maps_snake_case_args_to_tavily_params() {
    let tool = search_tool_with_key();
    let body = tool.build_body(
        &json!({
            "max_results": 3,
            "search_depth": "advanced",
            "topic": "news",
            "time_range": "week",
            "include_answer": true,
            "include_raw_content": true,
            "include_images": true,
            "include_domains": ["reuters.com"],
            "exclude_domains": ["spam.test"]
        }),
        "election updates",
    );

    assert_eq!(body["query"], "election updates");
    assert_eq!(body["max_results"], 3);
    assert_eq!(body["search_depth"], "advanced");
    assert_eq!(body["topic"], "news");
    assert_eq!(body["time_range"], "week");
    assert_eq!(body["include_answer"], true);
    assert_eq!(body["include_raw_content"], true);
    assert_eq!(body["include_images"], true);
    assert_eq!(body["include_domains"][0], "reuters.com");
    assert_eq!(body["exclude_domains"][0], "spam.test");
}

#[test]
fn search_body_supports_start_and_end_dates_instead_of_time_range() {
    let body = search_tool_with_key().build_body(
        &json!({
            "start_date": "2026-01-01",
            "end_date": "2026-06-30"
        }),
        "archive piece",
    );

    assert_eq!(body["start_date"], "2026-01-01");
    assert_eq!(body["end_date"], "2026-06-30");
    assert!(body.get("time_range").is_none());
}

#[test]
fn search_body_omits_unrequested_optional_params() {
    let body = search_tool_with_key().build_body(&json!({}), "plain query");
    assert_eq!(body["query"], "plain query");
    assert_eq!(body["max_results"], 5);
    assert!(body.get("topic").is_none());
    assert!(body.get("include_answer").is_none());
    assert!(body.get("include_domains").is_none());
}

#[test]
fn web_search_tool_slot_shares_the_full_tavily_body() {
    // The canonical `web_search_tool` slot is the same tool under a different
    // name (mirrors `exa_search` -> `web_search_tool`), so the full argument
    // surface stays available to the agent on the generic affordance too.
    let tool = TavilySearchTool::new_web_search_tool(Some("test-key".into()), None, 5, 15);
    let body = tool.build_body(
        &json!({
            "topic": "news",
            "search_depth": "advanced",
            "max_results": 7
        }),
        "generic query",
    );

    assert_eq!(body["query"], "generic query");
    assert_eq!(body["max_results"], 7);
    assert_eq!(body["topic"], "news");
    assert_eq!(body["search_depth"], "advanced");
}

#[test]
fn excerpt_prefers_content_then_raw_content() {
    let mut item = TavilyResultItem {
        raw_content: Some("full raw".into()),
        ..Default::default()
    };
    assert_eq!(item.excerpt().as_deref(), Some("full raw"));

    item.content = Some("chunk snippet".into());
    assert_eq!(item.excerpt().as_deref(), Some("chunk snippet"));
}

#[test]
fn extract_accepts_a_urls_array_a_bare_string_or_a_single_url() {
    assert_eq!(
        TavilyExtractTool::collect_urls(
            &json!({"urls": ["https://a.test", "  ", "https://b.test"]})
        )
        .unwrap(),
        vec!["https://a.test".to_string(), "https://b.test".to_string()]
    );
    assert_eq!(
        TavilyExtractTool::collect_urls(&json!({"urls": "https://a.test"})).unwrap(),
        vec!["https://a.test".to_string()]
    );
    assert_eq!(
        TavilyExtractTool::collect_urls(&json!({"url": "https://a.test"})).unwrap(),
        vec!["https://a.test".to_string()]
    );
    assert!(TavilyExtractTool::collect_urls(&json!({"urls": []})).is_err());
}

#[test]
fn extract_rejects_a_batch_over_twenty_urls() {
    let urls: Vec<String> = (0..25)
        .map(|i| format!("https://example.com/{i}"))
        .collect();
    let error = TavilyExtractTool::collect_urls(&json!({ "urls": urls }))
        .expect_err("over-limit batches must not be silently truncated");

    assert!(error.to_string().contains("at most 20 URLs"));
}

#[test]
fn extract_never_uses_a_sub_minute_http_timeout() {
    // Extraction transfers the full cleaned page, which can exceed a small
    // search-facing timeout by orders of magnitude. A config value of a few
    // seconds must not make `tavily_extract` fail on a large page after the
    // server already answered 200 (`operation timed out` while reading the
    // body). The tool floors its request budget at Tavily's own 60s cap.
    let tool = TavilyExtractTool::new(Some("test-key".into()), None, 5, 15);
    assert_eq!(tool.client.timeout_secs, 60);

    let tool = TavilyExtractTool::new(Some("test-key".into()), None, 5, 90);
    assert_eq!(tool.client.timeout_secs, 90);
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
async fn search_posts_directly_to_tavily_with_bearer_auth_and_renders_results() {
    let app = Router::new().route(
        "/search",
        post(
            |headers: axum::http::HeaderMap, Json(body): Json<Value>| async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default();
                assert_eq!(auth, "Bearer test-key");
                assert_eq!(body["query"], "tavily byok");
                assert_eq!(body["max_results"], 3);
                assert_eq!(body["include_domains"][0], "example.com");
                Json(one_result_payload())
            },
        ),
    );
    let base_url = spawn(app).await;

    let tool = TavilySearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let result = tool
        .execute(json!({
            "query": "tavily byok",
            "max_results": 3,
            "include_domains": ["example.com"]
        }))
        .await
        .expect("execute() should succeed");

    assert!(result.output().contains("via Tavily"));
    assert!(result.output().contains("Tavily Result"));
    assert!(result.output().contains("https://example.com/a"));
    assert!(result.output().contains("Snippet content from Tavily."));
}

#[tokio::test]
async fn search_surfaces_the_tavily_generated_answer_when_requested() {
    let app = Router::new().route(
        "/search",
        post(|Json(_): Json<Value>| async move { Json(one_result_payload()) }),
    );
    let base_url = spawn(app).await;

    let tool = TavilySearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let result = tool
        .execute_with_options(
            json!({"query": "tavily byok", "include_answer": true}),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("execute() should succeed");

    let production_output = result.output_for_llm(true);
    assert!(production_output.contains("## Answer"));
    assert!(production_output.contains("A concise answer to the query."));
    assert!(production_output
        .lines()
        .next()
        .unwrap_or_default()
        .ends_with("(via Tavily)"));
}

#[tokio::test]
async fn search_drops_an_answer_the_agent_did_not_request() {
    // `one_result_payload()` always carries an `answer`; the gate must keep it
    // out of the output unless `include_answer` was requested.
    let app = Router::new().route(
        "/search",
        post(|Json(_): Json<Value>| async move { Json(one_result_payload()) }),
    );
    let base_url = spawn(app).await;

    let tool = TavilySearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let result = tool
        .execute_with_options(
            json!({"query": "tavily byok"}),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("execute() should succeed");

    let production_output = result.output_for_llm(true);
    assert!(!production_output.contains("## Answer"));
    assert!(!production_output.contains("A concise answer to the query."));
}

#[tokio::test]
async fn search_surfaces_requested_raw_content_and_images_in_markdown() {
    let app = Router::new().route(
        "/search",
        post(|Json(_): Json<Value>| async move {
            Json(json!({
                "query": "rich result",
                "images": [{
                    "url": "https://images.example.com/query.png",
                    "description": "Query overview"
                }],
                "results": [{
                    "title": "Rich result",
                    "url": "https://example.com/rich",
                    "content": "Short snippet.",
                    "raw_content": "Full cleaned page content that must not be shadowed by the snippet.",
                    "images": ["https://images.example.com/result.png"]
                }]
            }))
        }),
    );
    let base_url = spawn(app).await;

    let tool = TavilySearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let result = tool
        .execute_with_options(
            json!({
                "query": "rich result",
                "include_raw_content": true,
                "include_images": true
            }),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("execute() should succeed");

    let production_output = result.output_for_llm(true);
    assert!(production_output.contains("### Full content"));
    assert!(production_output.contains("Full cleaned page content"));
    assert!(production_output.contains("https://images.example.com/query.png"));
    assert!(production_output.contains("https://images.example.com/result.png"));
}

#[tokio::test]
async fn extract_posts_urls_and_renders_the_cleaned_content() {
    let app = Router::new().route(
        "/extract",
        post(|Json(body): Json<Value>| async move {
            assert_eq!(body["urls"][0], "https://example.com/a");
            assert_eq!(body["format"], "markdown");
            assert_eq!(body["extract_depth"], "advanced");
            Json(json!({
                "results": [{
                    "url": "https://example.com/a",
                    "raw_content": "Full cleaned page text."
                }],
                "failed_results": [],
                "response_time": 1.2
            }))
        }),
    );
    let base_url = spawn(app).await;

    let tool = TavilyExtractTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let result = tool
        .execute(json!({
            "urls": ["https://example.com/a"],
            "format": "markdown",
            "extract_depth": "advanced"
        }))
        .await
        .expect("execute() should succeed");

    assert!(result.output().contains("https://example.com/a"));
    assert!(result.output().contains("Full cleaned page text."));
}

#[tokio::test]
async fn extract_marks_a_complete_failure_as_an_error_without_echoing_remote_details() {
    let app = Router::new().route(
        "/extract",
        post(|Json(_): Json<Value>| async move {
            Json(json!({
                "results": [],
                "failed_results": [{
                    "url": "https://example.com/blocked",
                    "error": "403 sensitive upstream detail"
                }],
                "response_time": 0.4
            }))
        }),
    );
    let base_url = spawn(app).await;

    let tool = TavilyExtractTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let result = tool
        .execute_with_options(
            json!({"urls": ["https://example.com/blocked"]}),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("execute() should succeed");

    assert!(
        result.is_error,
        "a response with no extracted URLs must fail"
    );
    let production_output = result.output_for_llm(true);
    assert!(production_output.contains("could not extract any of the requested URLs"));
    assert!(
        !production_output.contains("403 sensitive upstream detail"),
        "the remote error string must not reach the transcript"
    );
}

#[tokio::test]
async fn extract_keeps_partial_results_successful_and_reports_the_failed_count() {
    let app = Router::new().route(
        "/extract",
        post(|Json(_): Json<Value>| async move {
            Json(json!({
                "results": [{
                    "url": "https://example.com/ok",
                    "raw_content": "Extracted page content."
                }],
                "failed_results": [{
                    "url": "https://example.com/blocked",
                    "error": "403 sensitive upstream detail"
                }]
            }))
        }),
    );
    let base_url = spawn(app).await;

    let tool = TavilyExtractTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let result = tool
        .execute_with_options(
            json!({"urls": ["https://example.com/ok", "https://example.com/blocked"]}),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("partial extraction should return a tool result");

    assert!(
        !result.is_error,
        "one extracted URL makes this a partial success"
    );
    let production_output = result.output_for_llm(true);
    assert!(production_output.contains("Extracted page content."));
    assert!(production_output.contains("1 URL(s) could not be extracted"));
    assert!(
        !production_output.contains("403 sensitive upstream detail"),
        "the remote error string must not reach the transcript"
    );
}

#[tokio::test]
async fn unauthorized_status_names_the_key_without_leaking_the_body() {
    let app = Router::new().route(
        "/search",
        post(|| async {
            (
                StatusCode::UNAUTHORIZED,
                "secret query context should stay private",
            )
        }),
    );
    let base_url = spawn(app).await;

    let tool = TavilySearchTool::new(Some("bad-key".into()), Some(base_url), 5, 15);
    let err = tool
        .execute(json!({"query": "private search"}))
        .await
        .expect_err("401 must fail");
    let message = err.to_string();

    assert!(message.contains("Tavily rejected the configured API key"));
    assert!(message.contains("Connections > Search engine"));
    assert!(!message.contains("secret query context"));
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

    let tool = TavilySearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let err = tool
        .execute(json!({"query": "private search"}))
        .await
        .expect_err("non-2xx responses should fail");
    let message = err.to_string();

    assert!(message.contains("Tavily returned non-2xx status 400 Bad Request"));
    assert!(!message.contains("sensitive query context"));
}

/// Privacy epic S7 (#4441): `LocalOnly` must refuse every Tavily call before any
/// query or URL reaches api.tavily.com. The base URL points at a port nothing is
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
    let tool = TavilySearchTool::new(
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
async fn extract_is_refused_under_local_only() {
    let _mode = local_only();
    let tool = TavilyExtractTool::new(Some("k".into()), Some("http://127.0.0.1:1".into()), 5, 15);

    let result = tool
        .execute(json!({"urls": ["https://example.com/a"]}))
        .await
        .expect("blocked, not failed");
    assert!(result.is_error);
    assert!(result.output().contains("[policy-blocked]"));
}

#[test]
fn markdown_rendering_neutralizes_hostile_titles_and_urls() {
    // Title and URL are remote, attacker-controlled. An unescaped `]` would
    // close the link label and let a crafted page inject markdown into the
    // agent transcript.
    let client = TavilyClient::new(Some("k".into()), None, 5, 15);
    let results = vec![TavilyResultItem {
        url: "https://example.com/a_(b)".into(),
        title: Some("Pwned](https://evil.test) [click".into()),
        ..Default::default()
    }];

    let out = client.render_markdown(&results, &[], "q", 5, None, false, false);

    // The hostile `]` is escaped, so the label never closes early and the
    // injected `(https://evil.test)` cannot become a link destination. Bare
    // parens need no escaping inside a label.
    assert!(!out.contains("[Pwned](https://evil.test)"));
    assert!(out.contains(r"[Pwned\](https://evil.test) \[click]"));
    // Parenthesised URLs survive intact inside an angle-bracket destination.
    assert!(out.contains("(<https://example.com/a_(b)>)"));
}

#[tokio::test]
async fn empty_results_keep_the_provider_marker_in_markdown() {
    let app = Router::new().route(
        "/search",
        post(|Json(_): Json<Value>| async move { Json(json!({ "results": [] })) }),
    );
    let base_url = spawn(app).await;

    let tool = TavilySearchTool::new(Some("test-key".into()), Some(base_url), 5, 15);
    let result = tool
        .execute_with_options(
            json!({"query": "nothing here"}),
            ToolCallOptions {
                prefer_markdown: true,
            },
        )
        .await
        .expect("execute() should succeed");

    let production_output = result.output_for_llm(true);
    assert!(production_output.contains("No results for `nothing here`"));
    assert!(production_output.ends_with("(via Tavily)\n"));
}
