use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize)]
struct LocalAiTestConnectionParams {
    url: String,
}

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
struct AgentChatParams {
    message: String,
    model_override: Option<String>,
    temperature: Option<f64>,
    thread_id: Option<String>,
}

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

#[derive(Debug, Deserialize)]
struct LocalAiDownloadAssetParams {
    capability: String,
}

#[derive(Debug, Deserialize)]
struct LocalAiInstallWhisperParams {
    /// Optional model size (`tiny`, `base`, `small`, `medium`,
    /// `large-v3-turbo`). Defaults to `large-v3-turbo`.
    #[serde(default)]
    model_size: Option<String>,
    /// When true, blow away any existing model file and re-download.
    #[serde(default)]
    force: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct LocalAiInstallPiperParams {
    /// Optional Piper voice id (e.g. `en_US-lessac-medium`). Defaults to
    /// the bundled US-English Lessac voice.
    #[serde(default)]
    voice_id: Option<String>,
    /// When true, blow away any existing voice file and re-download.
    #[serde(default)]
    force: Option<bool>,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("agent_chat"),
        schemas("agent_chat_simple"),
        schemas("transcribe"),
        schemas("transcribe_bytes"),
        schemas("tts"),
        schemas("assets_status"),
        schemas("downloads_progress"),
        schemas("download_asset"),
        schemas("install_whisper"),
        schemas("install_piper"),
        schemas("whisper_install_status"),
        schemas("piper_install_status"),
        schemas("test_connection"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("agent_chat"),
            handler: handle_agent_chat,
        },
        RegisteredController {
            schema: schemas("agent_chat_simple"),
            handler: handle_agent_chat_simple,
        },
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
            schema: schemas("assets_status"),
            handler: handle_local_ai_assets_status,
        },
        RegisteredController {
            schema: schemas("downloads_progress"),
            handler: handle_local_ai_downloads_progress,
        },
        RegisteredController {
            schema: schemas("download_asset"),
            handler: handle_local_ai_download_asset,
        },
        RegisteredController {
            schema: schemas("install_whisper"),
            handler: handle_local_ai_install_whisper,
        },
        RegisteredController {
            schema: schemas("install_piper"),
            handler: handle_local_ai_install_piper,
        },
        RegisteredController {
            schema: schemas("whisper_install_status"),
            handler: handle_local_ai_whisper_install_status,
        },
        RegisteredController {
            schema: schemas("piper_install_status"),
            handler: handle_local_ai_piper_install_status,
        },
        RegisteredController {
            schema: schemas("test_connection"),
            handler: handle_local_ai_test_connection,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "agent_chat" => ControllerSchema {
            namespace: "inference",
            function: "agent_chat",
            description: "Run one-shot agent chat with optional model overrides.",
            inputs: vec![
                required_string("message", "User message."),
                optional_string("model_override", "Optional model override."),
                optional_f64("temperature", "Optional temperature override."),
                optional_string(
                    "thread_id",
                    "Optional backend thread id for cache grouping and inference logs.",
                ),
            ],
            outputs: vec![json_output("response", "Agent response payload.")],
        },
        "agent_chat_simple" => ControllerSchema {
            namespace: "inference",
            function: "agent_chat_simple",
            description: "Run one-shot lightweight provider chat.",
            inputs: vec![
                required_string("message", "User message."),
                optional_string("model_override", "Optional model override."),
                optional_f64("temperature", "Optional temperature override."),
                optional_string(
                    "thread_id",
                    "Optional backend thread id for cache grouping and inference logs.",
                ),
            ],
            outputs: vec![json_output("response", "Agent response payload.")],
        },
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
        "assets_status" => ControllerSchema {
            namespace: "inference",
            function: "assets_status",
            description: "Get local AI asset installation status.",
            inputs: vec![],
            outputs: vec![json_output("status", "Assets status payload.")],
        },
        "downloads_progress" => ControllerSchema {
            namespace: "inference",
            function: "downloads_progress",
            description: "Get local AI download progress.",
            inputs: vec![],
            outputs: vec![json_output("progress", "Download progress payload.")],
        },
        "download_asset" => ControllerSchema {
            namespace: "inference",
            function: "download_asset",
            description: "Trigger download for one local AI asset capability.",
            inputs: vec![required_string("capability", "Asset capability id.")],
            outputs: vec![json_output("status", "Assets status payload.")],
        },
        "install_whisper" => ControllerSchema {
            namespace: "inference",
            function: "install_whisper",
            description: "Download whisper.cpp's GGML model (and on Windows the whisper-cli binary) into the workspace so the local STT factory has everything it needs to run.",
            inputs: vec![
                optional_string(
                    "model_size",
                    "Whisper model size (tiny, base, small, medium, large-v3-turbo). Defaults to large-v3-turbo.",
                ),
                optional_bool(
                    "force",
                    "When true, re-download even if the workspace already has a matching model.",
                ),
            ],
            outputs: vec![json_output("status", "Whisper install status payload.")],
        },
        "install_piper" => ControllerSchema {
            namespace: "inference",
            function: "install_piper",
            description: "Download the Piper binary archive and the bundled en_US-lessac-medium voice files into the workspace.",
            inputs: vec![
                optional_string(
                    "voice_id",
                    "Piper voice id (e.g. en_US-lessac-medium). Defaults to en_US-lessac-medium.",
                ),
                optional_bool(
                    "force",
                    "When true, re-download even if the workspace already has the voice files.",
                ),
            ],
            outputs: vec![json_output("status", "Piper install status payload.")],
        },
        "whisper_install_status" => ControllerSchema {
            namespace: "inference",
            function: "whisper_install_status",
            description: "Query the Whisper install state (missing / installing / installed / broken / error) plus per-stage download progress.",
            inputs: vec![],
            outputs: vec![json_output("status", "Whisper install status payload.")],
        },
        "piper_install_status" => ControllerSchema {
            namespace: "inference",
            function: "piper_install_status",
            description: "Query the Piper install state (missing / installing / installed / broken / error) plus per-stage download progress.",
            inputs: vec![],
            outputs: vec![json_output("status", "Piper install status payload.")],
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

fn handle_agent_chat(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentChatParams>(params)?;
        let mut config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::local::ops::agent_chat(
                &mut config,
                &p.message,
                p.model_override,
                p.temperature,
                p.thread_id,
            )
            .await?,
        )
    })
}

