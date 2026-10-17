//! `voice.live_*` controllers: provider catalogue, settings and provider tests.
//!
//! The session itself is not an RPC — it runs over the `/ws/live-voice`
//! WebSocket (`ws.rs`).

use crate::core::runtime::DispatchCtx;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::ops;
use super::types::LiveSettingsPatch;
use crate::core::all::{ControllerFuture, RegisteredController};
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

fn handle_live_providers(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_value(ops::live_providers(&config).value)
    })
}

fn handle_live_settings_get(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_value(ops::live_settings_get(&config).value)
    })
}

fn handle_live_settings_set(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let patch: LiveSettingsPatch = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let mut config = ctx.load_config().await?;
        to_value(ops::live_settings_set(&mut config, patch).await?.value)
    })
}

fn handle_live_test_provider(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params: TestParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let config = ctx.load_config().await?;
        to_value(
            ops::live_test_provider(&config, &params.provider)
                .await
                .value,
        )
    })
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
