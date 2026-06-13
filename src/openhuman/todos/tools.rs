//! LLM-callable wrappers over the per-thread todo board (`todos` domain).
//!
//! These tools let the agent read and mutate the kanban-style task board
//! that scopes per conversation thread. Each tool is a thin shim over the
//! free functions in [`crate::openhuman::todos::ops`], constructing a
//! [`BoardLocation`] from the optional `thread_id` argument plus the
//! configured workspace dir (falling back to the in-memory scratch board
//! when no thread is supplied).
//!
//! `todo_list` (ReadOnly) and the bounded, reversible writers
//! (`todo_add` / `todo_edit` / `todo_update_status` / `todo_decide_plan`)
//! are default-enabled. The destructive writers — `todo_remove`,
//! `todo_replace`, `todo_clear` — ship default-OFF via
//! `tools/user_filter.rs` because they discard board state.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

use super::ops::{self, BoardLocation, CardPatch, TodosSnapshot};

/// Build a [`BoardLocation`] for a tool call: a thread-scoped board when
/// `thread_id` is present, else the process-global scratch board.
fn board_location(config: &Config, args: &serde_json::Value) -> BoardLocation {
    match args
        .get("thread_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(thread_id) => BoardLocation::Thread {
            workspace_dir: config.workspace_dir.clone(),
            thread_id: thread_id.to_string(),
        },
        None => BoardLocation::Scratch,
    }
}

fn read_required_str(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required string argument `{key}`"))
}

fn opt_str(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn opt_str_vec(args: &serde_json::Value, key: &str) -> Option<Vec<String>> {
    args.get(key)
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
}

/// Build a [`CardPatch`] from the common optional args shared by add/edit.
fn card_patch(args: &serde_json::Value) -> anyhow::Result<CardPatch> {
    let status = match opt_str(args, "status") {
        Some(raw) => Some(ops::parse_status(&raw).map_err(|e| anyhow::anyhow!(e))?),
        None => None,
    };
    Ok(CardPatch {
        content: opt_str(args, "content"),
        status,
        objective: opt_str(args, "objective"),
        plan: opt_str_vec(args, "plan"),
        assigned_agent: opt_str(args, "assigned_agent"),
        allowed_tools: opt_str_vec(args, "allowed_tools"),
        approval_mode: None,
        acceptance_criteria: opt_str_vec(args, "acceptance_criteria"),
        evidence: opt_str_vec(args, "evidence"),
        notes: opt_str(args, "notes"),
        blocker: opt_str(args, "blocker"),
        source_metadata: None,
    })
}

fn snapshot_to_result(snapshot: TodosSnapshot) -> anyhow::Result<ToolResult> {
    Ok(ToolResult::success(serde_json::to_string(&snapshot)?))
}

/// The optional thread-scoping arg, shared by every todo tool.
fn thread_id_prop() -> serde_json::Value {
    json!({
        "type": "string",
        "description": "Thread id scoping the board. Omit to use the in-memory scratch board for the current session."
    })
}

/// List the cards on a thread's todo board.
pub struct TodoListTool {
    config: Arc<Config>,
}

impl TodoListTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TodoListTool {
    fn name(&self) -> &str {
        "todo_list"
    }

    fn description(&self) -> &str {
        "List the todo cards on a thread's task board, with a markdown \
         rendering. Use to review outstanding/completed work before adding or \
         updating tasks. Each card has an `id`, `content`, `status`, and \
         optional objective/plan/notes/blocker fields."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": { "thread_id": thread_id_prop() } })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][todos] list invoked");
        let location = board_location(&self.config, &args);
        let snapshot = ops::list(&location).map_err(|e| anyhow::anyhow!("todo_list: {e}"))?;
        snapshot_to_result(snapshot)
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Add a new card to a thread's todo board.
pub struct TodoAddTool {
    config: Arc<Config>,
}

impl TodoAddTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TodoAddTool {
    fn name(&self) -> &str {
        "todo_add"
    }

    fn description(&self) -> &str {
        "Add a todo card to a thread's task board. `content` is the task \
         summary; optional fields capture an `objective`, an ordered `plan`, \
         `acceptance_criteria`, an `assigned_agent`, `allowed_tools`, free-form \
         `notes`, a `blocker`, and an initial `status` \
         (todo|awaiting_approval|ready|in_progress|blocked|done|rejected)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": thread_id_prop(),
                "content": { "type": "string", "description": "Task summary (required)." },
                "status": { "type": "string", "description": "Initial status (default todo)." },
                "objective": { "type": "string" },
                "plan": { "type": "array", "items": { "type": "string" } },
                "acceptance_criteria": { "type": "array", "items": { "type": "string" } },
                "assigned_agent": { "type": "string" },
                "allowed_tools": { "type": "array", "items": { "type": "string" } },
                "evidence": { "type": "array", "items": { "type": "string" } },
                "notes": { "type": "string" },
                "blocker": { "type": "string" }
            },
            "required": ["content"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][todos] add invoked");
        let content = read_required_str(&args, "content")?;
        let location = board_location(&self.config, &args);
        let patch = card_patch(&args)?;
        let snapshot =
            ops::add(&location, &content, patch).map_err(|e| anyhow::anyhow!("todo_add: {e}"))?;
        snapshot_to_result(snapshot)
    }
}

/// Edit an existing card (partial update).
pub struct TodoEditTool {
    config: Arc<Config>,
}

impl TodoEditTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TodoEditTool {
    fn name(&self) -> &str {
        "todo_edit"
    }

    fn description(&self) -> &str {
        "Edit fields on an existing todo card by `id`. Only the fields you \
         supply are changed; omitted fields are left untouched. Same field set \
         as `todo_add` (content/status/objective/plan/notes/blocker/…)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": thread_id_prop(),
                "id": { "type": "string", "description": "Card id to edit (required)." },
                "content": { "type": "string" },
                "status": { "type": "string" },
                "objective": { "type": "string" },
                "plan": { "type": "array", "items": { "type": "string" } },
                "acceptance_criteria": { "type": "array", "items": { "type": "string" } },
                "assigned_agent": { "type": "string" },
                "allowed_tools": { "type": "array", "items": { "type": "string" } },
                "evidence": { "type": "array", "items": { "type": "string" } },
                "notes": { "type": "string" },
                "blocker": { "type": "string" }
            },
            "required": ["id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][todos] edit invoked");
        let id = read_required_str(&args, "id")?;
        let location = board_location(&self.config, &args);
        let patch = card_patch(&args)?;
        let snapshot =
            ops::edit(&location, &id, patch).map_err(|e| anyhow::anyhow!("todo_edit: {e}"))?;
        snapshot_to_result(snapshot)
    }
}

