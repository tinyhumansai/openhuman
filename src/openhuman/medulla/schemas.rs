//! Controller schemas for the `medulla` RPC namespace.
//!
//! Handlers delegate straight to [`super::ops`]; no business logic lives here.
//! Registered under [`DomainGroup::Medulla`](crate::core::all::DomainGroup) at
//! the single site in `src/core/all.rs`, so a host that switches the family off
//! sees these methods as unknown rather than as failing.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::ops;

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CreateSessionParams {
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionIdParams {
    session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendMessageParams {
    session_id: String,
    body: String,
    /// Block until the backend replies. Defaults to false so a caller that
    /// omits it gets the non-blocking behaviour a UI wants.
    #[serde(default)]
    sync: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplayParams {
    session_id: String,
    /// Replay cursor: the last seq already seen. Absent replays from the start.
    #[serde(default)]
    after: Option<i64>,
}

/// Every schema in the namespace, for `/schema` introspection.
pub fn all_medulla_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        medulla_schemas("medulla_status"),
        medulla_schemas("medulla_list_sessions"),
        medulla_schemas("medulla_create_session"),
        medulla_schemas("medulla_get_session"),
        medulla_schemas("medulla_send_message"),
        medulla_schemas("medulla_abort"),
        medulla_schemas("medulla_list_messages"),
        medulla_schemas("medulla_list_events"),
        medulla_schemas("medulla_roster"),
    ]
}

/// Every controller in the namespace, for dispatch.
pub fn all_medulla_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: medulla_schemas("medulla_status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: medulla_schemas("medulla_list_sessions"),
            handler: handle_list_sessions,
        },
        RegisteredController {
            schema: medulla_schemas("medulla_create_session"),
            handler: handle_create_session,
        },
        RegisteredController {
            schema: medulla_schemas("medulla_get_session"),
            handler: handle_get_session,
        },
        RegisteredController {
            schema: medulla_schemas("medulla_send_message"),
            handler: handle_send_message,
        },
        RegisteredController {
            schema: medulla_schemas("medulla_abort"),
            handler: handle_abort,
        },
        RegisteredController {
            schema: medulla_schemas("medulla_list_messages"),
            handler: handle_list_messages,
        },
        RegisteredController {
            schema: medulla_schemas("medulla_list_events"),
            handler: handle_list_events,
        },
        RegisteredController {
            schema: medulla_schemas("medulla_roster"),
            handler: handle_roster,
        },
    ]
}

