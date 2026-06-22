//! Linear provider — incremental sync of issues assigned to the
//! authenticated user, with per-issue memory_tree ingest.
//!
//! On each sync pass:
//!
//!   1. Load persistent [`SyncState`] from the KV store.
//!   2. Check the daily request budget — bail early if exhausted.
//!   3. Resolve the viewer ID via `LINEAR_LIST_LINEAR_USERS { isMe: true }`.
//!   4. Page through `LINEAR_LIST_LINEAR_ISSUES` filtered to the viewer as
//!      assignee, ordered by `updatedAt` descending. Stop early once we hit
//!      issues older than the cursor or a page without a next-page cursor.
//!   5. For each issue, ingest into memory_tree if it's new *or* edited
//!      since the last sync.
//!   6. Advance the cursor to the newest `updatedAt` seen and save.
//!
//! Privacy posture: we only pull issues the user is assigned to, never
//! the whole workspace's issue graph. This mirrors the
//! "fetch-what-the-user-sees" model `gmail` / `notion` already follow
//! and avoids accidentally ingesting other teammates' private issues.

use async_trait::async_trait;
use serde_json::json;

use super::source::run_linear_sync;
use super::sync;
use crate::openhuman::memory_sync::composio::providers::sync_state::extract_item_id;
use crate::openhuman::memory_sync::composio::providers::{
    merge_extra, pick_str, resolve_sync_interval_secs, ComposioProvider, CuratedTool,
    NormalizedTask, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason, TaskFetchFilter,
    TaskKind,
};

pub(super) const ACTION_LIST_USERS: &str = "LINEAR_LIST_LINEAR_USERS";
pub(super) const ACTION_LIST_ISSUES: &str = "LINEAR_LIST_LINEAR_ISSUES";

/// Paths for extracting a Linear issue's unique ID.
pub(super) const ISSUE_ID_PATHS: &[&str] = &["id", "data.id", "identifier", "data.identifier"];

pub struct LinearProvider;

