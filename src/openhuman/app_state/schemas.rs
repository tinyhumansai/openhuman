use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::ops::StoredAppStatePatch;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateLocalStateParams {
    #[serde(default)]
    encryption_key: Option<Option<String>>,
    #[serde(default)]
    primary_wallet_address: Option<Option<String>>,
    #[serde(default)]
    onboarding_tasks: Option<Option<super::ops::StoredOnboardingTasks>>,
}

pub fn all_app_state_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        app_state_schemas("snapshot"),
        app_state_schemas("update_local_state"),
    ]
}

pub fn all_app_state_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: app_state_schemas("snapshot"),
            handler: handle_snapshot,
        },
        RegisteredController {
            schema: app_state_schemas("update_local_state"),
            handler: handle_update_local_state,
        },
    ]
}

pub fn app_state_schemas(function: &str) -> ControllerSchema {
    match function {
        "snapshot" => ControllerSchema {
            namespace: "app_state",
            function: "snapshot",
            description: "Fetch the core-owned app snapshot for the React shell.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Auth, current user, local app state, and compact runtime status for the React shell.",
                required: true,
            }],
        },
        "update_local_state" => ControllerSchema {
            namespace: "app_state",
            function: "update_local_state",
            description: "Update core-owned local app state persisted under the workspace.",
            inputs: vec![
                optional_json(
                    "encryptionKey",
                    "Set or clear the locally stored encryption key.",
                ),
                optional_json(
                    "primaryWalletAddress",
                    "Set or clear the locally stored wallet address.",
                ),
                optional_json(
                    "onboardingTasks",
                    "Set or clear locally stored onboarding task progress.",
                ),
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Updated locally persisted app state.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "app_state",
            function: "unknown",
            description: "Unknown app_state controller.",
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

fn handle_snapshot(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        crate::openhuman::app_state::snapshot()
            .await?
            .into_cli_compatible_json()
    })
}

fn handle_update_local_state(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: UpdateLocalStateParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        crate::openhuman::app_state::update_local_state(StoredAppStatePatch {
            encryption_key: payload.encryption_key,
            primary_wallet_address: payload.primary_wallet_address,
            onboarding_tasks: payload.onboarding_tasks,
        })
        .await?
        .into_cli_compatible_json()
    })
}

fn optional_json(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemas_returns_two() {
        assert_eq!(all_app_state_controller_schemas().len(), 2);
    }

    #[test]
    fn all_controllers_returns_two() {
        assert_eq!(all_app_state_registered_controllers().len(), 2);
    }

    #[test]
    fn snapshot_schema() {
        let s = app_state_schemas("snapshot");
        assert_eq!(s.namespace, "app_state");
        assert_eq!(s.function, "snapshot");
        assert!(s.inputs.is_empty());
        assert!(!s.outputs.is_empty());
    }

    #[test]
    fn update_local_state_schema() {
        let s = app_state_schemas("update_local_state");
        assert_eq!(s.namespace, "app_state");
        assert_eq!(s.function, "update_local_state");
        assert_eq!(s.inputs.len(), 3);
        for input in &s.inputs {
            assert!(!input.required, "input '{}' should be optional", input.name);
        }
    }

    #[test]
    fn unknown_function_returns_unknown() {
        let s = app_state_schemas("nonexistent");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.namespace, "app_state");
    }

    #[test]
    fn schemas_and_controllers_match() {
        let s = all_app_state_controller_schemas();
        let c = all_app_state_registered_controllers();
        assert_eq!(s.len(), c.len());
        for (schema, ctrl) in s.iter().zip(c.iter()) {
            assert_eq!(schema.function, ctrl.schema.function);
            assert_eq!(schema.namespace, ctrl.schema.namespace);
        }
    }

    #[test]
    fn all_schemas_use_app_state_namespace() {
        for s in all_app_state_controller_schemas() {
            assert_eq!(s.namespace, "app_state");
            assert!(!s.description.is_empty());
        }
    }

    #[test]
    fn optional_json_helper() {
        let f = optional_json("key", "desc");
        assert_eq!(f.name, "key");
        assert!(!f.required);
        assert!(matches!(f.ty, TypeSchema::Json));
    }

    #[test]
    fn deserialize_update_local_state_params_empty() {
        let params: UpdateLocalStateParams =
            serde_json::from_value(serde_json::Value::Object(Map::new())).unwrap();
        assert!(params.encryption_key.is_none());
        assert!(params.primary_wallet_address.is_none());
        assert!(params.onboarding_tasks.is_none());
    }

    #[test]
    fn deserialize_update_local_state_params_with_values() {
        let mut m = Map::new();
        // encryption_key is Option<Option<String>> — sending a string value sets Some(Some("..."))
        m.insert("encryptionKey".into(), serde_json::json!("my-key"));
        let params: UpdateLocalStateParams =
            serde_json::from_value(serde_json::Value::Object(m)).unwrap();
        assert!(params.encryption_key.is_some());
    }
}
