//! JSON-RPC surface for the per-thread todo list. Pairs with the agent
//! `todo` tool — both call into [`super::ops`] so user-driven and
//! agent-driven edits share the exact same persistence and rendering
//! logic.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::agent::task_board::{TaskApprovalMode, TaskBoardCard};

use super::ops::{self, BoardLocation, CardPatch, TodosSnapshot};
use super::runs;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list"),
        schemas("add"),
        schemas("edit"),
        schemas("update_status"),
        schemas("set_session_thread"),
        schemas("decide_plan"),
        schemas("revise_plan"),
        schemas("remove"),
        schemas("replace"),
        schemas("clear"),
        schemas("run_list"),
        schemas("run_get"),
        schemas("reclaim_stale"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("add"),
            handler: handle_add,
        },
        RegisteredController {
            schema: schemas("edit"),
            handler: handle_edit,
        },
        RegisteredController {
            schema: schemas("update_status"),
            handler: handle_update_status,
        },
        RegisteredController {
            schema: schemas("set_session_thread"),
            handler: handle_set_session_thread,
        },
        RegisteredController {
            schema: schemas("decide_plan"),
            handler: handle_decide_plan,
        },
        RegisteredController {
            schema: schemas("revise_plan"),
            handler: handle_revise_plan,
        },
        RegisteredController {
            schema: schemas("remove"),
            handler: handle_remove,
        },
        RegisteredController {
            schema: schemas("replace"),
            handler: handle_replace,
        },
        RegisteredController {
            schema: schemas("clear"),
            handler: handle_clear,
        },
        RegisteredController {
            schema: schemas("run_list"),
            handler: handle_run_list,
        },
        RegisteredController {
            schema: schemas("run_get"),
            handler: handle_run_get,
        },
        RegisteredController {
            schema: schemas("reclaim_stale"),
            handler: handle_reclaim_stale,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: "todos",
            function: "list",
            description:
                "Return the current todo list for a conversation thread as cards + markdown.",
            inputs: vec![thread_id_input()],
            outputs: vec![snapshot_output()],
        },
        "add" => ControllerSchema {
            namespace: "todos",
            function: "add",
            description: "Append a new todo card to a conversation thread's list.",
            inputs: vec![
                thread_id_input(),
                required_string("content", "Card title / description."),
                optional_string("status", "Initial status (todo|in_progress|blocked|done)."),
                optional_string("objective", "Task objective / desired outcome."),
                string_array_input("plan", "Ordered lightweight execution steps."),
                optional_string("assignedAgent", "Agent id expected to pick up this task."),
                string_array_input(
                    "allowedTools",
                    "Task-local allowed tool names or toolkit slugs.",
                ),
                optional_string(
                    "approvalMode",
                    "Task approval mode: required | not_required.",
                ),
                string_array_input(
                    "acceptanceCriteria",
                    "Checklist required before the task is done.",
                ),
                string_array_input("evidence", "Verification output, links, files, or notes."),
                optional_string("notes", "Free-text notes."),
                optional_string("blocker", "Reason the card is blocked, if any."),
                FieldSchema {
                    name: "sourceMetadata",
                    ty: TypeSchema::Json,
                    comment: "Originating task-source identifiers ({provider, external_id, …}) \
                              stamped onto a card promoted from the task-sources inbox.",
                    required: false,
                },
            ],
            outputs: vec![snapshot_output()],
        },
        "edit" => ControllerSchema {
            namespace: "todos",
            function: "edit",
            description: "Edit an existing todo card by id. Any omitted field is left unchanged.",
            inputs: vec![
                thread_id_input(),
                required_string("id", "Card identifier returned by `add` / `list`."),
                optional_string("content", "New title / description."),
                optional_string("status", "New status."),
                optional_string("objective", "Task objective / desired outcome."),
                string_array_input("plan", "Ordered lightweight execution steps."),
                optional_string("assignedAgent", "Agent id expected to pick up this task."),
                string_array_input(
                    "allowedTools",
                    "Task-local allowed tool names or toolkit slugs.",
                ),
                optional_string(
                    "approvalMode",
                    "Task approval mode: required | not_required (pass null to clear).",
                ),
                string_array_input(
                    "acceptanceCriteria",
                    "Checklist required before the task is done.",
                ),
                string_array_input("evidence", "Verification output, links, files, or notes."),
                optional_string("notes", "New notes (pass empty string to clear)."),
                optional_string(
                    "blocker",
                    "New blocker reason (pass empty string to clear).",
                ),
            ],
            outputs: vec![snapshot_output()],
        },
        "update_status" => ControllerSchema {
            namespace: "todos",
            function: "update_status",
            description: "Update only the status of a todo card.",
            inputs: vec![
                thread_id_input(),
                required_string("id", "Card identifier."),
                required_string(
                    "status",
                    "New status (todo|awaiting_approval|ready|in_progress|blocked|done|rejected).",
                ),
            ],
            outputs: vec![snapshot_output()],
        },
        "set_session_thread" => ControllerSchema {
            namespace: "todos",
            function: "set_session_thread",
            description: "Link a card to its agent session's conversation thread so the UI can \
                          offer a \"View session\" jump. Empty/absent sessionThreadId clears the link.",
            inputs: vec![
                thread_id_input(),
                required_string("id", "Card identifier."),
                optional_string(
                    "sessionThreadId",
                    "Conversation thread id of the card's agent session; omit or empty to clear.",
                ),
            ],
            outputs: vec![snapshot_output()],
        },
        "decide_plan" => ControllerSchema {
            namespace: "todos",
            function: "decide_plan",
            description: "Approve or reject a card awaiting plan approval \
                          (approve → ready/runnable; reject → rejected).",
            inputs: vec![
                thread_id_input(),
                required_string("id", "Card identifier."),
                FieldSchema {
                    name: "approve",
                    ty: TypeSchema::Bool,
                    comment: "true to approve (card becomes runnable), false to reject.",
                    required: true,
                },
            ],
            outputs: vec![snapshot_output()],
        },
        "revise_plan" => ControllerSchema {
            namespace: "todos",
            function: "revise_plan",
            description: "Reject every card awaiting plan approval so the orchestrator can \
                          re-plan from the user's feedback (the feedback is sent back into the \
                          thread as a message by the caller). Idempotent no-op when nothing is \
                          awaiting.",
            inputs: vec![
                thread_id_input(),
                optional_string(
                    "feedback",
                    "User's free-text revision request (recorded for logging/audit).",
                ),
            ],
            outputs: vec![snapshot_output()],
        },
        "remove" => ControllerSchema {
            namespace: "todos",
            function: "remove",
            description: "Remove a todo card from a thread's list.",
            inputs: vec![thread_id_input(), required_string("id", "Card identifier.")],
            outputs: vec![snapshot_output()],
        },
        "replace" => ControllerSchema {
            namespace: "todos",
            function: "replace",
            description: "Wholesale-replace the todo list for a thread.",
            inputs: vec![
                thread_id_input(),
                FieldSchema {
                    name: "cards",
                    ty: TypeSchema::Json,
                    comment: "Array of card objects (id may be empty — server generates).",
                    required: true,
                },
            ],
            outputs: vec![snapshot_output()],
        },
        "clear" => ControllerSchema {
            namespace: "todos",
            function: "clear",
            description: "Empty the todo list for a thread.",
            inputs: vec![thread_id_input()],
            outputs: vec![snapshot_output()],
        },
        "run_list" => ControllerSchema {
            namespace: "todos",
            function: "run_list",
            description: "List durable run records for a thread, optionally filtered by card id.",
            inputs: vec![
                thread_id_input(),
                optional_string("cardId", "Filter runs by card identifier."),
            ],
            outputs: vec![FieldSchema {
                name: "runs",
                ty: TypeSchema::Json,
                comment: "Array of TaskRun objects.",
                required: true,
            }],
        },
        "run_get" => ControllerSchema {
            namespace: "todos",
            function: "run_get",
            description: "Get a single run record by its run id.",
            inputs: vec![
                thread_id_input(),
                required_string("runId", "Run identifier."),
            ],
            outputs: vec![FieldSchema {
                name: "run",
                ty: TypeSchema::Json,
                comment: "TaskRun object or null if not found.",
                required: true,
            }],
        },
        "reclaim_stale" => ControllerSchema {
            namespace: "todos",
            function: "reclaim_stale",
            description: "Scan for stale/wedged runs and reclaim their cards. \
                 Cards are moved back to todo (re-dispatchable) or blocked \
                 (max reclaim count exceeded).",
            inputs: vec![
                thread_id_input(),
                FieldSchema {
                    name: "heartbeatStaleSecs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Heartbeat staleness threshold in seconds (default 300).",
                    required: false,
                },
                FieldSchema {
                    name: "claimTtlSecs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Claim TTL in seconds (default 3600).",
                    required: false,
                },
                FieldSchema {
                    name: "maxReclaimCount",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max reclaims before parking as blocked (default 3).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Object with reclaimedCount, blockedCount, and details array.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "todos",
            function: "unknown",
            description: "Unknown todos controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

#[derive(Debug, Deserialize)]
struct ThreadIdParams {
    thread_id: String,
}

#[derive(Debug, Deserialize)]
struct AddParams {
    thread_id: String,
    content: String,
    #[serde(default, alias = "sourceMetadata")]
    source_metadata: Option<Value>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    objective: Option<String>,
    #[serde(default)]
    plan: Option<Vec<String>>,
    #[serde(default, alias = "assignedAgent")]
    assigned_agent: Option<String>,
    #[serde(default)]
    #[serde(alias = "allowedTools")]
    allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    #[serde(alias = "approvalMode")]
    approval_mode: Option<String>,
    #[serde(default)]
    #[serde(alias = "acceptanceCriteria")]
    acceptance_criteria: Option<Vec<String>>,
    #[serde(default)]
    evidence: Option<Vec<String>>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    blocker: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EditParams {
    thread_id: String,
    id: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    objective: Option<String>,
    #[serde(default)]
    plan: Option<Vec<String>>,
    #[serde(default, alias = "assignedAgent")]
    assigned_agent: Option<String>,
    #[serde(default)]
    #[serde(alias = "allowedTools")]
    allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    #[serde(alias = "acceptanceCriteria")]
    acceptance_criteria: Option<Vec<String>>,
    #[serde(default)]
    evidence: Option<Vec<String>>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    blocker: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateStatusParams {
    thread_id: String,
    id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct RemoveParams {
    thread_id: String,
    id: String,
}

#[derive(Debug, Deserialize)]
struct SetSessionThreadParams {
    thread_id: String,
    id: String,
    #[serde(default, alias = "sessionThreadId")]
    session_thread_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DecidePlanParams {
    thread_id: String,
    id: String,
    approve: bool,
}

#[derive(Debug, Deserialize)]
struct RevisePlanParams {
    thread_id: String,
    #[serde(default)]
    feedback: String,
}

#[derive(Debug, Deserialize)]
struct ReplaceParams {
    thread_id: String,
    cards: Vec<TaskBoardCard>,
}

fn handle_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ThreadIdParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        tracing::debug!(thread_id = %p.thread_id, "[rpc][todos] list entry");
        snapshot_to_json(ops::list(&loc)?)
    })
}

fn handle_add(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<AddParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        let patch = CardPatch {
            content: None,
            status: p.status.as_deref().map(ops::parse_status).transpose()?,
            objective: p.objective,
            plan: p.plan,
            assigned_agent: p.assigned_agent,
            allowed_tools: p.allowed_tools,
            approval_mode: Some(parse_approval_mode(p.approval_mode)?),
            acceptance_criteria: p.acceptance_criteria,
            evidence: p.evidence,
            notes: p.notes,
            blocker: p.blocker,
            // Carry the originating source identifiers onto the promoted card so
            // the inbox can tell an item was already picked up (dedup/hide).
            source_metadata: p.source_metadata,
        };
        tracing::debug!(thread_id = %p.thread_id, "[rpc][todos] add entry");
        snapshot_to_json(ops::add(&loc, &p.content, patch)?)
    })
}

fn handle_edit(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let approval_mode = approval_mode_patch_from_params(&params)?;
        let p = parse::<EditParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        let patch = CardPatch {
            content: p.content,
            status: p.status.as_deref().map(ops::parse_status).transpose()?,
            objective: p.objective,
            plan: p.plan,
            assigned_agent: p.assigned_agent,
            allowed_tools: p.allowed_tools,
            approval_mode,
            acceptance_criteria: p.acceptance_criteria,
            evidence: p.evidence,
            notes: p.notes,
            blocker: p.blocker,
            source_metadata: None,
        };
        tracing::debug!(thread_id = %p.thread_id, id = %p.id, "[rpc][todos] edit entry");
        snapshot_to_json(ops::edit(&loc, &p.id, patch)?)
    })
}