/// Transition a card's status.
pub struct TodoUpdateStatusTool {
    config: Arc<Config>,
}

impl TodoUpdateStatusTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TodoUpdateStatusTool {
    fn name(&self) -> &str {
        "todo_update_status"
    }

    fn description(&self) -> &str {
        "Transition a todo card to a new `status` \
         (todo|awaiting_approval|ready|in_progress|blocked|done|rejected). Use \
         to mark work started, blocked, or completed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": thread_id_prop(),
                "id": { "type": "string", "description": "Card id (required)." },
                "status": { "type": "string", "description": "New status (required)." }
            },
            "required": ["id", "status"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][todos] update_status invoked");
        let id = read_required_str(&args, "id")?;
        let status_raw = read_required_str(&args, "status")?;
        let status = ops::parse_status(&status_raw).map_err(|e| anyhow::anyhow!(e))?;
        let location = board_location(&self.config, &args);
        let snapshot = ops::update_status(&location, &id, status)
            .map_err(|e| anyhow::anyhow!("todo_update_status: {e}"))?;
        snapshot_to_result(snapshot)
    }
}

/// Approve or reject a gated plan card.
pub struct TodoDecidePlanTool {
    config: Arc<Config>,
}

impl TodoDecidePlanTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TodoDecidePlanTool {
    fn name(&self) -> &str {
        "todo_decide_plan"
    }

    fn description(&self) -> &str {
        "Approve (`approve: true`) or reject (`approve: false`) a card that is \
         awaiting plan approval. Approving moves it to ready; rejecting moves \
         it to rejected."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": thread_id_prop(),
                "id": { "type": "string", "description": "Card id (required)." },
                "approve": { "type": "boolean", "description": "Approve or reject the plan (required)." }
            },
            "required": ["id", "approve"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][todos] decide_plan invoked");
        let id = read_required_str(&args, "id")?;
        let approve = args
            .get("approve")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| anyhow::anyhow!("missing required boolean argument `approve`"))?;
        let location = board_location(&self.config, &args);
        let snapshot = ops::decide_plan(&location, &id, approve)
            .map_err(|e| anyhow::anyhow!("todo_decide_plan: {e}"))?;
        snapshot_to_result(snapshot)
    }
}

/// Remove a single card. **Destructive** — default-OFF.
pub struct TodoRemoveTool {
    config: Arc<Config>,
}

impl TodoRemoveTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TodoRemoveTool {
    fn name(&self) -> &str {
        "todo_remove"
    }

    fn description(&self) -> &str {
        "Permanently remove a single todo card by `id` from a thread's board. \
         This is irreversible. Prefer `todo_update_status` to `done`/`rejected` \
         over deleting, unless the user wants the card gone entirely."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": thread_id_prop(),
                "id": { "type": "string", "description": "Card id to remove (required)." }
            },
            "required": ["id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][todos] remove invoked");
        let id = read_required_str(&args, "id")?;
        let location = board_location(&self.config, &args);
        let snapshot =
            ops::remove(&location, &id).map_err(|e| anyhow::anyhow!("todo_remove: {e}"))?;
        snapshot_to_result(snapshot)
    }
}

