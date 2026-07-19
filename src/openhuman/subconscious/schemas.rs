//! RPC endpoints for the subconscious agent loop.

use serde_json::{Map, Value};

use super::factory::SubconsciousKind;
use super::registry::{get_or_init_instance, registered_instances};
use super::store;
use super::types::SubconsciousStatus;
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
            description: "Get subconscious status. Legacy top-level fields mirror the memory \
                          instance; `instances` lists every registered world.",
            inputs: vec![],
            outputs: vec![field("result", TypeSchema::Json, "Engine status.")],
        },
        "trigger" => ControllerSchema {
            namespace: "subconscious",
            function: "trigger",
            description: "Manually trigger a subconscious tick for a world.",
            inputs: vec![optional_field(
                "kind",
                TypeSchema::String,
                "Which world to tick: \"memory\" (default) or \"all\".",
            )],
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

/// The `subconscious.status` response: today's flat status (mirroring the memory
/// instance for backward compatibility) plus one row per registered world.
#[derive(serde::Serialize)]
struct SubconsciousStatusResponse {
    #[serde(flatten)]
    legacy: SubconsciousStatus,
    instances: Vec<SubconsciousStatus>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let instances = registered_instances().await;
        if !instances.is_empty() {
            let mut rows = Vec::with_capacity(instances.len());
            for inst in &instances {
                rows.push(inst.status().await);
            }
            // Legacy top-level = the memory row (or the first registered world).
            let legacy = rows
                .iter()
                .find(|r| r.instance == "memory")
                .or_else(|| rows.first())
                .cloned()
                .expect("non-empty instances");
            let response = SubconsciousStatusResponse {
                legacy,
                instances: rows,
            };
            return to_json(RpcOutcome::single_log(response, "subconscious status"));
        }

        // Not bootstrapped yet — derive the memory row from config (as before).
        let config = load_config().await?;
        let hb = &config.heartbeat;

        let last_tick_at = store::with_connection(&config.workspace_dir, |conn| {
            store::get_last_tick_at(conn, "memory")
        })
        .ok();

        let provider_unavailable_reason = if hb.enabled && hb.inference_enabled {
            super::provider::subconscious_provider_unavailable_reason(&config)
        } else {
            None
        };
        let mode = hb.effective_subconscious_mode();
        let status = SubconsciousStatus {
            instance: "memory".to_string(),
            enabled: mode.is_enabled(),
            mode: mode.as_str().to_string(),
            provider_available: provider_unavailable_reason.is_none(),
            provider_unavailable_reason,
            interval_minutes: mode.default_interval_minutes().max(5),
            last_tick_at: last_tick_at.filter(|v| *v > 0.0),
            total_ticks: 0,
            consecutive_failures: 0,
        };
        let response = SubconsciousStatusResponse {
            legacy: status.clone(),
            instances: vec![status],
        };
        to_json(RpcOutcome::single_log(response, "subconscious status"))
    })
}

fn handle_trigger(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        // `kind`: "memory" (default) or "all". (`tinyplace` was retired with the
        // hosted-brain migration, #4738.)
        let raw = params
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("memory")
            .to_string();

        let kinds: Vec<SubconsciousKind> = if raw == "all" {
            SubconsciousKind::ALL.to_vec()
        } else {
            match SubconsciousKind::parse(&raw) {
                Some(k) => vec![k],
                None => {
                    return Err(format!(
                        "unknown subconscious kind '{raw}' (expected memory|all)"
                    ))
                }
            }
        };

        // Fire-and-forget: spawn each world's tick and return immediately.
        for kind in kinds {
            tokio::spawn(async move {
                match get_or_init_instance(kind).await {
                    Ok(inst) => match inst.tick().await {
                        Ok(result) => tracing::info!(
                            "[subconscious] manual {} tick: duration={}ms response_chars={}",
                            kind.id(),
                            result.duration_ms,
                            result.response_chars,
                        ),
                        Err(e) => {
                            tracing::warn!("[subconscious] manual {} tick error: {e}", kind.id())
                        }
                    },
                    Err(e) => tracing::warn!("[subconscious] manual {} init error: {e}", kind.id()),
                }
            });
        }

        to_json(RpcOutcome::single_log(
            serde_json::json!({"triggered": true, "kind": raw}),
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

fn optional_field(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
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
