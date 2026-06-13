//! Schema definitions and controller registry for the voice domain.

use crate::core::all::RegisteredController;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::handlers::{
    handle_overlay_stt_notify, handle_voice_cloud_transcribe, handle_voice_list_models,
    handle_voice_reply_synthesize, handle_voice_server_start, handle_voice_server_status,
    handle_voice_server_stop, handle_voice_set_providers, handle_voice_status,
    handle_voice_stt_dispatch, handle_voice_test_provider, handle_voice_transcribe,
    handle_voice_transcribe_bytes, handle_voice_tts, handle_voice_tts_dispatch,
    handle_voice_update_provider_settings,
};
use super::helpers::{json_output, optional_bool, optional_string, required_string};

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
        voice_schemas("voice_update_provider_settings"),
        voice_schemas("voice_list_models"),
        voice_schemas("voice_test_provider"),
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
            schema: voice_schemas("voice_update_provider_settings"),
            handler: handle_voice_update_provider_settings,
        },
        RegisteredController {
            schema: voice_schemas("voice_list_models"),
            handler: handle_voice_list_models,
        },
        RegisteredController {
            schema: voice_schemas("voice_test_provider"),
            handler: handle_voice_test_provider,
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
        "voice_update_provider_settings" => ControllerSchema {
            namespace: "voice",
            function: "update_provider_settings",
            description:
                "Persist the voice provider registry and STT/TTS routing strings. \
                 Mirrors openhuman.inference_update_model_settings for the voice domain.",
            inputs: vec![
                FieldSchema {
                    name: "voice_providers",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Array of voice provider entries (VoiceProviderCreds shape).",
                    required: false,
                },
                optional_string(
                    "stt_provider",
                    "STT routing string ('cloud', 'whisper', or '<slug>:<model>').",
                ),
                optional_string(
                    "tts_provider",
                    "TTS routing string ('cloud', 'piper', or '<slug>:<voice>').",
                ),
            ],
            outputs: vec![json_output(
                "settings",
                "Updated voice_providers + routing strings snapshot.",
            )],
        },
        "voice_list_models" => ControllerSchema {
            namespace: "voice",
            function: "list_models",
            description:
                "List available models or voices for a voice provider. Returns static \
                 presets for built-in slugs; probes /models for custom providers.",
            inputs: vec![
                required_string("provider_id", "Provider id or slug."),
                optional_string(
                    "capability",
                    "Filter by capability: 'stt' or 'tts'. Defaults to both.",
                ),
            ],
            outputs: vec![json_output(
                "models",
                "{ models: [{ id, label? }] }",
            )],
        },
        "voice_test_provider" => ControllerSchema {
            namespace: "voice",
            function: "test_provider",
            description:
                "Test a voice provider endpoint without saving. STT transcribes a \
                 silent audio clip; TTS synthesizes 'Hello' and discards.",
            inputs: vec![
                required_string("workload", "Workload to test: 'stt' or 'tts'."),
                required_string(
                    "provider",
                    "Provider string to test (e.g. 'deepgram:nova-2').",
                ),
                optional_bool(
                    "validate_only",
                    "When true, only validate the API key without synthesizing/transcribing.",
                ),
            ],
            outputs: vec![json_output(
                "result",
                "{ ok: bool, detail: string, latency_ms?: number }",
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
