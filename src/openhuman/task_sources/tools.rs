//! LLM-callable wrappers over the `task_sources` domain.
//!
//! These tools let the agent inspect external task sources (GitHub / Notion
//! / Linear / ClickUp issue & task feeds), trigger an on-demand fetch, list
//! ingested tasks, and dry-run a filter. Each tool is a thin shim over the
//! async functions in [`crate::openhuman::task_sources::ops`], which return
//! `RpcOutcome<T>`; the wrapper emits the inner value as JSON.
//!
//! The read/observe tools (`list` / `get` / `fetch` / `list_tasks` /
//! `preview_filter` / `status`) are default-enabled. The persistent-config
//! mutators — `add`, `update`, `remove` — change ingestion behaviour and
//! cascade history, so they ship default-OFF via `tools/user_filter.rs`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::task_sources::{FilterSpec, ProviderSlug, TaskSourcePatch};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

use super::ops;

fn read_required_str(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required string argument `{key}`"))
}

fn opt_u64(args: &serde_json::Value, key: &str) -> Option<u64> {
    args.get(key).and_then(serde_json::Value::as_u64)
}

fn opt_str(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn parse_provider(args: &serde_json::Value) -> anyhow::Result<ProviderSlug> {
    let raw = read_required_str(args, "provider")?;
    ProviderSlug::parse(&raw).map_err(|e| anyhow::anyhow!("invalid provider: {e}"))
}

fn parse_filter(args: &serde_json::Value) -> anyhow::Result<FilterSpec> {
    let val = args
        .get("filter")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing required object argument `filter`"))?;
    serde_json::from_value(val).map_err(|e| anyhow::anyhow!("invalid filter: {e}"))
}

macro_rules! emit {
    ($outcome:expr, $name:literal) => {{
        let outcome = $outcome.map_err(|e| anyhow::anyhow!(concat!($name, ": {}"), e))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }};
}

/// List configured external task sources.
pub struct TaskSourceListTool {
    config: Arc<Config>,
}

impl TaskSourceListTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TaskSourceListTool {
    fn name(&self) -> &str {
        "task_source_list"
    }

    fn description(&self) -> &str {
        "List configured external task sources (GitHub / Notion / Linear / \
         ClickUp feeds that ingest issues and tasks). Each entry carries `id`, \
         `provider`, `name`, `enabled`, `filter`, `interval_secs`, and \
         last-fetch metadata. Use to see what feeds exist before fetching or \
         editing one."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][task_sources] list invoked");
        emit!(ops::list(&self.config).await, "task_source_list")
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Read one task source by id.
pub struct TaskSourceGetTool {
    config: Arc<Config>,
}

impl TaskSourceGetTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TaskSourceGetTool {
    fn name(&self) -> &str {
        "task_source_get"
    }

    fn description(&self) -> &str {
        "Get one external task source by `id`, returning its full config \
         (provider, filter, interval, target, last fetch status)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "Task-source id." } },
            "required": ["id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][task_sources] get invoked");
        let id = read_required_str(&args, "id")?;
        emit!(ops::get(&self.config, &id).await, "task_source_get")
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Trigger an on-demand fetch of one task source.
pub struct TaskSourceFetchTool {
    config: Arc<Config>,
}

impl TaskSourceFetchTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TaskSourceFetchTool {
    fn name(&self) -> &str {
        "task_source_fetch"
    }

    fn description(&self) -> &str {
        "Fetch one task source now (by `id`) instead of waiting for its poll \
         interval. Returns counts of tasks fetched, newly routed, and skipped \
         as duplicates. Use when the user wants the latest issues/tasks pulled \
         immediately."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "Task-source id to fetch." } },
            "required": ["id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][task_sources] fetch invoked");
        let id = read_required_str(&args, "id")?;
        emit!(ops::fetch(&self.config, &id).await, "task_source_fetch")
    }
}

/// List ingested tasks for one source.
pub struct TaskSourceListTasksTool {
    config: Arc<Config>,
}

impl TaskSourceListTasksTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TaskSourceListTasksTool {
    fn name(&self) -> &str {
        "task_source_list_tasks"
    }

    fn description(&self) -> &str {
        "List the tasks already ingested from one task source (by `id`), most \
         recent first, optionally capped by `limit`. Use to see what was \
         pulled from a given feed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Task-source id." },
                "limit": { "type": "integer", "minimum": 1, "description": "Max tasks to return." }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][task_sources] list_tasks invoked");
        let id = read_required_str(&args, "id")?;
        let limit = opt_u64(&args, "limit").map(|v| v as usize);
        emit!(
            ops::list_tasks(&self.config, &id, limit).await,
            "task_source_list_tasks"
        )
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Dry-run a provider filter without persisting a source.
pub struct TaskSourcePreviewFilterTool {
    config: Arc<Config>,
}

impl TaskSourcePreviewFilterTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TaskSourcePreviewFilterTool {
    fn name(&self) -> &str {
        "task_source_preview_filter"
    }

    fn description(&self) -> &str {
        "Dry-run a task-source `filter` for a `provider` and return the tasks \
         it would match, WITHOUT creating a persistent source or ingesting \
         anything. Use to validate a filter before `task_source_add`. The \
         `filter` object is provider-tagged (e.g. \
         `{ \"provider\": \"github\", \"repo\": \"owner/name\", \"labels\": [\"bug\"] }`)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "provider": { "type": "string", "enum": ["github", "notion", "linear", "clickup"] },
                "filter": { "type": "object", "description": "Provider-tagged filter spec." },
                "connection_id": { "type": "string", "description": "Optional Composio connection id." },
                "max": { "type": "integer", "minimum": 1, "description": "Max tasks to preview." }
            },
            "required": ["provider", "filter"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][task_sources] preview_filter invoked");
        let provider = parse_provider(&args)?;
        let filter = parse_filter(&args)?;
        let connection_id = opt_str(&args, "connection_id");
        let max = opt_u64(&args, "max").map(|v| v as u32);
        emit!(
            ops::preview_filter(&self.config, provider, filter, connection_id, max).await,
            "task_source_preview_filter"
        )
    }
}

/// Report task-source subsystem status.
pub struct TaskSourceStatusTool {
    config: Arc<Config>,
}

impl TaskSourceStatusTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TaskSourceStatusTool {
    fn name(&self) -> &str {
        "task_source_status"
    }

    fn description(&self) -> &str {
        "Report task-source subsystem health: whether ingestion is enabled, \
         the default poll interval, and the total/enabled source counts."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][task_sources] status invoked");
        emit!(ops::status(&self.config).await, "task_source_status")
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Create a persistent task source. **Mutates config + spawns polling** —
/// default-OFF.
pub struct TaskSourceAddTool {
    config: Arc<Config>,
}

impl TaskSourceAddTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TaskSourceAddTool {
    fn name(&self) -> &str {
        "task_source_add"
    }

    fn description(&self) -> &str {
        "Create a persistent external task source that will be polled on an \
         interval. Requires `provider` and a provider-tagged `filter`; \
         optional `name`, `connection_id`, `interval_secs`, `target` \
         (agent_todo_proactive|todo_only), `max_tasks_per_fetch`, and \
         `assigned_executor`. Validate the filter with \
         `task_source_preview_filter` first."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "provider": { "type": "string", "enum": ["github", "notion", "linear", "clickup"] },
                "filter": { "type": "object", "description": "Provider-tagged filter spec." },
                "name": { "type": "string" },
                "connection_id": { "type": "string" },
                "interval_secs": { "type": "integer", "minimum": 1 },
                "target": { "type": "string", "enum": ["agent_todo_proactive", "todo_only"] },
                "max_tasks_per_fetch": { "type": "integer", "minimum": 1 },
                "assigned_executor": { "type": "string" }
            },
            "required": ["provider", "filter"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][task_sources] add invoked");
        let provider = parse_provider(&args)?;
        let filter = parse_filter(&args)?;
        let target = match args.get("target") {
            Some(v) => Some(
                serde_json::from_value(v.clone())
                    .map_err(|e| anyhow::anyhow!("invalid target: {e}"))?,
            ),
            None => None,
        };
        let max_tasks = opt_u64(&args, "max_tasks_per_fetch").map(|v| v as u32);
        emit!(
            ops::add(
                &self.config,
                provider,
                opt_str(&args, "connection_id"),
                opt_str(&args, "name"),
                filter,
                opt_u64(&args, "interval_secs"),
                target,
                max_tasks,
                opt_str(&args, "assigned_executor"),
            )
            .await,
            "task_source_add"
        )
    }
}

/// Patch an existing task source. **Mutates config** — default-OFF.
pub struct TaskSourceUpdateTool {
    config: Arc<Config>,
}

impl TaskSourceUpdateTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TaskSourceUpdateTool {
    fn name(&self) -> &str {
        "task_source_update"
    }

    fn description(&self) -> &str {
        "Patch an existing task source by `id`. Supply a `patch` object with \
         only the fields to change (name, enabled, filter, intervalSecs, \
         target, maxTasksPerFetch, connectionId, assignedExecutor). Omitted \
         fields are left untouched."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Task-source id to update." },
                "patch": { "type": "object", "description": "Partial TaskSourcePatch (camelCase fields)." }
            },
            "required": ["id", "patch"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][task_sources] update invoked");
        let id = read_required_str(&args, "id")?;
        let patch_val = args
            .get("patch")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing required object argument `patch`"))?;
        let patch: TaskSourcePatch =
            serde_json::from_value(patch_val).map_err(|e| anyhow::anyhow!("invalid patch: {e}"))?;
        emit!(
            ops::update(&self.config, &id, patch).await,
            "task_source_update"
        )
    }
}