fn handle_update_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<UpdateStatusParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        let status = ops::parse_status(&p.status)?;
        tracing::debug!(
            thread_id = %p.thread_id,
            id = %p.id,
            status = %p.status,
            "[rpc][todos] update_status entry"
        );
        snapshot_to_json(ops::update_status(&loc, &p.id, status)?)
    })
}

fn handle_set_session_thread(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<SetSessionThreadParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        tracing::debug!(
            thread_id = %p.thread_id,
            id = %p.id,
            session_thread_id = ?p.session_thread_id,
            "[rpc][todos] set_session_thread entry"
        );
        snapshot_to_json(ops::set_session_thread(&loc, &p.id, p.session_thread_id)?)
    })
}

fn handle_decide_plan(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<DecidePlanParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        tracing::debug!(
            thread_id = %p.thread_id,
            id = %p.id,
            approve = p.approve,
            "[rpc][todos] decide_plan entry"
        );
        snapshot_to_json(ops::decide_plan(&loc, &p.id, p.approve)?)
    })
}

fn handle_revise_plan(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<RevisePlanParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        tracing::debug!(
            thread_id = %p.thread_id,
            feedback_len = p.feedback.len(),
            "[rpc][todos] revise_plan entry"
        );
        snapshot_to_json(ops::revise_plan(&loc, &p.feedback)?)
    })
}

