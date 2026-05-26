//! Unit tests for the GitHub Composio provider.

use super::provider::build_search_query;
use super::sync::{
    extract_issue_id, extract_issue_title, extract_issue_updated_at, extract_issues,
    extract_user_login,
};
use super::GitHubProvider;
use crate::openhuman::memory_sync::composio::providers::ComposioProvider;
use serde_json::json;

// ── extract_issues ───────────────────────────────────────────────────────────

#[test]
fn extract_issues_walks_data_items_shape() {
    let data = json!({ "data": { "items": [{"id": 1u64}] } });
    assert_eq!(extract_issues(&data).len(), 1);
}

#[test]
fn extract_issues_walks_top_level_items_shape() {
    let data = json!({ "items": [{"id": 1u64}, {"id": 2u64}] });
    assert_eq!(extract_issues(&data).len(), 2);
}

#[test]
fn extract_issues_returns_empty_when_no_items_key() {
    let data = json!({ "foo": "bar" });
    assert!(extract_issues(&data).is_empty());
}

#[test]
fn extract_issues_handles_data_data_nesting() {
    let data = json!({ "data": { "data": { "items": [{"id": 9u64}] } } });
    assert_eq!(extract_issues(&data).len(), 1);
}

// ── extract_issue_id ─────────────────────────────────────────────────────────

#[test]
fn extract_issue_id_from_numeric_id() {
    let issue = json!({ "id": 123456789u64, "title": "Fix race" });
    assert_eq!(extract_issue_id(&issue), Some("123456789".to_string()));
}

#[test]
fn extract_issue_id_from_wrapped_data() {
    let issue = json!({ "data": { "id": 42u64 } });
    assert_eq!(extract_issue_id(&issue), Some("42".to_string()));
}

#[test]
fn extract_issue_id_falls_back_to_html_url_path() {
    let issue = json!({
        "html_url": "https://github.com/owner/repo/issues/7"
    });
    assert_eq!(extract_issue_id(&issue), Some("owner/repo#7".to_string()));
}

#[test]
fn extract_issue_id_none_when_no_id_or_url() {
    let issue = json!({ "title": "orphan" });
    assert!(extract_issue_id(&issue).is_none());
}

// ── extract_issue_title ──────────────────────────────────────────────────────

#[test]
fn extract_issue_title_builds_prefixed_title() {
    let issue = json!({
        "id": 1u64,
        "title": "Fix race condition",
        "html_url": "https://github.com/acme/core/issues/99"
    });
    assert_eq!(
        extract_issue_title(&issue),
        Some("GitHub: acme/core#99: Fix race condition".to_string())
    );
}

#[test]
fn extract_issue_title_pr_url_also_works() {
    let issue = json!({
        "id": 2u64,
        "title": "Add feature",
        "html_url": "https://github.com/org/repo/pull/101"
    });
    assert_eq!(
        extract_issue_title(&issue),
        Some("GitHub: org/repo#101: Add feature".to_string())
    );
}

#[test]
fn extract_issue_title_returns_raw_title_when_no_url() {
    let issue = json!({ "title": "Bare title" });
    assert_eq!(extract_issue_title(&issue), Some("Bare title".to_string()));
}

#[test]
fn extract_issue_title_none_when_no_title() {
    let issue = json!({ "id": 1u64 });
    assert!(extract_issue_title(&issue).is_none());
}

// ── extract_issue_updated_at ─────────────────────────────────────────────────

#[test]
fn extract_issue_updated_at_from_top_level() {
    let issue = json!({ "updated_at": "2024-05-21T15:30:00Z" });
    assert_eq!(
        extract_issue_updated_at(&issue),
        Some("2024-05-21T15:30:00Z".to_string())
    );
}

#[test]
fn extract_issue_updated_at_from_data_wrapper() {
    let issue = json!({ "data": { "updated_at": "2023-01-01T00:00:00Z" } });
    assert_eq!(
        extract_issue_updated_at(&issue),
        Some("2023-01-01T00:00:00Z".to_string())
    );
}

