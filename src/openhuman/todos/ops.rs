//! Core todo CRUD operations.
//!
//! Each operation loads the current cards for a thread (or the
//! process-global scratch store when no thread id is given), applies the
//! mutation, persists the result, and returns both the updated cards and
//! a markdown rendering. The agent-facing `todo` tool and the
//! `openhuman.todos_*` RPC handlers both call into this module so behavior
//! stays in lock-step across surfaces.

use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent::task_board::{
    normalise_board, TaskApprovalMode, TaskBoard, TaskBoardCard, TaskBoardStore, TaskCardStatus,
};
use chrono::Utc;
use parking_lot::{Mutex, MutexGuard};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;
use uuid::Uuid;

use super::store::{global_scratch_store, ScratchTodoStore};

/// Serialise scratch CRUD so each public op's load → mutate → save
/// sequence runs in one critical section. Per-thread ops are already
/// atomic at the file-rename level via `TaskBoardStore::put`.
fn scratch_serial_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock()
}

fn maybe_scratch_lock(location: &BoardLocation) -> Option<MutexGuard<'static, ()>> {
    matches!(location, BoardLocation::Scratch).then(scratch_serial_lock)
}

/// Stable string aliases accepted on the wire for [`TaskCardStatus`].
pub fn parse_status(raw: &str) -> Result<TaskCardStatus, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "todo" | "pending" => Ok(TaskCardStatus::Todo),
        "awaiting_approval" | "awaiting-approval" => Ok(TaskCardStatus::AwaitingApproval),
        "ready" | "approved" => Ok(TaskCardStatus::Ready),
        "in_progress" | "in-progress" | "inprogress" | "started" => Ok(TaskCardStatus::InProgress),
        "blocked" => Ok(TaskCardStatus::Blocked),
        "done" | "completed" | "complete" => Ok(TaskCardStatus::Done),
        "rejected" | "denied" => Ok(TaskCardStatus::Rejected),
        other => Err(format!(
            "invalid status '{other}' (expected todo|awaiting_approval|ready|in_progress|blocked|done|rejected)"
        )),
    }
}

/// A single CRUD outcome: the post-mutation cards plus a markdown rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodosSnapshot {
    pub thread_id: Option<String>,
    pub cards: Vec<TaskBoardCard>,
    pub markdown: String,
}

/// Optional fields supplied by `add` / `edit` callers.
#[derive(Debug, Default, Clone)]
pub struct CardPatch {
    pub content: Option<String>,
    pub status: Option<TaskCardStatus>,
    pub objective: Option<String>,
    pub plan: Option<Vec<String>>,
    pub assigned_agent: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub approval_mode: Option<Option<TaskApprovalMode>>,
    pub acceptance_criteria: Option<Vec<String>>,
    pub evidence: Option<Vec<String>>,
    pub notes: Option<String>,
    pub blocker: Option<String>,
    /// Provider/source identifiers for a task-source-ingested card. `Some`
    /// sets the card's `source_metadata`; `None` leaves it untouched.
    pub source_metadata: Option<serde_json::Value>,
}

/// Where to load/save the working set of cards.
#[derive(Debug, Clone)]
pub enum BoardLocation {
    /// Persisted to `<workspace>/agent_task_boards/<hex(thread_id)>.json`.
    Thread {
        workspace_dir: PathBuf,
        thread_id: String,
    },
    /// In-memory only, shared across the process.
    Scratch,
}

impl BoardLocation {
    pub fn thread_id(&self) -> Option<&str> {
        match self {
            Self::Thread { thread_id, .. } => Some(thread_id.as_str()),
            Self::Scratch => None,
        }
    }
}

fn load_cards(location: &BoardLocation) -> Result<Vec<TaskBoardCard>, String> {
    match location {
        BoardLocation::Thread {
            workspace_dir,
            thread_id,
        } => {
            let store = TaskBoardStore::new(workspace_dir.clone());
            Ok(store
                .get(thread_id)?
                .map(|board| board.cards)
                .unwrap_or_default())
        }
        BoardLocation::Scratch => Ok(global_scratch_store().snapshot()),
    }
}

