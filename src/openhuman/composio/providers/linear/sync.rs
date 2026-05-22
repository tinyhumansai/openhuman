//! Linear sync helpers — result extraction, title / cursor / user-id
//! / pagination, and time utilities.
//!
//! Linear's GraphQL API is wrapped by Composio actions into REST-shaped
//! responses. The exact envelope varies by action (`LINEAR_LIST_LINEAR_ISSUES`
//! returns `{ issues: { nodes: [...], pageInfo: {...} } }` while the
//! generic `data` wrapper may also nest it under `data.issues.nodes`).
//! The helpers here walk the union of common shapes so the provider
//! doesn't have to branch per envelope variant.

use serde_json::Value;

use crate::openhuman::composio::providers::pick_str;

/// Walk the Composio response envelope for Linear issue list results.
///
/// Linear's GraphQL `issues` query returns `{ nodes: [...] }` under
/// the issue connection. Composio re-wraps the upstream payload under
/// `data` or `data.data` depending on the action, and sometimes flattens
/// the connection. We probe each shape in order and return the first
/// array we find.
pub(crate) fn extract_issues(data: &Value) -> Vec<Value> {
    let candidates = [
        // Composio's standard "data.issues.nodes" shape for LINEAR_LIST_LINEAR_ISSUES.
        data.pointer("/data/issues/nodes"),
        data.pointer("/issues/nodes"),
        // Some Composio wrappings drop the GraphQL connection and surface a flat list.
        data.pointer("/data/issues"),
        data.pointer("/issues"),
        // Generic envelope fallbacks (Notion/ClickUp-style).
        data.pointer("/data/results"),
        data.pointer("/results"),
        data.pointer("/data/items"),
        data.pointer("/items"),
        data.pointer("/data/nodes"),
        data.pointer("/nodes"),
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
/// Linear issues store the human title at `title`. When missing we
/// fall back to the human-readable `identifier` (e.g. `"OH-123"`) so
/// chunks remain identifiable in the memory tree even if the issue
/// was created without a title.
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

/// Extract Linear's human-readable issue identifier (e.g. `"OH-123"`).
///
/// Used as a secondary surface for tag-record metadata, where the raw
/// UUID `id` is less useful than the workspace-prefixed identifier.
pub(crate) fn extract_issue_identifier(issue: &Value) -> Option<String> {
    pick_str(issue, &["identifier", "data.identifier"])
}

/// Extract the cursor timestamp from a Linear issue object.
///
/// Linear returns `updatedAt` as an ISO 8601 string
/// (`"2026-05-21T10:30:00.000Z"`); we keep it as a string so
/// lexicographic comparison against the stored cursor remains valid.
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

/// Extract the authenticated viewer's `id` (UUID string) from the
/// `LINEAR_GET_VIEWER` response.
///
/// Composio wraps the upstream `{ viewer: { id: …, name: … } }` GraphQL
/// shape; this walker is defensive against both raw and wrapped
/// payloads. The id is returned as a string because
/// `LINEAR_LIST_LINEAR_ISSUES` accepts the `assignee` filter as either
/// a string or `{ id: { eq: "..." } }`.
///
/// Only explicit viewer/user paths are probed — generic top-level
/// `id` / `data.id` fallbacks were intentionally removed. This value
/// drives the assignee filter for the whole sync, so picking up a
/// non-viewer identifier (e.g. the first item id in a list response
/// that Composio collapsed) would silently scope the sync to the
/// wrong user and leak issues from another teammate. Stricter is
/// safer; if Composio surfaces the viewer at a new shape we can add
/// it explicitly here.
pub(crate) fn extract_viewer_id(data: &Value) -> Option<String> {
    pick_str(
        data,
        &["viewer.id", "data.viewer.id", "user.id", "data.user.id"],
    )
}

/// Extract Linear's Relay-style end cursor for pagination, if the page
/// info indicates more results are available.
///
/// Linear's GraphQL connections expose `pageInfo: { hasNextPage, endCursor }`.
/// We surface the cursor only when `hasNextPage` is true so the
/// caller can use `.is_some()` as the "fetch another page" signal.
pub(crate) fn extract_pagination_end_cursor(data: &Value) -> Option<String> {
    let page_info_candidates = [
        data.pointer("/data/issues/pageInfo"),
        data.pointer("/issues/pageInfo"),
        data.pointer("/data/pageInfo"),
        data.pointer("/pageInfo"),
    ];
    for page_info in page_info_candidates.into_iter().flatten() {
        let has_next = page_info
            .get("hasNextPage")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !has_next {
            continue;
        }
        if let Some(cursor) = page_info.get("endCursor").and_then(Value::as_str) {
            let trimmed = cursor.trim();
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
