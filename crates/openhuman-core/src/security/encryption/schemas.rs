use crate::core::runtime::DispatchCtx;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::config::Config;
use crate::core::all::{
    BoundRegisteredController, ControllerEntry, ControllerFuture, ControllerTask,
    RegisteredController,
};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

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

pub(crate) fn all_controller_entries() -> Vec<ControllerEntry> {
    vec![
        ControllerEntry::bound(encrypt_secret_controller()),
        ControllerEntry::bound(decrypt_secret_controller()),
    ]
}

#[cfg(test)]
pub(crate) fn encryption_parameter_contracts() -> Vec<crate::core::all::ControllerParamContract> {
    tests::parameter_contracts()
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

fn encrypt_secret_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(
        schemas("encrypt_secret"),
        load_config,
        parse_encrypt_secret,
        run_encrypt_secret,
    )
}

fn decrypt_secret_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(
        schemas("decrypt_secret"),
        load_config,
        parse_decrypt_secret,
        run_decrypt_secret,
    )
}

fn handle_encrypt_secret(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    encrypt_secret_controller().invoke(ctx, params)
}

fn handle_decrypt_secret(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    decrypt_secret_controller().invoke(ctx, params)
}

fn load_config(ctx: DispatchCtx) -> ControllerTask<Config> {
    Box::pin(async move { ctx.load_config().await })
}

fn parse_encrypt_secret(params: Map<String, Value>) -> Result<String, String> {
    read_required(&params, "plaintext")
}

fn parse_decrypt_secret(params: Map<String, Value>) -> Result<String, String> {
    read_required(&params, "ciphertext")
}

fn run_encrypt_secret(_ctx: DispatchCtx, config: Config, plaintext: String) -> ControllerFuture {
    Box::pin(async move {
        to_json(crate::security::encryption::rpc::encrypt_secret(&config, &plaintext).await?)
    })
}

fn run_decrypt_secret(_ctx: DispatchCtx, config: Config, ciphertext: String) -> ControllerFuture {
    Box::pin(async move {
        to_json(crate::security::encryption::rpc::decrypt_secret(&config, &ciphertext).await?)
    })
}

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