fn save_cards(
    location: &BoardLocation,
    cards: Vec<TaskBoardCard>,
) -> Result<Vec<TaskBoardCard>, String> {
    match location {
        BoardLocation::Thread {
            workspace_dir,
            thread_id,
        } => {
            let mut board = TaskBoard {
                thread_id: thread_id.clone(),
                cards,
                updated_at: Utc::now().to_rfc3339(),
            };
            normalise_board(&mut board);
            let store = TaskBoardStore::new(workspace_dir.clone());
            Ok(store.put(board)?.cards)
        }
        BoardLocation::Scratch => {
            let mut board = TaskBoard {
                thread_id: "_scratch_".to_string(),
                cards,
                updated_at: Utc::now().to_rfc3339(),
            };
            normalise_board(&mut board);
            let scratch: std::sync::Arc<ScratchTodoStore> = global_scratch_store();
            scratch.replace(board.cards.clone());
            Ok(board.cards)
        }
    }
}

fn into_snapshot(location: &BoardLocation, cards: Vec<TaskBoardCard>) -> TodosSnapshot {
    let markdown = render_markdown(&cards);
    TodosSnapshot {
        thread_id: location.thread_id().map(|s| s.to_string()),
        cards,
        markdown,
    }
}

/// Render a card list as GitHub-flavored markdown. Each card becomes a
/// `- [ ]` / `- [x]` line (with `[~]` for in-progress and `[!]` for
/// blocked) followed by indented notes / blocker reasons.
pub fn render_markdown(cards: &[TaskBoardCard]) -> String {
    if cards.is_empty() {
        return "_No todos yet._".to_string();
    }
    let mut out = String::new();
    for card in cards {
        let marker = match card.status {
            TaskCardStatus::Todo | TaskCardStatus::Ready => "[ ]",
            TaskCardStatus::AwaitingApproval => "[?]",
            TaskCardStatus::InProgress => "[~]",
            TaskCardStatus::Blocked => "[!]",
            TaskCardStatus::Done => "[x]",
            TaskCardStatus::Rejected => "[-]",
        };
        out.push_str("- ");
        out.push_str(marker);
        out.push(' ');
        out.push_str(&card.title);
        out.push_str(&format!("  `({})`", card.id));
        out.push('\n');

        if let Some(objective) = card.objective.as_deref() {
            out.push_str("  - objective: ");
            out.push_str(objective);
            out.push('\n');
        }
        if let Some(agent) = card.assigned_agent.as_deref() {
            out.push_str("  - agent: ");
            out.push_str(agent);
            out.push('\n');
        }
        if !card.allowed_tools.is_empty() {
            out.push_str("  - tools: ");
            out.push_str(&card.allowed_tools.join(", "));
            out.push('\n');
        }
        if let Some(mode) = card.approval_mode.as_ref() {
            out.push_str("  - approval: ");
            out.push_str(mode.as_str());
            out.push('\n');
        }
        if !card.plan.is_empty() {
            out.push_str("  - plan:\n");
            for step in &card.plan {
                out.push_str("    - ");
                out.push_str(step);
                out.push('\n');
            }
        }
        if !card.acceptance_criteria.is_empty() {
            out.push_str("  - acceptance criteria:\n");
            for criterion in &card.acceptance_criteria {
                out.push_str("    - ");
                out.push_str(criterion);
                out.push('\n');
            }
        }
        if !card.evidence.is_empty() {
            out.push_str("  - evidence:\n");
            for item in &card.evidence {
                out.push_str("    - ");
                out.push_str(item);
                out.push('\n');
            }
        }

        if matches!(card.status, TaskCardStatus::Blocked) {
            if let Some(reason) = card.blocker.as_deref().or(card.notes.as_deref()) {
                out.push_str("  - _blocked:_ ");
                out.push_str(reason);
                out.push('\n');
            }
        } else if let Some(notes) = card.notes.as_deref() {
            out.push_str("  - ");
            out.push_str(notes);
            out.push('\n');
        }
    }
    out.trim_end().to_string()
}

