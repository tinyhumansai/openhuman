//! Behavior tests for `threads::ops::search`.

use super::*;

#[test]
fn snippet_keeps_short_text_whole() {
    assert_eq!(snippet("Hello  world\nagain", "world"), "Hello world again");
}

#[test]
fn snippet_centres_on_the_match_and_marks_cuts() {
    let content = format!("{} needle {}", "a".repeat(200), "b".repeat(200));
    let out = snippet(&content, "NEEDLE");
    assert!(out.starts_with('…') && out.ends_with('…'), "{out}");
    assert!(out.contains("needle"), "{out}");
    assert_eq!(out.chars().count(), 2 + 6 + 2 * SNIPPET_CONTEXT);
}

#[test]
fn snippet_without_a_verbatim_match_is_the_start() {
    let content = "x".repeat(300);
    let out = snippet(&content, "zzz");
    assert_eq!(out.chars().count(), 2 * SNIPPET_CONTEXT + 1);
    assert!(out.ends_with('…'));
}

#[test]
fn snippet_matches_a_query_with_irregular_whitespace() {
    let content = format!("{} needle nearby {}", "a".repeat(200), "b".repeat(200));
    for q in ["needle  nearby", "needle\nnearby"] {
        let out = snippet(&content, q);
        assert!(out.contains("needle nearby"), "{q:?} -> {out}");
        assert!(
            out.starts_with('…'),
            "{q:?} should centre on the match: {out}"
        );
    }
}

#[test]
fn request_limit_is_optional() {
    let parsed: ThreadSearchRequest =
        serde_json::from_value(serde_json::json!({ "query": "hi" })).unwrap();
    assert_eq!(parsed.query, "hi");
    assert_eq!(parsed.limit, None);
}

#[tokio::test]
async fn blank_query_answers_no_hits() {
    let out = thread_search(
        &crate::core::runtime::DispatchCtx::ambient(),
        ThreadSearchRequest {
            query: "   ".into(),
            limit: None,
        },
    )
    .await
    .unwrap();
    let json = out.into_cli_compatible_json().unwrap();
    assert_eq!(json["data"]["hits"], serde_json::json!([]), "{json}");
}
