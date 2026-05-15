//! Controller schemas and RPC handler dispatch for the voice domain.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

// ---------------------------------------------------------------------------
// Param structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TranscribeParams {
    audio_path: String,
    /// Optional conversation context for LLM post-processing.
    #[serde(default)]
    context: Option<String>,
    /// Skip LLM cleanup and return raw whisper output.
    #[serde(default)]
    skip_cleanup: bool,
}

#[derive(Debug, Deserialize)]
struct TranscribeBytesParams {
    audio_bytes: Vec<u8>,
    #[serde(default)]
    extension: Option<String>,
    /// Optional conversation context for LLM post-processing.
    #[serde(default)]
    context: Option<String>,
    /// Skip LLM cleanup and return raw whisper output.
    #[serde(default)]
    skip_cleanup: bool,
}

#[derive(Debug, Deserialize)]
struct TtsParams {
    text: String,
    #[serde(default)]
    output_path: Option<String>,
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

/// Factory-dispatched STT request. The caller can either pin a provider
/// explicitly (`"cloud"` / `"whisper"`) or let the controller resolve the
/// effective provider from `config.local_ai.stt_provider`. Keeps the
/// existing `voice_cloud_transcribe` RPC intact for back-compat — older
/// renderers still pin the cloud path directly.
#[derive(Debug, Deserialize)]
struct SttDispatchParams {
    audio_base64: String,
    /// Provider override; falls back to `config.local_ai.stt_provider`.
    #[serde(default)]
    provider: Option<String>,
    /// Model override (cloud branch ignores it).
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    language: Option<String>,
}

/// Factory-dispatched TTS request. Same provider-resolution rule as
/// [`SttDispatchParams`].
#[derive(Debug, Deserialize)]
struct TtsDispatchParams {
    text: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    voice: Option<String>,
}

/// Settings-panel update for the STT/TTS provider selectors. Both are
/// optional; omitted fields are left at their current value.
#[derive(Debug, Deserialize)]
struct SetProvidersParams {
    #[serde(default)]
    stt_provider: Option<String>,
    #[serde(default)]
    tts_provider: Option<String>,
    #[serde(default)]
    stt_model: Option<String>,
    #[serde(default)]
    tts_voice: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReplySynthesizeParams {
    text: String,
    #[serde(default)]
    voice_id: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    output_format: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OverlaySttState {
    RecordingStarted,
    TranscriptionDone,
    Cancelled,
    Error,
}

#[derive(Debug, Deserialize)]
struct OverlaySttNotifyParams {
    /// Voice state transition.
    state: OverlaySttState,
    /// Transcribed text (required when state is "transcription_done").
    #[serde(default)]
    text: Option<String>,
}

// ---------------------------------------------------------------------------
// Schema + registry exports
// ---------------------------------------------------------------------------

pub fn all_voice_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        voice_schemas("voice_status"),
        voice_schemas("voice_transcribe"),
        voice_schemas("voice_transcribe_bytes"),
        voice_schemas("voice_tts"),
        voice_schemas("voice_reply_synthesize"),
        voice_schemas("voice_cloud_transcribe"),
        voice_schemas("voice_stt_dispatch"),
        voice_schemas("voice_tts_dispatch"),
        voice_schemas("voice_set_providers"),
        voice_schemas("voice_server_start"),
        voice_schemas("voice_server_stop"),
        voice_schemas("voice_server_status"),
        voice_schemas("overlay_stt_notify"),
    ]
}

pub fn all_voice_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: voice_schemas("voice_status"),
            handler: handle_voice_status,
        },
        RegisteredController {
            schema: voice_schemas("voice_transcribe"),
            handler: handle_voice_transcribe,
        },
        RegisteredController {
            schema: voice_schemas("voice_transcribe_bytes"),
            handler: handle_voice_transcribe_bytes,
        },
        RegisteredController {
            schema: voice_schemas("voice_tts"),
            handler: handle_voice_tts,
        },
        RegisteredController {
            schema: voice_schemas("voice_reply_synthesize"),
            handler: handle_voice_reply_synthesize,
        },
        RegisteredController {
            schema: voice_schemas("voice_cloud_transcribe"),
            handler: handle_voice_cloud_transcribe,
        },
        RegisteredController {
            schema: voice_schemas("voice_stt_dispatch"),
            handler: handle_voice_stt_dispatch,
        },
        RegisteredController {
            schema: voice_schemas("voice_tts_dispatch"),
            handler: handle_voice_tts_dispatch,
        },
        RegisteredController {
            schema: voice_schemas("voice_set_providers"),
            handler: handle_voice_set_providers,
        },
        RegisteredController {
            schema: voice_schemas("voice_server_start"),
            handler: handle_voice_server_start,
        },
        RegisteredController {
            schema: voice_schemas("voice_server_stop"),
            handler: handle_voice_server_stop,
        },
        RegisteredController {
            schema: voice_schemas("voice_server_status"),
            handler: handle_voice_server_status,
        },
        RegisteredController {
            schema: voice_schemas("overlay_stt_notify"),
            handler: handle_overlay_stt_notify,
        },
    ]
}

