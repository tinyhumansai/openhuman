//! ClickUp provider — incremental sync of tasks assigned to the
//! authenticated user, with per-item persistence into the Memory Tree.
//!
//! On each sync pass:
//!
//!   1. Load persistent [`SyncState`] from the KV store.
//!   2. Check the daily request budget — bail early if exhausted.
//!   3. If we don't yet know the user's numeric ID, call
//!      `CLICKUP_GET_AUTHORIZED_USER` and cache the result in memory
//!      (it doesn't change for the lifetime of the connection).
//!   4. If we don't yet know which workspaces (teams) the connection
//!      can see, call `CLICKUP_GET_AUTHORIZED_TEAMS_WORKSPACES` and
//!      cache the list.
//!   5. For each workspace, page through
//!      `CLICKUP_GET_FILTERED_TEAM_TASKS` filtered to the user as
//!      assignee, sorted by `date_updated` descending. Stop a workspace
//!      early once we hit tasks older than the cursor.
//!   6. For each task, persist as a single memory document if it's new
//!      *or* edited since the last sync.
//!   7. Advance the cursor to the newest `date_updated` seen and save.
//!
//! Privacy posture: we only pull tasks the user is assigned to, never
//! the whole workspace's task graph. This mirrors the
//! "fetch-what-the-user-sees" model `gmail` / `notion` already follow
//! and avoids accidentally ingesting other teammates' private tasks.

use async_trait::async_trait;
use serde_json::json;

use super::source::run_clickup_sync;
use super::sync;
use crate::openhuman::memory_sync::composio::providers::{
    first_array_str, merge_extra, pick_str, resolve_sync_interval_secs, ComposioProvider,
    CuratedTool, NormalizedTask, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason,
    TaskFetchFilter, TaskKind,
};

pub(crate) const ACTION_GET_AUTHORIZED_USER: &str = "CLICKUP_GET_AUTHORIZED_USER";
pub(crate) const ACTION_GET_AUTHORIZED_TEAMS_WORKSPACES: &str =
    "CLICKUP_GET_AUTHORIZED_TEAMS_WORKSPACES";
pub(crate) const ACTION_GET_FILTERED_TEAM_TASKS: &str = "CLICKUP_GET_FILTERED_TEAM_TASKS";

/// Paths for extracting a task's unique ID. Composio sometimes wraps
/// the upstream payload under `data`, so we check both shapes.
pub(super) const TASK_ID_PATHS: &[&str] = &["id", "data.id", "task_id", "data.task_id"];

pub struct ClickUpProvider;

