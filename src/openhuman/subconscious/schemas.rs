//! RPC endpoints for the subconscious agent loop.

use serde_json::{Map, Value};

use super::global::get_or_init_engine;
use super::reflection_store;
use super::store;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("status"),
        schemas("trigger"),
        schemas("reflections_list"),
        schemas("reflections_act"),
        schemas("reflections_dismiss"),
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
            schema: schemas("reflections_list"),
            handler: handle_reflections_list,
        },
        RegisteredController {
            schema: schemas("reflections_act"),
            handler: handle_reflections_act,
        },
        RegisteredController {
            schema: schemas("reflections_dismiss"),
            handler: handle_reflections_dismiss,
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
        "reflections_list" => ControllerSchema {
            namespace: "subconscious",
            function: "reflections_list",
            description: "List recent subconscious thoughts. Newest first.",
            inputs: vec![
                field_opt("limit", TypeSchema::U64, "Max entries (default 50)."),
                field_opt(
                    "since_ts",
                    TypeSchema::F64,
                    "Epoch seconds — only return thoughts newer than this.",
                ),
            ],
            outputs: vec![field("reflections", TypeSchema::Json, "Thought records.")],
        },
        "reflections_act" => ControllerSchema {
            namespace: "subconscious",
            function: "reflections_act",
            description: "Act on a thought — creates a fresh conversation thread \
                 and seeds it with the thought body as the first ASSISTANT \
                 message. Returns the new thread id.",
            inputs: vec![field_req(
                "reflection_id",
                TypeSchema::String,
                "Thought ID.",
            )],
            outputs: vec![field(
                "result",
                TypeSchema::Json,
                "{reflection_id, thread_id}.",
            )],
        },
        "reflections_dismiss" => ControllerSchema {
            namespace: "subconscious",
            function: "reflections_dismiss",
            description: "Dismiss a thought card. Sets `dismissed_at`.",
            inputs: vec![field_req(
                "reflection_id",
                TypeSchema::String,
                "Thought ID.",
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
        // Prefer live engine status (includes in-memory tick counters).
        // Fall back to a config-derived snapshot when the engine is not yet
        // initialised — the counters will read 0, which is accurate at that
        // point because no ticks have run yet.
        let engine_arc = get_or_init_engine().await.ok();
        if let Some(arc) = engine_arc {
            let guard = arc.lock().await;
            if let Some(engine) = guard.as_ref() {
                let status = engine.status().await;
                return to_json(RpcOutcome::single_log(status, "subconscious status"));
            }
        }

        // Engine not yet initialised — build a snapshot from config.
        let config = load_config().await?;
        let hb = &config.heartbeat;

        let last_tick_at =
            store::with_connection(&config.workspace_dir, |conn| store::get_last_tick_at(conn))
                .ok();

        let provider_unavailable_reason = if hb.enabled && hb.inference_enabled {
            super::engine::subconscious_provider_unavailable_reason(&config)
        } else {
            None
        };
        let mode = hb.effective_subconscious_mode();
        // total_ticks and consecutive_failures are 0 here because the engine
        // has not started; the engine Mutex cannot be held during RPC.
        let status = super::types::SubconsciousStatus {
            enabled: mode.is_enabled(),
            mode: mode.as_str().to_string(),
            provider_available: provider_unavailable_reason.is_none(),
            provider_unavailable_reason,
            interval_minutes: mode.default_interval_minutes().max(5),
            last_tick_at: last_tick_at.filter(|v| *v > 0.0),
            total_ticks: 0,
            consecutive_failures: 0,
        };

        to_json(RpcOutcome::single_log(status, "subconscious status"))
    })
}

fn handle_trigger(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let lock = get_or_init_engine().await?;

        let lock_clone = std::sync::Arc::clone(&lock);
        tokio::spawn(async move {
            let guard = lock_clone.lock().await;
            if let Some(engine) = guard.as_ref() {
                match engine.tick().await {
                    Ok(result) => {
                        tracing::info!(
                            "[subconscious] manual tick: thoughts={} thread={:?} duration={}ms",
                            result.thoughts_count,
                            result.thread_id,
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

fn handle_reflections_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
        let since_ts = params.get("since_ts").and_then(|v| v.as_f64());
        let config = load_config().await?;
        let reflections = store::with_connection(&config.workspace_dir, |conn| {
            reflection_store::list_recent(conn, limit, since_ts)
        })
        .map_err(|e| format!("{e:#}"))?;
        to_json(RpcOutcome::single_log(reflections, "reflections listed"))
    })
}

fn handle_reflections_act(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let reflection_id = params
            .get("reflection_id")
            .and_then(|v| v.as_str())
            .ok_or("reflection_id is required")?
            .to_string();

        let config = load_config().await?;
        let reflection = store::with_connection(&config.workspace_dir, |conn| {
            reflection_store::get_reflection(conn, &reflection_id)
        })
        .map_err(|e| format!("{e:#}"))?
        .ok_or_else(|| format!("reflection not found: {reflection_id}"))?;

        let thread_id = uuid::Uuid::new_v4().to_string();
        let thread_title: String = {
            let mut s: String = reflection
                .body
                .chars()
                .filter(|c| !c.is_control())
                .take(60)
                .collect();
            if reflection.body.chars().count() > 60 {
                s.push('…');
            }
            if s.trim().is_empty() {
                format!(
                    "Reflection: {kind}",
                    kind = reflection.kind.as_str().replace('_', " ")
                )
            } else {
                s
            }
        };
        let now_iso = chrono::Utc::now().to_rfc3339();
        crate::openhuman::memory_conversations::ensure_thread(
            config.workspace_dir.clone(),
            crate::openhuman::memory_conversations::CreateConversationThread {
                id: thread_id.clone(),
                title: thread_title,
                created_at: now_iso.clone(),
                parent_thread_id: None,
                labels: Some(vec!["subconscious".to_string()]),
                personality_id: None,
            },
        )
        .map_err(|e| format!("ensure_thread (reflection-spawned) failed: {e}"))?;

        let body_md = match reflection.proposed_action.as_deref() {
            Some(action) if !action.trim().is_empty() => format!(
                "{body}\n\n_Proposed action_: {action}",
                body = reflection.body.trim(),
                action = action.trim()
            ),
            _ => reflection.body.trim().to_string(),
        };
        let extra_metadata = serde_json::json!({
            "reflection_id": reflection.id,
            "kind": reflection.kind.as_str(),
            "proposed_action": reflection.proposed_action,
            "source_refs": reflection.source_refs,
            "origin": "subconscious_reflection",
        });
        let seed_message = crate::openhuman::memory_conversations::ConversationMessage {
            id: uuid::Uuid::new_v4().to_string(),
            content: body_md,
            message_type: "text".to_string(),
            extra_metadata,
            sender: "assistant".to_string(),
            created_at: now_iso,
        };
        crate::openhuman::memory_conversations::append_message(
            config.workspace_dir.clone(),
            &thread_id,
            seed_message,
        )
        .map_err(|e| format!("append seed reflection message failed: {e}"))?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        if let Err(e) = store::with_connection(&config.workspace_dir, |conn| {
            reflection_store::mark_acted(conn, &reflection_id, now)
        }) {
            log::warn!(
                "[subconscious] failed to stamp acted_on_at reflection={} thread={}: {e}",
                reflection_id,
                thread_id
            );
        }

        to_json(RpcOutcome::single_log(
            serde_json::json!({
                "reflection_id": reflection_id,
                "thread_id": thread_id,
            }),
            "reflection acted",
        ))
    })
}

fn handle_reflections_dismiss(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let reflection_id = params
            .get("reflection_id")
            .and_then(|v| v.as_str())
            .ok_or("reflection_id is required")?
            .to_string();
        let config = load_config().await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        store::with_connection(&config.workspace_dir, |conn| {
            reflection_store::mark_dismissed(conn, &reflection_id, now)
        })
        .map_err(|e| format!("{e:#}"))?;
        to_json(RpcOutcome::single_log(
            serde_json::json!({"dismissed": reflection_id}),
            "reflection dismissed",
        ))
    })
}

// ── Helpers ──────────────────────────────────────────────────────────────────

async fn load_config() -> Result<crate::openhuman::config::Config, String> {
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
