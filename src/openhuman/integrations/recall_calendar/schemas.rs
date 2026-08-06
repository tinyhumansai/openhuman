//! Controller schemas + registered handlers for the `recall_calendar` domain.
//!
//! Exposes the backend-proxied Recall.ai Calendar V1 surface over the shared
//! registry at `openhuman.recall_calendar_*`:
//!   - `recall_calendar.connect`       → `openhuman.recall_calendar_connect`
//!   - `recall_calendar.status`        → `openhuman.recall_calendar_status`
//!   - `recall_calendar.disconnect`    → `openhuman.recall_calendar_disconnect`
//!   - `recall_calendar.list_meetings` → `openhuman.recall_calendar_list_meetings`

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "connect" => ControllerSchema {
            namespace: "recall_calendar",
            function: "connect",
            description:
                "Start the Recall.ai Calendar V1 OAuth flow and return the Google consent URL.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "connectUrl",
                ty: TypeSchema::String,
                comment: "Google OAuth consent URL to open in a browser.",
                required: true,
            }],
        },
        "status" => ControllerSchema {
            namespace: "recall_calendar",
            function: "status",
            description: "Report whether the integration is enabled and the calendar is connected.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "enabled",
                    ty: TypeSchema::Bool,
                    comment: "Whether the backend has the Recall calendar path enabled.",
                    required: true,
                },
                FieldSchema {
                    name: "connected",
                    ty: TypeSchema::Bool,
                    comment: "Whether this user has a connected Google Calendar.",
                    required: true,
                },
                FieldSchema {
                    name: "email",
                    ty: TypeSchema::String,
                    comment: "Connected calendar email, when known.",
                    required: false,
                },
            ],
        },
        "disconnect" => ControllerSchema {
            namespace: "recall_calendar",
            function: "disconnect",
            description: "Disconnect the user's Google calendar from Recall.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "disconnected",
                ty: TypeSchema::Bool,
                comment: "True once the calendar has been disconnected.",
                required: true,
            }],
        },
        "list_meetings" => ControllerSchema {
            namespace: "recall_calendar",
            function: "list_meetings",
            description: "List upcoming meetings from the connected calendar (join URLs only).",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "meetings",
                ty: TypeSchema::Json,
                comment: "Array of {id, title, meetingUrl, startTime, endTime, platform, botId}.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "recall_calendar",
            function: "unknown",
            description: "Unknown recall_calendar controller function.",
            inputs: vec![FieldSchema {
                name: "function",
                ty: TypeSchema::String,
                comment: "Unknown function requested for schema lookup.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("connect"),
        schemas("status"),
        schemas("disconnect"),
        schemas("list_meetings"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("connect"),
            handler: handle_connect,
        },
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("disconnect"),
            handler: handle_disconnect,
        },
        RegisteredController {
            schema: schemas("list_meetings"),
            handler: handle_list_meetings,
        },
    ]
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn handle_connect(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::ops::connect(&config).await?)
    })
}

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::ops::status(&config).await?)
    })
}

fn handle_disconnect(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::ops::disconnect(&config).await?)
    })
}

fn handle_list_meetings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::ops::list_meetings(&config).await?)
    })
}
