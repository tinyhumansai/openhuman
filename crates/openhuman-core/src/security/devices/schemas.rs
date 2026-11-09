//! Controller schemas and registry for the devices domain.
//!
//! Follows the exact pattern from `cron/schemas.rs`.

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{
    BoundRegisteredController, ControllerEntry, ControllerFuture, ControllerTask,
    RegisteredController,
};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

// ---------------------------------------------------------------------------
// Public registry functions
// ---------------------------------------------------------------------------

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("create_pairing"),
        schemas("import_legacy"),
        schemas("list"),
        schemas("revoke"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("create_pairing"),
            handler: handle_create_pairing,
        },
        RegisteredController {
            schema: schemas("import_legacy"),
            handler: handle_import_legacy,
        },
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("revoke"),
            handler: handle_revoke,
        },
    ]
}

pub(crate) fn all_controller_entries() -> Vec<ControllerEntry> {
    vec![
        ControllerEntry::bound(create_pairing_controller()),
        ControllerEntry::bound(import_legacy_controller()),
        ControllerEntry::bound(list_controller()),
        ControllerEntry::bound(revoke_controller()),
    ]
}

#[cfg(test)]
pub(crate) fn devices_parameter_contracts() -> Vec<crate::core::all::ControllerParamContract> {
    tests::parameter_contracts()
}

// ---------------------------------------------------------------------------
// Schema definitions
// ---------------------------------------------------------------------------

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "create_pairing" => ControllerSchema {
            namespace: "devices",
            function: "create_pairing",
            description: "Register a new pairing channel with the backend tunnel. \
                          Returns the QR-code fields (channelId, pairingToken, corePubkey, \
                          rpcUrl?, expiresAt) needed by the iOS app to join the channel.",
            inputs: vec![FieldSchema {
                name: "label",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Human-readable device label, e.g. 'iPhone 15'.",
                required: false,
            }],
            outputs: vec![
                FieldSchema {
                    name: "channel_id",
                    ty: TypeSchema::String,
                    comment: "128-bit base32 channel identifier from the backend tunnel.",
                    required: true,
                },
                FieldSchema {
                    name: "pairing_token",
                    ty: TypeSchema::String,
                    comment:
                        "Base64url single-use pairing token (TTL'd, hashed at rest on backend).",
                    required: true,
                },
                FieldSchema {
                    name: "core_pubkey",
                    ty: TypeSchema::String,
                    comment: "Base64url X25519 public key of the core for E2E key agreement.",
                    required: true,
                },
                FieldSchema {
                    name: "rpc_url",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "LAN URL for direct HTTP fast path (omitted if not on LAN).",
                    required: false,
                },
                FieldSchema {
                    name: "expires_at",
                    ty: TypeSchema::String,
                    comment: "ISO 8601 expiry timestamp for the pairing token.",
                    required: true,
                },
            ],
        },

        "list" => ControllerSchema {
            namespace: "devices",
            function: "list",
            description: "List all non-revoked paired mobile devices.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "devices",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("PairedDevice"))),
                comment: "Paired devices ordered by creation time.",
                required: true,
            }],
        },

        "import_legacy" => ControllerSchema {
            namespace: "devices",
            function: "import_legacy",
            description: "Import available legacy device identities after direct user approval.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "imported",
                ty: TypeSchema::U64,
                comment: "Number of legacy devices recovered into TinySecurity.",
                required: true,
            }],
        },

        "revoke" => ControllerSchema {
            namespace: "devices",
            function: "revoke",
            description: "Revoke a paired device. Marks the device revoked in local storage \
                          and removes tunnel state. The backend channel expires naturally after \
                          the pairing token TTL.",
            inputs: vec![FieldSchema {
                name: "channel_id",
                ty: TypeSchema::String,
                comment: "channel_id of the device to revoke.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "success",
                ty: TypeSchema::Bool,
                comment: "True when the device was found and marked revoked.",
                required: true,
            }],
        },

        _other => ControllerSchema {
            namespace: "devices",
            function: "unknown",
            description: "Unknown devices controller function.",
            inputs: vec![FieldSchema {
                name: "function",
                ty: TypeSchema::String,
                comment: "Unknown function requested for schema lookup.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

// ---------------------------------------------------------------------------
// Handler bridges
// ---------------------------------------------------------------------------

fn handle_create_pairing(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    create_pairing_controller().invoke(ctx, params)
}

fn handle_list(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    list_controller().invoke(ctx, params)
}

fn handle_import_legacy(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    import_legacy_controller().invoke(ctx, params)
}

fn handle_revoke(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    revoke_controller().invoke(ctx, params)
}

fn create_pairing_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(
        schemas("create_pairing"),
        load_config,
        parse_create_pairing,
        run_create_pairing,
    )
}

fn list_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(schemas("list"), load_config, parse_list, run_list)
}

fn import_legacy_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(
        schemas("import_legacy"),
        load_config,
        parse_list,
        run_import_legacy,
    )
}

fn revoke_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(schemas("revoke"), load_config, parse_revoke, run_revoke)
}

fn load_config(ctx: DispatchCtx) -> ControllerTask<Config> {
    Box::pin(async move { ctx.load_config().await })
}

fn run_create_pairing(ctx: DispatchCtx, config: Config, label: Option<String>) -> ControllerFuture {
    Box::pin(async move {
        to_json(crate::security::devices::rpc::devices_create_pairing(&ctx, &config, label).await?)
    })
}

fn run_list(ctx: DispatchCtx, config: Config, (): ()) -> ControllerFuture {
    Box::pin(async move {
        if ctx.invocation_source()
            == crate::core::runtime::InvocationSource::AuthenticatedUserController
        {
            #[cfg(feature = "security-module")]
            {
                crate::modules::security::ensure_device_pairing_unlocked(&config).await?;
                crate::security::devices::rpc::resume_device_tunnels(&config).await;
            }
        }
        to_json(crate::security::devices::rpc::devices_list(&config).await?)
    })
}

fn run_import_legacy(ctx: DispatchCtx, config: Config, (): ()) -> ControllerFuture {
    Box::pin(async move {
        to_json(crate::security::devices::rpc::devices_import_legacy(&ctx, &config).await?)
    })
}

fn run_revoke(ctx: DispatchCtx, config: Config, channel_id: String) -> ControllerFuture {
    Box::pin(async move {
        to_json(crate::security::devices::rpc::devices_revoke(&ctx, &config, channel_id).await?)
    })
}

fn parse_create_pairing(params: Map<String, Value>) -> Result<Option<String>, String> {
    read_optional_string(&params, "label")
}

fn parse_list(_params: Map<String, Value>) -> Result<(), String> {
    Ok(())
}

fn parse_revoke(params: Map<String, Value>) -> Result<String, String> {
    read_required::<String>(&params, "channel_id")
}

// ---------------------------------------------------------------------------
// Param helpers (mirrors cron/schemas.rs helpers)
// ---------------------------------------------------------------------------

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

fn read_optional_string(params: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(format!(
            "invalid '{key}': expected string, got {}",
            type_name(other)
        )),
    }
}

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