/// Replace the entire board with a supplied card set. **Destructive** —
/// default-OFF.
pub struct TodoReplaceTool {
    config: Arc<Config>,
}

impl TodoReplaceTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TodoReplaceTool {
    fn name(&self) -> &str {
        "todo_replace"
    }

    fn description(&self) -> &str {
        "Wholesale-replace a thread's todo board with the supplied `cards` \
         array, discarding the previous contents. Irreversible. Use only when \
         rebuilding a board from scratch."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": thread_id_prop(),
                "cards": {
                    "type": "array",
                    "description": "Full replacement card set (TaskBoardCard objects).",
                    "items": { "type": "object" }
                }
            },
            "required": ["cards"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][todos] replace invoked");
        let cards_val = args
            .get("cards")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing required array argument `cards`"))?;
        let cards = serde_json::from_value(cards_val)
            .map_err(|e| anyhow::anyhow!("todo_replace: invalid cards: {e}"))?;
        let location = board_location(&self.config, &args);
        let snapshot =
            ops::replace(&location, cards).map_err(|e| anyhow::anyhow!("todo_replace: {e}"))?;
        snapshot_to_result(snapshot)
    }
}

/// Empty the board. **Destructive** — default-OFF.
pub struct TodoClearTool {
    config: Arc<Config>,
}

impl TodoClearTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for TodoClearTool {
    fn name(&self) -> &str {
        "todo_clear"
    }

    fn description(&self) -> &str {
        "Remove every card from a thread's todo board, leaving it empty. \
         Irreversible. Only use when the user wants to clear the whole board."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": { "thread_id": thread_id_prop() } })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][todos] clear invoked");
        let location = board_location(&self.config, &args);
        let snapshot = ops::clear(&location).map_err(|e| anyhow::anyhow!("todo_clear: {e}"))?;
        snapshot_to_result(snapshot)
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
        assert_eq!(TodoListTool::new(c.clone()).name(), "todo_list");
        assert_eq!(
            TodoListTool::new(c.clone()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            TodoAddTool::new(c.clone()).permission_level(),
            PermissionLevel::Write
        );
        assert_eq!(
            TodoRemoveTool::new(c.clone()).permission_level(),
            PermissionLevel::Write
        );
        assert_eq!(TodoListTool::new(c).scope(), ToolScope::All);
    }

    #[test]
    fn board_location_prefers_thread_then_scratch() {
        let c = cfg();
        let with_thread = board_location(&c, &json!({ "thread_id": "abc" }));
        assert_eq!(with_thread.thread_id(), Some("abc"));
        let scratch = board_location(&c, &json!({ "thread_id": "  " }));
        assert!(matches!(scratch, BoardLocation::Scratch));
        let absent = board_location(&c, &json!({}));
        assert!(matches!(absent, BoardLocation::Scratch));
    }

    #[test]
    fn card_patch_parses_fields() {
        let patch = card_patch(&json!({
            "content": "do it",
            "status": "in_progress",
            "plan": ["a", "b"],
            "notes": "n"
        }))
        .expect("patch");
        assert_eq!(patch.content.as_deref(), Some("do it"));
        assert!(patch.status.is_some());
        assert_eq!(patch.plan.as_ref().map(|p| p.len()), Some(2));
    }

    #[test]
    fn card_patch_rejects_bad_status() {
        let err = card_patch(&json!({ "status": "nope" })).expect_err("bad status");
        assert!(err.to_string().contains("invalid status"));
    }

    #[tokio::test]
    async fn add_requires_content() {
        let err = TodoAddTool::new(cfg())
            .execute(json!({}))
            .await
            .expect_err("missing content");
        assert!(err.to_string().contains("content"));
    }

    #[tokio::test]
    async fn scratch_board_add_then_list_roundtrips() {
        // Using the scratch board (no thread_id) avoids any filesystem
        // dependency, exercising the full add → list path deterministically.
        let c = cfg();
        let added = TodoAddTool::new(c.clone())
            .execute(json!({ "content": "scratch task" }))
            .await
            .expect("add");
        assert!(added.output_for_llm(false).contains("scratch task"));
        let listed = TodoListTool::new(c).execute(json!({})).await.expect("list");
        assert!(listed.output_for_llm(false).contains("scratch task"));
    }

    #[tokio::test]
    async fn decide_plan_requires_approve_bool() {
        let err = TodoDecidePlanTool::new(cfg())
            .execute(json!({ "id": "x" }))
            .await
            .expect_err("missing approve");
        assert!(err.to_string().contains("approve"));
    }
}
