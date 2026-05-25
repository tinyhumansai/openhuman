//! Linear sync helpers — result extraction, issue-title extraction,
//! viewer identity, cursor extraction, and time utilities.
//!
//! Linear's GraphQL API (and therefore Composio's wrapping of it) returns
//! connection-style lists (`{ nodes: [...], pageInfo: {...} }`) at the top
//! level or nested under `data`. The functions here walk the union of
//! common shapes so the provider does not have to branch per Composio
//! envelope variant.

use serde_json::Value;

use crate::openhuman::memory_sync::composio::providers::pick_str;

/// Walk the Composio response envelope for Linear issue list results.
///
/// Linear's list endpoints return `{ nodes: [...] }` or
/// `{ issues: { nodes: [...] } }` shapes; Composio may re-wrap the
/// upstream payload under `data` or `data.data`. We probe each shape
/// in order and return the first array we find.
pub(crate) fn extract_issues(data: &Value) -> Vec<Value> {
    let candidates = [
        data.pointer("/data/nodes"),
        data.pointer("/nodes"),
        data.pointer("/data/data/nodes"),
        data.pointer("/data/issues/nodes"),
        data.pointer("/data/results"),
        data.pointer("/results"),
        data.pointer("/data/items"),
        data.pointer("/items"),
    ];
    for cand in candidates.into_iter().flatten() {
        if let Some(arr) = cand.as_array() {
            return arr.clone();
        }
    }
    Vec::new()
}

/// Extract a human-readable title from a Linear issue object.
///
/// Linear issues store the name at `title` (or `data.title` after
/// Composio envelope wrapping). Falls back to `name` / `identifier`
/// so the chunk remains identifiable even for unusual response shapes.
pub(crate) fn extract_issue_title(issue: &Value) -> Option<String> {
    pick_str(
        issue,
        &[
            "title",
            "data.title",
            "name",
            "data.name",
            "identifier",
            "data.identifier",
        ],
    )
}

/// Extract a stable cursor timestamp from a Linear issue object.
///
/// Linear uses ISO-8601 strings for timestamps (`updatedAt`). We keep
/// the value as a string so lexicographic comparison against the stored
/// cursor is valid.
pub(crate) fn extract_issue_updated(issue: &Value) -> Option<String> {
    pick_str(
        issue,
        &[
            "updatedAt",
            "data.updatedAt",
            "updated_at",
            "data.updated_at",
        ],
    )
}

/// Extract the viewer (authenticated user) object from a
/// `LINEAR_LIST_LINEAR_USERS { isMe: true }` response.
///
/// Linear's GraphQL viewer endpoint returns `{ nodes: [{ id, email, … }] }`.
/// Composio may wrap this under `data` or `data.data`. We probe each
/// shape and return the first element of the nodes array, falling back
/// to the payload itself if it looks like a direct user object (has
/// `id` or `email`).
pub(crate) fn extract_viewer(data: &Value) -> Option<Value> {
    let array_candidates = [
        data.pointer("/data/nodes"),
        data.pointer("/nodes"),
        data.pointer("/data/data/nodes"),
        data.pointer("/data/users/nodes"),
    ];
    for cand in array_candidates.into_iter().flatten() {
        if let Some(arr) = cand.as_array() {
            if let Some(first) = arr.first() {
                return Some(first.clone());
            }
        }
    }
    // Fallback: if the payload itself looks like a user object, return it.
    if data.get("id").is_some() || data.get("email").is_some() {
        return Some(data.clone());
    }
    None
}

/// Extract the viewer's ID string from a `LINEAR_LIST_LINEAR_USERS`
/// response. Returns `None` if the payload does not contain a
/// recognizable user ID.
pub(crate) fn extract_viewer_id(data: &Value) -> Option<String> {
    let viewer = extract_viewer(data)?;
    pick_str(&viewer, &["id", "data.id"])
}

