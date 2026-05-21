//! GitHub sync helpers — payload extraction, title / cursor / viewer
//! and the human-readable `owner/repo#number` identifier composer.
//!
//! Composio wraps GitHub's REST API responses; the exact envelope
//! varies by action (`GITHUB_SEARCH_ISSUES` returns the search
//! response under `data.items` while non-search list endpoints
//! return arrays directly). The helpers here walk the union of
//! common shapes so the provider doesn't have to branch per envelope
//! variant.

use serde_json::Value;

use crate::openhuman::composio::providers::pick_str;

/// Walk the Composio response envelope for a GitHub issue list.
///
/// `GITHUB_SEARCH_ISSUES` returns `{ total_count, items: [...] }` —
/// Composio re-wraps that under `data` or `data.data` depending on
/// the action. Non-search list endpoints (`GITHUB_LIST_REPOSITORY_ISSUES`)
/// can also surface a flat array under `data`. We probe each shape
/// in order and return the first array we find.
pub(crate) fn extract_issues(data: &Value) -> Vec<Value> {
    let candidates = [
        // Search response: { items: [...] } under Composio's data wrapper.
        data.pointer("/data/items"),
        data.pointer("/items"),
        // Flat list responses (LIST_REPOSITORY_ISSUES).
        data.pointer("/data/issues"),
        data.pointer("/issues"),
        // Generic envelope fallbacks (Notion/ClickUp/Linear-style).
        data.pointer("/data/results"),
        data.pointer("/results"),
    ];
    for cand in candidates.into_iter().flatten() {
        if let Some(arr) = cand.as_array() {
            return arr.clone();
        }
    }
    Vec::new()
}

/// Extract a human-readable title from a GitHub issue object.
///
/// GitHub issues store the title at the top-level `title` field; we
/// fall back to a few shapes Composio's wrapping might produce.
pub(crate) fn extract_issue_title(issue: &Value) -> Option<String> {
    pick_str(issue, &["title", "data.title", "name", "data.name"])
}

/// Extract the cursor timestamp from a GitHub issue object.
///
/// GitHub returns `updated_at` as an ISO 8601 string
/// (`"2026-05-21T10:30:00Z"`); we keep it as a string so
/// lexicographic comparison against the stored cursor remains valid.
pub(crate) fn extract_issue_updated(issue: &Value) -> Option<String> {
    pick_str(
        issue,
        &[
            "updated_at",
            "data.updated_at",
            "updatedAt",
            "data.updatedAt",
        ],
    )
}

/// Compose GitHub's canonical human-readable issue identifier
/// (`owner/repo#number`, e.g. `tinyhumansai/openhuman#2400`).
///
/// The issue payload doesn't store `owner/repo` directly — it lives
/// in `repository_url` (`https://api.github.com/repos/owner/repo`)
/// and `repository.full_name` depending on whether the result came
/// from `LIST_REPOSITORY_ISSUES` or `SEARCH_ISSUES`. We probe both
/// shapes and combine with the per-repo `number`.
pub(crate) fn extract_repo_qualified_identifier(issue: &Value) -> Option<String> {
    let number = issue
        .get("number")
        .or_else(|| issue.get("data").and_then(|d| d.get("number")))
        .and_then(|n| {
            // GitHub issue numbers are integers in the API, but be
            // tolerant of stringified numbers from Composio wrappers.
            n.as_u64()
                .map(|v| v.to_string())
                .or_else(|| n.as_i64().map(|v| v.to_string()))
                .or_else(|| n.as_str().map(|s| s.to_string()))
        })?;

    // `repository.full_name` is the easy case — present on
    // SEARCH_ISSUES results.
    if let Some(full) = pick_str(
        issue,
        &[
            "repository.full_name",
            "data.repository.full_name",
            "repo.full_name",
            "data.repo.full_name",
        ],
    ) {
        return Some(format!("{full}#{number}"));
    }

    // `repository_url` is `https://api.github.com/repos/<owner>/<repo>`
    // — parse the suffix.
    if let Some(repo_url) = pick_str(issue, &["repository_url", "data.repository_url"]) {
        let owner_repo = repo_url
            .rsplit_once("/repos/")
            .map(|(_, tail)| tail)
            .unwrap_or(&repo_url)
            .trim_matches('/');
        if !owner_repo.is_empty() && owner_repo.contains('/') {
            return Some(format!("{owner_repo}#{number}"));
        }
    }

    None
}