pub fn voice_schemas(function: &str) -> ControllerSchema {
    match function {
        "voice_status" => ControllerSchema {
            namespace: "voice",
            function: "status",
            description: "Check availability of STT/TTS binaries and models.",
            inputs: vec![],
            outputs: vec![json_output("status", "Voice availability status.")],
        },
        "voice_transcribe" => ControllerSchema {
            namespace: "voice",
            function: "transcribe",
            description:
                "Transcribe audio from a file path using whisper.cpp, with optional LLM cleanup.",
            inputs: vec![
                required_string("audio_path", "Path to the audio file."),
                optional_string("context", "Conversation context for LLM post-processing."),
                optional_bool(
                    "skip_cleanup",
                    "Skip LLM cleanup, return raw whisper output.",
                ),
            ],
            outputs: vec![json_output(
                "speech",
                "Transcription result with text and raw_text.",
            )],
        },
        "voice_transcribe_bytes" => ControllerSchema {
            namespace: "voice",
            function: "transcribe_bytes",
            description:
                "Transcribe audio from raw bytes using whisper.cpp, with optional LLM cleanup.",
            inputs: vec![
                FieldSchema {
                    name: "audio_bytes",
                    ty: TypeSchema::Bytes,
                    comment: "Raw audio bytes.",
                    required: true,
                },
                optional_string("extension", "Audio file extension (default: webm)."),
                optional_string("context", "Conversation context for LLM post-processing."),
                optional_bool(
                    "skip_cleanup",
                    "Skip LLM cleanup, return raw whisper output.",
                ),
            ],
            outputs: vec![json_output(
                "speech",
                "Transcription result with text and raw_text.",
            )],
        },
        "voice_tts" => ControllerSchema {
            namespace: "voice",
            function: "tts",
            description: "Synthesize speech from text using piper.",
            inputs: vec![
                required_string("text", "Text to synthesize."),
                optional_string("output_path", "Optional output file path."),
            ],
            outputs: vec![json_output("tts", "TTS result with output path.")],
        },
        "voice_reply_synthesize" => ControllerSchema {
            namespace: "voice",
            function: "reply_synthesize",
            description:
                "Synthesize an agent reply via the hosted backend (ElevenLabs) and return \
                 base64 audio plus an Oculus-15 viseme alignment for mascot lip-sync.",
            inputs: vec![
                required_string("text", "Text to synthesize."),
                optional_string(
                    "voice_id",
                    "Override voice id (defaults to backend selection).",
                ),
                optional_string("model_id", "Override model id."),
                optional_string("output_format", "Override audio format (e.g. mp3_44100)."),
            ],
            outputs: vec![json_output(
                "reply",
                "ReplySpeechResult: { audio_base64, audio_mime, visemes, alignment? }.",
            )],
        },
        "voice_stt_dispatch" => ControllerSchema {
            namespace: "voice",
            function: "stt_dispatch",
            description:
                "Factory-dispatched speech-to-text. Routes to the cloud Whisper proxy or the \
                 local whisper.cpp binary based on `provider` (or `config.local_ai.stt_provider` \
                 when unspecified). Returns the same `{ text }` payload either way.",
            inputs: vec![
                required_string(
                    "audio_base64",
                    "Base64-encoded audio bytes (e.g. webm/opus from MediaRecorder).",
                ),
                optional_string(
                    "provider",
                    "Override provider: 'cloud' or 'whisper'. Defaults to config.local_ai.stt_provider.",
                ),
                optional_string("model", "Whisper model id (whisper branch only)."),
                optional_string("mime_type", "Audio MIME type (default: audio/webm)."),
                optional_string("file_name", "Filename hint (default: audio.webm)."),
                optional_string("language", "BCP-47 language hint, e.g. 'en'."),
            ],
            outputs: vec![json_output(
                "result",
                "SttResult: { text, provider }.",
            )],
        },
        "voice_tts_dispatch" => ControllerSchema {
            namespace: "voice",
            function: "tts_dispatch",
            description:
                "Factory-dispatched text-to-speech. Routes to the cloud ElevenLabs proxy \
                 (returns rich viseme alignment) or local Piper (returns audio + a synthetic \
                 viseme timeline) based on `provider` (or `config.local_ai.tts_provider`).",
            inputs: vec![
                required_string("text", "Text to synthesize."),
                optional_string(
                    "provider",
                    "Override provider: 'cloud' or 'piper'. Defaults to config.local_ai.tts_provider.",
                ),
                optional_string(
                    "voice",
                    "Voice id (provider-specific). Piper expects an id like 'en_US-lessac-medium'.",
                ),
            ],
            outputs: vec![json_output(
                "reply",
                "ReplySpeechResult: { audio_base64, audio_mime, visemes, alignment? }.",
            )],
        },
        "voice_set_providers" => ControllerSchema {
            namespace: "voice",
            function: "set_providers",
            description:
                "Persist the STT / TTS provider selection (and optional model/voice id) into \
                 `config.local_ai.{stt,tts}_provider` so subsequent voice_stt_dispatch / \
                 voice_tts_dispatch calls resolve without an explicit provider param.",
            inputs: vec![
                optional_string(
                    "stt_provider",
                    "STT provider id ('cloud' or 'whisper'). Omitted = unchanged.",
                ),
                optional_string(
                    "tts_provider",
                    "TTS provider id ('cloud' or 'piper'). Omitted = unchanged.",
                ),
                optional_string("stt_model", "Whisper model id (e.g. 'whisper-large-v3-turbo')."),
                optional_string("tts_voice", "Piper voice id (e.g. 'en_US-lessac-medium')."),
            ],
            outputs: vec![json_output(
                "providers",
                "Updated provider selectors: { stt_provider, tts_provider, stt_model_id, tts_voice_id }.",
            )],
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
        "voice_server_start" => ControllerSchema {
            namespace: "voice",
            function: "server_start",
            description:
                "Start the voice dictation server (hotkey → record → transcribe → insert text).",
            inputs: vec![
                optional_string("hotkey", "Hotkey combination (default: Fn)."),
                optional_string(
                    "activation_mode",
                    "Activation mode: tap or push (default: push).",
                ),
                optional_bool("skip_cleanup", "Skip LLM post-processing."),
            ],
            outputs: vec![json_output("status", "Voice server status after start.")],
        },
        "voice_server_stop" => ControllerSchema {
            namespace: "voice",
            function: "server_stop",
            description: "Stop the voice dictation server.",
            inputs: vec![],
            outputs: vec![json_output("status", "Voice server status after stop.")],
        },
        "voice_server_status" => ControllerSchema {
            namespace: "voice",
            function: "server_status",
            description: "Get the current voice dictation server status.",
            inputs: vec![],
            outputs: vec![json_output("status", "Current voice server status.")],
        },
        "overlay_stt_notify" => ControllerSchema {
            namespace: "voice",
            function: "overlay_stt_notify",
            description:
                "Notify the overlay of a voice/STT state change from the chat prompt button.",
            inputs: vec![
                required_string(
                    "state",
                    "State transition: recording_started, transcription_done, cancelled, error.",
                ),
                optional_string(
                    "text",
                    "Transcribed text (when state is transcription_done).",
                ),
            ],
            outputs: vec![json_output("result", "Notification acknowledgement.")],
        },
        _ => ControllerSchema {
            namespace: "voice",
            function: "unknown",
            description: "Unknown voice controller.",
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

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn handle_voice_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::voice::voice_status(&config).await?)
    })
}

