use super::*;
use tinysearch_bus::{Citation, SearchResult};

fn response(role: Option<Role>) -> ExecuteToolResponse {
    ExecuteToolResponse {
        provider: "exa".into(),
        results: vec![SearchResult {
            title: "Tokio select".into(),
            url: "https://tokio.rs/select".into(),
            snippet: Some("Waits on multiple futures".into()),
            published: Some("2026-09-01".into()),
        }],
        citations: Vec::new(),
        answer: None,
        status: SearchStatus::Ok,
        provider_data: None,
        role,
        fallback_from: Vec::new(),
    }
}

#[test]
fn search_results_carry_the_via_marker_and_structured_payload() {
    let result = render(&response(Some(Role::Search)), "tokio select", 5, false);
    assert!(result
        .output()
        .starts_with("Search results for: tokio select (via Exa)"));
    assert!(result.output().contains("1. Tokio select"));
    assert!(result.markdown_formatted.is_none());
    let meta = result.metadata.expect("structured payload");
    assert_eq!(meta["kind"], "web_search");
    assert_eq!(meta["provider"], "Exa");
    assert_eq!(meta["role"], "search");
    assert_eq!(meta["results"][0]["url"], "https://tokio.rs/select");
    assert_eq!(meta["results"][0]["published"], "2026-09-01");
    assert!(meta.get("answer").is_none());
}

#[test]
fn answers_render_text_citations_and_fallbacks() {
    let mut answer = response(Some(Role::Answer));
    answer.provider = "gemini".into();
    answer.results.clear();
    answer.answer = Some("It returns the first future to complete.".into());
    answer.citations = vec![Citation {
        url: "https://docs.rs/tokio".into(),
        title: Some("tokio docs".into()),
    }];
    answer.fallback_from = vec!["exa".into()];
    let result = render(&answer, "what does select do", 5, true);
    let text = result.output();
    assert!(text.starts_with("Answer for: what does select do (via Gemini, after Exa)"));
    assert!(text.contains("It returns the first future"));
    assert!(text.contains("[1] tokio docs — https://docs.rs/tokio"));
    let markdown = result.markdown_formatted.clone().expect("markdown");
    assert!(markdown.contains("### Sources"));
    let meta = result.metadata.expect("payload");
    assert_eq!(meta["answer"], "It returns the first future to complete.");
    assert_eq!(meta["citations"][0]["url"], "https://docs.rs/tokio");
    assert_eq!(meta["fallback_from"][0], "Exa");
}

#[test]
fn empty_results_still_attribute_the_provider() {
    let mut empty = response(None);
    empty.results.clear();
    empty.status = SearchStatus::Empty;
    let result = render(&empty, "nothing", 5, false);
    assert_eq!(result.output(), "No results found for: nothing (via Exa)");
}

#[test]
fn in_progress_research_is_flagged() {
    let mut running = response(Some(Role::Answer));
    running.provider = "gemini_deep_research".into();
    running.results.clear();
    running.status = SearchStatus::InProgress;
    let result = render(&running, "deep topic", 5, false);
    assert!(result
        .output()
        .starts_with("Research still running for: deep topic (via Gemini Deep Research)"));
    assert_eq!(result.metadata.unwrap()["in_progress"], true);
}

#[test]
fn results_are_capped_and_subject_reads_urls() {
    let mut many = response(None);
    many.results = (0..8)
        .map(|i| SearchResult {
            title: format!("r{i}"),
            url: format!("https://e.example/{i}"),
            snippet: None,
            published: None,
        })
        .collect();
    let result = render(&many, "q", 3, false);
    assert_eq!(
        result.metadata.unwrap()["results"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        subject(&serde_json::json!({"urls": ["https://a", "https://b"]})),
        "https://a, https://b"
    );
    assert_eq!(subject(&serde_json::json!({"query": " q "})), "q");
    assert_eq!(provider_label("searxng"), "SearXNG");
    assert_eq!(provider_label("keenable"), "Keenable");
    assert_eq!(provider_label("custom"), "custom");
}
