//! Sandbox domain controller schemas and RPC handlers.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("status"),
        schemas("resolve_policy"),
        schemas("cleanup_orphans"),
        schemas("validate_policy"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("resolve_policy"),
            handler: handle_resolve_policy,
        },
        RegisteredController {
            schema: schemas("cleanup_orphans"),
            handler: handle_cleanup_orphans,
        },
        RegisteredController {
            schema: schemas("validate_policy"),
            handler: handle_validate_policy,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "status" => ControllerSchema {
            namespace: "sandbox",
            function: "status",
            description: "Return sandbox backend status and availability.",
            inputs: vec![
                FieldSchema {
                    name: "backend",
                    ty: TypeSchema::String,
                    comment: "Sandbox MODE to resolve, not the backend that is \
                              reported: 'docker'/'local' both map to the sandboxed \
                              mode, 'none' to no sandbox. The ACTUAL backend is \
                              resolved from the loaded runtime config (`[runtime] \
                              kind`) plus `is_remote`, so it reflects what an agent \
                              session would really get — e.g. 'docker' on a \
                              native-config host reports the Local backend.",
                    required: false,
                },
                FieldSchema {
                    name: "is_remote",
                    ty: TypeSchema::Bool,
                    comment: "Whether this is a remote/channel session. When true, a \
                              sandboxed session resolves to the Docker backend even if \
                              the runtime is not configured for Docker. Defaults to false.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Json,
                comment: "Backend handle with kind, status, and backend_id.",
                required: true,
            }],
        },
        "resolve_policy" => ControllerSchema {
            namespace: "sandbox",
            function: "resolve_policy",
            description: "Resolve sandbox policy for a given sandbox mode and session context.",
            inputs: vec![
                FieldSchema {
                    name: "sandbox_mode",
                    ty: TypeSchema::String,
                    comment: "Agent sandbox mode: 'none', 'read_only', or 'sandboxed'.",
                    required: true,
                },
                FieldSchema {
                    name: "is_remote",
                    ty: TypeSchema::Bool,
                    comment: "Whether this is a remote/channel session.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "policy",
                ty: TypeSchema::Json,
                comment: "Resolved SandboxPolicy.",
                required: true,
            }],
        },
        "cleanup_orphans" => ControllerSchema {
            namespace: "sandbox",
            function: "cleanup_orphans",
            description: "Clean up orphaned sandbox containers.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "cleaned",
                ty: TypeSchema::U64,
                comment: "Number of orphaned containers cleaned up.",
                required: true,
            }],
        },
        "validate_policy" => ControllerSchema {
            namespace: "sandbox",
            function: "validate_policy",
            description: "Validate a sandbox policy for dangerous configurations.",
            inputs: vec![FieldSchema {
                name: "policy",
                ty: TypeSchema::Json,
                comment: "SandboxPolicy to validate.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "valid",
                    ty: TypeSchema::Bool,
                    comment: "Whether the policy is safe.",
                    required: true,
                },
                FieldSchema {
                    name: "issues",
                    ty: TypeSchema::Json,
                    comment: "List of security issues found (empty if valid).",
                    required: true,
                },
            ],
        },
        _ => ControllerSchema {
            namespace: "sandbox",
            function: "unknown",
            description: "Unknown sandbox controller function.",
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

/// `sandbox.status` — report the backend an agent session would actually get.
///
/// The `backend` param selects the sandbox MODE to resolve, not the backend to
/// report: `"docker"`/`"local"` both mean "resolve the sandboxed mode" and
/// `"none"` means "no sandbox". The concrete backend is then derived from the
/// loaded runtime config (`[runtime] kind`) plus `is_remote` — never from the
/// requested name — so `backend:"docker"` on a native-config host reports Local,
/// matching what a real session on that host would run. Reporting the requested
/// backend instead was the conflation fixed in #6081.
fn handle_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let backend_str = params
            .get("backend")
            .and_then(|v| v.as_str())
            .unwrap_or("none");
        // `is_remote` is an explicit caller-supplied flag, matching
        // `handle_resolve_policy`. It must NOT be inferred from `backend_str`:
        // naming the docker backend to check its availability is not the same
        // as running a remote/channel session, and conflating the two forced
        // Docker resolution for a local status probe (#6081).
        let is_remote = params
            .get("is_remote")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mode = match backend_str {
            "docker" | "local" => {
                crate::openhuman::agent::harness::definition::SandboxMode::Sandboxed
            }
            _ => crate::openhuman::agent::harness::definition::SandboxMode::None,
        };

        // Load the user's live config so the reported policy reflects the
        // configured runtime (`[runtime] kind` + `[runtime.docker]` overrides)
        // and the mounted workspace, not the compiled-in `RuntimeConfig::default()`
        // (#6081). A failed config load surfaces to the caller rather than
        // silently answering from defaults.
        let config = match config_rpc::load_config_with_timeout().await {
            Ok(config) => config,
            Err(err) => {
                log::warn!("[sandbox] handle_status config load failed error={err}");
                return Err(err);
            }
        };

        let policy = super::ops::resolve_sandbox_policy(
            mode,
            &config.workspace_dir,
            &config.runtime,
            is_remote,
        );
        let handle = super::ops::create_sandbox_backend(&policy).await;
        to_json(RpcOutcome::new(handle, vec![]))
    })
}