fn handle_voice_transcribe(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<TranscribeParams>(params)?;
        to_json(
            crate::openhuman::voice::voice_transcribe(
                &config,
                &p.audio_path,
                p.context.as_deref(),
                p.skip_cleanup,
            )
            .await?,
        )
    })
}

fn handle_voice_transcribe_bytes(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<TranscribeBytesParams>(params)?;
        to_json(
            crate::openhuman::voice::voice_transcribe_bytes(
                &config,
                &p.audio_bytes,
                p.extension,
                p.context.as_deref(),
                p.skip_cleanup,
            )
            .await?,
        )
    })
}

fn handle_voice_tts(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<TtsParams>(params)?;
        to_json(
            crate::openhuman::voice::voice_tts(&config, &p.text, p.output_path.as_deref()).await?,
        )
    })
}

fn handle_voice_reply_synthesize(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<ReplySynthesizeParams>(params)?;
        // Dispatch through the TTS factory so the user's `tts_provider`
        // setting (cloud / piper / …) is honored on the spoken-reply path,
        // not just the dedicated `voice_tts_dispatch` RPC. Without this
        // routing, the settings dropdown was effectively decorative —
        // selecting "piper" persisted to config but conversation replies
        // still hit the cloud TTS proxy.
        let provider_name = effective_tts_provider(&config);
        // Only default to the Piper voice id when the active provider is
        // actually Piper. Passing a Piper voice id to a cloud TTS provider
        // would send an invalid voice to the upstream API.
        let voice = p
            .voice_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if provider_name == "piper" {
                    crate::openhuman::voice::DEFAULT_PIPER_VOICE.to_string()
                } else {
                    String::new()
                }
            });
        let effective_voice = if voice.is_empty() {
            None
        } else {
            Some(voice.as_str())
        };
        log::debug!(
            "[voice-factory] voice_reply_synthesize dispatch provider={provider_name} voice={voice}"
        );
        let provider =
            crate::openhuman::voice::create_tts_provider(&provider_name, &voice, &config)
                .map_err(|e| e.to_string())?;
        to_json(
            provider
                .synthesize(&config, &p.text, effective_voice)
                .await?,
        )
    })
}

