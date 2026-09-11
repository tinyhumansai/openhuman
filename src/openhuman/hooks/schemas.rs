//! RPC surface for the `hooks` namespace.
//!
//! Three functions, and the split between them is deliberate: **list** answers
//! "what is configured and where did it come from", **reload** re-reads the
//! files, and **test** fires one synthetic event so an author can see what a
//! hook actually decides without provoking the real moment. Debugging a hook by
//! trying to trigger it — asking the agent to run `rm -rf` to see whether the
//! deny rule fires — is the failure mode this surface exists to prevent.

use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;

use super::context::TurnIdentity;
use super::types::{HookEvent, HookPayload};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("list"), schemas("reload"), schemas("test")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("reload"),
            handler: handle_reload,
        },
        RegisteredController {
            schema: schemas("test"),
            handler: handle_test,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: "hooks",
            function: "list",
            description: "List configured hooks, their source files, and any load warnings.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "hooks",
                ty: TypeSchema::Json,
                comment: "Hooks grouped by event, plus sources and warnings.",
                required: true,
            }],
        },
        "reload" => ControllerSchema {
            namespace: "hooks",
            function: "reload",
            description: "Re-read every hooks.json layer and swap in the result.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "hooks",
                ty: TypeSchema::Json,
                comment: "The freshly loaded configuration.",
                required: true,
            }],
        },
        "test" => ControllerSchema {
            namespace: "hooks",
            function: "test",
            description: "Fire one synthetic event and report what each matching hook decided.",
            inputs: vec![
                FieldSchema {
                    name: "event",
                    ty: TypeSchema::String,
                    comment: "Event name, e.g. 'preToolUse' or 'beforeShellExecution'.",
                    required: true,
                },
                FieldSchema {
                    name: "payload",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Event body. Defaults to an empty body.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Merged decision plus per-hook runs.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "hooks",
            function: "unknown",
            description: "Unknown hooks controller function.",
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

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { Ok(describe(super::engine::global().snapshot().await.as_ref())) })
}

fn handle_reload(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        super::ops::init(&config).await;
        Ok(describe(super::engine::global().snapshot().await.as_ref()))
    })
}

fn handle_test(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let event_name: String = read_required(&params, "event")?;
        let event = HookEvent::parse(&event_name).ok_or_else(|| {
            format!(
                "unknown hook event '{event_name}'; known events: {}",
                HookEvent::ALL
                    .iter()
                    .map(|event| event.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        let payload = HookPayload::from_value_for(
            event,
            params.get("payload").cloned().unwrap_or(Value::Null),
        )
        .map_err(|error| format!("invalid payload for '{event}': {error}"))?;

        let input = super::context::build_input(event, TurnIdentity::default(), payload);
        // A test fire is always run in the foreground, even for an
        // observational event: the point of the endpoint is to show the author
        // what happened, and a detached dispatch would report nothing.
        let outcome = super::engine::global()
            .dispatch_for_test(event, input)
            .await;
        Ok(json!({
            "result": {
                "event": event.as_str(),
                "decision": outcome.output,
                "runs": outcome.runs.iter().map(|run| json!({
                    "hook": run.label,
                    "duration_ms": run.duration.as_millis() as u64,
                    "error": run.error,
                    "output": run.output,
                })).collect::<Vec<_>>(),
            }
        }))
    })
}

/// Render a config for the wire, naming the source file of every definition.
fn describe(config: &super::config::HookConfig) -> Value {
    let by_event: Map<String, Value> = config
        .by_event
        .iter()
        .map(|(event, definitions)| {
            let rendered: Vec<Value> = definitions
                .iter()
                .map(|definition| {
                    json!({
                        "command": definition.command,
                        "type": definition.kind,
                        "matcher": definition.matcher,
                        "timeout": definition.timeout,
                        "fail_closed": definition.fail_closed,
                        "enabled": definition.enabled,
                        "layer": definition.layer.map(super::config::HookLayer::as_str),
                        "source_dir": definition.source_dir.as_ref().map(|dir| dir.display().to_string()),
                    })
                })
                .collect();
            (
                event.as_str().to_string(),
                json!({ "wired": event.is_wired(), "definitions": rendered }),
            )
        })
        .collect();
    json!({
        "hooks": {
            "by_event": by_event,
            "total": config.len(),
            "sources": config.sources.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
            "warnings": config.warnings,
        }
    })
}

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .ok_or_else(|| format!("missing required parameter '{key}'"))?;
    serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid parameter '{key}': {error}"))
}
