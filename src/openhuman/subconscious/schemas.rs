//! RPC endpoints for the subconscious task system.

use serde_json::{Map, Value};

use super::global::get_or_init_engine;
use super::store;
use super::types::{EscalationStatus, TaskPatch, TaskRecurrence, TaskSource};
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("status"),
        schemas("trigger"),
        schemas("tasks_list"),
        schemas("tasks_add"),
        schemas("tasks_update"),
        schemas("tasks_remove"),
        schemas("log_list"),
        schemas("escalations_list"),
        schemas("escalations_approve"),
        schemas("escalations_dismiss"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("trigger"),
            handler: handle_trigger,
        },
        RegisteredController {
            schema: schemas("tasks_list"),
            handler: handle_tasks_list,
        },
        RegisteredController {
            schema: schemas("tasks_add"),
            handler: handle_tasks_add,
        },
        RegisteredController {
            schema: schemas("tasks_update"),
            handler: handle_tasks_update,
        },
        RegisteredController {
            schema: schemas("tasks_remove"),
            handler: handle_tasks_remove,
        },
        RegisteredController {
            schema: schemas("log_list"),
            handler: handle_log_list,
        },
        RegisteredController {
            schema: schemas("escalations_list"),
            handler: handle_escalations_list,
        },
        RegisteredController {
            schema: schemas("escalations_approve"),
            handler: handle_escalations_approve,
        },
        RegisteredController {
            schema: schemas("escalations_dismiss"),
            handler: handle_escalations_dismiss,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "status" => ControllerSchema {
            namespace: "subconscious",
            function: "status",
            description: "Get the current subconscious engine status.",
            inputs: vec![],
            outputs: vec![field("result", TypeSchema::Json, "Engine status.")],
        },
        "trigger" => ControllerSchema {
            namespace: "subconscious",
            function: "trigger",
            description: "Manually trigger a subconscious tick.",
            inputs: vec![],
            outputs: vec![field("result", TypeSchema::Json, "Tick result.")],
        },
        "tasks_list" => ControllerSchema {
            namespace: "subconscious",
            function: "tasks_list",
            description: "List all subconscious tasks.",
            inputs: vec![field_opt(
                "enabled_only",
                TypeSchema::Bool,
                "Filter to enabled tasks only.",
            )],
            outputs: vec![field("tasks", TypeSchema::Json, "Array of tasks.")],
        },
        "tasks_add" => ControllerSchema {
            namespace: "subconscious",
            function: "tasks_add",
            description: "Add a new task. The agent classifies it as one-off or recurrent.",
            inputs: vec![
                field_req(
                    "title",
                    TypeSchema::String,
                    "Natural language task description.",
                ),
                field_opt(
                    "source",
                    TypeSchema::String,
                    "Task source: 'user' (default) or 'system'.",
                ),
            ],
            outputs: vec![field("task", TypeSchema::Json, "The created task.")],
        },
        "tasks_update" => ControllerSchema {
            namespace: "subconscious",
            function: "tasks_update",
            description: "Update a task.",
            inputs: vec![
                field_req("task_id", TypeSchema::String, "Task ID to update."),
                field_opt("title", TypeSchema::String, "New title."),
                field_opt(
                    "recurrence",
                    TypeSchema::String,
                    "New recurrence: 'once' | 'cron:<expr>' | 'pending'.",
                ),
                field_opt("enabled", TypeSchema::Bool, "Enable or disable."),
            ],
            outputs: vec![field("result", TypeSchema::Json, "Update confirmation.")],
        },
        "tasks_remove" => ControllerSchema {
            namespace: "subconscious",
            function: "tasks_remove",
            description: "Remove a task.",
            inputs: vec![field_req(
                "task_id",
                TypeSchema::String,
                "Task ID to remove.",
            )],
            outputs: vec![field("result", TypeSchema::Json, "Removal confirmation.")],
        },
        "log_list" => ControllerSchema {
            namespace: "subconscious",
            function: "log_list",
            description: "List execution log entries.",
            inputs: vec![
                field_opt("task_id", TypeSchema::String, "Filter by task ID."),
                field_opt("limit", TypeSchema::U64, "Max entries (default 50)."),
            ],
            outputs: vec![field("entries", TypeSchema::Json, "Log entries.")],
        },
        "escalations_list" => ControllerSchema {
            namespace: "subconscious",
            function: "escalations_list",
            description: "List escalations.",
            inputs: vec![field_opt(
                "status",
                TypeSchema::String,
                "Filter: 'pending' | 'approved' | 'dismissed'.",
            )],
            outputs: vec![field(
                "escalations",
                TypeSchema::Json,
                "Escalation records.",
            )],
        },
        "escalations_approve" => ControllerSchema {
            namespace: "subconscious",
            function: "escalations_approve",
            description: "Approve an escalation — execute the task.",
            inputs: vec![field_req(
                "escalation_id",
                TypeSchema::String,
                "Escalation ID.",
            )],
            outputs: vec![field("result", TypeSchema::Json, "Approval confirmation.")],
        },
        "escalations_dismiss" => ControllerSchema {
            namespace: "subconscious",
            function: "escalations_dismiss",
            description: "Dismiss an escalation — don't execute.",
            inputs: vec![field_req(
                "escalation_id",
                TypeSchema::String,
                "Escalation ID.",
            )],
            outputs: vec![field("result", TypeSchema::Json, "Dismissal confirmation.")],
        },
        _other => ControllerSchema {
            namespace: "subconscious",
            function: "unknown",
            description: "Unknown subconscious function.",
            inputs: vec![],
            outputs: vec![field("error", TypeSchema::String, "Error details.")],
        },
    }
}