fn handle_resolve_policy(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mode_str = params
            .get("sandbox_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("none");
        let is_remote = params
            .get("is_remote")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mode = match mode_str {
            "sandboxed" => crate::openhuman::agent::harness::definition::SandboxMode::Sandboxed,
            "read_only" => crate::openhuman::agent::harness::definition::SandboxMode::ReadOnly,
            _ => crate::openhuman::agent::harness::definition::SandboxMode::None,
        };

        // Load the user's live config so the resolved policy honors the
        // configured runtime (`[runtime] kind` + `[runtime.docker]` overrides)
        // and the user's action dir, not the compiled-in `RuntimeConfig::default()`
        // and `default_action_dir()` (#6081). A failed config load surfaces to
        // the caller rather than silently answering from defaults.
        let config = match config_rpc::load_config_with_timeout().await {
            Ok(config) => config,
            Err(err) => {
                log::warn!("[sandbox] handle_resolve_policy config load failed error={err}");
                return Err(err);
            }
        };

        // Use the already-resolved `config.action_dir`. The loader fully
        // resolves this field on every path: `load_or_init` sets it from the
        // env `OPENHUMAN_ACTION_DIR` > persisted `action_dir_override` > default
        // precedence (`resolve_action_dir` + `apply_env_overrides`), and an
        // embedder-supplied config carries whatever `CoreBuilder::action_dir(..)`
        // set directly. Re-deriving it here from `action_dir_override` alone
        // would silently ignore an embedder's programmatic `action_dir` (#6081).
        let policy = super::ops::resolve_sandbox_policy(
            mode,
            &config.action_dir,
            &config.runtime,
            is_remote,
        );
        to_json(RpcOutcome::new(policy, vec![]))
    })
}

fn handle_cleanup_orphans(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        match super::docker::cleanup_orphaned_containers().await {
            Ok(count) => to_json(RpcOutcome::new(
                serde_json::json!({ "cleaned": count }),
                vec![],
            )),
            Err(e) => Err(format!("Cleanup failed: {e}")),
        }
    })
}

fn handle_validate_policy(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let policy_val = params.get("policy").cloned().unwrap_or(Value::Null);
        let policy: super::types::SandboxPolicy = match serde_json::from_value(policy_val) {
            Ok(p) => p,
            Err(e) => return Err(format!("Invalid policy: {e}")),
        };
        let result = match super::docker::validate_docker_policy(&policy) {
            Ok(()) => serde_json::json!({ "valid": true, "issues": [] }),
            Err(issues) => serde_json::json!({ "valid": false, "issues": issues }),
        };
        to_json(RpcOutcome::new(result, vec![]))
    })
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
