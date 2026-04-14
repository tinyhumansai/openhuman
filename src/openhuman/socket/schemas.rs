//! Controller schemas and RPC handlers for the `socket` namespace.

use serde_json::{json, Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::manager::global_socket_manager;

// ---------------------------------------------------------------------------
// Schema catalog
// ---------------------------------------------------------------------------

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("connect"),
        schemas("disconnect"),
        schemas("state"),
        schemas("emit"),
        schemas("connect_with_session"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("connect"),
            handler: handle_connect,
        },
        RegisteredController {
            schema: schemas("disconnect"),
            handler: handle_disconnect,
        },
        RegisteredController {
            schema: schemas("state"),
            handler: handle_state,
        },
        RegisteredController {
            schema: schemas("emit"),
            handler: handle_emit,
        },
        RegisteredController {
            schema: schemas("connect_with_session"),
            handler: handle_connect_with_session,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "connect" => ControllerSchema {
            namespace: "socket",
            function: "connect",
            description: "Connect to the backend Socket.IO server.",
            inputs: vec![
                FieldSchema {
                    name: "url",
                    ty: TypeSchema::String,
                    comment: "Backend WebSocket base URL.",
                    required: true,
                },
                FieldSchema {
                    name: "token",
                    ty: TypeSchema::String,
                    comment: "Authentication JWT token.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::String,
                comment: "Connection status after initiating.",
                required: true,
            }],
        },
        "disconnect" => ControllerSchema {
            namespace: "socket",
            function: "disconnect",
            description: "Disconnect from the backend Socket.IO server.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::String,
                comment: "Connection status after disconnect.",
                required: true,
            }],
        },
        "state" => ControllerSchema {
            namespace: "socket",
            function: "state",
            description: "Get the current socket connection state.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "state",
                ty: TypeSchema::Json,
                comment: "Current socket state (status, socket_id, error).",
                required: true,
            }],
        },
        "emit" => ControllerSchema {
            namespace: "socket",
            function: "emit",
            description: "Emit a Socket.IO event to the backend server.",
            inputs: vec![
                FieldSchema {
                    name: "event",
                    ty: TypeSchema::String,
                    comment: "Event name to emit.",
                    required: true,
                },
                FieldSchema {
                    name: "data",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Event payload data.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Whether the emit succeeded.",
                required: true,
            }],
        },
        "connect_with_session" => ControllerSchema {
            namespace: "socket",
            function: "connect_with_session",
            description:
                "Connect to the backend using the stored session token and configured API URL.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::String,
                comment: "Connection status after initiating.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "socket",
            function: "unknown",
            description: "Unknown socket controller function.",
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

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn require_manager() -> Result<&'static std::sync::Arc<super::SocketManager>, String> {
    global_socket_manager()
        .ok_or_else(|| "SocketManager not initialized — runtime not bootstrapped".to_string())
}

fn handle_connect(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mgr = require_manager()?;
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or("missing required param 'url'")?;
        let token = params
            .get("token")
            .and_then(|v| v.as_str())
            .ok_or("missing required param 'token'")?;

        log::info!("[socket:rpc] connect url={}", url);
        mgr.connect(url, token).await?;

        let state = mgr.get_state();
        Ok(json!({ "status": format!("{:?}", state.status) }))
    })
}

fn handle_disconnect(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mgr = require_manager()?;
        log::info!("[socket:rpc] disconnect");
        mgr.disconnect().await?;

        let state = mgr.get_state();
        Ok(json!({ "status": format!("{:?}", state.status) }))
    })
}

fn handle_state(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mgr = require_manager()?;
        let state = mgr.get_state();
        log::debug!("[socket:rpc] state → {:?}", state.status);
        serde_json::to_value(state).map_err(|e| format!("serialize: {e}"))
    })
}

fn handle_emit(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mgr = require_manager()?;
        let event = params
            .get("event")
            .and_then(|v| v.as_str())
            .ok_or("missing required param 'event'")?;
        let data = params.get("data").cloned().unwrap_or(Value::Null);

        log::debug!("[socket:rpc] emit event={}", event);
        mgr.emit(event, data).await?;
        Ok(json!({ "ok": true }))
    })
}

