//! Controllers `storage.migrate`, `storage.status` and `storage.restore`.
//!
//! Registered through `core/all.rs`; the CLI reaches them by the generic
//! namespace dispatch (`openhuman-core storage migrate --dry_run true`).

use serde_json::{Map, Value};

use super::{migrate, restore, status, Options, Target};
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::runtime::DispatchCtx;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

/// Schemas of the storage layout controllers.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("migrate"), schemas("status"), schemas("restore")]
}

/// The registered controllers.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("migrate"),
            handler: handle_migrate,
        },
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("restore"),
            handler: handle_restore,
        },
    ]
}

fn field(name: &'static str, ty: TypeSchema, comment: &'static str, required: bool) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required,
    }
}

/// The schema of `function`.
pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "migrate" => ControllerSchema {
            namespace: "storage",
            function: "migrate",
            description: "Move the active profile from storage layout v1 (workspace/ files) to layout v2 (one storage.db plus files/). \
                          Imports the legacy files, copies and verifies the database, moves user files, archives the originals to \
                          <root>/backups/<user>-<timestamp>.tar.zst, then deletes only what was archived. Resumable. \
                          dry_run returns the plan and writes nothing. Stop the core before running it for real.",
            inputs: vec![field("dry_run", TypeSchema::Option(Box::new(TypeSchema::Bool)), "Plan only: list what would be archived, moved, deleted and left alone, with byte totals.", false)],
            outputs: vec![field("result", TypeSchema::String, "noop, dry_run or migrated, with the plan or report alongside.", true)],
        },
        "status" => ControllerSchema {
            namespace: "storage",
            function: "status",
            description: "Layout version, schema version per owner, database size and last migration of the active profile. Read-only.",
            inputs: vec![],
            outputs: vec![field("layout_version", TypeSchema::U64, "1 or 2.", true)],
        },
        "restore" => ControllerSchema {
            namespace: "storage",
            function: "restore",
            description: "Put the active profile back to layout v1 from a migration archive: verifies the archive, restores its entries under workspace/, \
                          sets the v2 storage.db aside and removes the layout marker.",
            inputs: vec![field("archive", TypeSchema::String, "Path of the .tar.zst archive.", true)],
            outputs: vec![field("entries", TypeSchema::U64, "Top-level entries restored.", true)],
        },
        _ => ControllerSchema {
            namespace: "storage",
            function: "unknown",
            description: "Unknown storage controller function.",
            inputs: vec![field("function", TypeSchema::String, "Unknown function requested for schema lookup.", true)],
            outputs: vec![field("error", TypeSchema::String, "Lookup error details.", true)],
        },
    }
}

async fn target_of(ctx: &DispatchCtx) -> Result<(Target, crate::config::Config), String> {
    let config = ctx.load_config().await?;
    let target = Target::from_config(&config).ok_or_else(|| {
        "this config is not a users/<id> profile, so it has no storage layout to migrate".to_owned()
    })?;
    Ok((target, config))
}

fn env_url() -> Option<String> {
    std::env::var(crate::storage::STORAGE_URL_VAR).ok()
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

fn handle_migrate(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let dry_run = match params.get("dry_run") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(other) => return Err(format!("invalid 'dry_run': expected bool, got {other}")),
        };
        let (target, config) = target_of(&ctx).await?;
        let options = Options {
            dry_run,
            env_url: env_url(),
            ..Options::default()
        };
        let outcome = migrate(&target, &config, &options)
            .await
            .map_err(|e| e.to_string())?;
        to_value(&outcome)
    })
}

fn handle_status(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let (target, config) = target_of(&ctx).await?;
        let report = status(&target, &config, env_url())
            .await
            .map_err(|e| e.to_string())?;
        to_value(&report)
    })
}

fn handle_restore(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let archive = params
            .get("archive")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing required param 'archive'".to_owned())?;
        let (target, _config) = target_of(&ctx).await?;
        let report = restore(&target, std::path::Path::new(archive))
            .await
            .map_err(|e| e.to_string())?;
        to_value(&report)
    })
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