fn parse_approval_mode(raw: Option<String>) -> Result<Option<TaskApprovalMode>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    match raw.trim() {
        "required" => Ok(Some(TaskApprovalMode::Required)),
        "not_required" => Ok(Some(TaskApprovalMode::NotRequired)),
        other => Err(format!(
            "invalid approval_mode '{other}' (expected required|not_required)"
        )),
    }
}

fn approval_mode_patch_from_params(
    params: &Map<String, Value>,
) -> Result<Option<Option<TaskApprovalMode>>, String> {
    let Some(value) = params
        .get("approvalMode")
        .or_else(|| params.get("approval_mode"))
    else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(Some(None));
    }
    let Some(raw) = value.as_str() else {
        return Err("invalid approval_mode type (expected required|not_required|null)".to_string());
    };
    parse_approval_mode(Some(raw.to_string())).map(Some)
}

fn handle_remove(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<RemoveParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        tracing::debug!(thread_id = %p.thread_id, id = %p.id, "[rpc][todos] remove entry");
        snapshot_to_json(ops::remove(&loc, &p.id)?)
    })
}

fn handle_replace(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ReplaceParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        tracing::debug!(
            thread_id = %p.thread_id,
            card_count = p.cards.len(),
            "[rpc][todos] replace entry"
        );
        snapshot_to_json(ops::replace(&loc, p.cards)?)
    })
}