/// Delete a task source and its ingested history. **Destructive** —
/// default-OFF.
pub struct TaskSourceRemoveTool {
    config: Arc<Config>,
}

impl TaskSourceRemoveTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TaskSourceRemoveTool {
    fn name(&self) -> &str {
        "task_source_remove"
    }

    fn description(&self) -> &str {
        "Delete a task source by `id`, also removing all of its ingested-task \
         history (cascade). Irreversible. Only use when the user wants the \
         feed gone."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "Task-source id to remove." } },
            "required": ["id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][task_sources] remove invoked");
        let id = read_required_str(&args, "id")?;
        emit!(ops::remove(&self.config, &id).await, "task_source_remove")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::ToolScope;

    fn cfg() -> Arc<Config> {
        Arc::new(Config::default())
    }

    #[test]
    fn names_and_levels() {
        let c = cfg();
        assert_eq!(
            TaskSourceListTool::new(c.clone()).name(),
            "task_source_list"
        );
        assert_eq!(
            TaskSourceListTool::new(c.clone()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            TaskSourceFetchTool::new(c.clone()).permission_level(),
            PermissionLevel::Execute
        );
        assert_eq!(
            TaskSourceAddTool::new(c.clone()).permission_level(),
            PermissionLevel::Write
        );
        assert_eq!(
            TaskSourceRemoveTool::new(c.clone()).permission_level(),
            PermissionLevel::Dangerous
        );
        assert_eq!(TaskSourceListTool::new(c).scope(), ToolScope::All);
    }

    #[test]
    fn read_tools_concurrency_safe() {
        let c = cfg();
        assert!(TaskSourceListTool::new(c.clone()).is_concurrency_safe(&serde_json::Value::Null));
        assert!(TaskSourceGetTool::new(c).is_concurrency_safe(&serde_json::Value::Null));
    }

    #[tokio::test]
    async fn get_requires_id() {
        let err = TaskSourceGetTool::new(cfg())
            .execute(json!({}))
            .await
            .expect_err("missing id");
        assert!(err.to_string().contains("id"));
    }

    #[tokio::test]
    async fn add_requires_provider_and_filter() {
        let err = TaskSourceAddTool::new(cfg())
            .execute(json!({ "filter": { "provider": "github" } }))
            .await
            .expect_err("missing provider");
        assert!(err.to_string().contains("provider"));
    }

    #[test]
    fn parse_provider_rejects_unknown() {
        let err = parse_provider(&json!({ "provider": "jira" })).expect_err("unknown provider");
        assert!(err.to_string().contains("provider"));
    }
}
