//! RPC endpoints for the subconscious agent loop.

use serde_json::{Map, Value};

use super::global::get_or_init_engine;
use super::store;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("status"), schemas("trigger")]
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
        let engine_arc = get_or_init_engine().await.ok();
        if let Some(arc) = engine_arc {
            let guard = arc.lock().await;
            if let Some(engine) = guard.as_ref() {
                let status = engine.status().await;
                return to_json(RpcOutcome::single_log(status, "subconscious status"));
            }
        }

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
                            "[subconscious] manual tick: duration={}ms response_chars={}",
                            result.duration_ms,
                            result.response_chars,
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

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
