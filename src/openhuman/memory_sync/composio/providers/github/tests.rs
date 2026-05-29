//! Unit tests for the GitHub Composio provider.

use super::provider::{build_search_query, ACTION_GET_AUTHENTICATED_USER, ACTION_SEARCH_ISSUES};
use super::sync::{
    extract_issue_id, extract_issue_title, extract_issue_updated_at, extract_issues,
    extract_user_login,
};
use super::tools::GITHUB_CURATED;
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
    assert!(slugs.contains(&"GITHUB_GET_THE_AUTHENTICATED_USER"));
    assert!(slugs.contains(&"GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS"));
    assert!(slugs.contains(&"GITHUB_LIST_REPOSITORY_ISSUES"));
    assert!(slugs.contains(&"GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER"));
    assert!(slugs.contains(&"GITHUB_CREATE_A_REPOSITORY_FOR_THE_AUTHENTICATED_USER"));
    // DELETE_A_REFERENCE replaces DELETE_A_BRANCH (Composio v3 rename).
    assert!(slugs.contains(&"GITHUB_DELETE_A_REFERENCE"));
    // CLOSE_AN_ISSUE was removed — callers must use UPDATE_AN_ISSUE with state:"closed".
    assert!(
        !slugs.contains(&"GITHUB_CLOSE_AN_ISSUE"),
        "GITHUB_CLOSE_AN_ISSUE was removed — use GITHUB_UPDATE_AN_ISSUE with state:closed"
    );
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

// ── slug regression tests (#2768) ───────────────────────────────────────────
//
// Guard the current Composio action slug values used by the GitHub provider.
// Outdated slugs (e.g. GITHUB_USERS_GET_AUTHENTICATED, GITHUB_LIST_REPOS,
// GITHUB_LIST_ISSUES) were previously scattered across tests; these assertions
// pin the correct values in one place so a slug rename is caught immediately.

#[test]
fn action_get_authenticated_user_slug_is_current() {
    // The Composio v3 slug is GITHUB_GET_THE_AUTHENTICATED_USER.
    // Regression: was mistakenly referenced as GITHUB_USERS_GET_AUTHENTICATED
    // in tests (see issue #2768).
    assert_eq!(
        ACTION_GET_AUTHENTICATED_USER, "GITHUB_GET_THE_AUTHENTICATED_USER",
        "slug must match Composio v3 catalog; old slug GITHUB_USERS_GET_AUTHENTICATED is retired"
    );
}

#[test]
fn action_search_issues_slug_is_current() {
    assert_eq!(
        ACTION_SEARCH_ISSUES, "GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS",
        "slug must match Composio v3 catalog"
    );
}

#[test]
fn curated_list_does_not_contain_retired_slugs() {
    // Guard against re-introducing removed slugs that no longer exist in the
    // Composio v3 GitHub app catalog.
    const RETIRED: &[&str] = &[
        "GITHUB_USERS_GET_AUTHENTICATED", // replaced by GITHUB_GET_THE_AUTHENTICATED_USER
        "GITHUB_LIST_REPOS", // replaced by GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER
        "GITHUB_LIST_ISSUES", // replaced by GITHUB_LIST_REPOSITORY_ISSUES
        "GITHUB_COMMIT_MULTIPLE_FILES", // removed from Composio catalog
        "GITHUB_CLOSE_AN_ISSUE", // removed; use GITHUB_UPDATE_AN_ISSUE with state=closed
        "GITHUB_DELETE_A_BRANCH", // removed; use GITHUB_DELETE_A_REFERENCE
    ];

    let slugs: Vec<&str> = GITHUB_CURATED.iter().map(|t| t.slug).collect();
    for retired in RETIRED {
        assert!(
            !slugs.contains(retired),
            "curated list must not contain retired slug {retired} (see #2768)"
        );
    }
}

#[test]
fn curated_list_contains_current_read_slugs() {
    // Verify that the primary read-tier actions are present with their correct
    // v3 slug names (not the old v1/v2 names).
    let slugs: Vec<&str> = GITHUB_CURATED.iter().map(|t| t.slug).collect();
    let required = [
        "GITHUB_GET_THE_AUTHENTICATED_USER",
        "GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER",
        "GITHUB_LIST_REPOSITORY_ISSUES",
        "GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS",
        "GITHUB_LIST_PULL_REQUESTS",
        "GITHUB_GET_A_PULL_REQUEST",
    ];
    for slug in required {
        assert!(
            slugs.contains(&slug),
            "curated list must contain current slug {slug} (see #2768)"
        );
    }
}

#[test]
fn curated_list_contains_current_write_slugs() {
    let slugs: Vec<&str> = GITHUB_CURATED.iter().map(|t| t.slug).collect();
    let required = [
        "GITHUB_CREATE_AN_ISSUE",
        "GITHUB_UPDATE_AN_ISSUE",
        "GITHUB_CREATE_A_PULL_REQUEST",
        "GITHUB_MERGE_A_PULL_REQUEST",
    ];
    for slug in required {
        assert!(
            slugs.contains(&slug),
            "curated list must contain current write slug {slug} (see #2768)"
        );
    }
}
