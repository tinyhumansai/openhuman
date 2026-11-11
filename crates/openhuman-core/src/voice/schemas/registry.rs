//! Schema definitions and controller registry for the voice domain.

use crate::core::all::{ControllerEntry, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::handlers::{
    handle_voice_list_models, handle_voice_reply_synthesize, handle_voice_set_providers,
    handle_voice_status, handle_voice_stt_dispatch, handle_voice_test_provider,
    handle_voice_transcribe, handle_voice_transcribe_bytes, handle_voice_tts,
    handle_voice_tts_dispatch, handle_voice_update_provider_settings, voice_list_models_controller,
    voice_reply_synthesize_controller, voice_set_providers_controller, voice_status_controller,
    voice_stt_dispatch_controller, voice_test_provider_controller,
    voice_transcribe_bytes_controller, voice_transcribe_controller, voice_tts_controller,
    voice_tts_dispatch_controller, voice_update_provider_settings_controller,
};
use super::helpers::{json_output, optional_bool, optional_string, required_string};

pub fn all_voice_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        voice_schemas("voice_status"),
        voice_schemas("voice_transcribe"),
        voice_schemas("voice_transcribe_bytes"),
        voice_schemas("voice_tts"),
        voice_schemas("voice_reply_synthesize"),
        voice_schemas("voice_stt_dispatch"),
        voice_schemas("voice_tts_dispatch"),
        voice_schemas("voice_set_providers"),
        voice_schemas("voice_update_provider_settings"),
        voice_schemas("voice_list_models"),
        voice_schemas("voice_test_provider"),
    ]
    .into_iter()
    .chain(crate::voice::live::live_controller_schemas())
    .collect()
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
    ]
    .into_iter()
    .chain(crate::voice::live::live_registered_controllers())
    .collect()
}

pub(crate) fn all_voice_controller_entries() -> Vec<ControllerEntry> {
    vec![
        voice_status_controller(),
        voice_transcribe_controller(),
        voice_transcribe_bytes_controller(),
        voice_tts_controller(),
        voice_reply_synthesize_controller(),
        voice_stt_dispatch_controller(),
        voice_tts_dispatch_controller(),
        voice_set_providers_controller(),
        voice_update_provider_settings_controller(),
        voice_list_models_controller(),
        voice_test_provider_controller(),
    ]
    .into_iter()
    .map(ControllerEntry::bound)
    .chain(crate::voice::live::schemas::bound_live_controller_entries())
    .collect()
}

pub fn voice_schemas(function: &str) -> ControllerSchema {
    match function {
        "voice_status" => ControllerSchema {
            namespace: "voice",
            function: "status",
            description: "Check whether the configured STT engine resolves and whether the local TTS binary/voice are installed.",
            inputs: vec![],
            outputs: vec![json_output("status", "Voice availability status.")],
        },
        "voice_transcribe" => ControllerSchema {
            namespace: "voice",
            function: "transcribe",
            description:
                "Transcribe audio from a file path through the configured STT engine, with optional LLM cleanup.",
            inputs: vec![
                required_string("audio_path", "Path to the audio file."),
                optional_string("context", "Conversation context for LLM post-processing."),
                optional_bool(
                    "skip_cleanup",
                    "Skip LLM cleanup, return the raw engine output.",
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
                "Transcribe audio from raw bytes through the configured STT engine, with optional LLM cleanup.",
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
                    "Skip LLM cleanup, return the raw engine output.",
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
                "Factory-dispatched speech-to-text. Routes to the OpenHuman backend proxy or a \
                 third-party provider based on `provider` (or, when unspecified, the resolved \
                 `voice_server.stt_engine`). Returns the same `{ text }` payload either way.",
            inputs: vec![
                required_string(
                    "audio_base64",
                    "Base64-encoded audio bytes (e.g. webm/opus from MediaRecorder).",
                ),
                optional_string(
                    "provider",
                    "Override provider: 'cloud' or '<slug>[:<model>]'. Defaults to the resolved voice_server.stt_engine.",
                ),
                optional_string("model", "Model id for the selected engine (e.g. 'scribe_v1', 'whisper-1')."),
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
                    "STT provider id ('cloud' or '<slug>[:<model>]'). Omitted = unchanged.",
                ),
                optional_string(
                    "tts_provider",
                    "TTS provider id ('cloud' or 'piper'). Omitted = unchanged.",
                ),
                optional_string("stt_model", "STT model id (e.g. 'whisper-1', 'scribe_v1')."),
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
                    "STT routing string ('cloud' or '<slug>:<model>').",
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
                 silent audio clip; TTS synthesizes 'Hello' and discards. With \
                 `validate_only` it is a dry run: a lightweight authenticated GET, \
                 no inference call and nothing persisted.",
            inputs: vec![
                required_string("workload", "Workload to test: 'stt' or 'tts'."),
                required_string(
                    "provider",
                    "Provider string to test (e.g. 'deepgram:nova-2').",
                ),
                optional_bool(
                    "validate_only",
                    "When true, only validate the API key without synthesizing/transcribing. \
                     Honoured for both workloads.",
                ),
                optional_string(
                    "api_key",
                    "Candidate API key to validate without storing it, so a key can be \
                     checked before the user commits to saving it. Omit to use the stored \
                     credential. Only read when `validate_only` is true.",
                ),
            ],
            outputs: vec![json_output(
                "result",
                "{ ok: bool, detail: string, latency_ms?: number }",
            )],
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
