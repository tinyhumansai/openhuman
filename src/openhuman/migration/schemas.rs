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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn all_controller_schemas_advertises_openclaw_only() {
        let names: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        assert_eq!(names, vec!["openclaw"]);
    }

    #[test]
    fn all_registered_controllers_has_one_handler() {
        let ctrl = all_registered_controllers();
        assert_eq!(ctrl.len(), 1);
        assert_eq!(ctrl[0].schema.function, "openclaw");
    }

    #[test]
    fn openclaw_schema_describes_optional_source_and_dry_run() {
        let s = schemas("openclaw");
        assert_eq!(s.namespace, "migrate");
        assert_eq!(s.function, "openclaw");
        let names: Vec<_> = s.inputs.iter().map(|f| f.name).collect();
        assert!(names.contains(&"source_workspace"));
        assert!(names.contains(&"dry_run"));
        for f in &s.inputs {
            assert!(!f.required, "input `{}` must be optional", f.name);
        }
        assert_eq!(s.outputs[0].name, "report");
    }

    #[test]
    fn unknown_function_returns_unknown_placeholder() {
        let s = schemas("bogus");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.namespace, "migrate");
        assert_eq!(s.outputs[0].name, "error");
    }

    #[test]
    fn migrate_openclaw_params_tolerates_empty_object() {
        let params: MigrateOpenClawParams = serde_json::from_value(json!({})).unwrap();
        assert!(params.source_workspace.is_none());
        assert!(params.dry_run.is_none());
    }

    #[test]
    fn migrate_openclaw_params_parses_both_fields() {
        let params: MigrateOpenClawParams = serde_json::from_value(json!({
            "source_workspace": "/tmp/old",
            "dry_run": false
        }))
        .unwrap();
        assert_eq!(params.source_workspace.as_deref(), Some("/tmp/old"));
        assert_eq!(params.dry_run, Some(false));
    }

    #[test]
    fn to_json_wraps_rpc_outcome_result_envelope() {
        let v = to_json(RpcOutcome::single_log(json!({"done": true}), "done")).unwrap();
        assert!(v.get("logs").is_some() || v.get("result").is_some());
    }
}