fn handle_voice_cloud_transcribe(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<CloudTranscribeParams>(params)?;
        let opts = crate::openhuman::voice::cloud_transcribe::CloudTranscribeOptions {
            model: p.model,
            language: p.language,
            mime_type: p.mime_type,
            file_name: p.file_name,
        };
        to_json(
            crate::openhuman::voice::cloud_transcribe::transcribe_cloud(
                &config,
                &p.audio_base64,
                &opts,
            )
            .await?,
        )
    })
}

fn handle_voice_stt_dispatch(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<SttDispatchParams>(params)?;
        let provider_name = p
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| effective_stt_provider(&config));
        let model = p
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| crate::openhuman::voice::DEFAULT_WHISPER_MODEL.to_string());

        log::debug!(
            "[voice-factory] RPC voice_stt_dispatch provider={provider_name} model={model}"
        );
        let provider =
            crate::openhuman::voice::create_stt_provider(&provider_name, &model, &config)
                .map_err(|e| e.to_string())?;
        let outcome = provider
            .transcribe(
                &config,
                &p.audio_base64,
                p.mime_type.as_deref(),
                p.file_name.as_deref(),
                p.language.as_deref(),
            )
            .await?;
        let value = serde_json::json!({
            "text": outcome.value.text,
            "provider": outcome.value.provider,
        });
        Ok(value)
    })
}

fn handle_voice_tts_dispatch(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<TtsDispatchParams>(params)?;
        let provider_name = p
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| effective_tts_provider(&config));
        // Only fall back to the Piper default voice id when the provider is
        // Piper; sending a Piper voice id to a cloud TTS endpoint is invalid.
        let voice = p
            .voice
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if provider_name == "piper" {
                    crate::openhuman::voice::DEFAULT_PIPER_VOICE.to_string()
                } else {
                    String::new()
                }
            });
        let effective_voice = if voice.is_empty() {
            None
        } else {
            Some(voice.as_str())
        };

        log::debug!(
            "[voice-factory] RPC voice_tts_dispatch provider={provider_name} voice={voice}"
        );
        let provider =
            crate::openhuman::voice::create_tts_provider(&provider_name, &voice, &config)
                .map_err(|e| e.to_string())?;
        let outcome = provider
            .synthesize(&config, &p.text, effective_voice)
            .await?;
        to_json(outcome)
    })
}