/// Extract a pagination cursor from a Linear connection `pageInfo` block.
///
/// Returns `Some(endCursor)` only when `hasNextPage` is `true`;
/// `None` when the last page has been reached or when the envelope does
/// not carry `pageInfo` at all.
pub(crate) fn extract_pagination_cursor(data: &Value) -> Option<String> {
    let page_info_candidates = [
        data.pointer("/data/pageInfo"),
        data.pointer("/pageInfo"),
        data.pointer("/data/data/pageInfo"),
        data.pointer("/data/issues/pageInfo"),
    ];
    for cand in page_info_candidates.into_iter().flatten() {
        let has_next = cand
            .get("hasNextPage")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if has_next {
            if let Some(cursor) = cand.get("endCursor").and_then(|v| v.as_str()) {
                let trimmed = cursor.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
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

    // ── extract_issues ───────────────────────────────────────────────

    #[test]
    fn extract_issues_from_data_nodes() {
        let data = json!({ "data": { "nodes": [{"id": "i1"}, {"id": "i2"}] } });
        assert_eq!(extract_issues(&data).len(), 2);
    }

    #[test]
    fn extract_issues_from_top_level_nodes() {
        let data = json!({ "nodes": [{"id": "i3"}] });
        assert_eq!(extract_issues(&data).len(), 1);
    }

    #[test]
    fn extract_issues_from_data_issues_nodes() {
        let data = json!({ "data": { "issues": { "nodes": [{"id": "i4"}, {"id": "i5"}, {"id": "i6"}] } } });
        assert_eq!(extract_issues(&data).len(), 3);
    }

    #[test]
    fn extract_issues_from_results() {
        let data = json!({ "results": [{"id": "i7"}] });
        assert_eq!(extract_issues(&data).len(), 1);
    }

    #[test]
    fn extract_issues_empty_when_missing() {
        let data = json!({ "foo": "bar" });
        assert!(extract_issues(&data).is_empty());
    }

    // ── extract_issue_title ──────────────────────────────────────────

    #[test]
    fn extract_issue_title_from_title_field() {
        let issue = json!({ "id": "i1", "title": "Fix the login bug" });
        assert_eq!(
            extract_issue_title(&issue),
            Some("Fix the login bug".into())
        );
    }

    #[test]
    fn extract_issue_title_falls_back_to_wrapped_data() {
        let issue = json!({ "data": { "title": "Wrapped issue" } });
        assert_eq!(extract_issue_title(&issue), Some("Wrapped issue".into()));
    }

    #[test]
    fn extract_issue_title_falls_back_to_identifier() {
        let issue = json!({ "identifier": "ENG-42" });
        assert_eq!(extract_issue_title(&issue), Some("ENG-42".into()));
    }

    // ── extract_issue_updated ────────────────────────────────────────

    #[test]
    fn extract_issue_updated_from_updated_at() {
        let issue = json!({ "updatedAt": "2026-03-01T12:00:00.000Z" });
        assert_eq!(
            extract_issue_updated(&issue),
            Some("2026-03-01T12:00:00.000Z".to_string())
        );
    }

    #[test]
    fn extract_issue_updated_falls_back_to_snake_case() {
        let issue = json!({ "data": { "updated_at": "2026-01-15T08:30:00.000Z" } });
        assert_eq!(
            extract_issue_updated(&issue),
            Some("2026-01-15T08:30:00.000Z".to_string())
        );
    }

    // ── extract_viewer ───────────────────────────────────────────────

    #[test]
    fn extract_viewer_from_data_nodes() {
        let data = json!({ "data": { "nodes": [{ "id": "usr_1", "email": "a@b.com" }] } });
        let v = extract_viewer(&data).expect("should find viewer");
        assert_eq!(v["id"], "usr_1");
    }

    #[test]
    fn extract_viewer_from_top_level_nodes() {
        let data = json!({ "nodes": [{ "id": "usr_2" }] });
        let v = extract_viewer(&data).expect("should find viewer");
        assert_eq!(v["id"], "usr_2");
    }

    #[test]
    fn extract_viewer_fallback_direct_object() {
        let data = json!({ "id": "usr_direct", "name": "Direct User" });
        let v = extract_viewer(&data).expect("should return direct object");
        assert_eq!(v["id"], "usr_direct");
    }

    #[test]
    fn extract_viewer_returns_none_when_absent() {
        let data = json!({ "foo": "bar" });
        assert!(extract_viewer(&data).is_none());
    }

    // ── extract_pagination_cursor ────────────────────────────────────

    #[test]
    fn extract_pagination_cursor_returns_cursor_when_has_next_page() {
        let data = json!({
            "data": {
                "pageInfo": {
                    "hasNextPage": true,
                    "endCursor": "cursor_abc"
                }
            }
        });
        assert_eq!(
            extract_pagination_cursor(&data),
            Some("cursor_abc".to_string())
        );
    }

    #[test]
    fn extract_pagination_cursor_returns_none_when_last_page() {
        let data = json!({
            "pageInfo": {
                "hasNextPage": false,
                "endCursor": "cursor_xyz"
            }
        });
        assert!(extract_pagination_cursor(&data).is_none());
    }

    #[test]
    fn extract_pagination_cursor_returns_none_when_absent() {
        let data = json!({ "nodes": [{"id": "i1"}] });
        assert!(extract_pagination_cursor(&data).is_none());
    }

    // ── now_ms ───────────────────────────────────────────────────────

    #[test]
    fn now_ms_returns_nonzero() {
        assert!(now_ms() > 0);
    }
}