/// Schema for one function in the namespace.
pub fn medulla_schemas(function: &str) -> ControllerSchema {
    match function {
        "medulla_status" => ControllerSchema {
            namespace: "medulla",
            function: "status",
            description: "Whether the Medulla integration is configured and signed in. Never performs a network call.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Json,
                comment: "Readiness: configured flag, resolved base URL, session-token presence, and a stable reason when unconfigured.",
                required: true,
            }],
        },
        "medulla_list_sessions" => ControllerSchema {
            namespace: "medulla",
            function: "list_sessions",
            description: "List the operator's durable Medulla sessions.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "sessions",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Session summaries ordered by the backend.",
                required: true,
            }],
        },
        "medulla_create_session" => ControllerSchema {
            namespace: "medulla",
            function: "create_session",
            description: "Create a durable Medulla session.",
            inputs: vec![FieldSchema {
                name: "title",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional title. Omitted lets the backend name the session.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "session",
                ty: TypeSchema::Json,
                comment: "The created session's identifier.",
                required: true,
            }],
        },
        "medulla_get_session" => ControllerSchema {
            namespace: "medulla",
            function: "get_session",
            description: "Fetch one session's current state.",
            inputs: vec![session_id_input()],
            outputs: vec![FieldSchema {
                name: "session",
                ty: TypeSchema::Json,
                comment: "Full session detail.",
                required: true,
            }],
        },
        "medulla_send_message" => ControllerSchema {
            namespace: "medulla",
            function: "send_message",
            description: "Send a message to a session, optionally blocking until it replies.",
            inputs: vec![
                session_id_input(),
                FieldSchema {
                    name: "body",
                    ty: TypeSchema::String,
                    comment: "Message text.",
                    required: true,
                },
                FieldSchema {
                    name: "sync",
                    ty: TypeSchema::Bool,
                    comment: "Block until the backend replies. Defaults to false.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Cycle id and sequence, plus the reply when sync was set.",
                required: true,
            }],
        },
        "medulla_abort" => ControllerSchema {
            namespace: "medulla",
            function: "abort",
            description: "Abort a session's running cycle.",
            inputs: vec![session_id_input()],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Whether a cycle was actually aborted.",
                required: true,
            }],
        },
        "medulla_list_messages" => ControllerSchema {
            namespace: "medulla",
            function: "list_messages",
            description: "Replay a session's messages after a sequence cursor.",
            inputs: vec![session_id_input(), after_input()],
            outputs: vec![FieldSchema {
                name: "messages",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Messages newer than the cursor.",
                required: true,
            }],
        },
        "medulla_list_events" => ControllerSchema {
            namespace: "medulla",
            function: "list_events",
            description: "Replay a session's events after a sequence cursor.",
            inputs: vec![session_id_input(), after_input()],
            outputs: vec![FieldSchema {
                name: "events",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Sequenced events newer than the cursor.",
                required: true,
            }],
        },
        "medulla_roster" => ControllerSchema {
            namespace: "medulla",
            function: "roster",
            description: "Read the roster of workers currently connected to the Medulla backend.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "workers",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Connected worker entries.",
                required: true,
            }],
        },
        other => panic!("unknown medulla controller function: {other}"),
    }
}

/// The session identifier every per-session method takes.
fn session_id_input() -> FieldSchema {
    FieldSchema {
        name: "sessionId",
        ty: TypeSchema::String,
        comment: "Target session identifier.",
        required: true,
    }
}

/// The replay cursor shared by the two list methods.
fn after_input() -> FieldSchema {
    FieldSchema {
        name: "after",
        ty: TypeSchema::Option(Box::new(TypeSchema::I64)),
        comment: "Last sequence already seen. Absent replays from the start.",
        required: false,
    }
}

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(ops::status(&load_config().await?).await?) })
}

fn handle_list_sessions(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(ops::list_sessions(&load_config().await?).await?) })
}

fn handle_create_session(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: CreateSessionParams = deserialize_params(params)?;
        to_json(ops::create_session(&load_config().await?, p.title.as_deref()).await?)
    })
}

fn handle_get_session(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SessionIdParams = deserialize_params(params)?;
        to_json(ops::get_session(&load_config().await?, &p.session_id).await?)
    })
}

fn handle_send_message(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SendMessageParams = deserialize_params(params)?;
        to_json(ops::send_message(&load_config().await?, &p.session_id, &p.body, p.sync).await?)
    })
}

fn handle_abort(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SessionIdParams = deserialize_params(params)?;
        to_json(ops::abort(&load_config().await?, &p.session_id).await?)
    })
}

fn handle_list_messages(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: ReplayParams = deserialize_params(params)?;
        to_json(ops::list_messages(&load_config().await?, &p.session_id, p.after).await?)
    })
}

fn handle_list_events(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: ReplayParams = deserialize_params(params)?;
        to_json(ops::list_events(&load_config().await?, &p.session_id, p.after).await?)
    })
}

fn handle_roster(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(ops::roster(&load_config().await?).await?) })
}

/// Load the ambient config for a handler.
async fn load_config() -> Result<crate::openhuman::config::Config, String> {
    crate::openhuman::config::ops::load_config_with_timeout().await
}

/// Decode a controller's params into its typed shape.
fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

/// Serialize an outcome through the shared CLI-compatible envelope.
fn to_json<T: serde::Serialize>(outcome: crate::rpc::RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