fn handle_agent_chat_simple(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<AgentChatParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::local::ops::agent_chat_simple(
                &config,
                &p.message,
                p.model_override,
                p.temperature,
                p.thread_id,
            )
            .await?,
        )
    })
}

fn handle_local_ai_transcribe(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiTranscribeParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::local::ops::local_ai_transcribe(
                &config,
                p.audio_path.trim(),
            )
            .await?,
        )
    })
}

fn handle_local_ai_transcribe_bytes(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiTranscribeBytesParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::local::ops::local_ai_transcribe_bytes(
                &config,
                &p.audio_bytes,
                p.extension,
            )
            .await?,
        )
    })
}

fn handle_local_ai_tts(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiTtsParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::local::ops::local_ai_tts(
                &config,
                &p.text,
                p.output_path.as_deref(),
            )
            .await?,
        )
    })
}

fn handle_local_ai_assets_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::inference::local::ops::local_ai_assets_status(&config).await?)
    })
}

fn handle_local_ai_downloads_progress(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::local::ops::local_ai_downloads_progress(&config).await?,
        )
    })
}

fn handle_local_ai_download_asset(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiDownloadAssetParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(
            crate::openhuman::inference::local::ops::local_ai_download_asset(
                &config,
                p.capability.trim(),
            )
            .await?,
        )
    })
}

// The install RPCs are intentionally fire-and-forget: a binary+model
// download can take minutes (1.6 GB GGML model, ~5 MB Piper binary
// archive) but the core JSON-RPC client times out at
// VITE_CORE_RPC_TIMEOUT_MS (default 30s). Blocking the handler on the
// full download would force the UI into a retry loop that deletes the
// in-flight .part on each retry, looping forever.
//
// Shape: mark the engine as `installing(0%)` in the shared status table,
// spawn the real install on a background tokio task, return the
// just-written status immediately. The UI's status-polling RPC
// (handle_local_ai_*_install_status) reads from the same table and
// renders real-time progress. The eventual `installed` / `error`
// transition lands on the table when the background task finishes;
// no caller awaits it.