fn handle_clear(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ThreadIdParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        tracing::debug!(thread_id = %p.thread_id, "[rpc][todos] clear entry");
        snapshot_to_json(ops::clear(&loc)?)
    })
}

// ── run handlers ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RunListParams {
    thread_id: String,
    #[serde(default, alias = "cardId")]
    card_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RunGetParams {
    thread_id: String,
    #[serde(alias = "runId")]
    run_id: String,
}

#[derive(Debug, Deserialize)]
struct ReclaimStaleParams {
    thread_id: String,
    #[serde(default, alias = "heartbeatStaleSecs")]
    heartbeat_stale_secs: Option<u64>,
    #[serde(default, alias = "claimTtlSecs")]
    claim_ttl_secs: Option<u64>,
    #[serde(default, alias = "maxReclaimCount")]
    max_reclaim_count: Option<u32>,
}

fn handle_run_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<RunListParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        tracing::debug!(
            thread_id = %p.thread_id,
            card_id = ?p.card_id,
            "[rpc][todos] run_list entry"
        );
        let run_list = runs::list_runs(&loc, p.card_id.as_deref())?;
        serde_json::to_value(&run_list).map_err(|e| format!("serialize runs: {e}"))
    })
}

fn handle_run_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<RunGetParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        tracing::debug!(
            thread_id = %p.thread_id,
            run_id = %p.run_id,
            "[rpc][todos] run_get entry"
        );
        let run = runs::get_run(&loc, &p.run_id)?;
        serde_json::to_value(&run).map_err(|e| format!("serialize run: {e}"))
    })
}