/// Append a new card. `content` is required; missing status defaults to
/// `todo`.
pub fn add(
    location: &BoardLocation,
    content: &str,
    patch: CardPatch,
) -> Result<TodosSnapshot, String> {
    tracing::debug!(
        thread_id = ?location.thread_id(),
        content_len = content.len(),
        "[todos][ops] add entry"
    );
    let _scratch_guard = maybe_scratch_lock(location);
    let content = content.trim();
    if content.is_empty() {
        return Err("todo content must not be empty".to_string());
    }
    let mut cards = load_cards(location)?;
    let new_card = TaskBoardCard {
        id: format!("task-{}", Uuid::new_v4()),
        title: content.to_string(),
        status: patch.status.unwrap_or(TaskCardStatus::Todo),
        objective: patch.objective.and_then(non_empty),
        plan: patch.plan.unwrap_or_default(),
        assigned_agent: patch.assigned_agent.and_then(non_empty),
        allowed_tools: patch.allowed_tools.unwrap_or_default(),
        approval_mode: patch.approval_mode.flatten(),
        acceptance_criteria: patch.acceptance_criteria.unwrap_or_default(),
        evidence: patch.evidence.unwrap_or_default(),
        notes: patch.notes.and_then(non_empty),
        blocker: patch.blocker.and_then(non_empty),
        source_metadata: patch.source_metadata,
        order: cards.len() as u32,
        updated_at: Utc::now().to_rfc3339(),
    };
    cards.push(new_card);
    enforce_single_in_progress(&cards)?;
    let cards = save_cards(location, cards)?;
    emit_progress(location, &cards);
    Ok(into_snapshot(location, cards))
}

/// Edit an existing card's content / notes / blocker / status. Any field
/// left as `None` in `patch` is left untouched. Errors if `id` is unknown.
pub fn edit(location: &BoardLocation, id: &str, patch: CardPatch) -> Result<TodosSnapshot, String> {
    tracing::debug!(
        thread_id = ?location.thread_id(),
        id,
        "[todos][ops] edit entry"
    );
    let _scratch_guard = maybe_scratch_lock(location);
    let mut cards = load_cards(location)?;
    let card = cards
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("todo id '{id}' not found"))?;
    if let Some(content) = patch.content {
        let trimmed = content.trim().to_string();
        if trimmed.is_empty() {
            return Err("todo content must not be empty".to_string());
        }
        card.title = trimmed;
    }
    if let Some(status) = patch.status {
        card.status = status;
    }
    if let Some(objective) = patch.objective {
        card.objective = non_empty(objective);
    }
    if let Some(plan) = patch.plan {
        card.plan = plan;
    }
    if let Some(assigned_agent) = patch.assigned_agent {
        card.assigned_agent = non_empty(assigned_agent);
    }
    if let Some(allowed_tools) = patch.allowed_tools {
        card.allowed_tools = allowed_tools;
    }
    if let Some(approval_mode) = patch.approval_mode {
        card.approval_mode = approval_mode;
    }
    if let Some(acceptance_criteria) = patch.acceptance_criteria {
        card.acceptance_criteria = acceptance_criteria;
    }
    if let Some(evidence) = patch.evidence {
        card.evidence = evidence;
    }
    if let Some(notes) = patch.notes {
        card.notes = non_empty(notes);
    }
    if let Some(blocker) = patch.blocker {
        card.blocker = non_empty(blocker);
    }
    if let Some(source_metadata) = patch.source_metadata {
        card.source_metadata = Some(source_metadata);
    }
    card.updated_at = Utc::now().to_rfc3339();
    enforce_single_in_progress(&cards)?;
    let cards = save_cards(location, cards)?;
    emit_progress(location, &cards);
    Ok(into_snapshot(location, cards))
}

/// Update only the status of a card.
pub fn update_status(
    location: &BoardLocation,
    id: &str,
    status: TaskCardStatus,
) -> Result<TodosSnapshot, String> {
    edit(
        location,
        id,
        CardPatch {
            status: Some(status),
            ..Default::default()
        },
    )
}

