//! Unit tests for the Linear provider.

use super::sync::{
    extract_issue_identifier, extract_issue_title, extract_issue_updated, extract_issues,
    extract_pagination_end_cursor, extract_viewer_id,
};
use super::LinearProvider;
use crate::openhuman::composio::providers::ComposioProvider;
use serde_json::json;

#[test]
fn extract_issues_walks_common_shapes() {
    let v1 = json!({ "data": { "issues": { "nodes": [{"id": "a"}, {"id": "b"}] } } });
    let v2 = json!({ "nodes": [{"id": "c"}] });
    let v3 = json!({ "data": {} });
    assert_eq!(extract_issues(&v1).len(), 2);
    assert_eq!(extract_issues(&v2).len(), 1);
    assert_eq!(extract_issues(&v3).len(), 0);
}

#[test]
fn extract_issues_handles_flat_data_issues_array() {
    // Some Composio wrappings drop the GraphQL connection node and
    // surface a flat list under `data.issues`.
    let data = json!({ "data": { "issues": [{"id": "x"}, {"id": "y"}] } });
    assert_eq!(extract_issues(&data).len(), 2);
}

#[test]
fn extract_issue_title_finds_title_field() {
    let issue = json!({ "id": "u1", "title": "Build feature X" });
    assert_eq!(extract_issue_title(&issue), Some("Build feature X".into()));
}

#[test]
fn extract_issue_title_falls_back_to_identifier() {
    // If `title` is missing, the workspace-prefixed identifier
    // (e.g. `OH-42`) is the next-best human-readable handle.
    let issue = json!({ "id": "u1", "identifier": "OH-42" });
    assert_eq!(extract_issue_title(&issue), Some("OH-42".into()));
}

#[test]
fn extract_issue_title_none_when_missing() {
    let issue = json!({ "id": "u1" });
    assert!(extract_issue_title(&issue).is_none());
}

#[test]
fn extract_issue_identifier_returns_workspace_prefix() {
    let issue = json!({ "id": "u1", "identifier": "OH-123" });
    assert_eq!(extract_issue_identifier(&issue), Some("OH-123".into()));
}

#[test]
fn extract_issue_identifier_none_when_missing() {
    // identifier-less issue must not be conflated with the UUID id.
    let issue = json!({ "id": "uuid-here", "title": "Untitled" });
    assert!(extract_issue_identifier(&issue).is_none());
}

#[test]
fn extract_issue_updated_handles_iso_string() {
    let issue = json!({ "updatedAt": "2026-05-21T10:30:00.000Z" });
    assert_eq!(
        extract_issue_updated(&issue),
        Some("2026-05-21T10:30:00.000Z".to_string())
    );
}

#[test]
fn extract_issue_updated_handles_snake_case_alias() {
    let issue = json!({ "data": { "updated_at": "2026-04-01T00:00:00Z" } });
    assert_eq!(
        extract_issue_updated(&issue),
        Some("2026-04-01T00:00:00Z".to_string())
    );
}

#[test]
fn extract_issue_updated_none_when_missing() {
    let issue = json!({ "id": "x" });
    assert!(extract_issue_updated(&issue).is_none());
}

#[test]
fn extract_viewer_id_from_data_viewer() {
    let data = json!({ "data": { "viewer": { "id": "user-uuid-1" } } });
    assert_eq!(extract_viewer_id(&data), Some("user-uuid-1".into()));
}

#[test]
fn extract_viewer_id_none_when_missing() {
    let data = json!({ "foo": "bar" });
    assert!(extract_viewer_id(&data).is_none());
}

#[test]
fn extract_pagination_end_cursor_returns_cursor_when_has_next() {
    let data = json!({
        "data": {
            "issues": {
                "pageInfo": {
                    "hasNextPage": true,
                    "endCursor": "cur-abc-123"
                }
            }
        }
    });
    assert_eq!(
        extract_pagination_end_cursor(&data),
        Some("cur-abc-123".to_string())
    );
}

#[test]
fn extract_pagination_end_cursor_none_when_has_next_is_false() {
    // hasNextPage: false MUST suppress the cursor — otherwise the
    // caller would infinite-loop on the same end-of-results page.
    let data = json!({
        "data": {
            "issues": {
                "pageInfo": {
                    "hasNextPage": false,
                    "endCursor": "cur-final"
                }
            }
        }
    });
    assert_eq!(extract_pagination_end_cursor(&data), None);
}

#[test]
fn extract_pagination_end_cursor_none_when_pageinfo_missing() {
    let data = json!({ "data": { "issues": { "nodes": [] } } });
    assert_eq!(extract_pagination_end_cursor(&data), None);
}

#[test]
fn extract_pagination_end_cursor_skips_empty_cursor_string() {
    // hasNextPage:true but endCursor is whitespace — must NOT return
    // the blank cursor, otherwise the caller would loop forever
    // requesting the same end-of-results page. Moved here from the
    // sync.rs inline tests as part of the dedup pass on graycyrus's
    // #2402 review feedback.
    let data = json!({
        "pageInfo": {
            "hasNextPage": true,
            "endCursor": "   "
        }
    });
    assert_eq!(extract_pagination_end_cursor(&data), None);
}

#[test]
fn provider_metadata_is_stable() {
    let p = LinearProvider::new();
    assert_eq!(p.toolkit_slug(), "linear");
    assert_eq!(p.sync_interval_secs(), Some(30 * 60));
    assert!(p.curated_tools().is_some());
}

#[test]
fn curated_tools_contains_core_read_surface() {
    let p = LinearProvider::new();
    let curated = p.curated_tools().expect("LINEAR_CURATED is registered");
    let slugs: Vec<&str> = curated.iter().map(|t| t.slug).collect();
    // The two actions the sync path depends on must be advertised.
    assert!(
        slugs.contains(&"LINEAR_GET_VIEWER"),
        "LINEAR_GET_VIEWER must be curated — sync depends on it for viewer-id resolution"
    );
    assert!(
        slugs.contains(&"LINEAR_LIST_LINEAR_ISSUES"),
        "LINEAR_LIST_LINEAR_ISSUES must be curated — sync depends on it for paginated fetch"
    );
}

#[test]
fn default_impl_matches_new() {
    // `LinearProvider` is a unit struct, so we compare observable
    // trait surface instead of deriving `PartialEq`. Same shape as
    // the equivalent test in `clickup::tests` — catches a future
    // regression where `new()` and `default()` drift apart.
    let a = LinearProvider::new();
    let b = LinearProvider::default();
    assert_eq!(a.toolkit_slug(), b.toolkit_slug());
    assert_eq!(a.sync_interval_secs(), b.sync_interval_secs());
    assert_eq!(
        a.curated_tools().map(<[_]>::len),
        b.curated_tools().map(<[_]>::len),
    );
}