fn handle_voice_set_providers(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<SetProvidersParams>(params)?;
        let mut config = config_rpc::load_config_with_timeout().await?;

        if let Some(stt) = p
            .stt_provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            validate_stt_provider(stt)?;
            config.local_ai.stt_provider = stt.to_string();
        }
        if let Some(tts) = p
            .tts_provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            validate_tts_provider(tts)?;
            config.local_ai.tts_provider = tts.to_string();
        }
        if let Some(model) = p
            .stt_model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            config.local_ai.stt_model_id = model.to_string();
        }
        if let Some(voice) = p
            .tts_voice
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            config.local_ai.tts_voice_id = voice.to_string();
        }

        config.save().await.map_err(|e| e.to_string())?;
        log::debug!(
            "[voice-factory] persisted providers stt={} tts={} stt_model={} tts_voice={}",
            config.local_ai.stt_provider,
            config.local_ai.tts_provider,
            config.local_ai.stt_model_id,
            config.local_ai.tts_voice_id
        );

        Ok(serde_json::json!({
            "stt_provider": config.local_ai.stt_provider,
            "tts_provider": config.local_ai.tts_provider,
            "stt_model_id": config.local_ai.stt_model_id,
            "tts_voice_id": config.local_ai.tts_voice_id,
        }))
    })
}

fn validate_stt_provider(provider: &str) -> Result<(), String> {
    match provider {
        "cloud" | "whisper" => Ok(()),
        other => Err(format!(
            "invalid stt_provider '{other}' (valid: 'cloud', 'whisper')"
        )),
    }
}

fn validate_tts_provider(provider: &str) -> Result<(), String> {
    match provider {
        "cloud" | "piper" => Ok(()),
        other => Err(format!(
            "invalid tts_provider '{other}' (valid: 'cloud', 'piper')"
        )),
    }
}

/// Read the user-selected STT provider from config. Defaults to `"cloud"`
/// for fresh installs — keeps the existing renderer behaviour unchanged
/// until the user opts into the local stack.
fn effective_stt_provider(config: &crate::openhuman::config::Config) -> String {
    let raw = config.local_ai.stt_provider.trim();
    if raw.is_empty() {
        "cloud".to_string()
    } else {
        raw.to_string()
    }
}

fn effective_tts_provider(config: &crate::openhuman::config::Config) -> String {
    let raw = config.local_ai.tts_provider.trim();
    if raw.is_empty() {
        "cloud".to_string()
    } else {
        raw.to_string()
    }
}

fn handle_voice_server_start(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        use crate::openhuman::voice::hotkey::ActivationMode;
        use crate::openhuman::voice::server::{global_server, VoiceServerConfig};

        let config = config_rpc::load_config_with_timeout().await?;

        let hotkey = params
            .get("hotkey")
            .and_then(|v| v.as_str())
            .unwrap_or(&config.voice_server.hotkey)
            .to_string();

        let activation_mode = match params.get("activation_mode").and_then(|v| v.as_str()) {
            Some("push") => ActivationMode::Push,
            Some("tap") => ActivationMode::Tap,
            Some(other) => {
                log::warn!(
                    "[voice_server] unrecognized activation_mode '{}', defaulting to Push",
                    other
                );
                ActivationMode::Push
            }
            None => match config.voice_server.activation_mode {
                crate::openhuman::config::VoiceActivationMode::Push => ActivationMode::Push,
                crate::openhuman::config::VoiceActivationMode::Tap => ActivationMode::Tap,
            },
        };

        let skip_cleanup = params
            .get("skip_cleanup")
            .and_then(|v| v.as_bool())
            .unwrap_or(config.voice_server.skip_cleanup);

        let server_config = VoiceServerConfig {
            hotkey,
            activation_mode,
            skip_cleanup,
            context: None,
            min_duration_secs: config.voice_server.min_duration_secs,
            silence_threshold: config.voice_server.silence_threshold,
            custom_dictionary: config.voice_server.custom_dictionary.clone(),
        };

        // Check if a server is already running with a different config.
        if let Some(existing) = crate::openhuman::voice::server::try_global_server() {
            let existing_status = existing.status().await;
            if existing_status.state != crate::openhuman::voice::server::ServerState::Stopped {
                if existing_status.hotkey != server_config.hotkey
                    || existing_status.activation_mode != server_config.activation_mode
                {
                    return Err(format!(
                        "voice server already running (hotkey={}, mode={:?}); \
                         stop it first before starting with different config",
                        existing_status.hotkey, existing_status.activation_mode
                    ));
                }
                // Same config, already running — return current status.
                return serde_json::to_value(existing_status)
                    .map_err(|e| format!("serialize error: {e}"));
            }
        }

        let server = global_server(server_config);
        let config_clone = config.clone();
        let server_for_err = server.clone();

        tokio::spawn(async move {
            if let Err(e) = server.run(&config_clone).await {
                log::error!("[voice_server] server exited with error: {e}");
                server_for_err.set_last_error(&e).await;
            }
        });

        // Give the server a moment to start.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        if let Some(s) = crate::openhuman::voice::server::try_global_server() {
            let status = s.status().await;
            serde_json::to_value(status).map_err(|e| format!("serialize error: {e}"))
        } else {
            Err("voice server failed to initialize".to_string())
        }
    })
}

