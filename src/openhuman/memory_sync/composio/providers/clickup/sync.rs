//! ClickUp sync helpers — result extraction, task-title extraction,
//! and time utilities.
//!
//! ClickUp's REST API (and therefore Composio's wrapping of it) returns
//! task lists in a small handful of shapes depending on which endpoint
//! is called. The functions here walk the union of common shapes so the
//! provider doesn't have to branch per Composio envelope variant.

use serde_json::Value;

use crate::openhuman::memory_sync::composio::providers::pick_str;

/// Walk the Composio response envelope for ClickUp task list results.
///
/// ClickUp's "filtered team tasks" endpoint returns `{ "tasks": [...] }`
/// at the top level; Composio re-wraps the upstream payload under
/// `data` or `data.data` depending on the action. We probe each shape
/// in order and return the first array we find.
pub(crate) fn extract_tasks(data: &Value) -> Vec<Value> {
    let candidates = [
        data.pointer("/data/tasks"),
        data.pointer("/tasks"),
        data.pointer("/data/data/tasks"),
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

/// Extract a human-readable title from a ClickUp task object.
///
/// ClickUp tasks store the name at `name` (or `data.name` after Composio
/// envelope wrapping). When the name is missing we fall back to the
/// task ID so chunks remain identifiable.
pub(crate) fn extract_task_name(task: &Value) -> Option<String> {
    pick_str(task, &["name", "data.name", "title", "data.title"])
}

/// Extract a stable cursor timestamp (milliseconds since epoch as a
/// string) from a ClickUp task object.
///
/// The ClickUp API returns `date_updated` as a stringified epoch ms
/// (e.g. `"1733412345678"`); we keep it as a string so lexicographic
/// comparison against the stored cursor remains valid as long as the
/// length doesn't change (it won't until year 33658).
pub(crate) fn extract_task_updated(task: &Value) -> Option<String> {
    pick_str(
        task,
        &[
            "date_updated",
            "data.date_updated",
            "updated_at",
            "data.updated_at",
            "dateUpdated",
            "data.dateUpdated",
        ],
    )
}

/// Current wall-clock time in milliseconds since the UNIX epoch.
pub(crate) fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Extract the authorized user's numeric ID from the
/// `CLICKUP_GET_AUTHORIZED_USER` response.
///
/// Composio wraps the upstream `{"user": {"id": …}}` shape; this walker
/// is defensive against both raw and wrapped payloads. Returns the ID
/// as a string because `CLICKUP_GET_FILTERED_TEAM_TASKS` accepts the
/// `assignees` filter as a string array.
pub(crate) fn extract_user_id(data: &Value) -> Option<String> {
    let candidates = [
        data.pointer("/user/id"),
        data.pointer("/data/user/id"),
        data.pointer("/id"),
        data.pointer("/data/id"),
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

/// Extract a list of workspace (team) IDs from the
/// `CLICKUP_GET_AUTHORIZED_TEAMS_WORKSPACES` response.
///
/// ClickUp returns `{"teams": [{"id": "...", "name": "..."}, …]}`. We
/// keep the IDs as strings — `CLICKUP_GET_FILTERED_TEAM_TASKS` requires
/// a `team_id` (string) argument.
pub(crate) fn extract_workspace_ids(data: &Value) -> Vec<String> {
    let candidates = [
        data.pointer("/teams"),
        data.pointer("/data/teams"),
        data.pointer("/workspaces"),
        data.pointer("/data/workspaces"),
    ];
    for cand in candidates.into_iter().flatten() {
        if let Some(arr) = cand.as_array() {
            return arr
                .iter()
                .filter_map(|t| pick_str(t, &["id", "team_id", "workspace_id"]))
                .collect();
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_tasks_from_data_tasks() {
        let data = json!({ "data": { "tasks": [{"id": "t1"}] } });
        assert_eq!(extract_tasks(&data).len(), 1);
    }

    #[test]
    fn extract_tasks_from_top_level_tasks() {
        let data = json!({ "tasks": [{"id": "a"}, {"id": "b"}] });
        assert_eq!(extract_tasks(&data).len(), 2);
    }

    #[test]
    fn extract_tasks_empty_when_missing() {
        let data = json!({ "foo": "bar" });
        assert!(extract_tasks(&data).is_empty());
    }

    #[test]
    fn extract_task_name_from_top_level() {
        let task = json!({ "id": "t1", "name": "Build feature X" });
        assert_eq!(extract_task_name(&task), Some("Build feature X".into()));
    }

    #[test]
    fn extract_task_name_falls_back_to_data_name() {
        let task = json!({ "data": { "name": "Wrapped" } });
        assert_eq!(extract_task_name(&task), Some("Wrapped".into()));
    }

    #[test]
    fn extract_task_name_none_when_missing() {
        let task = json!({ "id": "t1" });
        assert!(extract_task_name(&task).is_none());
    }

    #[test]
    fn extract_task_updated_handles_string_form() {
        let task = json!({ "date_updated": "1733412345678" });
        assert_eq!(
            extract_task_updated(&task),
            Some("1733412345678".to_string())
        );
    }

    #[test]
    fn extract_task_updated_handles_nested_data() {
        let task = json!({ "data": { "dateUpdated": "1700000000000" } });
        assert_eq!(
            extract_task_updated(&task),
            Some("1700000000000".to_string())
        );
    }

    #[test]
    fn extract_user_id_handles_numeric_id() {
        let data = json!({ "user": { "id": 12345 } });
        assert_eq!(extract_user_id(&data), Some("12345".to_string()));
    }

    #[test]
    fn extract_user_id_handles_wrapped_payload() {
        let data = json!({ "data": { "user": { "id": "777" } } });
        assert_eq!(extract_user_id(&data), Some("777".to_string()));
    }

    #[test]
    fn extract_user_id_none_when_missing() {
        let data = json!({ "foo": "bar" });
        assert!(extract_user_id(&data).is_none());
    }

    #[test]
    fn extract_workspace_ids_from_teams_array() {
        let data = json!({
            "teams": [
                { "id": "ws1", "name": "Personal" },
                { "id": "ws2", "name": "Acme" },
            ]
        });
        assert_eq!(extract_workspace_ids(&data), vec!["ws1", "ws2"]);
    }

    #[test]
    fn extract_workspace_ids_empty_when_no_teams() {
        let data = json!({ "foo": "bar" });
        assert!(extract_workspace_ids(&data).is_empty());
    }

    #[test]
    fn now_ms_returns_nonzero() {
        assert!(now_ms() > 0);
    }
}
