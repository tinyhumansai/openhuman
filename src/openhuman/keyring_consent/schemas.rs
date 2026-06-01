//! Controller registration for keyring consent RPC methods.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub fn all_keyring_consent_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        keyring_consent_schema("status"),
        keyring_consent_schema("decide"),
        keyring_consent_schema("retry_probe"),
    ]
}

pub fn all_keyring_consent_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: keyring_consent_schema("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: keyring_consent_schema("decide"),
            handler: handle_decide,
        },
        RegisteredController {
            schema: keyring_consent_schema("retry_probe"),
            handler: handle_retry_probe,
        },
    ]
}

fn keyring_consent_schema(function: &str) -> ControllerSchema {
    match function {
        "status" => ControllerSchema {
            namespace: "keyring_consent",
            function: "status",
            description: "Returns the current keyring availability, failure reason, active storage mode, and backend name.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Structured keyring status.",
                required: true,
            }],
        },
        "decide" => ControllerSchema {
            namespace: "keyring_consent",
            function: "decide",
            description: "Record the user's consent decision for local secret storage fallback.",
            inputs: vec![FieldSchema {
                name: "mode",
                ty: TypeSchema::String,
                comment: "Either 'local_encrypted' (consent to local storage) or 'declined' (refuse local storage).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Persisted consent preference.",
                required: true,
            }],
        },
        "retry_probe" => ControllerSchema {
            namespace: "keyring_consent",
            function: "retry_probe",
            description: "Reset the cached keyring probe and re-test OS keyring availability.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Updated keyring status after re-probe.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "keyring_consent",
            function: "unknown",
            description: "Unknown keyring_consent controller.",
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

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        super::ops::keyring_status()
            .await?
            .into_cli_compatible_json()
    })
}

fn handle_decide(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let mode = params
            .get("mode")
            .and_then(|v| v.as_str())
            .ok_or("missing required param 'mode'")?
            .to_string();
        super::ops::keyring_consent_decide(mode)
            .await?
            .into_cli_compatible_json()
    })
}

fn handle_retry_probe(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        super::ops::keyring_retry_probe()
            .await?
            .into_cli_compatible_json()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_and_controllers_match() {
        let s = all_keyring_consent_controller_schemas();
        let c = all_keyring_consent_registered_controllers();
        assert_eq!(s.len(), c.len());
        for (schema, ctrl) in s.iter().zip(c.iter()) {
            assert_eq!(schema.function, ctrl.schema.function);
            assert_eq!(schema.namespace, ctrl.schema.namespace);
        }
    }

    #[test]
    fn all_schemas_use_keyring_consent_namespace() {
        for s in all_keyring_consent_controller_schemas() {
            assert_eq!(s.namespace, "keyring_consent");
            assert!(!s.description.is_empty());
        }
    }

    #[test]
    fn decide_schema_requires_mode() {
        let s = keyring_consent_schema("decide");
        assert_eq!(s.inputs.len(), 1);
        assert!(s.inputs[0].required);
        assert_eq!(s.inputs[0].name, "mode");
    }
}
