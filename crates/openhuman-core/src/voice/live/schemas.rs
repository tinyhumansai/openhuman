//! `voice.live_*` controllers: provider catalogue, settings and provider tests.
//!
//! The session itself is not an RPC — it runs over the `/ws/live-voice`
//! WebSocket (`ws.rs`).

use crate::core::runtime::DispatchCtx;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::ops;
use super::types::LiveSettingsPatch;
use crate::config::Config;
use crate::core::all::{
    BoundRegisteredController, ControllerEntry, ControllerFuture, ControllerTask,
    RegisteredController,
};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn optional(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(ty)),
        comment,
        required: false,
    }
}

/// The live voice controller schema for `function` (the registry key).
pub fn live_schemas(function: &str) -> ControllerSchema {
    match function {
        "voice_live_providers" => ControllerSchema {
            namespace: "voice",
            function: "live_providers",
            description:
                "List live voice agent providers, whether each is usable, and the default.",
            inputs: vec![],
            outputs: vec![
                json_output("default_provider", "The provider Tiny uses by default."),
                json_output(
                    "providers",
                    "Every live provider with readiness, voices and languages.",
                ),
            ],
        },
        "voice_live_settings_get" => ControllerSchema {
            namespace: "voice",
            function: "live_settings_get",
            description: "Read the live voice agent settings.",
            inputs: vec![],
            outputs: vec![json_output(
                "settings",
                "Default provider and per-provider options.",
            )],
        },
        "voice_live_settings_set" => ControllerSchema {
            namespace: "voice",
            function: "live_settings_set",
            description: "Update the live voice agent settings; omitted fields are kept.",
            inputs: vec![
                optional(
                    "default_provider",
                    TypeSchema::String,
                    "Provider id to use by default.",
                ),
                optional(
                    "gemini",
                    TypeSchema::Json,
                    "Gemini options: model, voice, language.",
                ),
                optional(
                    "sarvam",
                    TypeSchema::Json,
                    "Sarvam options: language, speaker, model.",
                ),
                optional(
                    "elevenlabs",
                    TypeSchema::Json,
                    "ElevenLabs options: voice_id.",
                ),
            ],
            outputs: vec![json_output("settings", "The settings after the update.")],
        },
        "voice_live_test_provider" => ControllerSchema {
            namespace: "voice",
            function: "live_test_provider",
            description:
                "Open a live session on a provider, wait until it is ready, then close it.",
            inputs: vec![FieldSchema {
                name: "provider",
                ty: TypeSchema::String,
                comment: "Provider id to test.",
                required: true,
            }],
            outputs: vec![json_output("result", "ok, latency_ms and error.")],
        },
        _ => ControllerSchema {
            namespace: "voice",
            function: "unknown",
            description: "Unknown live voice controller function.",
            inputs: vec![],
            outputs: vec![json_output("error", "Lookup error details.")],
        },
    }
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| format!("serialize error: {e}"))
}

#[derive(Debug, Deserialize)]
struct TestParams {
    provider: String,
}

fn load_config(ctx: DispatchCtx) -> ControllerTask<Config> {
    Box::pin(async move { ctx.load_config().await })
}

fn ignored_params(_params: Map<String, Value>) -> Result<(), String> {
    Ok(())
}

fn live_providers_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(
        live_schemas("voice_live_providers"),
        load_config,
        ignored_params,
        run_live_providers,
    )
}

fn live_settings_get_controller() -> BoundRegisteredController {
    BoundRegisteredController::after_gate(
        live_schemas("voice_live_settings_get"),
        load_config,
        ignored_params,
        run_live_settings_get,
    )
}

fn live_settings_set_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(
        live_schemas("voice_live_settings_set"),
        parse_params::<LiveSettingsPatch>,
        run_live_settings_set,
    )
}

fn live_test_provider_controller() -> BoundRegisteredController {
    BoundRegisteredController::typed(
        live_schemas("voice_live_test_provider"),
        parse_params::<TestParams>,
        run_live_test_provider,
    )
}

fn run_live_providers(_ctx: DispatchCtx, config: Config, (): ()) -> ControllerFuture {
    Box::pin(async move { to_value(ops::live_providers(&config).await.value) })
}

fn run_live_settings_get(_ctx: DispatchCtx, config: Config, (): ()) -> ControllerFuture {
    Box::pin(async move { to_value(ops::live_settings_get(&config).value) })
}

fn run_live_settings_set(ctx: DispatchCtx, patch: LiveSettingsPatch) -> ControllerFuture {
    Box::pin(async move {
        let mut config = ctx.load_config().await?;
        to_value(ops::live_settings_set(&mut config, patch).await?.value)
    })
}

fn run_live_test_provider(ctx: DispatchCtx, params: TestParams) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_value(
            ops::live_test_provider(&config, &params.provider)
                .await
                .value,
        )
    })
}

pub(crate) fn bound_live_controller_entries() -> Vec<ControllerEntry> {
    vec![
        live_providers_controller(),
        live_settings_get_controller(),
        live_settings_set_controller(),
        live_test_provider_controller(),
    ]
    .into_iter()
    .map(ControllerEntry::bound)
    .collect()
}

fn handle_live_providers(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    live_providers_controller().invoke(ctx, params)
}

fn handle_live_settings_get(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    live_settings_get_controller().invoke(ctx, params)
}

fn handle_live_settings_set(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    live_settings_set_controller().invoke(ctx, params)
}

fn handle_live_test_provider(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    live_test_provider_controller().invoke(ctx, params)
}

/// Every live voice controller.
pub fn live_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: live_schemas("voice_live_providers"),
            handler: handle_live_providers,
        },
        RegisteredController {
            schema: live_schemas("voice_live_settings_get"),
            handler: handle_live_settings_get,
        },
        RegisteredController {
            schema: live_schemas("voice_live_settings_set"),
            handler: handle_live_settings_set,
        },
        RegisteredController {
            schema: live_schemas("voice_live_test_provider"),
            handler: handle_live_test_provider,
        },
    ]
}

/// Every live voice controller schema.
pub fn live_controller_schemas() -> Vec<ControllerSchema> {
    live_registered_controllers()
        .into_iter()
        .map(|c| c.schema)
        .collect()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;

fn parse_params<T: serde::de::DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

#[cfg(test)]
#[path = "schema_parameter_contracts.rs"]
mod parameter_contracts;
#[cfg(test)]
pub(crate) use parameter_contracts::contracts as parameter_contracts;