impl LinearProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LinearProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ComposioProvider for LinearProvider {
    fn toolkit_slug(&self) -> &'static str {
        "linear"
    }

    fn curated_tools(&self) -> Option<&'static [CuratedTool]> {
        Some(super::tools::LINEAR_CURATED)
    }

    fn sync_interval_secs(&self) -> Option<u64> {
        // 30 minutes — same cadence as ClickUp/Notion. Linear issues change
        // more slowly than chat but faster than email.
        Some(resolve_sync_interval_secs("linear", 30 * 60))
    }

    async fn fetch_user_profile(
        &self,
        ctx: &ProviderContext,
    ) -> Result<ProviderUserProfile, String> {
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            "[composio:linear] fetch_user_profile via {ACTION_LIST_USERS}"
        );

        let resp = ctx
            .execute(ACTION_LIST_USERS, Some(json!({ "isMe": true })))
            .await
            .map_err(|e| format!("[composio:linear] {ACTION_LIST_USERS} failed: {e:#}"))?;

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!("[composio:linear] {ACTION_LIST_USERS}: {err}"));
        }

        let data = &resp.data;
        let viewer = sync::extract_viewer(data);
        let viewer_ref = viewer.as_ref().unwrap_or(data);

        let display_name = pick_str(viewer_ref, &["name", "data.name", "displayName"]);
        let email = pick_str(viewer_ref, &["email", "data.email"]);
        let username = pick_str(viewer_ref, &["id", "data.id"]);
        let avatar_url = pick_str(viewer_ref, &["avatarUrl", "data.avatarUrl"]);
        let profile_url = pick_str(viewer_ref, &["url", "data.url"]);

        Ok(ProviderUserProfile {
            toolkit: "linear".to_string(),
            connection_id: ctx.connection_id.clone(),
            display_name,
            email,
            username,
            avatar_url,
            profile_url,
            extras: data.clone(),
        })
    }

    /// Incremental sync via the generic
    /// [`orchestrator`](crate::openhuman::memory_sync::composio::providers::orchestrator):
    /// viewer resolution, pagination, dedup, the `max_items` cap, the
    /// `sync_depth_days` window, and cursor handling live in `run_sync`; the
    /// Linear-specific primitives live in [`super::source`].
    async fn sync(&self, ctx: &ProviderContext, reason: SyncReason) -> Result<SyncOutcome, String> {
        run_linear_sync(ctx, reason).await
    }

    async fn fetch_tasks(
        &self,
        ctx: &ProviderContext,
        filter: &TaskFetchFilter,
    ) -> Result<Vec<NormalizedTask>, String> {
        let max = filter.effective_max();
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            max,
            team_id = ?filter.team_id,
            assignee_is_me = filter.assignee_is_me,
            "[composio:linear] fetch_tasks"
        );

        let mut args = json!({
            "first": max.min(100) as u64,
            "orderBy": "updatedAt",
        });
        if filter.assignee_is_me {
            let resp = ctx
                .execute(ACTION_LIST_USERS, Some(json!({ "isMe": true })))
                .await
                .map_err(|e| format!("[composio:linear] {ACTION_LIST_USERS}: {e:#}"))?;
            // Fail closed: a failed viewer lookup must not silently widen
            // the query beyond "assigned to me".
            if !resp.successful {
                return Err(format!(
                    "[composio:linear] {ACTION_LIST_USERS}: {}",
                    resp.error.unwrap_or_else(|| "provider failure".into())
                ));
            }
            let viewer_id = sync::extract_viewer_id(&resp.data).ok_or_else(|| {
                "[composio:linear] LINEAR_LIST_LINEAR_USERS returned no viewer id".to_string()
            })?;
            args["assigneeId"] = json!(viewer_id);
        }
        if let Some(team) = filter
            .team_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            args["teamId"] = json!(team);
        }
        merge_extra(&mut args, &filter.extra);

        let resp = ctx
            .execute(ACTION_LIST_ISSUES, Some(args))
            .await
            .map_err(|e| format!("[composio:linear] {ACTION_LIST_ISSUES}: {e:#}"))?;
        if !resp.successful {
            return Err(format!(
                "[composio:linear] {ACTION_LIST_ISSUES}: {}",
                resp.error.unwrap_or_else(|| "provider failure".into())
            ));
        }

        let want_state = filter
            .state
            .as_deref()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());

        let mut out: Vec<NormalizedTask> = Vec::new();
        for issue in sync::extract_issues(&resp.data) {
            if out.len() >= max {
                break;
            }
            let Some(nt) = normalize_linear_issue(&issue) else {
                continue;
            };
            if let Some(ref want) = want_state {
                let matches = nt
                    .status
                    .as_deref()
                    .map(|s| s.to_ascii_lowercase() == *want)
                    .unwrap_or(false);
                if !matches {
                    continue;
                }
            }
            out.push(nt);
        }
        tracing::debug!(count = out.len(), "[composio:linear] fetch_tasks complete");
        Ok(out)
    }
}

/// Map a raw Linear issue payload into a [`NormalizedTask`].
fn normalize_linear_issue(issue: &serde_json::Value) -> Option<NormalizedTask> {
    let external_id = extract_item_id(issue, ISSUE_ID_PATHS)?;
    let title =
        sync::extract_issue_title(issue).unwrap_or_else(|| format!("Linear issue {external_id}"));
    Some(NormalizedTask {
        external_id,
        source_id: String::new(),
        provider: "linear".to_string(),
        kind: TaskKind::Generic,
        title,
        body: pick_str(issue, &["description", "data.description"]),
        url: pick_str(issue, &["url", "data.url"]),
        status: pick_str(issue, &["state.name", "data.state.name", "state.type"]),
        assignee: pick_str(issue, &["assignee.name", "data.assignee.name"]),
        due: pick_str(issue, &["dueDate", "data.dueDate"]),
        labels: extract_linear_labels(issue),
        priority: pick_str(issue, &["priorityLabel", "data.priorityLabel"]),
        updated_at: sync::extract_issue_updated(issue),
        raw: issue.clone(),
    })
}

/// Extract label names from a Linear issue (`labels.nodes[].name`).
fn extract_linear_labels(issue: &serde_json::Value) -> Vec<String> {
    let arr = issue
        .get("labels")
        .or_else(|| issue.get("data").and_then(|d| d.get("labels")))
        .and_then(|l| l.get("nodes"))
        .and_then(|v| v.as_array());
    match arr {
        Some(items) => items
            .iter()
            .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
            .map(|s| s.to_string())
            .collect(),
        None => Vec::new(),
    }
}
