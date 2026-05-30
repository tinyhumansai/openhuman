use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::workspace::rpc as workspace_rpc;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("file_read"),
        schemas("file_write"),
        schemas("file_reset"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("file_read"),
            handler: handle_file_read,
        },
        RegisteredController {
            schema: schemas("file_write"),
            handler: handle_file_write,
        },
        RegisteredController {
            schema: schemas("file_reset"),
            handler: handle_file_reset,
        },
    ]
}

fn filename_input() -> FieldSchema {
    FieldSchema {
        name: "filename",
        ty: TypeSchema::String,
        comment: "Editable persona file name; must be SOUL.md or IDENTITY.md.",
        required: true,
    }
}

fn workspace_file_outputs() -> Vec<FieldSchema> {
    vec![
        FieldSchema {
            name: "filename",
            ty: TypeSchema::String,
            comment: "The resolved persona file name.",
            required: true,
        },
        FieldSchema {
            name: "contents",
            ty: TypeSchema::String,
            comment: "Current effective contents of the file.",
            required: true,
        },
        FieldSchema {
            name: "is_default",
            ty: TypeSchema::Bool,
            comment: "True when the contents are the bundled default (file missing on read, or just reset).",
            required: true,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "file_read" => ControllerSchema {
            namespace: "workspace",
            function: "file_read",
            description: "Read an editable persona file (SOUL.md / IDENTITY.md), falling back to the bundled default when the workspace copy is missing.",
            inputs: vec![filename_input()],
            outputs: workspace_file_outputs(),
        },
        "file_write" => ControllerSchema {
            namespace: "workspace",
            function: "file_write",
            description: "Overwrite an editable persona file with new contents.",
            inputs: vec![
                filename_input(),
                FieldSchema {
                    name: "contents",
                    ty: TypeSchema::String,
                    comment: "New file contents (size-capped server-side).",
                    required: true,
                },
            ],
            outputs: workspace_file_outputs(),
        },
        "file_reset" => ControllerSchema {
            namespace: "workspace",
            function: "file_reset",
            description: "Restore an editable persona file to its bundled default.",
            inputs: vec![filename_input()],
            outputs: workspace_file_outputs(),
        },
        _other => ControllerSchema {
            namespace: "workspace",
            function: "unknown",
            description: "Unknown workspace controller function.",
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

fn handle_file_read(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let filename = read_required::<String>(&params, "filename")?;
        let filename = filename.trim();
        log::debug!("[workspace][rpc] handle_file_read filename='{filename}'");
        let result = workspace_rpc::read_workspace_file(&config.workspace_dir, filename);
        if let Err(ref e) = result {
            log::debug!("[workspace][rpc] handle_file_read error filename='{filename}': {e}");
        }
        to_json(result?)
    })
}

fn handle_file_write(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let filename = read_required::<String>(&params, "filename")?;
        let contents = read_required::<String>(&params, "contents")?;
        let filename = filename.trim();
        log::debug!(
            "[workspace][rpc] handle_file_write filename='{filename}' bytes={}",
            contents.len()
        );
        let result =
            workspace_rpc::write_workspace_file(&config.workspace_dir, filename, &contents);
        if let Err(ref e) = result {
            log::debug!("[workspace][rpc] handle_file_write error filename='{filename}': {e}");
        }
        to_json(result?)
    })
}

fn handle_file_reset(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let filename = read_required::<String>(&params, "filename")?;
        let filename = filename.trim();
        log::debug!("[workspace][rpc] handle_file_reset filename='{filename}'");
        let result = workspace_rpc::reset_workspace_file(&config.workspace_dir, filename);
        if let Err(ref e) = result {
            log::debug!("[workspace][rpc] handle_file_reset error filename='{filename}': {e}");
        }
        to_json(result?)
    })
}

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemas_use_workspace_namespace() {
        let all = all_controller_schemas();
        assert_eq!(all.len(), 3);
        for schema in &all {
            assert_eq!(schema.namespace, "workspace");
        }
    }

    #[test]
    fn registered_controllers_expose_expected_rpc_methods() {
        let methods: Vec<String> = all_registered_controllers()
            .iter()
            .map(|c| c.rpc_method_name())
            .collect();
        assert!(methods.contains(&"openhuman.workspace_file_read".to_string()));
        assert!(methods.contains(&"openhuman.workspace_file_write".to_string()));
        assert!(methods.contains(&"openhuman.workspace_file_reset".to_string()));
    }

    #[test]
    fn file_write_schema_requires_filename_and_contents() {
        let schema = schemas("file_write");
        let input_names: Vec<&str> = schema.inputs.iter().map(|f| f.name).collect();
        assert!(input_names.contains(&"filename"));
        assert!(input_names.contains(&"contents"));
    }

    #[test]
    fn unknown_function_yields_unknown_schema() {
        let schema = schemas("nope");
        assert_eq!(schema.function, "unknown");
    }
}