fn handle_connect_with_session(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mgr = require_manager()?;

        log::info!("[socket:rpc] connect_with_session — resolving credentials");

        // Load config for API URL and session token.
        let config = crate::openhuman::config::rpc::load_config_with_timeout().await?;
        let api_url = crate::api::config::effective_api_url(&config.api_url);
        let token = crate::api::jwt::get_session_token(&config)
            .map_err(|e| format!("failed to read session token: {e}"))?
            .ok_or("no session token stored — user must log in first")?;

        log::info!(
            "[socket:rpc] connect_with_session url={} token_len={}",
            api_url,
            token.len()
        );

        mgr.connect(&api_url, &token).await?;

        let state = mgr.get_state();
        Ok(json!({ "status": format!("{:?}", state.status) }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_all_five_controllers() {
        let schemas = all_controller_schemas();
        assert_eq!(schemas.len(), 5);
        let names: Vec<&str> = schemas.iter().map(|s| s.function).collect();
        assert!(names.contains(&"connect"));
        assert!(names.contains(&"disconnect"));
        assert!(names.contains(&"state"));
        assert!(names.contains(&"emit"));
        assert!(names.contains(&"connect_with_session"));
    }

    #[test]
    fn registered_controllers_match_schemas_count() {
        let schemas = all_controller_schemas();
        let handlers = all_registered_controllers();
        assert_eq!(schemas.len(), handlers.len());
    }

    #[test]
    fn all_schemas_use_socket_namespace() {
        for s in all_controller_schemas() {
            assert_eq!(s.namespace, "socket", "function {}", s.function);
            assert!(
                !s.description.is_empty(),
                "function {} has empty description",
                s.function
            );
        }
    }

    #[test]
    fn connect_schema_requires_url_and_token() {
        let s = schemas("connect");
        let required: Vec<&str> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert!(required.contains(&"url"));
        assert!(required.contains(&"token"));
    }

    #[test]
    fn disconnect_and_state_have_no_inputs() {
        assert!(schemas("disconnect").inputs.is_empty());
        assert!(schemas("state").inputs.is_empty());
        assert!(schemas("connect_with_session").inputs.is_empty());
    }

    #[test]
    fn emit_schema_data_is_optional() {
        let s = schemas("emit");
        let event = s.inputs.iter().find(|f| f.name == "event").unwrap();
        let data = s.inputs.iter().find(|f| f.name == "data").unwrap();
        assert!(event.required);
        assert!(!data.required);
    }

    #[test]
    fn unknown_function_returns_unknown_fallback_schema() {
        let s = schemas("no_such_fn");
        assert_eq!(s.namespace, "socket");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.outputs.len(), 1);
        assert_eq!(s.outputs[0].name, "error");
    }

    #[test]
    fn every_schema_has_at_least_one_output_field() {
        for s in all_controller_schemas() {
            assert!(
                !s.outputs.is_empty(),
                "schema `{}` must expose ≥1 output field for RPC callers",
                s.function
            );
        }
    }

    #[test]
    fn all_registered_controllers_have_socket_namespace() {
        for h in all_registered_controllers() {
            assert_eq!(h.schema.namespace, "socket");
            assert!(!h.schema.function.is_empty());
        }
    }

    #[test]
    fn connect_schema_inputs_contain_url_and_token() {
        let s = schemas("connect");
        let names: Vec<&str> = s.inputs.iter().map(|f| f.name).collect();
        assert!(names.contains(&"url"));
        assert!(names.contains(&"token"));
    }

    // ── handlers (without manager): require_manager errors ─────────

    #[tokio::test]
    async fn handlers_error_without_initialized_manager() {
        // Production bootstrap calls `set_global_socket_manager` once; in
        // these unit tests the global singleton is intentionally NOT set,
        // so every handler should hit the `SocketManager not initialized`
        // branch via `require_manager()` first.
        //
        // We can't reliably clear a OnceLock once set. If another test in
        // the same binary has already installed a global manager, skip
        // rather than cross-contaminating.
        if super::global_socket_manager().is_some() {
            eprintln!(
                "[socket:schemas tests] global manager already installed — \
                 skipping require_manager error-path assertions"
            );
            return;
        }

        let err = handle_disconnect(Map::new()).await.unwrap_err();
        assert!(err.contains("SocketManager not initialized"));

        let err = handle_state(Map::new()).await.unwrap_err();
        assert!(err.contains("SocketManager not initialized"));

        let err = handle_connect(Map::new()).await.unwrap_err();
        assert!(err.contains("SocketManager not initialized"));

        let err = handle_emit(Map::new()).await.unwrap_err();
        assert!(err.contains("SocketManager not initialized"));
    }
}
