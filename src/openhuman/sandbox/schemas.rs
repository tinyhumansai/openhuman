//! Sandbox domain controller schemas and RPC handlers.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
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
            inputs: vec![FieldSchema {
                name: "backend",
                ty: TypeSchema::String,
                comment: "Backend kind to check: 'docker', 'local', or 'none'.",
                required: false,
            }],
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

fn handle_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let backend_str = params
            .get("backend")
            .and_then(|v| v.as_str())
            .unwrap_or("none");

        let mode = match backend_str {
            "docker" | "local" => {
                crate::openhuman::agent::harness::definition::SandboxMode::Sandboxed
            }
            _ => crate::openhuman::agent::harness::definition::SandboxMode::None,
        };

        let config = crate::openhuman::config::RuntimeConfig::default();
        let is_remote = backend_str == "docker";
        let policy = super::ops::resolve_sandbox_policy(
            mode,
            std::path::Path::new("/tmp"),
            &config,
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

        let config = crate::openhuman::config::RuntimeConfig::default();
        let action_dir = crate::openhuman::config::default_action_dir();
        let policy = super::ops::resolve_sandbox_policy(mode, &action_dir, &config, is_remote);
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
mod tests {
    use super::*;

    #[test]
    fn all_schemas_are_in_sandbox_namespace() {
        for schema in all_controller_schemas() {
            assert_eq!(schema.namespace, "sandbox");
        }
    }

    #[test]
    fn registered_controllers_match_schemas() {
        let schemas = all_controller_schemas();
        let controllers = all_registered_controllers();
        assert_eq!(schemas.len(), controllers.len());
        for (s, c) in schemas.iter().zip(controllers.iter()) {
            assert_eq!(s.function, c.schema.function);
        }
    }

    #[tokio::test]
    async fn handle_status_returns_json() {
        let result = handle_status(Map::new()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn handle_resolve_policy_none() {
        let mut params = Map::new();
        params.insert("sandbox_mode".into(), Value::String("none".into()));
        let result = handle_resolve_policy(params).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn handle_resolve_policy_sandboxed_remote() {
        let mut params = Map::new();
        params.insert("sandbox_mode".into(), Value::String("sandboxed".into()));
        params.insert("is_remote".into(), Value::Bool(true));
        let result = handle_resolve_policy(params).await;
        assert!(result.is_ok());
        let val = result.unwrap();
        let backend = val.get("backend").and_then(|b| b.as_str());
        assert_eq!(backend, Some("docker"));
    }

    #[tokio::test]
    async fn handle_validate_policy_valid() {
        let policy = super::super::types::SandboxPolicy {
            backend: super::super::types::SandboxBackendKind::Docker,
            workspace_root: std::path::PathBuf::from("/tmp/safe"),
            read_only_mounts: vec![],
            allow_network: false,
            env_passthrough: vec![],
            docker_overrides: None,
        };
        let mut params = Map::new();
        params.insert("policy".into(), serde_json::to_value(&policy).unwrap());
        let result = handle_validate_policy(params).await;
        assert!(result.is_ok());
        let val = result.unwrap();
        assert_eq!(val.get("valid").and_then(|v| v.as_bool()), Some(true));
    }

    #[tokio::test]
    async fn handle_validate_policy_dangerous() {
        let policy = super::super::types::SandboxPolicy {
            backend: super::super::types::SandboxBackendKind::Docker,
            workspace_root: std::path::PathBuf::from("/"),
            read_only_mounts: vec![],
            allow_network: false,
            env_passthrough: vec![],
            docker_overrides: Some(super::super::types::DockerOverrides {
                network: Some("host".into()),
                ..Default::default()
            }),
        };
        let mut params = Map::new();
        params.insert("policy".into(), serde_json::to_value(&policy).unwrap());
        let result = handle_validate_policy(params).await;
        assert!(result.is_ok());
        let val = result.unwrap();
        assert_eq!(val.get("valid").and_then(|v| v.as_bool()), Some(false));
    }
}
