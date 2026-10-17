//! Controller schema + handler for `openhuman.session_import_run`: a manual
//! retry of the boot-time import of legacy session files into the backend.

use crate::core::runtime::DispatchCtx;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

#[derive(Debug, Deserialize, Default)]
struct SessionImportRunParams {
    #[serde(default)]
    dry_run: bool,
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
                "Import the workspace's legacy session files (session_db/sessions.db, \
                 session_raw transcripts, turn-state JSON) into the storage backend. Runs \
                 once per workspace (the boot-time import already did, so this is for \
                 retrying a failed one); sources are never modified and records already in \
                 the backend are never overwritten.",
            inputs: vec![
                optional_bool("dry_run", "Report which legacy sources exist and write nothing."),
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
                comment: "Import summary: whether it was a dry run, the legacy sources found, and per-store results.",
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

fn handle_session_import_run(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: SessionImportRunParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;

        let overridden = payload.workspace.is_some();
        let workspace = match payload.workspace {
            Some(dir) => std::path::PathBuf::from(dir),
            None => {
                let config = crate::config::Config::load_or_init()
                    .await
                    .map_err(|e| format!("failed to load config: {e}"))?;
                config.workspace_dir
            }
        };

        tracing::info!(
            workspace = %workspace.display(),
            dry_run = payload.dry_run,
            "[session-import] rpc run"
        );
        let sources = legacy_sources(&workspace);
        if payload.dry_run {
            return Outcome::new(
                json!({ "dry_run": true, "workspace": workspace, "sources": sources, "imports": [] }),
                Vec::new(),
            )
            .into_cli_compatible_json();
        }
        let configured = crate::config::Config::load_or_init()
            .await
            .map_err(|e| format!("failed to load config: {e}"))?;
        let configured_workspace = configured.workspace_dir.clone();
        let config = crate::config::Config {
            workspace_dir: workspace.clone(),
            ..configured
        };
        // A workspace override that differs from the configured one must not
        // write into the installed backend (bound to the configured workspace).
        let backend = if !overridden || workspace == configured_workspace {
            match crate::storage::installed() {
                Some(backend) => backend,
                None => crate::storage::local::backend_for(&workspace)
                    .map_err(|e| format!("open the storage backend: {e:#}"))?,
            }
        } else {
            crate::storage::open(&crate::storage::config::default_url(&workspace))
                .await
                .map_err(|e| format!("open the storage backend: {e:#}"))?
        };
        let reports = tokio::task::spawn_blocking(move || {
            crate::storage::import::sessions::run_with(&backend, &config)
        })
        .await
        .map_err(|e| format!("session import task failed: {e}"))?;
        let imports: Vec<Value> = reports
            .iter()
            .map(|report| match &report.outcome {
                Ok(outcome) => {
                    json!({ "domain": report.domain, "outcome": format!("{outcome:?}") })
                }
                Err(error) => json!({ "domain": report.domain, "error": format!("{error:#}") }),
            })
            .collect();
        let logs = reports
            .iter()
            .filter_map(|report| report.outcome.as_ref().err())
            .map(|error| format!("{error:#}"))
            .collect();
        Outcome::new(
            json!({ "dry_run": false, "workspace": workspace, "sources": sources, "imports": imports }),
            logs,
        )
        .into_cli_compatible_json()
    })
}

/// Which legacy session sources `workspace` still holds.
fn legacy_sources(workspace: &std::path::Path) -> Value {
    json!({
        "sessions_db": workspace.join("session_db").join("sessions.db").is_file(),
        "session_raw": workspace.join("session_raw").is_dir(),
        "turn_states": workspace.join("memory").join("conversations").join("turn_states").is_dir(),
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
#[path = "schemas_tests.rs"]
mod tests;
