use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("encrypt_secret"), schemas("decrypt_secret")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("encrypt_secret"),
            handler: handle_encrypt_secret,
        },
        RegisteredController {
            schema: schemas("decrypt_secret"),
            handler: handle_decrypt_secret,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "encrypt_secret" => ControllerSchema {
            namespace: "encrypt",
            function: "secret",
            description: "Encrypt a plaintext secret using local secret storage.",
            inputs: vec![FieldSchema {
                name: "plaintext",
                ty: TypeSchema::String,
                comment: "Plaintext value to encrypt.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "ciphertext",
                ty: TypeSchema::String,
                comment: "Encrypted secret payload.",
                required: true,
            }],
        },
        "decrypt_secret" => ControllerSchema {
            namespace: "decrypt",
            function: "secret",
            description: "Decrypt a previously encrypted secret payload.",
            inputs: vec![FieldSchema {
                name: "ciphertext",
                ty: TypeSchema::String,
                comment: "Encrypted secret payload to decrypt.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "plaintext",
                ty: TypeSchema::String,
                comment: "Decrypted plaintext secret.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "encryption",
            function: "unknown",
            description: "Unknown encryption controller function.",
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

fn handle_encrypt_secret(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let plaintext = read_required::<String>(&params, "plaintext")?;
        to_json(
            crate::openhuman::security::encryption::rpc::encrypt_secret(&config, &plaintext)
                .await?,
        )
    })
}

fn handle_decrypt_secret(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let ciphertext = read_required::<String>(&params, "ciphertext")?;
        to_json(
            crate::openhuman::security::encryption::rpc::decrypt_secret(&config, &ciphertext)
                .await?,
        )
    })
}

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