// ── Handlers ─────────────────────────────────────────────────────────────────

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        // Read status entirely from DB — never touch the engine mutex.
        // The engine lock is held for the full tick duration, so any RPC
        // that acquires it would block until the tick completes.
        let config = load_config().await?;
        let hb = &config.heartbeat;

        let (task_count, pending_escalations, last_tick_at, total_ticks) =
            store::with_connection(&config.workspace_dir, |conn| {
                let tc = store::task_count(conn).unwrap_or(0);
                let pe = store::pending_escalation_count(conn).unwrap_or(0);
                let (lt, tt) = conn
                    .query_row(
                        "SELECT MAX(tick_at), COUNT(DISTINCT tick_at) FROM subconscious_log",
                        [],
                        |row| Ok((row.get::<_, Option<f64>>(0)?, row.get::<_, u64>(1)?)),
                    )
                    .unwrap_or((None, 0));
                Ok((tc, pe, lt, tt))
            })
            .map_err(|e| e.to_string())?;

        let status = super::types::SubconsciousStatus {
            enabled: hb.enabled && hb.inference_enabled,
            interval_minutes: hb.interval_minutes.max(5),
            last_tick_at,
            total_ticks,
            task_count,
            pending_escalations,
            consecutive_failures: 0, // Only available from in-memory state; 0 is fine for UI
        };

        to_json(RpcOutcome::single_log(status, "subconscious status"))
    })
}

fn handle_trigger(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let lock = get_or_init_engine().await?;

        // Spawn the tick in the background so the RPC returns immediately.
        // The frontend can poll status/log to see in_progress → final transitions.
        let lock_clone = std::sync::Arc::clone(&lock);
        tokio::spawn(async move {
            let guard = lock_clone.lock().await;
            if let Some(engine) = guard.as_ref() {
                match engine.tick().await {
                    Ok(result) => {
                        tracing::info!(
                            "[subconscious] manual tick: executed={} escalated={} duration={}ms",
                            result.executed,
                            result.escalated,
                            result.duration_ms
                        );
                    }
                    Err(e) => {
                        tracing::warn!("[subconscious] manual tick error: {e}");
                    }
                }
            }
        });

        to_json(RpcOutcome::single_log(
            serde_json::json!({"triggered": true}),
            "subconscious tick triggered",
        ))
    })
}

fn handle_tasks_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let enabled_only = params
            .get("enabled_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let config = load_config().await?;
        let tasks = store::with_connection(&config.workspace_dir, |conn| {
            store::list_tasks(conn, enabled_only)
        })
        .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::single_log(tasks, "tasks listed"))
    })
}

fn handle_tasks_add(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let title = params
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or("title is required")?
            .to_string();
        let source = match params.get("source").and_then(|v| v.as_str()) {
            Some("system") => TaskSource::System,
            _ => TaskSource::User,
        };
        let lock = get_or_init_engine().await?;
        let guard = lock.lock().await;
        let engine = guard.as_ref().ok_or("engine not initialized")?;
        let task = engine
            .add_task(&title, source)
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::single_log(task, "task added"))
    })
}

