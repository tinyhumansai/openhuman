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
                comment: "Structured keyring status: { available, activeMode, backendName, failureReason? }. activeMode says where secrets actually are and is derived from the backend, not from availability — os_keyring (working OS credential store) | local_encrypted (OS keyring failed, user consented to the local encrypted fallback) | local_encrypted_file (encrypted_file backend: {workspace}/secrets.enc) | local_plaintext_file (file/mock backend: plaintext dev-keychain.json) | consent_pending | declined. available reports whether the active backend is usable, which is true for the file backends.",
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
                comment: "Updated keyring status after re-probe, same shape as keyring_consent.status. Only the os backend can change here: the file backends always probe available, so their activeMode is unaffected by a re-probe.",
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
#[path = "schemas_tests.rs"]
mod tests;
