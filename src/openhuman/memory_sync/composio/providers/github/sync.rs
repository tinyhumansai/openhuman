//! GitHub sync helpers — result extraction, identity helpers, and time utilities.
//!
//! GitHub's REST API (proxied through Composio) returns search results and
//! authenticated-user payloads in a small number of shapes. The functions here
//! walk the union of common Composio envelope variants so the provider stays
//! clean and branch-free.

use serde_json::Value;

use crate::openhuman::memory_sync::composio::providers::pick_str;

/// Walk the Composio response envelope for GitHub search issue results.
///
/// `GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS` wraps GitHub's `GET /search/issues` response, which
/// returns `{"total_count": N, "items": [...]}`. Composio may re-wrap this under
/// `data` or `data.data`; we probe each shape in order.
pub(crate) fn extract_issues(data: &Value) -> Vec<Value> {
    let candidates = [
        data.pointer("/data/items"),
        data.pointer("/items"),
        data.pointer("/data/data/items"),
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

/// Extract a stable, globally unique identifier for a GitHub issue or PR.
///
/// GitHub's internal `id` field is a large integer unique across all issues
/// and PRs on github.com. We convert it to a string for use as a sync key.
/// Falls back to composing from `html_url` path if `id` is absent.
pub(crate) fn extract_issue_id(issue: &Value) -> Option<String> {
    // Primary: numeric internal GitHub ID.
    if let Some(id) = issue.get("id").or_else(|| issue.pointer("/data/id")) {
        if let Some(n) = id.as_u64() {
            return Some(n.to_string());
        }
        if let Some(s) = id.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    // Fallback: parse owner/repo/number from html_url path segments.
    // URL shape: https://github.com/{owner}/{repo}/issues/{number}
    if let Some(url) = pick_str(issue, &["html_url", "data.html_url", "url", "data.url"]) {
        if let Some(slug) = github_url_to_slug(&url) {
            return Some(slug);
        }
    }
    None
}

/// Build a human-readable document title for a GitHub issue/PR.
///
/// Format: `GitHub: {owner}/{repo}#{number}: {title}`.
/// Falls back to just the title or a placeholder when fields are missing.
pub(crate) fn extract_issue_title(issue: &Value) -> Option<String> {
    let title = pick_str(issue, &["title", "data.title"])?;

    // Best-effort: extract owner/repo#N from html_url for the prefix.
    let prefix = pick_str(issue, &["html_url", "data.html_url"])
        .and_then(|url| github_url_to_slug(&url))
        .unwrap_or_default();

    if prefix.is_empty() {
        Some(title)
    } else {
        Some(format!("GitHub: {prefix}: {title}"))
    }
}

/// Parse `https://github.com/{owner}/{repo}/issues/{number}` (or `/pull/`)
/// into `"{owner}/{repo}#{number}"`. Returns `None` for unrecognised shapes.
fn github_url_to_slug(url: &str) -> Option<String> {
    let segs: Vec<&str> = url.trim_end_matches('/').split('/').collect();
    // Minimum: ["https:", "", "github.com", owner, repo, "issues", number]
    if segs.len() >= 7 {
        let number = segs[segs.len() - 1];
        let _kind = segs[segs.len() - 2]; // "issues" or "pull" — ignored
        let repo = segs[segs.len() - 3];
        let owner = segs[segs.len() - 4];
        if !owner.is_empty() && !repo.is_empty() && !number.is_empty() {
            return Some(format!("{owner}/{repo}#{number}"));
        }
    }
    None
}

/// Extract the `updated_at` ISO 8601 timestamp from a GitHub issue.
///
/// GitHub returns `updated_at` as `"2024-05-21T15:30:00Z"`. ISO 8601 strings
/// sort lexicographically, so we use them directly as the sync cursor.
pub(crate) fn extract_issue_updated_at(issue: &Value) -> Option<String> {
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

/// Extract the authenticated user's login handle from a
/// `GITHUB_GET_THE_AUTHENTICATED_USER` response.
pub(crate) fn extract_user_login(data: &Value) -> Option<String> {
    pick_str(data, &["login", "data.login"])
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
    fn extract_issues_from_data_items() {
        let data = json!({ "data": { "items": [{"id": 1}] } });
        assert_eq!(extract_issues(&data).len(), 1);
    }

    #[test]
    fn extract_issues_from_top_level_items() {
        let data = json!({ "items": [{"id": 1}, {"id": 2}] });
        assert_eq!(extract_issues(&data).len(), 2);
    }

    #[test]
    fn extract_issues_empty_when_missing() {
        let data = json!({ "foo": "bar" });
        assert!(extract_issues(&data).is_empty());
    }

    #[test]
    fn extract_issue_id_from_numeric_field() {
        let issue = json!({ "id": 123456789u64, "title": "Fix bug" });
        assert_eq!(extract_issue_id(&issue), Some("123456789".to_string()));
    }

    #[test]
    fn extract_issue_id_from_wrapped_data() {
        let issue = json!({ "data": { "id": 99u64 } });
        assert_eq!(extract_issue_id(&issue), Some("99".to_string()));
    }

    #[test]
    fn extract_issue_id_falls_back_to_html_url() {
        let issue = json!({
            "html_url": "https://github.com/owner/repo/issues/42"
        });
        assert_eq!(extract_issue_id(&issue), Some("owner/repo#42".to_string()));
    }

    #[test]
    fn extract_issue_id_none_when_missing() {
        let issue = json!({ "title": "No ID here" });
        assert!(extract_issue_id(&issue).is_none());
    }

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
    fn extract_issue_title_returns_raw_title_when_no_url() {
        let issue = json!({ "title": "Bare title" });
        assert_eq!(extract_issue_title(&issue), Some("Bare title".to_string()));
    }

    #[test]
    fn extract_issue_title_none_when_missing() {
        let issue = json!({ "id": 1u64 });
        assert!(extract_issue_title(&issue).is_none());
    }

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

    #[test]
    fn now_ms_returns_nonzero() {
        assert!(now_ms() > 0);
    }
}
