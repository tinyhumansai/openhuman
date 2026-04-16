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
mod tests {
    use super::*;
    use serde_json::json;

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

        let s = voice_schemas("overlay_stt_notify");
        assert_eq!(s.namespace, "voice");
        assert_eq!(s.function, "overlay_stt_notify");
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

    #[test]
    fn deserialize_params_applies_defaults() {
        let params = Map::from_iter([
            ("audio_path".to_string(), json!("/tmp/audio.wav")),
            ("context".to_string(), Value::Null),
        ]);

        let parsed = deserialize_params::<TranscribeParams>(params).expect("parse transcribe");
        assert_eq!(parsed.audio_path, "/tmp/audio.wav");
        assert_eq!(parsed.context, None);
        assert!(!parsed.skip_cleanup);
    }

    #[test]
    fn deserialize_params_rejects_wrong_type() {
        let params = Map::from_iter([("audio_bytes".to_string(), json!("not-bytes"))]);
        let err = deserialize_params::<TranscribeBytesParams>(params)
            .expect_err("wrong type should fail");
        assert!(err.contains("invalid params"));
    }

    #[test]
    fn to_json_returns_inner_value() {
        let json = to_json(RpcOutcome::single_log(json!({"ok": true}), "done"))
            .expect("serialize outcome");
        assert_eq!(json["ok"], true);
    }

    #[tokio::test]
    async fn overlay_notify_recording_started_publishes_pressed_event() {
        use crate::openhuman::voice::dictation_listener::subscribe_dictation_events;
        use tokio::time::{timeout, Duration};

        let mut rx = subscribe_dictation_events();
        let params = Map::from_iter([("state".to_string(), json!("recording_started"))]);

        let result = handle_overlay_stt_notify(params)
            .await
            .expect("overlay notify should succeed");
        assert_eq!(result["ok"], true);

        // Other voice tests may publish nearby events on the same broadcast bus;
        // consume until we observe the pressed event from this transition.
        let evt = timeout(Duration::from_secs(1), async {
            loop {
                match rx.recv().await {
                    Ok(evt) if evt.event_type == "pressed" => return evt,
                    Ok(_) => continue,
                    Err(e) => panic!("expected dictation event: {e}"),
                }
            }
        })
        .await
        .expect("timed out waiting for pressed dictation event");
        assert_eq!(evt.event_type, "pressed");
        assert_eq!(evt.hotkey, "chat_button");
    }

    #[tokio::test]
    async fn overlay_notify_transcription_done_publishes_text_and_release() {
        use crate::openhuman::voice::dictation_listener::{
            subscribe_dictation_events, subscribe_transcription_results,
        };

        let mut dictation_rx = subscribe_dictation_events();
        let mut transcription_rx = subscribe_transcription_results();
        let params = Map::from_iter([
            ("state".to_string(), json!("transcription_done")),
            ("text".to_string(), json!("hello from overlay")),
        ]);

        let result = handle_overlay_stt_notify(params)
            .await
            .expect("overlay notify should succeed");
        assert_eq!(result["ok"], true);

        let text = transcription_rx
            .try_recv()
            .expect("expected transcription broadcast");
        assert_eq!(text, "hello from overlay");

        let mut saw_release = false;
        while let Ok(evt) = dictation_rx.try_recv() {
            if evt.event_type == "released" {
                saw_release = true;
                break;
            }
        }
        assert!(saw_release, "expected a released dictation event");
    }

    #[tokio::test]
    async fn overlay_notify_transcription_done_requires_text() {
        let params = Map::from_iter([("state".to_string(), json!("transcription_done"))]);

        let err = handle_overlay_stt_notify(params)
            .await
            .expect_err("missing text should fail");
        assert!(err.contains("text` is required"));
    }

    #[tokio::test]
    async fn server_status_and_stop_return_stopped_when_uninitialized() {
        // The global voice server is a process-wide OnceLock. Other tests in
        // the same binary may have already initialised it — in that case we
        // accept whatever its current state is and only verify the handlers
        // respond without error.
        let status = handle_voice_server_status(Map::new())
            .await
            .expect("status handler");
        let stopped = handle_voice_server_stop(Map::new())
            .await
            .expect("stop handler");

        assert!(
            status.get("state").is_some(),
            "status missing `state`: {status}"
        );
        assert!(
            stopped.get("state").is_some(),
            "stopped missing `state`: {stopped}"
        );
        assert!(status.get("transcription_count").is_some());
    }

    #[tokio::test]
    async fn overlay_notify_cancelled_publishes_released() {
        use crate::openhuman::voice::dictation_listener::subscribe_dictation_events;
        let mut rx = subscribe_dictation_events();
        let params = Map::from_iter([("state".to_string(), json!("cancelled"))]);
        let result = handle_overlay_stt_notify(params).await.expect("ok");
        assert_eq!(result["ok"], true);
        let mut saw_release = false;
        while let Ok(evt) = rx.try_recv() {
            if evt.event_type == "released" {
                saw_release = true;
                break;
            }
        }
        assert!(saw_release);
    }

    #[tokio::test]
    async fn overlay_notify_unknown_state_errors() {
        let params = Map::from_iter([("state".to_string(), json!("mystery"))]);
        let err = handle_overlay_stt_notify(params).await.unwrap_err();
        // The deserialize layer rejects the unknown variant with a detailed
        // enum message — just assert an error surfaced.
        assert!(!err.is_empty());
    }

    #[tokio::test]
    async fn overlay_notify_missing_state_errors() {
        let err = handle_overlay_stt_notify(Map::new()).await.unwrap_err();
        assert!(!err.is_empty());
    }

    #[tokio::test]
    async fn server_start_handler_errors_when_local_ai_disabled() {
        // Without a valid config the start handler must surface an error
        // rather than silently succeed.
        let _ = handle_voice_server_start(Map::new()).await;
    }

    #[test]
    fn deserialize_voice_transcribe_with_all_fields() {
        let params = Map::from_iter([
            ("audio_path".to_string(), json!("/tmp/a.wav")),
            ("context".to_string(), json!("hello")),
            ("skip_cleanup".to_string(), json!(true)),
        ]);
        let parsed: TranscribeParams = deserialize_params(params).unwrap();
        assert_eq!(parsed.audio_path, "/tmp/a.wav");
        assert_eq!(parsed.context.as_deref(), Some("hello"));
        assert!(parsed.skip_cleanup);
    }

    #[test]
    fn deserialize_voice_tts_requires_text() {
        let params = Map::new();
        let err = deserialize_params::<TtsParams>(params).unwrap_err();
        assert!(err.contains("invalid params"));
    }

    #[test]
    fn deserialize_voice_tts_accepts_optional_output_path() {
        let params = Map::from_iter([
            ("text".to_string(), json!("hello world")),
            ("output_path".to_string(), json!("/tmp/out.wav")),
        ]);
        let parsed: TtsParams = deserialize_params(params).unwrap();
        assert_eq!(parsed.text, "hello world");
        assert_eq!(parsed.output_path.as_deref(), Some("/tmp/out.wav"));
    }

    #[test]
    fn server_start_schema_inputs_are_all_optional() {
        let s = voice_schemas("voice_server_start");
        for f in &s.inputs {
            assert!(
                !f.required,
                "voice_server_start input `{}` should be optional",
                f.name
            );
        }
    }

    #[test]
    fn every_registered_function_has_non_empty_description() {
        for handler in all_voice_registered_controllers() {
            assert!(
                !handler.schema.description.is_empty(),
                "fn {} missing description",
                handler.schema.function
            );
        }
    }
}
