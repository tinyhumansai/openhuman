use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("version"),
        schemas("check"),
        schemas("apply"),
        schemas("run"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("version"),
            handler: handle_version,
        },
        RegisteredController {
            schema: schemas("check"),
            handler: handle_check,
        },
        RegisteredController {
            schema: schemas("apply"),
            handler: handle_apply,
        },
        RegisteredController {
            schema: schemas("run"),
            handler: handle_run,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "version" => ControllerSchema {
            namespace: "update",
            function: "version",
            description: "Report the running core binary's version + target triple.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "version_info",
                ty: TypeSchema::Json,
                comment: "Current version and platform target triple.",
                required: true,
            }],
        },
        "run" => ControllerSchema {
            namespace: "update",
            function: "run",
            description: "Orchestrated update: check GitHub, stage a newer binary if available, \
                 then publish a self-restart. The process exits shortly after returning.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "run_result",
                ty: TypeSchema::Json,
                comment: "Summary of what the orchestrator did (checked / applied / restarted).",
                required: true,
            }],
        },
        "check" => ControllerSchema {
            namespace: "update",
            function: "check",
            description: "Check GitHub Releases for a newer version of the core binary.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "update_info",
                ty: TypeSchema::Json,
                comment: "Version comparison and download URL for available update.",
                required: true,
            }],
        },
        "apply" => ControllerSchema {
            namespace: "update",
            function: "apply",
            description:
                "Download and stage an updated core binary. Requires a restart to take effect.",
            inputs: vec![
                FieldSchema {
                    name: "download_url",
                    ty: TypeSchema::String,
                    comment: "GitHub asset download URL.",
                    required: true,
                },
                FieldSchema {
                    name: "asset_name",
                    ty: TypeSchema::String,
                    comment: "Asset file name (e.g. openhuman-core-aarch64-apple-darwin).",
                    required: true,
                },
                FieldSchema {
                    name: "staging_dir",
                    ty: TypeSchema::String,
                    comment:
                        "Directory to stage the binary in. Defaults to the current exe directory.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "apply_result",
                ty: TypeSchema::Json,
                comment: "Staging result with installed version and path.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "update",
            function: "unknown",
            description: "Unknown update controller function.",
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

fn handle_version(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(crate::openhuman::update::rpc::update_version().await) })
}

fn handle_check(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(crate::openhuman::update::rpc::update_check().await) })
}

fn handle_run(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(crate::openhuman::update::rpc::update_run().await) })
}

fn handle_apply(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let download_url = params
            .get("download_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required param 'download_url'".to_string())?
            .to_string();
        let asset_name = params
            .get("asset_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required param 'asset_name'".to_string())?
            .to_string();
        let staging_dir = params
            .get("staging_dir")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        to_json(
            crate::openhuman::update::rpc::update_apply(download_url, asset_name, staging_dir)
                .await,
        )
    })
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemas_returns_four() {
        assert_eq!(all_controller_schemas().len(), 4);
    }

    #[test]
    fn all_controllers_returns_four() {
        assert_eq!(all_registered_controllers().len(), 4);
    }

    #[test]
    fn version_schema_has_no_inputs() {
        let s = schemas("version");
        assert_eq!(s.namespace, "update");
        assert_eq!(s.function, "version");
        assert!(s.inputs.is_empty());
        assert!(!s.outputs.is_empty());
    }

    #[test]
    fn run_schema_has_no_inputs() {
        let s = schemas("run");
        assert_eq!(s.namespace, "update");
        assert_eq!(s.function, "run");
        assert!(s.inputs.is_empty());
        assert!(!s.outputs.is_empty());
    }

    #[test]
    fn check_schema() {
        let s = schemas("check");
        assert_eq!(s.namespace, "update");
        assert_eq!(s.function, "check");
        assert!(s.inputs.is_empty());
        assert!(!s.outputs.is_empty());
    }

    #[test]
    fn apply_schema_requires_download_url_and_asset_name() {
        let s = schemas("apply");
        assert_eq!(s.function, "apply");
        let required: Vec<&str> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert!(required.contains(&"download_url"));
        assert!(required.contains(&"asset_name"));
    }

    #[test]
    fn apply_schema_has_optional_staging_dir() {
        let s = schemas("apply");
        let staging = s.inputs.iter().find(|f| f.name == "staging_dir");
        assert!(staging.is_some_and(|f| !f.required));
    }

    #[test]
    fn unknown_function_returns_unknown() {
        let s = schemas("nonexistent");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.namespace, "update");
    }

    #[test]
    fn schemas_and_controllers_match() {
        let s = all_controller_schemas();
        let c = all_registered_controllers();
        for (schema, ctrl) in s.iter().zip(c.iter()) {
            assert_eq!(schema.function, ctrl.schema.function);
        }
    }
}