fn handle_voice_server_stop(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        if let Some(server) = crate::openhuman::voice::server::try_global_server() {
            server.stop().await;
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let status = server.status().await;
            serde_json::to_value(status).map_err(|e| format!("serialize error: {e}"))
        } else {
            // Not running — return a stopped status rather than an error.
            let status = crate::openhuman::voice::server::VoiceServerStatus {
                state: crate::openhuman::voice::server::ServerState::Stopped,
                hotkey: String::new(),
                activation_mode: crate::openhuman::voice::hotkey::ActivationMode::Push,
                transcription_count: 0,
                last_error: None,
            };
            serde_json::to_value(status).map_err(|e| format!("serialize error: {e}"))
        }
    })
}

fn handle_voice_server_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        if let Some(server) = crate::openhuman::voice::server::try_global_server() {
            let status = server.status().await;
            serde_json::to_value(status).map_err(|e| format!("serialize error: {e}"))
        } else {
            let status = crate::openhuman::voice::server::VoiceServerStatus {
                state: crate::openhuman::voice::server::ServerState::Stopped,
                hotkey: String::new(),
                activation_mode: crate::openhuman::voice::hotkey::ActivationMode::Push,
                transcription_count: 0,
                last_error: None,
            };
            serde_json::to_value(status).map_err(|e| format!("serialize error: {e}"))
        }
    })
}

fn handle_overlay_stt_notify(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<OverlaySttNotifyParams>(params)?;
        log::debug!(
            "[overlay_stt_notify] state={:?}, has_text={}, text_len={}",
            p.state,
            p.text.is_some(),
            p.text.as_deref().map_or(0, |t| t.len())
        );

        use crate::openhuman::voice::dictation_listener::{
            publish_dictation_event, publish_transcription, DictationEvent,
        };

        match p.state {
            OverlaySttState::RecordingStarted => {
                publish_dictation_event(DictationEvent {
                    event_type: "pressed".to_string(),
                    hotkey: "chat_button".to_string(),
                    activation_mode: "toggle".to_string(),
                });
            }
            OverlaySttState::TranscriptionDone => {
                let text = p.text.ok_or_else(|| {
                    "invalid params: `text` is required for transcription_done".to_string()
                })?;
                publish_transcription(text);
                publish_dictation_event(DictationEvent {
                    event_type: "released".to_string(),
                    hotkey: "chat_button".to_string(),
                    activation_mode: "toggle".to_string(),
                });
            }
            OverlaySttState::Cancelled | OverlaySttState::Error => {
                publish_dictation_event(DictationEvent {
                    event_type: "released".to_string(),
                    hotkey: "chat_button".to_string(),
                    activation_mode: "toggle".to_string(),
                });
            }
        }

        Ok(serde_json::json!({ "ok": true }))
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    let json_val =
        serde_json::to_value(outcome.value).map_err(|e| format!("serialize error: {e}"))?;
    Ok(json_val)
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