/// Resolve a plan-approval decision: approve (→`Ready`, so the dispatcher runs
/// it) or reject (→`Rejected`). Errors unless the card is currently
/// `AwaitingApproval`, so a stale/duplicate decision can't resurrect a card
/// that already moved on.
pub fn decide_plan(
    location: &BoardLocation,
    id: &str,
    approve: bool,
) -> Result<TodosSnapshot, String> {
    let cards = load_cards(location)?;
    let current = cards
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("todo id '{id}' not found"))?;
    if current.status != TaskCardStatus::AwaitingApproval {
        return Err(format!(
            "card '{id}' is not awaiting approval (status: {})",
            current.status.as_str()
        ));
    }
    let new_status = if approve {
        TaskCardStatus::Ready
    } else {
        TaskCardStatus::Rejected
    };
    update_status(location, id, new_status)
}

/// Remove a card by id. Errors if `id` is unknown.
pub fn remove(location: &BoardLocation, id: &str) -> Result<TodosSnapshot, String> {
    tracing::debug!(
        thread_id = ?location.thread_id(),
        id,
        "[todos][ops] remove entry"
    );
    let _scratch_guard = maybe_scratch_lock(location);
    let mut cards = load_cards(location)?;
    let before = cards.len();
    cards.retain(|c| c.id != id);
    if cards.len() == before {
        return Err(format!("todo id '{id}' not found"));
    }
    let cards = save_cards(location, cards)?;
    emit_progress(location, &cards);
    Ok(into_snapshot(location, cards))
}

/// Wholesale replace the list. Generates ids for cards missing them.
pub fn replace(
    location: &BoardLocation,
    cards: Vec<TaskBoardCard>,
) -> Result<TodosSnapshot, String> {
    tracing::debug!(
        thread_id = ?location.thread_id(),
        card_count = cards.len(),
        "[todos][ops] replace entry"
    );
    let _scratch_guard = maybe_scratch_lock(location);
    enforce_single_in_progress(&cards)?;
    let cards = save_cards(location, cards)?;
    emit_progress(location, &cards);
    Ok(into_snapshot(location, cards))
}

/// Empty the list.
pub fn clear(location: &BoardLocation) -> Result<TodosSnapshot, String> {
    tracing::debug!(thread_id = ?location.thread_id(), "[todos][ops] clear entry");
    let _scratch_guard = maybe_scratch_lock(location);
    let cards = save_cards(location, Vec::new())?;
    emit_progress(location, &cards);
    Ok(into_snapshot(location, cards))
}

/// Snapshot the current list without mutating.
pub fn list(location: &BoardLocation) -> Result<TodosSnapshot, String> {
    let _scratch_guard = maybe_scratch_lock(location);
    let cards = load_cards(location)?;
    Ok(into_snapshot(location, cards))
}

fn non_empty(s: String) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn enforce_single_in_progress(cards: &[TaskBoardCard]) -> Result<(), String> {
    let in_progress = cards
        .iter()
        .filter(|c| matches!(c.status, TaskCardStatus::InProgress))
        .count();
    if in_progress > 1 {
        return Err(format!(
            "only one todo may be `in_progress` at a time (got {in_progress})"
        ));
    }
    Ok(())
}

fn emit_progress(location: &BoardLocation, cards: &[TaskBoardCard]) {
    let BoardLocation::Thread { thread_id, .. } = location else {
        return;
    };
    let Some(parent) = crate::openhuman::agent::harness::fork_context::current_parent() else {
        return;
    };
    let Some(tx) = parent.on_progress else {
        return;
    };
    let board = TaskBoard {
        thread_id: thread_id.clone(),
        cards: cards.to_vec(),
        updated_at: Utc::now().to_rfc3339(),
    };
    if let Err(err) = tx.try_send(AgentProgress::TaskBoardUpdated { board }) {
        tracing::debug!(
            thread_id = %thread_id,
            error = %err,
            "[todos][ops] task board progress dropped"
        );
    }
}

