//! Controller registry for `desktop_companion`.
//!
//! Exposes the companion session lifecycle over JSON-RPC so the Tauri
//! shell and frontend can drive the desktop companion loop.

use log::{debug, warn};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::RegisteredController;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::memory::EmptyRequest;

use super::session;
use super::types::*;

const LOG_PREFIX: &str = "[companion_rpc]";

pub fn all_desktop_companion_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("start_session"),
        schemas("stop_session"),
        schemas("status"),
        schemas("config_get"),
        schemas("config_set"),
    ]
}

pub fn all_desktop_companion_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("start_session"),
            handler: handle_start_session,
        },
        RegisteredController {
            schema: schemas("stop_session"),
            handler: handle_stop_session,
        },
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("config_get"),
            handler: handle_config_get,
        },
        RegisteredController {
            schema: schemas("config_set"),
            handler: handle_config_set,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "start_session" => ControllerSchema {
            namespace: "companion",
            function: "start_session",
            description: "Start a desktop companion session with explicit consent.",
            inputs: vec![
                field(
                    "consent",
                    TypeSchema::Bool,
                    "User consent for screen monitoring and audio capture.",
                ),
                optional(
                    "ttl_secs",
                    TypeSchema::U64,
                    "Session time-to-live in seconds. 0 = no expiry.",
                ),
            ],
            outputs: vec![json_output(
                "result",
                "Session start result with session_id and state.",
            )],
        },
        "stop_session" => ControllerSchema {
            namespace: "companion",
            function: "stop_session",
            description: "Stop the active desktop companion session.",
            inputs: vec![optional(
                "reason",
                TypeSchema::String,
                "Optional reason for stopping.",
            )],
            outputs: vec![json_output("result", "Session stop result.")],
        },
        "status" => ControllerSchema {
            namespace: "companion",
            function: "status",
            description: "Get the current desktop companion session status.",
            inputs: vec![],
            outputs: vec![json_output(
                "result",
                "Current session status including state and TTL.",
            )],
        },
        "config_get" => ControllerSchema {
            namespace: "companion",
            function: "config_get",
            description: "Get the current desktop companion configuration.",
            inputs: vec![],
            outputs: vec![json_output("result", "Current companion configuration.")],
        },
        "config_set" => ControllerSchema {
            namespace: "companion",
            function: "config_set",
            description: "Update desktop companion configuration.",
            inputs: vec![
                optional(
                    "hotkey",
                    TypeSchema::String,
                    "Hotkey string for activation.",
                ),
                optional(
                    "activation_mode",
                    TypeSchema::String,
                    "Activation mode: push or tap.",
                ),
                optional(
                    "ttl_secs",
                    TypeSchema::U64,
                    "Default session TTL in seconds.",
                ),
                optional(
                    "capture_screen",
                    TypeSchema::Bool,
                    "Whether to capture screenshots.",
                ),
                optional(
                    "include_app_context",
                    TypeSchema::Bool,
                    "Whether to include foreground app info.",
                ),
            ],
            outputs: vec![json_output("result", "Updated companion configuration.")],
        },
        _ => ControllerSchema {
            namespace: "companion",
            function: "unknown",
            description: "Unknown companion controller.",
            inputs: vec![],
            outputs: vec![field("error", TypeSchema::String, "Lookup error details.")],
        },
    }
}

// ── Handlers ──────────────────────────────────────────────────────────

fn handle_start_session(params: Map<String, Value>) -> crate::core::all::ControllerFuture {
    Box::pin(async move {
        debug!("{LOG_PREFIX} start_session entry");
        let req: StartCompanionSessionParams = parse_params(params)?;
        let result = session::start_session(&req).map_err(|e| {
            warn!("{LOG_PREFIX} start_session failed: {e}");
            e
        })?;
        debug!(
            "{LOG_PREFIX} start_session done session_id={}",
            result.session_id
        );
        serde_json::to_value(result).map_err(|e| format!("serialize error: {e}"))
    })
}

fn handle_stop_session(params: Map<String, Value>) -> crate::core::all::ControllerFuture {
    Box::pin(async move {
        debug!("{LOG_PREFIX} stop_session entry");
        let req: StopCompanionSessionParams = parse_params(params)?;
        let result = session::stop_session(&req).map_err(|e| {
            warn!("{LOG_PREFIX} stop_session failed: {e}");
            e
        })?;
        debug!("{LOG_PREFIX} stop_session done stopped={}", result.stopped);
        serde_json::to_value(result).map_err(|e| format!("serialize error: {e}"))
    })
}

fn handle_status(params: Map<String, Value>) -> crate::core::all::ControllerFuture {
    Box::pin(async move {
        debug!("{LOG_PREFIX} status entry");
        let _: EmptyRequest = parse_params(params)?;
        let result = session::session_status();
        debug!("{LOG_PREFIX} status done active={}", result.active);
        serde_json::to_value(result).map_err(|e| format!("serialize error: {e}"))
    })
}

fn handle_config_get(params: Map<String, Value>) -> crate::core::all::ControllerFuture {
    Box::pin(async move {
        debug!("{LOG_PREFIX} config_get entry");
        let _: EmptyRequest = parse_params(params)?;
        let config = CompanionConfig::default();
        debug!("{LOG_PREFIX} config_get done");
        serde_json::to_value(config).map_err(|e| format!("serialize error: {e}"))
    })
}

fn handle_config_set(_params: Map<String, Value>) -> crate::core::all::ControllerFuture {
    Box::pin(async move {
        warn!("{LOG_PREFIX} config_set called but persistence is not yet implemented");
        Err("companion config_set is not yet persisted — changes are not saved".to_string())
    })
}

// ── Helpers ───────────────────────────────────────────────────────────

fn parse_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn field(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required: true,
    }
}

fn optional(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(ty)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemas_count() {
        assert_eq!(all_desktop_companion_controller_schemas().len(), 5);
    }

    #[test]
    fn all_controllers_count() {
        assert_eq!(all_desktop_companion_registered_controllers().len(), 5);
    }

    #[test]
    fn status_schema_has_no_inputs() {
        let schema = schemas("status");
        assert!(schema.inputs.is_empty());
        assert_eq!(schema.namespace, "companion");
    }

    #[test]
    fn start_session_schema_requires_consent() {
        let schema = schemas("start_session");
        let consent_field = schema.inputs.iter().find(|f| f.name == "consent");
        assert!(consent_field.is_some());
        assert!(consent_field.unwrap().required);
    }
}
