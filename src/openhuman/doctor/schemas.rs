use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("report"), schemas("models")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("report"),
            handler: handle_report,
        },
        RegisteredController {
            schema: schemas("models"),
            handler: handle_models,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "report" => ControllerSchema {
            namespace: "doctor",
            function: "report",
            description: "Run diagnostics for workspace and runtime configuration.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "report",
                ty: TypeSchema::Ref("DoctorReport"),
                comment: "Aggregated diagnostics report.",
                required: true,
            }],
        },
        "models" => ControllerSchema {
            namespace: "doctor",
            function: "models",
            description: "Probe provider model availability and auth status.",
            inputs: vec![FieldSchema {
                name: "use_cache",
                ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                comment: "Reuse cached provider metadata when available.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "report",
                ty: TypeSchema::Ref("ModelProbeReport"),
                comment: "Model probe summary grouped by provider.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "doctor",
            function: "unknown",
            description: "Unknown doctor controller function.",
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

fn handle_report(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::doctor::rpc::doctor_report(&config).await?)
    })
}

fn handle_models(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let use_cache = read_optional::<bool>(&params, "use_cache")?.unwrap_or(true);
        to_json(crate::openhuman::doctor::rpc::doctor_models(&config, use_cache).await?)
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemas_returns_two() {
        assert_eq!(all_controller_schemas().len(), 2);
    }

    #[test]
    fn all_controllers_returns_two() {
        assert_eq!(all_registered_controllers().len(), 2);
    }

    #[test]
    fn report_schema() {
        let s = schemas("report");
        assert_eq!(s.namespace, "doctor");
        assert_eq!(s.function, "report");
        assert!(s.inputs.is_empty());
    }

    #[test]
    fn models_schema_has_optional_use_cache() {
        let s = schemas("models");
        assert_eq!(s.function, "models");
        let use_cache = s.inputs.iter().find(|f| f.name == "use_cache");
        assert!(use_cache.is_some_and(|f| !f.required));
    }

    #[test]
    fn unknown_function_returns_unknown() {
        let s = schemas("nonexistent");
        assert_eq!(s.function, "unknown");
    }

    #[test]
    fn schemas_and_controllers_match() {
        let s = all_controller_schemas();
        let c = all_registered_controllers();
        for (schema, ctrl) in s.iter().zip(c.iter()) {
            assert_eq!(schema.function, ctrl.schema.function);
        }
    }

    #[test]
    fn read_optional_returns_none_for_missing() {
        let m = Map::new();
        let result: Option<bool> = read_optional(&m, "use_cache").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn read_optional_returns_none_for_null() {
        let mut m = Map::new();
        m.insert("use_cache".into(), Value::Null);
        let result: Option<bool> = read_optional(&m, "use_cache").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn read_optional_returns_some_for_value() {
        let mut m = Map::new();
        m.insert("use_cache".into(), Value::Bool(true));
        let result: Option<bool> = read_optional(&m, "use_cache").unwrap();
        assert_eq!(result, Some(true));
    }

    #[test]
    fn read_optional_errors_on_wrong_type() {
        let mut m = Map::new();
        m.insert("use_cache".into(), Value::String("yes".into()));
        let err = read_optional::<bool>(&m, "use_cache").unwrap_err();
        assert!(err.contains("invalid"));
    }
}