/// Process-global lock that test code (here and in
/// `agent::tools::todo`) uses to serialize access to the shared
/// scratch store under `cargo test`'s parallel runner.
#[cfg(test)]
pub(crate) fn scratch_test_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::Mutex;
    use std::sync::OnceLock;
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn thread_loc(dir: &std::path::Path, id: &str) -> BoardLocation {
        BoardLocation::Thread {
            workspace_dir: dir.to_path_buf(),
            thread_id: id.to_string(),
        }
    }

    #[test]
    fn parse_status_accepts_aliases() {
        assert_eq!(parse_status("todo").unwrap(), TaskCardStatus::Todo);
        assert_eq!(parse_status("PENDING").unwrap(), TaskCardStatus::Todo);
        assert_eq!(
            parse_status("in_progress").unwrap(),
            TaskCardStatus::InProgress
        );
        assert_eq!(parse_status("blocked").unwrap(), TaskCardStatus::Blocked);
        assert_eq!(parse_status("done").unwrap(), TaskCardStatus::Done);
        assert_eq!(
            parse_status("awaiting_approval").unwrap(),
            TaskCardStatus::AwaitingApproval
        );
        assert_eq!(parse_status("ready").unwrap(), TaskCardStatus::Ready);
        assert_eq!(parse_status("approved").unwrap(), TaskCardStatus::Ready);
        assert_eq!(parse_status("rejected").unwrap(), TaskCardStatus::Rejected);
        assert!(parse_status("nope").is_err());
    }

    #[test]
    fn add_appends_and_returns_markdown() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "t1");
        let snap = add(
            &loc,
            "First task",
            CardPatch {
                objective: Some("Ship a richer handoff".into()),
                plan: Some(vec![
                    "Inspect existing board".into(),
                    "Update schema".into(),
                ]),
                assigned_agent: Some("planner".into()),
                allowed_tools: Some(vec!["todo".into(), "spawn_subagent".into()]),
                approval_mode: Some(Some(TaskApprovalMode::Required)),
                acceptance_criteria: Some(vec!["Tests pass".into()]),
                evidence: Some(vec!["cargo test".into()]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(snap.cards.len(), 1);
        assert!(snap.markdown.contains("[ ] First task"));
        assert!(snap.markdown.contains("objective: Ship a richer handoff"));
        assert!(snap.markdown.contains("agent: planner"));
        assert!(snap.markdown.contains("tools: todo, spawn_subagent"));
        assert!(snap.markdown.contains("approval: required"));
        assert!(snap.markdown.contains("Inspect existing board"));
        assert!(snap.markdown.contains("Tests pass"));
        assert!(snap.markdown.contains("cargo test"));
        assert!(snap.markdown.contains(&snap.cards[0].id));
    }

    #[test]
    fn edit_updates_fields_by_id() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "t1");
        let added = add(&loc, "Draft plan", CardPatch::default()).unwrap();
        let id = added.cards[0].id.clone();
        let snap = edit(
            &loc,
            &id,
            CardPatch {
                content: Some("Refined plan".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(snap.cards[0].title, "Refined plan");
    }

    #[test]
    fn source_metadata_round_trips_through_add_and_edit() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "t1");
        let added = add(
            &loc,
            "ingested task",
            CardPatch {
                source_metadata: Some(serde_json::json!({
                    "provider": "github",
                    "external_id": "7",
                })),
                ..Default::default()
            },
        )
        .unwrap();
        let id = added.cards[0].id.clone();
        assert_eq!(
            added.cards[0].source_metadata.as_ref().unwrap()["external_id"],
            serde_json::json!("7")
        );

        // A subsequent edit with `Some(..)` replaces the stamped metadata.
        let snap = edit(
            &loc,
            &id,
            CardPatch {
                source_metadata: Some(serde_json::json!({
                    "provider": "github",
                    "external_id": "8",
                })),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            snap.cards[0].source_metadata.as_ref().unwrap()["external_id"],
            serde_json::json!("8")
        );

        // An edit that leaves `source_metadata: None` preserves the value.
        let snap2 = edit(
            &loc,
            &id,
            CardPatch {
                notes: Some("touch".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            snap2.cards[0].source_metadata.as_ref().unwrap()["external_id"],
            serde_json::json!("8")
        );
    }

    #[test]
    fn decide_plan_approves_and_rejects_only_when_awaiting() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "t1");
        let added = add(&loc, "task", CardPatch::default()).unwrap();
        let id = added.cards[0].id.clone();

        // A todo card isn't awaiting approval yet → decision rejected.
        assert!(decide_plan(&loc, &id, true).is_err());

        // Park it, then approve → Ready.
        update_status(&loc, &id, TaskCardStatus::AwaitingApproval).unwrap();
        let approved = decide_plan(&loc, &id, true).unwrap();
        assert_eq!(approved.cards[0].status, TaskCardStatus::Ready);

        // Re-park, then reject → Rejected.
        update_status(&loc, &id, TaskCardStatus::AwaitingApproval).unwrap();
        let rejected = decide_plan(&loc, &id, false).unwrap();
        assert_eq!(rejected.cards[0].status, TaskCardStatus::Rejected);
    }

    #[test]
    fn edit_can_clear_approval_mode() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "t1");
        let added = add(
            &loc,
            "Draft plan",
            CardPatch {
                approval_mode: Some(Some(TaskApprovalMode::Required)),
                ..Default::default()
            },
        )
        .unwrap();
        let id = added.cards[0].id.clone();

        let snap = edit(
            &loc,
            &id,
            CardPatch {
                approval_mode: Some(None),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(snap.cards[0].approval_mode, None);
    }

    #[test]
    fn edit_unknown_id_errors() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "t1");
        let err = edit(&loc, "task-missing", CardPatch::default()).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn update_status_changes_only_status() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "t1");
        let added = add(&loc, "Write tests", CardPatch::default()).unwrap();
        let id = added.cards[0].id.clone();
        let snap = update_status(&loc, &id, TaskCardStatus::Done).unwrap();
        assert_eq!(snap.cards[0].status, TaskCardStatus::Done);
        assert!(snap.markdown.contains("[x] Write tests"));
    }

    #[test]
    fn remove_drops_card_by_id() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "t1");
        let a = add(&loc, "A", CardPatch::default()).unwrap();
        let _ = add(&loc, "B", CardPatch::default()).unwrap();
        let snap = remove(&loc, &a.cards[0].id).unwrap();
        assert_eq!(snap.cards.len(), 1);
        assert_eq!(snap.cards[0].title, "B");
    }

    #[test]
    fn replace_enforces_single_in_progress() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "t1");
        let cards = vec![
            TaskBoardCard {
                id: "a".into(),
                title: "A".into(),
                status: TaskCardStatus::InProgress,
                objective: None,
                plan: Vec::new(),
                assigned_agent: None,
                allowed_tools: Vec::new(),
                approval_mode: None,
                acceptance_criteria: Vec::new(),
                evidence: Vec::new(),
                notes: None,
                blocker: None,
                source_metadata: None,
                order: 0,
                updated_at: String::new(),
            },
            TaskBoardCard {
                id: "b".into(),
                title: "B".into(),
                status: TaskCardStatus::InProgress,
                objective: None,
                plan: Vec::new(),
                assigned_agent: None,
                allowed_tools: Vec::new(),
                approval_mode: None,
                acceptance_criteria: Vec::new(),
                evidence: Vec::new(),
                notes: None,
                blocker: None,
                source_metadata: None,
                order: 1,
                updated_at: String::new(),
            },
        ];
        let err = replace(&loc, cards).unwrap_err();
        assert!(err.contains("in_progress"));
    }

    #[test]
    fn clear_empties_the_list() {
        let dir = tempdir().unwrap();
        let loc = thread_loc(dir.path(), "t1");
        let _ = add(&loc, "A", CardPatch::default()).unwrap();
        let snap = clear(&loc).unwrap();
        assert!(snap.cards.is_empty());
        assert!(snap.markdown.contains("No todos"));
    }

    #[test]
    fn scratch_store_works_without_thread_context() {
        let _guard = super::scratch_test_lock();
        global_scratch_store().replace(Vec::new());
        let loc = BoardLocation::Scratch;
        let snap = add(&loc, "Scratch task", CardPatch::default()).unwrap();
        assert_eq!(snap.cards.len(), 1);
        assert!(snap.thread_id.is_none());
        let listed = list(&loc).unwrap();
        assert_eq!(listed.cards.len(), 1);
        global_scratch_store().replace(Vec::new());
    }
}
