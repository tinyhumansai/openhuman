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

// ---------------------------------------------------------------------------
// Schema + registry exports
// ---------------------------------------------------------------------------

pub fn all_voice_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        voice_schemas("voice_status"),
        voice_schemas("voice_transcribe"),
        voice_schemas("voice_transcribe_bytes"),
        voice_schemas("voice_tts"),
        voice_schemas("voice_server_start"),
        voice_schemas("voice_server_stop"),
        voice_schemas("voice_server_status"),
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

        tokio::spawn(async move {
            if let Err(e) = server.run(&config_clone).await {
                log::error!("[voice_server] server exited with error: {e}");
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
mod tests {
    use super::*;

    #[test]
    fn schema_names_are_stable() {
        let s = voice_schemas("voice_status");
        assert_eq!(s.namespace, "voice");
        assert_eq!(s.function, "status");

        let s = voice_schemas("voice_transcribe");
        assert_eq!(s.namespace, "voice");
        assert_eq!(s.function, "transcribe");

        let s = voice_schemas("voice_transcribe_bytes");
        assert_eq!(s.namespace, "voice");
        assert_eq!(s.function, "transcribe_bytes");

        let s = voice_schemas("voice_tts");
        assert_eq!(s.namespace, "voice");
        assert_eq!(s.function, "tts");
    }

    #[test]
    fn controller_lists_match_lengths() {
        assert_eq!(
            all_voice_controller_schemas().len(),
            all_voice_registered_controllers().len()
        );
    }

    #[test]
    fn status_schema_has_no_inputs() {
        let s = voice_schemas("voice_status");
        assert!(s.inputs.is_empty());
    }

    #[test]
    fn transcribe_schema_requires_audio_path() {
        let s = voice_schemas("voice_transcribe");
        assert!(s
            .inputs
            .iter()
            .any(|i| i.name == "audio_path" && i.required));
    }

    #[test]
    fn transcribe_bytes_schema_requires_audio_bytes() {
        let s = voice_schemas("voice_transcribe_bytes");
        assert!(s
            .inputs
            .iter()
            .any(|i| i.name == "audio_bytes" && i.required));
    }

    #[test]
    fn transcribe_bytes_schema_has_optional_extension() {
        let s = voice_schemas("voice_transcribe_bytes");
        let ext = s.inputs.iter().find(|i| i.name == "extension").unwrap();
        assert!(!ext.required);
    }

    #[test]
    fn tts_schema_requires_text() {
        let s = voice_schemas("voice_tts");
        assert!(s.inputs.iter().any(|i| i.name == "text" && i.required));
    }

    #[test]
    fn tts_schema_has_optional_output_path() {
        let s = voice_schemas("voice_tts");
        let output_path = s.inputs.iter().find(|i| i.name == "output_path").unwrap();
        assert!(!output_path.required);
    }

    #[test]
    fn unknown_schema_returns_fallback() {
        let s = voice_schemas("voice_nonexistent");
        assert_eq!(s.function, "unknown");
    }
}