fn handle_tasks_update(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let task_id = params
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or("task_id is required")?
            .to_string();
        let patch = TaskPatch {
            title: params
                .get("title")
                .and_then(|v| v.as_str())
                .map(String::from),
            recurrence: params.get("recurrence").and_then(|v| v.as_str()).map(|s| {
                if s == "once" {
                    TaskRecurrence::Once
                } else if let Some(expr) = s.strip_prefix("cron:") {
                    TaskRecurrence::Cron(expr.to_string())
                } else {
                    TaskRecurrence::Pending
                }
            }),
            enabled: params.get("enabled").and_then(|v| v.as_bool()),
        };
        let config = load_config().await?;
        store::with_connection(&config.workspace_dir, |conn| {
            store::update_task(conn, &task_id, &patch)
        })
        .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::single_log(
            serde_json::json!({"updated": task_id}),
            "task updated",
        ))
    })
}

fn handle_tasks_remove(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let task_id = params
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or("task_id is required")?
            .to_string();
        let config = load_config().await?;
        store::with_connection(&config.workspace_dir, |conn| {
            store::remove_task(conn, &task_id)
        })
        .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::single_log(
            serde_json::json!({"removed": task_id}),
            "task removed",
        ))
    })
}

fn handle_log_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let task_id = params.get("task_id").and_then(|v| v.as_str());
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
        let config = load_config().await?;
        let entries = store::with_connection(&config.workspace_dir, |conn| {
            store::list_log_entries(conn, task_id, limit)
        })
        .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::single_log(entries, "log entries listed"))
    })
}

fn handle_escalations_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let status_filter = params
            .get("status")
            .and_then(|v| v.as_str())
            .map(|s| match s {
                "approved" => EscalationStatus::Approved,
                "dismissed" => EscalationStatus::Dismissed,
                _ => EscalationStatus::Pending,
            });
        let config = load_config().await?;
        let escalations = store::with_connection(&config.workspace_dir, |conn| {
            store::list_escalations(conn, status_filter.as_ref())
        })
        .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::single_log(escalations, "escalations listed"))
    })
}

fn handle_escalations_approve(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let escalation_id = params
            .get("escalation_id")
            .and_then(|v| v.as_str())
            .ok_or("escalation_id is required")?
            .to_string();
        let lock = get_or_init_engine().await?;
        let guard = lock.lock().await;
        let engine = guard.as_ref().ok_or("engine not initialized")?;
        engine
            .approve_escalation(&escalation_id)
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::single_log(
            serde_json::json!({"approved": escalation_id}),
            "escalation approved and executed",
        ))
    })
}

fn handle_escalations_dismiss(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let escalation_id = params
            .get("escalation_id")
            .and_then(|v| v.as_str())
            .ok_or("escalation_id is required")?
            .to_string();
        let lock = get_or_init_engine().await?;
        let guard = lock.lock().await;
        let engine = guard.as_ref().ok_or("engine not initialized")?;
        engine
            .dismiss_escalation(&escalation_id)
            .await
            .map_err(|e| e.to_string())?;
        to_json(RpcOutcome::single_log(
            serde_json::json!({"dismissed": escalation_id}),
            "escalation dismissed",
        ))
    })
}

// ── Helpers ──────────────────────────────────────────────────────────────────

async fn load_config() -> Result<crate::openhuman::config::Config, String> {
    // Use the same 30s-bounded loader every other JSON-RPC domain uses
    // (see cron/schemas.rs, webhooks/schemas.rs, etc.). Raw
    // `Config::load_or_init()` can stall on `SecretStore::new` plus a chain
    // of `decrypt_optional_secret` calls that may IPC to an OS keychain,
    // so the subconscious handlers used to be the only unbounded outlier
    // in the entire JSON-RPC surface. Under the Intelligence page's 3s
    // poll that chokepoint let a slow keychain call pin the frontend's
    // `Promise.all` and freeze the activity log on a stale snapshot.
    crate::openhuman::config::load_config_with_timeout().await
}

fn field(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required: true,
    }
}

fn field_req(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required: true,
    }
}

fn field_opt(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required: false,
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
