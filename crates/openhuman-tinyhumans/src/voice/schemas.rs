//! The hosted voice controllers: `voice.agent_signed_url` and
//! `voice.cloud_transcribe`.
//!
//! They share the `voice` namespace with the core's controllers (the registry
//! keys by `namespace.function`), keep their exact RPC names and wire shapes,
//! and register through a controller extension in the `Voice` domain group so
//! the ambient `DomainSet` gates them with the rest of voice.

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::core::all::{
    register_controller_extension, ControllerExtension, ControllerFuture, DomainGroup,
    RegisteredController,
};
use openhuman_embed::__host::core::runtime::DispatchCtx;
use openhuman_embed::__host::core::{ControllerSchema, FieldSchema, Outcome, TypeSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tinyinference_voice::cloud::CloudTranscribeOptions;

use super::cloud_transcribe::transcribe_cloud;
use super::realtime::mint_voice_agent_signed_url;

const LOG_PREFIX: &str = "[voice-hosted-rpc]";

fn field(name: &'static str, ty: TypeSchema, comment: &'static str, required: bool) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required,
    }
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    field(name, TypeSchema::String, comment, true)
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    field(
        name,
        TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        false,
    )
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    field(name, TypeSchema::Json, comment, true)
}

/// Schema for one hosted voice controller.
pub fn hosted_voice_schemas(function: &str) -> ControllerSchema {
    match function {
        "voice_agent_signed_url" => ControllerSchema {
            namespace: "voice",
            function: "agent_signed_url",
            description: "Mint a short-lived signed URL for a realtime voice-agent session.",
            inputs: vec![],
            outputs: vec![
                json_output(
                    "signed_url",
                    "Signed WebSocket URL for the voice-agent session.",
                ),
                json_output("agent_id", "The voice agent the URL was minted for."),
            ],
        },
        "voice_cloud_transcribe" => ControllerSchema {
            namespace: "voice",
            function: "cloud_transcribe",
            description:
                "Transcribe audio bytes via the hosted backend's STT endpoint. Used by the \
                 mascot's mic-only composer so we don't ship a provider API key in the desktop app.",
            inputs: vec![
                required_string(
                    "audio_base64",
                    "Base64-encoded audio bytes (e.g. webm/opus from MediaRecorder).",
                ),
                optional_string("mime_type", "Audio MIME type (default: audio/webm)."),
                optional_string("file_name", "Original filename hint (default: audio.webm)."),
                optional_string("model", "Backend STT model id (default: whisper-v1)."),
                optional_string("language", "BCP-47 language hint, e.g. 'en'."),
            ],
            outputs: vec![json_output("result", "CloudTranscribeResult: { text }.")],
        },
        _ => ControllerSchema {
            namespace: "voice",
            function: "unknown",
            description: "Unknown hosted voice controller.",
            inputs: vec![],
            outputs: vec![json_output("error", "Lookup error details.")],
        },
    }
}

/// The hosted voice controllers, built as a core domain builds its own.
pub fn hosted_voice_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: hosted_voice_schemas("voice_agent_signed_url"),
            handler: handle_voice_agent_signed_url,
        },
        RegisteredController {
            schema: hosted_voice_schemas("voice_cloud_transcribe"),
            handler: handle_voice_cloud_transcribe,
        },
    ]
}

#[derive(Debug, Deserialize)]
struct CloudTranscribeParams {
    audio_base64: String,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    language: Option<String>,
}

fn to_json<T: Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    serde_json::to_value(outcome.value).map_err(|e| format!("serialize error: {e}"))
}

async fn load_config(ctx: &DispatchCtx) -> Result<Config, String> {
    ctx.load_config().await
}

/// Mint a realtime voice-agent signed WebSocket URL (#5399). Takes no params -
/// the agent is server-configured; the desktop only asks for a fresh signed URL.
fn handle_voice_agent_signed_url(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("{LOG_PREFIX} voice.agent_signed_url");
        let config = load_config(&ctx).await?;
        to_json(mint_voice_agent_signed_url(&config).await?)
    })
}

fn handle_voice_cloud_transcribe(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("{LOG_PREFIX} voice.cloud_transcribe");
        let config = load_config(&ctx).await?;
        let p: CloudTranscribeParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let opts = CloudTranscribeOptions {
            model: p.model,
            language: p.language,
            mime_type: p.mime_type,
            file_name: p.file_name,
        };
        to_json(transcribe_cloud(&config, &p.audio_base64, &opts).await?)
    })
}

/// Registers the hosted voice controllers with the core. Idempotent.
pub(super) fn register() {
    let extension = ControllerExtension {
        group: DomainGroup::Voice,
        controllers: hosted_voice_controllers(),
        namespaces: &[],
    };
    if let Err(error) = register_controller_extension(extension) {
        log::warn!("{LOG_PREFIX} controller extension refused: {error}");
    }
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
