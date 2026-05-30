use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

fn vault_id_input(comment: &'static str) -> FieldSchema {
    FieldSchema {
        name: "vault_id",
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("create"),
        schemas("list"),
        schemas("get"),
        schemas("files"),
        schemas("write_markdown"),
        schemas("remove"),
        schemas("sync"),
        schemas("sync_status"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("create"),
            handler: handle_create,
        },
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("get"),
            handler: handle_get,
        },
        RegisteredController {
            schema: schemas("files"),
            handler: handle_files,
        },
        RegisteredController {
            schema: schemas("write_markdown"),
            handler: handle_write_markdown,
        },
        RegisteredController {
            schema: schemas("remove"),
            handler: handle_remove,
        },
        RegisteredController {
            schema: schemas("sync"),
            handler: handle_sync,
        },
        RegisteredController {
            schema: schemas("sync_status"),
            handler: handle_sync_status,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "create" => ControllerSchema {
            namespace: "vault",
            function: "create",
            description: "Register a new local folder as a knowledge vault.",
            inputs: vec![
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::String,
                    comment: "Display name for the vault.",
                    required: true,
                },
                FieldSchema {
                    name: "root_path",
                    ty: TypeSchema::String,
                    comment: "Absolute path to the folder on disk.",
                    required: true,
                },
                FieldSchema {
                    name: "include_globs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Optional include patterns (substring match).",
                    required: false,
                },
                FieldSchema {
                    name: "exclude_globs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Optional exclude patterns (substring match, case-insensitive).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "vault",
                ty: TypeSchema::Ref("Vault"),
                comment: "The newly created vault.",
                required: true,
            }],
        },
        "list" => ControllerSchema {
            namespace: "vault",
            function: "list",
            description: "List all registered vaults.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "vaults",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("Vault"))),
                comment: "All registered vaults.",
                required: true,
            }],
        },
        "get" => ControllerSchema {
            namespace: "vault",
            function: "get",
            description: "Fetch one vault by id.",
            inputs: vec![vault_id_input("Identifier of the vault to fetch.")],
            outputs: vec![FieldSchema {
                name: "vault",
                ty: TypeSchema::Ref("Vault"),
                comment: "The requested vault.",
                required: true,
            }],
        },
        "files" => ControllerSchema {
            namespace: "vault",
            function: "files",
            description: "List per-file ledger entries for a vault.",
            inputs: vec![vault_id_input("Identifier of the vault.")],
            outputs: vec![FieldSchema {
                name: "files",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("VaultFile"))),
                comment: "Per-file ledger rows.",
                required: true,
            }],
        },
        "write_markdown" => ControllerSchema {
            namespace: "vault",
            function: "write_markdown",
            description: "Write an explicitly approved markdown/wiki file inside a writable vault.",
            inputs: vec![
                vault_id_input("Identifier of the vault to write into."),
                FieldSchema {
                    name: "rel_path",
                    ty: TypeSchema::String,
                    comment: "Relative .md/.markdown path inside the vault.",
                    required: true,
                },
                FieldSchema {
                    name: "content",
                    ty: TypeSchema::String,
                    comment: "Markdown content to write.",
                    required: true,
                },
                FieldSchema {
                    name: "overwrite",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "When true, update an existing markdown file.",
                    required: false,
                },
                FieldSchema {
                    name: "approved",
                    ty: TypeSchema::Bool,
                    comment: "Must be true after explicit user approval for arbitrary vault writes.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "report",
                ty: TypeSchema::Ref("VaultWriteMarkdownReport"),
                comment: "Write result with relative path and byte count.",
                required: true,
            }],
        },
        "remove" => ControllerSchema {
            namespace: "vault",
            function: "remove",
            description: "Remove a vault. Optionally purge its memory namespace.",
            inputs: vec![
                vault_id_input("Identifier of the vault to remove."),
                FieldSchema {
                    name: "purge_memory",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "When true, also clear the vault's memory namespace.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "vault_id",
                            ty: TypeSchema::String,
                            comment: "Identifier requested for removal.",
                            required: true,
                        },
                        FieldSchema {
                            name: "removed",
                            ty: TypeSchema::Bool,
                            comment: "True when the vault row was deleted.",
                            required: true,
                        },
                        FieldSchema {
                            name: "purged",
                            ty: TypeSchema::Bool,
                            comment: "True when the memory namespace was also cleared.",
                            required: true,
                        },
                        FieldSchema {
                            name: "purge_error",
                            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                            comment: "Error detail when purge was requested but failed.",
                            required: false,
                        },
                    ],
                },
                comment: "Removal result payload.",
                required: true,
            }],
        },
        "sync" => ControllerSchema {
            namespace: "vault",
            function: "sync",
            description: "Start a background sync of a vault's root folder. Returns immediately. Poll `vault.sync_status` for progress.",
            inputs: vec![vault_id_input("Identifier of the vault to sync.")],
            outputs: vec![
                FieldSchema {
                    name: "status",
                    ty: TypeSchema::String,
                    comment: "Always `\"started\"` on success.",
                    required: true,
                },
                FieldSchema {
                    name: "vault_id",
                    ty: TypeSchema::String,
                    comment: "The vault that started syncing.",
                    required: true,
                },
            ],
        },
        "sync_status" => ControllerSchema {
            namespace: "vault",
            function: "sync_status",
            description: "Return the current sync progress and outcome for a vault.",
            inputs: vec![vault_id_input("Identifier of the vault to query.")],
            outputs: vec![FieldSchema {
                name: "state",
                ty: TypeSchema::Ref("VaultSyncState"),
                comment: "Current sync state (Idle / Running / Completed / Failed).",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "vault",
            function: "unknown",
            description: "Unknown vault controller function.",
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

fn handle_create(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let name = read_required::<String>(&params, "name")?;
        let root_path = read_required::<String>(&params, "root_path")?;
        let include_globs =
            read_optional::<Vec<String>>(&params, "include_globs")?.unwrap_or_default();
        let exclude_globs =
            read_optional::<Vec<String>>(&params, "exclude_globs")?.unwrap_or_default();
        to_json(
            crate::openhuman::vault::ops::vault_create(
                &config,
                &name,
                &root_path,
                include_globs,
                exclude_globs,
            )
            .await?,
        )
    })
}

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::vault::ops::vault_list(&config).await?)
    })
}

