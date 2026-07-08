//! Controller schema + handler for `openhuman.session_import_run`.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

use super::ops::run_import;
use super::types::ImportOptions;

#[derive(Debug, Deserialize, Default)]
struct SessionImportRunParams {
    #[serde(default)]
    dry_run: bool,
    #[serde(default)]
    only: Option<String>,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    verbose: bool,
    /// Workspace override for tooling; defaults to the configured workspace.
    #[serde(default)]
    workspace: Option<String>,
}

pub fn all_session_import_controller_schemas() -> Vec<ControllerSchema> {
    vec![session_import_schemas("session_import_run")]
}

pub fn all_session_import_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: session_import_schemas("session_import_run"),
        handler: handle_session_import_run,
    }]
}

pub fn session_import_schemas(function: &str) -> ControllerSchema {
    match function {
        "session_import_run" => ControllerSchema {
            namespace: "session_import",
            function: "run",
            description:
                "One-time import of legacy session transcripts (session_raw JSONL, legacy \
                 Markdown) into TinyAgents store/journal records under \
                 {workspace}/tinyagents_store. Sources are never modified; re-runs are \
                 idempotent.",
            inputs: vec![
                optional_bool("dry_run", "Plan and report only; write nothing."),
                FieldSchema {
                    name: "only",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment:
                        "Glob over session stems to import a subset (skips the global marker).",
                    required: false,
                },
                optional_bool("force", "Re-import even when markers say the work is done."),
                optional_bool("verbose", "Per-item info logging."),
                FieldSchema {
                    name: "workspace",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Workspace directory override (defaults to the configured workspace).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "summary",
                ty: TypeSchema::Json,
                comment: "Import summary: counters, per-item reports, warnings.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "session_import",
            function: "unknown",
            description: "Unknown session_import controller.",
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

fn handle_session_import_run(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: SessionImportRunParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;

        let workspace = match payload.workspace {
            Some(dir) => std::path::PathBuf::from(dir),
            None => {
                let config = crate::openhuman::config::Config::load_or_init()
                    .await
                    .map_err(|e| format!("failed to load config: {e}"))?;
                config.workspace_dir
            }
        };
        let opts = ImportOptions {
            dry_run: payload.dry_run,
            only: payload.only,
            force: payload.force,
            verbose: payload.verbose,
        };

        tracing::info!(
            workspace = %workspace.display(),
            dry_run = opts.dry_run,
            "[session-import] rpc run"
        );
        let summary = run_import(&workspace, &opts)
            .await
            .map_err(|e| format!("session import failed: {e:#}"))?;
        let logs = summary.warnings.clone();
        RpcOutcome::new(summary, logs).into_cli_compatible_json()
    })
}

fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment,
        required: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_schema_inventory_is_stable() {
        let schemas = all_session_import_controller_schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].namespace, "session_import");
        assert_eq!(schemas[0].function, "run");

        let controllers = all_session_import_registered_controllers();
        assert_eq!(controllers.len(), 1);
        assert_eq!(controllers[0].schema.function, "run");
    }

    #[tokio::test]
    async fn handler_runs_dry_against_explicit_workspace() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut params = Map::new();
        params.insert("dry_run".into(), Value::Bool(true));
        params.insert(
            "workspace".into(),
            Value::String(tmp.path().to_string_lossy().to_string()),
        );

        let result = handle_session_import_run(params).await.expect("handler ok");
        assert_eq!(result["dry_run"], Value::Bool(true));
        assert_eq!(result["scanned"], 0);
    }

    #[tokio::test]
    async fn handler_rejects_invalid_params() {
        let mut params = Map::new();
        params.insert("only".into(), Value::Bool(true)); // wrong type
        let err = handle_session_import_run(params).await.unwrap_err();
        assert!(err.contains("invalid params"), "{err}");
    }
}