fn handle_local_ai_install_whisper(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiInstallWhisperParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        let force = p.force.unwrap_or(false);

        // Atomic install-start guard. A duplicate click while an install
        // is already in flight (or a parallel auto-install firing
        // alongside a manual click) must be a no-op — not a second
        // concurrent download racing on the same `.part` file inside
        // `download_to_file`. The previous read_status -> check ->
        // write_status sequence was non-atomic and let two callers slip
        // through; `try_acquire_install_slot` does the check-and-claim
        // under a single mutex acquisition.
        let slot =
            match crate::openhuman::inference::local::voice_install_common::try_acquire_install_slot(
                crate::openhuman::inference::local::voice_install_common::ENGINE_WHISPER,
            ) {
                Some(slot) => slot,
                None => {
                    tracing::debug!(
                        "[voice-install:whisper] slot already held — returning current status"
                    );
                    let current = crate::openhuman::inference::local::voice_install_common::read_status(
                    crate::openhuman::inference::local::voice_install_common::ENGINE_WHISPER,
                );
                    return serde_json::to_value(current)
                        .map_err(|e| format!("serialize whisper status: {e}"));
                }
            };

        // Mark "installing" before the spawn so the very next status poll
        // (≤ 2s away) reflects the new state without a stale read.
        crate::openhuman::inference::local::voice_install_common::write_status(
            crate::openhuman::inference::local::voice_install_common::VoiceInstallStatus {
                engine: crate::openhuman::inference::local::voice_install_common::ENGINE_WHISPER
                    .to_string(),
                state:
                    crate::openhuman::inference::local::voice_install_common::VoiceInstallState::Installing,
                progress: Some(0),
                downloaded_bytes: None,
                total_bytes: None,
                stage: Some("queued".to_string()),
                error_detail: None,
            },
        );

        tracing::debug!(
            model_size = ?p.model_size,
            force,
            "[voice-install:whisper] spawning background install"
        );
        let model_size = p.model_size.clone();
        // Move the slot into the spawned task so it lives for the actual
        // install duration (download + extract + validate), not just the
        // RPC handler's lifetime. The slot's Drop releases the
        // single-writer guard on task exit, including via panic.
        tokio::spawn(async move {
            let _slot = slot;
            if let Err(e) = crate::openhuman::inference::local::install_whisper::install_whisper(
                &config, model_size, force,
            )
            .await
            {
                log::warn!("[voice-install:whisper] background install failed: {e}");
            }
        });

        let status = crate::openhuman::inference::local::voice_install_common::read_status(
            crate::openhuman::inference::local::voice_install_common::ENGINE_WHISPER,
        );
        serde_json::to_value(status).map_err(|e| format!("serialize whisper status: {e}"))
    })
}

fn handle_local_ai_install_piper(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiInstallPiperParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        let force = p.force.unwrap_or(false);

        // See the whisper handler above for why this is an atomic slot
        // acquisition rather than a read_status / write_status pair.
        let slot =
            match crate::openhuman::inference::local::voice_install_common::try_acquire_install_slot(
                crate::openhuman::inference::local::voice_install_common::ENGINE_PIPER,
            ) {
                Some(slot) => slot,
                None => {
                    tracing::debug!(
                        "[voice-install:piper] slot already held — returning current status"
                    );
                    let current =
                        crate::openhuman::inference::local::voice_install_common::read_status(
                            crate::openhuman::inference::local::voice_install_common::ENGINE_PIPER,
                        );
                    return serde_json::to_value(current)
                        .map_err(|e| format!("serialize piper status: {e}"));
                }
            };

        crate::openhuman::inference::local::voice_install_common::write_status(
            crate::openhuman::inference::local::voice_install_common::VoiceInstallStatus {
                engine: crate::openhuman::inference::local::voice_install_common::ENGINE_PIPER.to_string(),
                state:
                    crate::openhuman::inference::local::voice_install_common::VoiceInstallState::Installing,
                progress: Some(0),
                downloaded_bytes: None,
                total_bytes: None,
                stage: Some("queued".to_string()),
                error_detail: None,
            },
        );

        tracing::debug!(
            voice_id = ?p.voice_id,
            force,
            "[voice-install:piper] spawning background install"
        );
        let voice_id = p.voice_id.clone();
        // Move the slot into the spawned task — same rationale as the
        // whisper handler.
        tokio::spawn(async move {
            let _slot = slot;
            if let Err(e) = crate::openhuman::inference::local::install_piper::install_piper(
                &config, voice_id, force,
            )
            .await
            {
                log::warn!("[voice-install:piper] background install failed: {e}");
            }
        });

        let status = crate::openhuman::inference::local::voice_install_common::read_status(
            crate::openhuman::inference::local::voice_install_common::ENGINE_PIPER,
        );
        serde_json::to_value(status).map_err(|e| format!("serialize piper status: {e}"))
    })
}

fn handle_local_ai_test_connection(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LocalAiTestConnectionParams>(params)?;
        let result =
            crate::openhuman::inference::local::service::ollama_admin::test_ollama_connection(
                &p.url,
            )
            .await?;
        serde_json::to_value(result).map_err(|e| format!("serialize test_connection result: {e}"))
    })
}

fn handle_local_ai_whisper_install_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let status = crate::openhuman::inference::local::install_whisper::status(&config);
        serde_json::to_value(status).map_err(|e| format!("serialize whisper status: {e}"))
    })
}

fn handle_local_ai_piper_install_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let status = crate::openhuman::inference::local::install_piper::status(&config);
        serde_json::to_value(status).map_err(|e| format!("serialize piper status: {e}"))
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

fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
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

fn optional_u64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
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

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
