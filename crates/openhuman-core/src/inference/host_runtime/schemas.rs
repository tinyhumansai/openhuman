use crate::core::runtime::DispatchCtx;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize)]
struct LocalAiTestConnectionParams {
    url: String,
}

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

#[derive(Debug, Deserialize)]
struct LocalAiTranscribeParams {
    audio_path: String,
}

#[derive(Debug, Deserialize)]
struct LocalAiTranscribeBytesParams {
    audio_bytes: Vec<u8>,
    extension: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalAiTtsParams {
    text: String,
    output_path: Option<String>,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("transcribe"),
        schemas("transcribe_bytes"),
        schemas("tts"),
        schemas("test_connection"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("transcribe"),
            handler: handle_local_ai_transcribe,
        },
        RegisteredController {
            schema: schemas("transcribe_bytes"),
            handler: handle_local_ai_transcribe_bytes,
        },
        RegisteredController {
            schema: schemas("tts"),
            handler: handle_local_ai_tts,
        },
        RegisteredController {
            schema: schemas("test_connection"),
            handler: handle_local_ai_test_connection,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "transcribe" => ControllerSchema {
            namespace: "inference",
            function: "transcribe",
            description: "Transcribe audio from file path.",
            inputs: vec![required_string("audio_path", "Input audio path.")],
            outputs: vec![json_output("speech", "Transcription payload.")],
        },
        "transcribe_bytes" => ControllerSchema {
            namespace: "inference",
            function: "transcribe_bytes",
            description: "Transcribe audio from raw bytes.",
            inputs: vec![
                FieldSchema {
                    name: "audio_bytes",
                    ty: TypeSchema::Bytes,
                    comment: "Raw audio bytes.",
                    required: true,
                },
                optional_string("extension", "Optional audio extension."),
            ],
            outputs: vec![json_output("speech", "Transcription payload.")],
        },
        "tts" => ControllerSchema {
            namespace: "inference",
            function: "tts",
            description: "Synthesize speech from text.",
            inputs: vec![
                required_string("text", "Input text."),
                optional_string("output_path", "Optional output path."),
            ],
            outputs: vec![json_output("tts", "TTS result payload.")],
        },
        "test_connection" => ControllerSchema {
            namespace: "inference",
            function: "test_connection",
            description: "Test connectivity to an Ollama server URL. Returns reachable status and model count.",
            inputs: vec![required_string("url", "Ollama server URL to test.")],
            outputs: vec![json_output("result", "Connection test result.")],
        },
        _ => ControllerSchema {
            namespace: "inference",
            function: "unknown",
            description: "Unknown local inference controller function.",
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

fn handle_local_ai_transcribe(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiTranscribeParams>(params)?;
        let config = ctx.load_config().await?;
        to_json(
            crate::inference::host_runtime::ops::local_ai_transcribe(&config, p.audio_path.trim())
                .await?,
        )
    })
}

fn handle_local_ai_transcribe_bytes(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiTranscribeBytesParams>(params)?;
        let config = ctx.load_config().await?;
        to_json(
            crate::inference::host_runtime::ops::local_ai_transcribe_bytes(
                &config,
                &p.audio_bytes,
                p.extension,
            )
            .await?,
        )
    })
}

fn handle_local_ai_tts(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiTtsParams>(params)?;
        let config = ctx.load_config().await?;
        to_json(
            crate::inference::host_runtime::ops::local_ai_tts(
                &config,
                &p.text,
                p.output_path.as_deref(),
            )
            .await?,
        )
    })
}

fn handle_local_ai_test_connection(
    _ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiTestConnectionParams>(params)?;
        let result = tinyinference_local::service::test_ollama_connection(&p.url).await?;
        serde_json::to_value(result).map_err(|e| format!("serialize test_connection result: {e}"))
    })
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

fn optional_f64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
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

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
