//! Unit tests for the GitHub provider.

use super::sync::{
    extract_issue_title, extract_issue_updated, extract_issues, extract_repo_qualified_identifier,
    extract_viewer_id, extract_viewer_login,
};
use super::GitHubProvider;
use crate::openhuman::composio::providers::ComposioProvider;
use serde_json::json;

#[test]
fn extract_issues_walks_search_and_list_shapes() {
    let v1 = json!({ "data": { "items": [{"id": 1}, {"id": 2}] } });
    let v2 = json!({ "items": [{"id": 3}] });
    let v3 = json!({ "data": { "issues": [{"id": 4}, {"id": 5}, {"id": 6}] } });
    let v4 = json!({ "data": {} });
    assert_eq!(extract_issues(&v1).len(), 2);
    assert_eq!(extract_issues(&v2).len(), 1);
    assert_eq!(extract_issues(&v3).len(), 3);
    assert_eq!(extract_issues(&v4).len(), 0);
}

#[test]
fn extract_issue_title_finds_title_field() {
    let issue = json!({ "id": 1, "title": "Build feature X" });
    assert_eq!(extract_issue_title(&issue), Some("Build feature X".into()));
}

#[test]
fn extract_issue_title_falls_back_to_wrapped_data() {
    let issue = json!({ "data": { "title": "Wrapped title" } });
    assert_eq!(extract_issue_title(&issue), Some("Wrapped title".into()));
}

#[test]
fn extract_issue_title_returns_none_when_missing() {
    let issue = json!({ "id": 1 });
    assert!(extract_issue_title(&issue).is_none());
}

#[test]
fn extract_issue_updated_handles_iso_string() {
    let issue = json!({ "updated_at": "2026-05-21T10:30:00Z" });
    assert_eq!(
        extract_issue_updated(&issue),
        Some("2026-05-21T10:30:00Z".to_string())
    );
}

#[test]
fn extract_issue_updated_handles_camelcase_alias() {
    // Some Composio wrappings camelcase the JSON keys regardless of
    // the upstream snake_case; the walker tolerates both.
    let issue = json!({ "data": { "updatedAt": "2026-04-01T00:00:00Z" } });
    assert_eq!(
        extract_issue_updated(&issue),
        Some("2026-04-01T00:00:00Z".to_string())
    );
}

#[test]
fn extract_issue_updated_returns_none_when_missing() {
    let issue = json!({ "id": 1 });
    assert!(extract_issue_updated(&issue).is_none());
}

#[test]
fn extract_repo_qualified_identifier_from_full_name() {
    let issue = json!({
        "number": 2408,
        "repository": { "full_name": "tinyhumansai/openhuman" }
    });
    assert_eq!(
        extract_repo_qualified_identifier(&issue),
        Some("tinyhumansai/openhuman#2408".to_string())
    );
}

#[test]
fn extract_repo_qualified_identifier_from_repository_url() {
    // SEARCH_ISSUES exposes `repository_url` rather than a nested
    // `repository` object — make sure we parse the suffix.
    let issue = json!({
        "number": 42,
        "repository_url": "https://api.github.com/repos/octocat/Hello-World"
    });
    assert_eq!(
        extract_repo_qualified_identifier(&issue),
        Some("octocat/Hello-World#42".to_string())
    );
}

#[test]
fn extract_repo_qualified_identifier_handles_stringified_number() {
    // Composio wrappings sometimes stringify numeric fields.
    let issue = json!({
        "number": "123",
        "repository": { "full_name": "owner/repo" }
    });
    assert_eq!(
        extract_repo_qualified_identifier(&issue),
        Some("owner/repo#123".to_string())
    );
}

#[test]
fn extract_repo_qualified_identifier_none_when_repo_missing() {
    // Without a repo path the canonical form `owner/repo#number`
    // can't be constructed — must be None, not a misleading partial.
    let issue = json!({ "number": 1 });
    assert!(extract_repo_qualified_identifier(&issue).is_none());
}

#[test]
fn extract_viewer_login_from_top_level() {
    let data = json!({ "login": "octocat" });
    assert_eq!(extract_viewer_login(&data), Some("octocat".to_string()));
}

#[test]
fn extract_viewer_login_from_wrapped_payload() {
    let data = json!({ "data": { "login": "octocat" } });
    assert_eq!(extract_viewer_login(&data), Some("octocat".to_string()));
}

#[test]
fn extract_viewer_login_strict_no_fallback_to_id_or_name() {
    // MUST NOT fall back to a top-level `id` or `name` field —
    // viewer_login drives the assignee search filter, and picking up
    // a non-login identifier would silently scope the sync to the
    // wrong user. Same safety property as Linear's
    // `extract_viewer_id` after CodeRabbit's #2402 feedback.
    let data = json!({ "id": 12345, "name": "Some Name" });
    assert!(extract_viewer_login(&data).is_none());
}

#[test]
fn extract_viewer_id_handles_numeric_id() {
    let data = json!({ "id": 12345 });
    assert_eq!(extract_viewer_id(&data), Some("12345".to_string()));
}

#[test]
fn extract_viewer_id_handles_wrapped_payload() {
    let data = json!({ "data": { "id": 777 } });
    assert_eq!(extract_viewer_id(&data), Some("777".to_string()));
}

#[test]
fn provider_metadata_is_stable() {
    let p = GitHubProvider::new();
    assert_eq!(p.toolkit_slug(), "github");
    assert_eq!(p.sync_interval_secs(), Some(30 * 60));
    assert!(p.curated_tools().is_some());
}

#[test]
fn curated_tools_contains_core_read_surface() {
    let p = GitHubProvider::new();
    let curated = p.curated_tools().expect("GITHUB_CURATED is registered");
    let slugs: Vec<&str> = curated.iter().map(|t| t.slug).collect();
    // The two actions the sync path depends on must be advertised.
    assert!(
        slugs.contains(&"GITHUB_GET_AUTHENTICATED_USER"),
        "GITHUB_GET_AUTHENTICATED_USER must be curated — sync depends on it for viewer-login resolution"
    );
    assert!(
        slugs.contains(&"GITHUB_SEARCH_ISSUES"),
        "GITHUB_SEARCH_ISSUES must be curated — sync depends on it for paginated issue fetch"
    );
}

#[test]
fn default_impl_matches_new() {
    // `GitHubProvider` is a unit struct, so we compare observable
    // trait surface instead of deriving `PartialEq`. Same shape as
    // the equivalent tests in `clickup` / `linear` — catches a
    // future regression where `new()` and `default()` drift apart.
    let a = GitHubProvider::new();
    let b = GitHubProvider::default();
    assert_eq!(a.toolkit_slug(), b.toolkit_slug());
    assert_eq!(a.sync_interval_secs(), b.sync_interval_secs());
    assert_eq!(
        a.curated_tools().map(<[_]>::len),
        b.curated_tools().map(<[_]>::len),
    );
}