impl ClickUpProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ClickUpProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ComposioProvider for ClickUpProvider {
    fn toolkit_slug(&self) -> &'static str {
        "clickup"
    }

    fn curated_tools(&self) -> Option<&'static [CuratedTool]> {
        Some(super::tools::CLICKUP_CURATED)
    }

    fn sync_interval_secs(&self) -> Option<u64> {
        // 30 minutes — same cadence as Notion. ClickUp tasks change
        // more slowly than chat but faster than email, so this is in
        // the middle.
        Some(resolve_sync_interval_secs("clickup", 30 * 60))
    }

    async fn fetch_user_profile(
        &self,
        ctx: &ProviderContext,
    ) -> Result<ProviderUserProfile, String> {
        tracing::debug!(
            connection_id = ?ctx.connection_id,
            "[composio:clickup] fetch_user_profile via {ACTION_GET_AUTHORIZED_USER}"
        );

        let resp = ctx
            .execute(ACTION_GET_AUTHORIZED_USER, Some(json!({})))
            .await
            .map_err(|e| {
                format!("[composio:clickup] {ACTION_GET_AUTHORIZED_USER} failed: {e:#}")
            })?;

        if !resp.successful {
            let err = resp
                .error
                .clone()
                .unwrap_or_else(|| "provider reported failure".to_string());
            return Err(format!(
                "[composio:clickup] {ACTION_GET_AUTHORIZED_USER}: {err}"
            ));
        }

        // Composio's wrapping puts ClickUp's `{user: {…}}` payload at
        // `data` or `data.user`. We probe both — `pick_str` walks dotted
        // paths so `user.username` and `data.user.username` both work.
        let data = &resp.data;
        let display_name = pick_str(data, &["user.username", "data.user.username", "username"]);
        let email = pick_str(data, &["user.email", "data.user.email", "email"]);
        let username = sync::extract_user_id(data);
        let avatar_url = pick_str(
            data,
            &[
                "user.profilePicture",
                "data.user.profilePicture",
                "profilePicture",
            ],
        );
        let profile_url = None;

        Ok(ProviderUserProfile {
            toolkit: "clickup".to_string(),
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
    /// user/workspace resolution, the per-workspace page loop, dedup, the
    /// `max_items` cap, the epoch-ms `sync_depth_days` window, and cursor
    /// handling live in `run_sync`; the ClickUp-specific primitives live in
    /// [`super::source`].
    async fn sync(&self, ctx: &ProviderContext, reason: SyncReason) -> Result<SyncOutcome, String> {
        run_clickup_sync(ctx, reason).await
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
            "[composio:clickup] fetch_tasks"
        );

        // Resolve which workspaces (teams) to query. An explicit
        // `team_id` from the filter wins; otherwise enumerate every
        // workspace the connection can see.
        let workspaces = match &filter.team_id {
            Some(team) if !team.trim().is_empty() => vec![team.trim().to_string()],
            _ => {
                let resp = ctx
                    .execute(ACTION_GET_AUTHORIZED_TEAMS_WORKSPACES, Some(json!({})))
                    .await
                    .map_err(|e| {
                        format!(
                            "[composio:clickup] {ACTION_GET_AUTHORIZED_TEAMS_WORKSPACES}: {e:#}"
                        )
                    })?;
                if !resp.successful {
                    return Err(format!(
                        "[composio:clickup] {ACTION_GET_AUTHORIZED_TEAMS_WORKSPACES}: {}",
                        resp.error.unwrap_or_else(|| "provider failure".into())
                    ));
                }
                sync::extract_workspace_ids(&resp.data)
            }
        };

        // Resolve the current user id only when the filter scopes to
        // "assigned to me".
        let assignees: Vec<String> = if filter.assignee_is_me {
            let resp = ctx
                .execute(ACTION_GET_AUTHORIZED_USER, Some(json!({})))
                .await
                .map_err(|e| format!("[composio:clickup] {ACTION_GET_AUTHORIZED_USER}: {e:#}"))?;
            // Fail closed: if we can't resolve the user, error rather than
            // silently dropping the assignee filter and fetching the whole
            // workspace's tasks.
            if !resp.successful {
                return Err(format!(
                    "[composio:clickup] {ACTION_GET_AUTHORIZED_USER}: {}",
                    resp.error.unwrap_or_else(|| "provider failure".into())
                ));
            }
            let id = sync::extract_user_id(&resp.data).ok_or_else(|| {
                "[composio:clickup] CLICKUP_GET_AUTHORIZED_USER returned no user.id".to_string()
            })?;
            vec![id]
        } else {
            Vec::new()
        };

        let mut out: Vec<NormalizedTask> = Vec::new();
        'workspaces: for workspace_id in &workspaces {
            let mut args = json!({
                "team_id": workspace_id,
                "order_by": "updated",
                "reverse": true,
                "page": 0,
                "page_size": max.min(100) as u32,
                "subtasks": true,
            });
            if !assignees.is_empty() {
                args["assignees"] = json!(assignees);
            }
            if let Some(list_id) = filter.list_id.as_deref().filter(|s| !s.trim().is_empty()) {
                args["list_ids"] = json!([list_id]);
            }
            merge_extra(&mut args, &filter.extra);

            let resp = ctx
                .execute(ACTION_GET_FILTERED_TEAM_TASKS, Some(args))
                .await
                .map_err(|e| {
                    format!("[composio:clickup] {ACTION_GET_FILTERED_TEAM_TASKS} ws={workspace_id}: {e:#}")
                })?;
            if !resp.successful {
                return Err(format!(
                    "[composio:clickup] {ACTION_GET_FILTERED_TEAM_TASKS} ws={workspace_id}: {}",
                    resp.error.unwrap_or_else(|| "provider failure".into())
                ));
            }

            for task in sync::extract_tasks(&resp.data) {
                if out.len() >= max {
                    break 'workspaces;
                }
                if let Some(nt) = normalize_clickup_task(&task) {
                    out.push(nt);
                }
            }
        }

        tracing::debug!(count = out.len(), "[composio:clickup] fetch_tasks complete");
        Ok(out)
    }
}

/// Map a raw ClickUp task payload into a [`NormalizedTask`]. Returns
/// `None` only when the task has no extractable id (unroutable).
fn normalize_clickup_task(task: &serde_json::Value) -> Option<NormalizedTask> {
    let external_id =
        crate::openhuman::memory_sync::composio::providers::sync_state::extract_item_id(
            task,
            TASK_ID_PATHS,
        )?;
    let title =
        sync::extract_task_name(task).unwrap_or_else(|| format!("ClickUp task {external_id}"));
    Some(NormalizedTask {
        external_id,
        source_id: String::new(),
        provider: "clickup".to_string(),
        kind: TaskKind::Generic,
        title,
        body: pick_str(task, &["description", "data.description", "text_content"]),
        url: pick_str(task, &["url", "data.url"]),
        status: pick_str(task, &["status.status", "data.status.status", "status"]),
        assignee: first_array_str(
            task,
            &["assignees", "data.assignees"],
            &["username", "email"],
        ),
        due: pick_str(task, &["due_date", "data.due_date"]),
        labels: Vec::new(),
        priority: pick_str(task, &["priority.priority", "data.priority.priority"]),
        updated_at: sync::extract_task_updated(task),
        raw: task.clone(),
    })
}