/// Extract the authenticated viewer's `login` (GitHub username) from
/// a `GITHUB_GET_AUTHENTICATED_USER` response.
///
/// Only explicit `login` paths are probed — generic top-level `id` /
/// `name` fallbacks were intentionally **not** included. This value
/// drives the `assignee:<login>` search filter for the whole sync, so
/// picking up a non-viewer identifier (e.g. a top-level `id` that
/// Composio surfaced from a different action) would silently scope
/// the sync to the wrong user and could leak issues from another
/// contributor. Same safety rationale as `linear::extract_viewer_id`
/// (per the CodeRabbit feedback on PR #2402).
pub(crate) fn extract_viewer_login(data: &Value) -> Option<String> {
    pick_str(
        data,
        &[
            "login",
            "data.login",
            "user.login",
            "data.user.login",
            "viewer.login",
            "data.viewer.login",
        ],
    )
}

/// Extract the authenticated viewer's numeric `id` from a
/// `GITHUB_GET_AUTHENTICATED_USER` response. Returned as a string
/// because it's used as metadata, not arithmetic.
///
/// Optional — sync doesn't require it. Useful for profile facets.
pub(crate) fn extract_viewer_id(data: &Value) -> Option<String> {
    let candidates = [
        data.pointer("/id"),
        data.pointer("/data/id"),
        data.pointer("/user/id"),
        data.pointer("/data/user/id"),
    ];
    for cand in candidates.into_iter().flatten() {
        if let Some(n) = cand.as_u64() {
            return Some(n.to_string());
        }
        if let Some(n) = cand.as_i64() {
            return Some(n.to_string());
        }
        if let Some(s) = cand.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Current wall-clock time in milliseconds since the UNIX epoch.
pub(crate) fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_issues_from_search_data_items() {
        let data = json!({
            "data": { "items": [{"id": 1}, {"id": 2}] }
        });
        assert_eq!(extract_issues(&data).len(), 2);
    }

    #[test]
    fn extract_issues_from_top_level_items() {
        let data = json!({ "items": [{"id": 1}] });
        assert_eq!(extract_issues(&data).len(), 1);
    }

    #[test]
    fn extract_issues_from_flat_data_issues() {
        let data = json!({ "data": { "issues": [{"id": 1}, {"id": 2}, {"id": 3}] } });
        assert_eq!(extract_issues(&data).len(), 3);
    }

    #[test]
    fn extract_issues_empty_when_missing() {
        let data = json!({ "foo": "bar" });
        assert!(extract_issues(&data).is_empty());
    }

    #[test]
    fn extract_issue_title_from_title_field() {
        let issue = json!({ "title": "Bug: thing broken" });
        assert_eq!(
            extract_issue_title(&issue),
            Some("Bug: thing broken".into())
        );
    }

    #[test]
    fn extract_issue_title_falls_back_to_data_title() {
        let issue = json!({ "data": { "title": "Wrapped title" } });
        assert_eq!(extract_issue_title(&issue), Some("Wrapped title".into()));
    }

    #[test]
    fn extract_issue_title_none_when_missing() {
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
        let issue = json!({ "data": { "updatedAt": "2026-04-01T00:00:00Z" } });
        assert_eq!(
            extract_issue_updated(&issue),
            Some("2026-04-01T00:00:00Z".to_string())
        );
    }

    #[test]
    fn extract_issue_updated_none_when_missing() {
        let issue = json!({ "id": 1 });
        assert!(extract_issue_updated(&issue).is_none());
    }

    #[test]
    fn extract_repo_qualified_identifier_from_repository_full_name() {
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
        // SEARCH_ISSUES results expose `repository_url` rather than a
        // nested `repository` object — make sure we parse the suffix.
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
    fn extract_repo_qualified_identifier_none_when_number_missing() {
        let issue = json!({
            "repository": { "full_name": "owner/repo" }
        });
        assert!(extract_repo_qualified_identifier(&issue).is_none());
    }

    #[test]
    fn extract_repo_qualified_identifier_none_when_repo_missing() {
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
    fn extract_viewer_login_none_when_missing() {
        // No login / user.login / viewer.login present — must NOT fall
        // back to a top-level `id` or `name` (would scope the assignee
        // search filter to the wrong identifier).
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
    fn now_ms_returns_nonzero() {
        assert!(now_ms() > 0);
    }
}