fn handle_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let vault_id = read_required::<String>(&params, "vault_id")?;
        to_json(crate::openhuman::vault::ops::vault_get(&config, vault_id.trim()).await?)
    })
}

fn handle_files(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let vault_id = read_required::<String>(&params, "vault_id")?;
        to_json(crate::openhuman::vault::ops::vault_files(&config, vault_id.trim()).await?)
    })
}

fn handle_write_markdown(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let vault_id = read_required::<String>(&params, "vault_id")?;
        let rel_path = read_required::<String>(&params, "rel_path")?;
        let content = read_required::<String>(&params, "content")?;
        let overwrite = read_optional::<bool>(&params, "overwrite")?.unwrap_or(false);
        let approved = read_required::<bool>(&params, "approved")?;
        to_json(
            crate::openhuman::vault::ops::vault_write_markdown(
                &config, &vault_id, &rel_path, &content, overwrite, approved,
            )
            .await?,
        )
    })
}

fn handle_remove(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let vault_id = read_required::<String>(&params, "vault_id")?;
        let purge_memory = read_optional::<bool>(&params, "purge_memory")?.unwrap_or(false);
        to_json(
            crate::openhuman::vault::ops::vault_remove(&config, vault_id.trim(), purge_memory)
                .await?,
        )
    })
}

fn handle_sync(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let vault_id = read_required::<String>(&params, "vault_id")?;
        to_json(crate::openhuman::vault::ops::vault_sync(&config, vault_id.trim()).await?)
    })
}

fn handle_sync_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let vault_id = read_required::<String>(&params, "vault_id")?;
        to_json(crate::openhuman::vault::ops::vault_sync_status(vault_id.trim()).await?)
    })
}

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

fn read_optional<T: DeserializeOwned>(
    params: &Map<String, Value>,
    key: &str,
) -> Result<Option<T>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|e| format!("invalid '{key}': {e}")),
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
