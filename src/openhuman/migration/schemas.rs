use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
struct MigrateOpenClawParams {
    source_workspace: Option<String>,
    dry_run: Option<bool>,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("openclaw")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schemas("openclaw"),
        handler: handle_migrate_openclaw,
    }]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "openclaw" => ControllerSchema {
            namespace: "migrate",
            function: "openclaw",
            description: "Migrate OpenClaw memory into current workspace.",
            inputs: vec![
                FieldSchema {
                    name: "source_workspace",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional source workspace path override.",
                    required: false,
                },
                FieldSchema {
                    name: "dry_run",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "When true, report migration plan only.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "report",
                ty: TypeSchema::Ref("MigrationReport"),
                comment: "Migration report and stats.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "migrate",
            function: "unknown",
            description: "Unknown migration controller function.",
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

fn handle_migrate_openclaw(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload: MigrateOpenClawParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let source = payload.source_workspace.map(std::path::PathBuf::from);
        to_json(
            crate::openhuman::migration::rpc::migrate_openclaw(
                &config,
                source,
                payload.dry_run.unwrap_or(true),
            )
            .await?,
        )
    })
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
