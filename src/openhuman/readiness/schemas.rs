//! JSON-RPC controllers for the first-run readiness aggregator (issue #4252).
//!
//! Namespace `readiness`:
//! - `readiness.check_all` — full aggregated report (all checks).
//! - `readiness.validate_model_connection` — re-probe just the model gate.
//! - `readiness.ollama_models` — local Ollama installed-model discovery.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("check_all"),
        schemas("validate_model_connection"),
        schemas("ollama_models"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("check_all"),
            handler: handle_check_all,
        },
        RegisteredController {
            schema: schemas("validate_model_connection"),
            handler: handle_validate_model_connection,
        },
        RegisteredController {
            schema: schemas("ollama_models"),
            handler: handle_ollama_models,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "check_all" => ControllerSchema {
            namespace: "readiness",
            function: "check_all",
            description: "Run all first-run readiness checks (core, storage, file access, desktop \
                 permissions, model connection, local Ollama) and return a plain-language report.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "report",
                ty: TypeSchema::Json,
                comment: "Aggregated readiness report with per-check status and remediation.",
                required: true,
            }],
        },
        "validate_model_connection" => ControllerSchema {
            namespace: "readiness",
            function: "validate_model_connection",
            description:
                "Validate that at least one configured model provider is reachable and usable. \
                 Gates onboarding completion.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "probe",
                ty: TypeSchema::Json,
                comment: "Model-connection probe result (ok, provider_id, error).",
                required: true,
            }],
        },
        "ollama_models" => ControllerSchema {
            namespace: "readiness",
            function: "ollama_models",
            description: "Detect a local Ollama server and discover installed models.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "probe",
                ty: TypeSchema::Json,
                comment: "Ollama probe result (reachable, models).",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "readiness",
            function: "unknown",
            description: "Unknown readiness controller function.",
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

fn handle_check_all(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::readiness::rpc::readiness_check_all(&config).await?)
    })
}

fn handle_validate_model_connection(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::readiness::rpc::validate_model_connection(&config).await?)
    })
}

fn handle_ollama_models(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(crate::openhuman::readiness::rpc::ollama_models().await?) })
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemas_returns_three() {
        assert_eq!(all_controller_schemas().len(), 3);
    }

    #[test]
    fn all_controllers_returns_three() {
        assert_eq!(all_registered_controllers().len(), 3);
    }

    #[test]
    fn check_all_schema() {
        let s = schemas("check_all");
        assert_eq!(s.namespace, "readiness");
        assert_eq!(s.function, "check_all");
        assert!(s.inputs.is_empty());
        assert!(!s.outputs.is_empty());
    }

    #[test]
    fn unknown_function_returns_unknown() {
        assert_eq!(schemas("bad").function, "unknown");
    }

    #[test]
    fn schemas_and_controllers_match() {
        let s = all_controller_schemas();
        let c = all_registered_controllers();
        assert_eq!(s.len(), c.len());
        for (schema, controller) in s.iter().zip(c.iter()) {
            assert_eq!(schema.function, controller.schema.function);
        }
    }
}