#[test]
fn extract_issue_updated_at_none_when_missing() {
    let issue = json!({ "id": 1u64 });
    assert!(extract_issue_updated_at(&issue).is_none());
}

// ── extract_user_login ───────────────────────────────────────────────────────

#[test]
fn extract_user_login_from_top_level() {
    let data = json!({ "login": "octocat" });
    assert_eq!(extract_user_login(&data), Some("octocat".to_string()));
}

#[test]
fn extract_user_login_from_data_wrapper() {
    let data = json!({ "data": { "login": "monalisa" } });
    assert_eq!(extract_user_login(&data), Some("monalisa".to_string()));
}

#[test]
fn extract_user_login_none_when_missing() {
    let data = json!({ "id": 1u64 });
    assert!(extract_user_login(&data).is_none());
}

// ── provider metadata ────────────────────────────────────────────────────────

#[test]
fn provider_metadata_is_stable() {
    let p = GitHubProvider::new();
    assert_eq!(p.toolkit_slug(), "github");
    assert_eq!(p.sync_interval_secs(), Some(30 * 60));
    assert!(p.curated_tools().is_some());
}

#[test]
fn curated_tools_contains_core_actions() {
    let p = GitHubProvider::new();
    let curated = p.curated_tools().expect("GITHUB_CURATED is registered");
    let slugs: Vec<&str> = curated.iter().map(|t| t.slug).collect();
    assert!(slugs.contains(&"GITHUB_GET_AUTHENTICATED_USER"));
    assert!(slugs.contains(&"GITHUB_SEARCH_ISSUES"));
    assert!(slugs.contains(&"GITHUB_LIST_REPOSITORY_ISSUES"));
}

#[test]
fn default_impl_matches_new() {
    let a = GitHubProvider::new();
    let b = GitHubProvider::default();
    assert_eq!(a.toolkit_slug(), b.toolkit_slug());
    assert_eq!(a.sync_interval_secs(), b.sync_interval_secs());
    assert_eq!(
        a.curated_tools().map(<[_]>::len),
        b.curated_tools().map(<[_]>::len),
    );
}

// ── build_search_query ──────────────────────────────────────────────────────
//
// Regression coverage for #2418: the GitHub Memory Provider must scope the
// periodic sync to `involves:{login}` — GitHub's logical-OR over `author`,
// `assignee`, `mentions`, and `commenter` — rather than the narrower
// `assignee:{login}`. Without these assertions the qualifier could silently
// regress to assignee-only and lose author / mention / commenter coverage
// for OSS contributors who are rarely explicitly assigned.

#[test]
fn build_search_query_uses_involves_qualifier_without_cursor() {
    let query = build_search_query("octocat", None);
    assert_eq!(query, "involves:octocat");
}

#[test]
fn build_search_query_does_not_fall_back_to_assignee_qualifier() {
    let query = build_search_query("octocat", None);
    assert!(
        !query.contains("assignee:"),
        "query must not use the narrower assignee-only qualifier (see #2418): {query}"
    );
    assert!(query.starts_with("involves:"));
}

#[test]
fn build_search_query_appends_updated_clause_when_cursor_present() {
    let query = build_search_query("octocat", Some("2026-05-25T00:00:00Z"));
    assert_eq!(
        query,
        "involves:octocat updated:>2026-05-25T00:00:00Z",
        "cursor must be threaded through as an updated:> clause so incremental syncs only refetch changed items"
    );
}

#[test]
fn build_search_query_interpolates_login_verbatim() {
    let query = build_search_query("Hyphen-User_99", Some("2026-01-02T03:04:05Z"));
    assert!(query.contains("involves:Hyphen-User_99"));
    assert!(query.contains("updated:>2026-01-02T03:04:05Z"));
}
