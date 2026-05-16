use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

use super::ops;
use super::types::{CanvasTrackerSettings, LocalStatus};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("get_settings"),
        schemas("update_settings"),
        schemas("sync_now"),
        schemas("list_tasks"),
        schemas("update_local_status"),
        schemas("list_reminders"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("get_settings"),
            handler: handle_get_settings,
        },
        RegisteredController {
            schema: schemas("update_settings"),
            handler: handle_update_settings,
        },
        RegisteredController {
            schema: schemas("sync_now"),
            handler: handle_sync_now,
        },
        RegisteredController {
            schema: schemas("list_tasks"),
            handler: handle_list_tasks,
        },
        RegisteredController {
            schema: schemas("update_local_status"),
            handler: handle_update_local_status,
        },
        RegisteredController {
            schema: schemas("list_reminders"),
            handler: handle_list_reminders,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "get_settings" => ControllerSchema {
            namespace: "canvas_tracker",
            function: "get_settings",
            description: "Load Canvas assignment tracker settings without exposing the token.",
            inputs: vec![],
            outputs: vec![json_output(
                "settings",
                "Canvas tracker settings with token_set only.",
            )],
        },
        "update_settings" => ControllerSchema {
            namespace: "canvas_tracker",
            function: "update_settings",
            description: "Save Canvas tracker settings and optionally update the local token.",
            inputs: vec![
                required_json("settings", "Canvas tracker settings to persist."),
                optional_string("token", "Optional Canvas token to store locally."),
                FieldSchema {
                    name: "clear_token",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Whether to clear the stored Canvas token before saving.",
                    required: false,
                },
            ],
            outputs: vec![json_output(
                "settings",
                "Saved settings with token_set only.",
            )],
        },
        "sync_now" => ControllerSchema {
            namespace: "canvas_tracker",
            function: "sync_now",
            description: "Read allowed Canvas assignments and sync them into local SQLite.",
            inputs: vec![],
            outputs: vec![json_output("summary", "Sync summary.")],
        },
        "list_tasks" => ControllerSchema {
            namespace: "canvas_tracker",
            function: "list_tasks",
            description: "List locally synced Canvas assignment tracker tasks.",
            inputs: vec![],
            outputs: vec![json_output("tasks", "Canvas tracker tasks.")],
        },
        "update_local_status" => ControllerSchema {
            namespace: "canvas_tracker",
            function: "update_local_status",
            description: "Update local assignment status in SQLite only.",
            inputs: vec![
                required_string("course_id", "Canvas course id for the local task."),
                required_string("assignment_id", "Canvas assignment id for the local task."),
                FieldSchema {
                    name: "status",
                    ty: TypeSchema::Enum {
                        variants: vec![
                            "not_started",
                            "in_progress",
                            "waiting",
                            "submitted",
                            "done",
                            "unclear",
                        ],
                    },
                    comment: "New local status.",
                    required: true,
                },
            ],
            outputs: vec![json_output("result", "Update result.")],
        },
        "list_reminders" => ControllerSchema {
            namespace: "canvas_tracker",
            function: "list_reminders",
            description: "List reminder recommendations from locally synced tasks.",
            inputs: vec![],
            outputs: vec![json_output("reminders", "Reminder recommendations.")],
        },
        _ => ControllerSchema {
            namespace: "canvas_tracker",
            function: "unknown",
            description: "Unknown Canvas tracker controller function.",
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

fn handle_get_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(ops::get_settings(&config).await?)
    })
}

fn handle_update_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<UpdateSettingsParams>(params)?;
        to_json(
            ops::update_settings(&config, p.settings, p.token, p.clear_token.unwrap_or(false))
                .await?,
        )
    })
}

fn handle_sync_now(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(ops::sync_now(&config).await?)
    })
}

fn handle_list_tasks(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(ops::list_tasks(&config).await?)
    })
}

fn handle_update_local_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<UpdateLocalStatusParams>(params)?;
        to_json(ops::update_local_status(&config, &p.course_id, &p.assignment_id, p.status).await?)
    })
}

fn handle_list_reminders(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(ops::list_reminders(&config).await?)
    })
}

#[derive(Debug, Deserialize)]
struct UpdateSettingsParams {
    settings: CanvasTrackerSettings,
    token: Option<String>,
    clear_token: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct UpdateLocalStatusParams {
    course_id: String,
    assignment_id: String,
    status: LocalStatus,
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
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

fn required_json(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
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

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_tracker_schemas_have_expected_namespace() {
        let schemas = all_controller_schemas();
        let names: Vec<String> = schemas
            .iter()
            .map(|schema| format!("{}.{}", schema.namespace, schema.function))
            .collect();

        assert!(names.contains(&"canvas_tracker.get_settings".to_string()));
        assert!(names.contains(&"canvas_tracker.update_settings".to_string()));
        assert!(names.contains(&"canvas_tracker.sync_now".to_string()));
        assert!(names.contains(&"canvas_tracker.list_tasks".to_string()));
        assert!(names.contains(&"canvas_tracker.update_local_status".to_string()));
        assert!(names.contains(&"canvas_tracker.list_reminders".to_string()));
    }

    #[test]
    fn schema_and_handler_counts_match() {
        assert_eq!(
            all_controller_schemas().len(),
            all_registered_controllers().len()
        );
    }
}