fn handle_reclaim_stale(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ReclaimStaleParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        let limits = runs::RunLimits {
            heartbeat_stale_secs: p
                .heartbeat_stale_secs
                .unwrap_or(runs::DEFAULT_HEARTBEAT_STALE_SECS),
            claim_ttl_secs: p.claim_ttl_secs.unwrap_or(runs::DEFAULT_CLAIM_TTL_SECS),
            max_reclaim_count: p
                .max_reclaim_count
                .unwrap_or(runs::DEFAULT_MAX_RECLAIM_COUNT),
        };
        tracing::debug!(
            thread_id = %p.thread_id,
            ?limits,
            "[rpc][todos] reclaim_stale entry"
        );
        let result = runs::reclaim_stale(&loc, &limits)?;
        serde_json::to_value(&result).map_err(|e| format!("serialize reclaim result: {e}"))
    })
}

// ── helpers ──────────────────────────────────────────────────────────

async fn thread_location(thread_id: &str) -> Result<BoardLocation, String> {
    let trimmed = thread_id.trim();
    if trimmed.is_empty() {
        return Err("thread_id must not be empty".to_string());
    }
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    Ok(BoardLocation::Thread {
        workspace_dir: config.workspace_dir,
        thread_id: trimmed.to_string(),
    })
}

fn parse<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn snapshot_to_json(snap: TodosSnapshot) -> Result<Value, String> {
    serde_json::to_value(&snap).map_err(|e| format!("serialize snapshot: {e}"))
}

fn thread_id_input() -> FieldSchema {
    FieldSchema {
        name: "thread_id",
        ty: TypeSchema::String,
        comment: "Conversation thread identifier (same id used by `threads.task_board_*`).",
        required: true,
    }
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn string_array_input(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
        comment,
        required: false,
    }
}

fn snapshot_output() -> FieldSchema {
    FieldSchema {
        name: "snapshot",
        ty: TypeSchema::Json,
        comment: "Object with `threadId`, `cards`, and a `markdown` rendering of the list.",
        required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_lists_match_lengths() {
        assert_eq!(
            all_controller_schemas().len(),
            all_registered_controllers().len()
        );
    }

    #[test]
    fn schemas_have_todos_namespace() {
        for schema in all_controller_schemas() {
            assert_eq!(schema.namespace, "todos", "function={}", schema.function);
        }
    }
}
