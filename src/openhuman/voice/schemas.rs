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
}

#[derive(Debug, Deserialize)]
struct TranscribeBytesParams {
    audio_bytes: Vec<u8>,
    #[serde(default)]
    extension: Option<String>,
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
            description: "Transcribe audio from a file path using whisper.cpp.",
            inputs: vec![required_string("audio_path", "Path to the audio file.")],
            outputs: vec![json_output("speech", "Transcription result.")],
        },
        "voice_transcribe_bytes" => ControllerSchema {
            namespace: "voice",
            function: "transcribe_bytes",
            description: "Transcribe audio from raw bytes using whisper.cpp.",
            inputs: vec![
                FieldSchema {
                    name: "audio_bytes",
                    ty: TypeSchema::Bytes,
                    comment: "Raw audio bytes.",
                    required: true,
                },
                optional_string("extension", "Audio file extension (default: webm)."),
            ],
            outputs: vec![json_output("speech", "Transcription result.")],
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
        to_json(crate::openhuman::voice::voice_transcribe(&config, &p.audio_path).await?)
    })
}

fn handle_voice_transcribe_bytes(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let p = deserialize_params::<TranscribeBytesParams>(params)?;
        to_json(
            crate::openhuman::voice::voice_transcribe_bytes(&config, &p.audio_bytes, p.extension)
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
