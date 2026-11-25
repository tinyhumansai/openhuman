use crate::core::all::ControllerParamContract;
use serde_json::json;

/// Independent parameter fixtures for the production-bound voice entries.
/// Domain-specific defaults, values and errors remain in schemas_tests.rs.
pub(crate) fn contracts() -> Vec<ControllerParamContract> {
    let mut contracts = vec![
        ControllerParamContract {
            method: "voice.status",
            required: &[],
            optional: &[],
            examples: vec![json!({})],
        },
        ControllerParamContract {
            method: "voice.transcribe",
            required: &["audio_path"],
            optional: &["context", "skip_cleanup"],
            examples: vec![
                json!({"audio_path":"fixture.wav"}),
                json!({"audio_path":"fixture.wav","context":"example","skip_cleanup":true}),
            ],
        },
        ControllerParamContract {
            method: "voice.transcribe_bytes",
            required: &["audio_bytes"],
            optional: &["extension", "context", "skip_cleanup"],
            examples: vec![
                json!({"audio_bytes":[0,1]}),
                json!({"audio_bytes":[0,1],"extension":"wav","context":"example","skip_cleanup":true}),
            ],
        },
        ControllerParamContract {
            method: "voice.tts",
            required: &["text"],
            optional: &["output_path"],
            examples: vec![
                json!({"text":"Hello"}),
                json!({"text":"Hello","output_path":"fixture.wav"}),
            ],
        },
        ControllerParamContract {
            method: "voice.reply_synthesize",
            required: &["text"],
            optional: &["voice_id", "model_id", "output_format"],
            examples: vec![
                json!({"text":"Hello"}),
                json!({"text":"Hello","voice_id":"fixture","model_id":"fixture","output_format":"mp3_44100"}),
            ],
        },
        ControllerParamContract {
            method: "voice.stt_dispatch",
            required: &["audio_base64"],
            optional: &["provider", "model", "mime_type", "file_name", "language"],
            examples: vec![
                json!({"audio_base64":"AA=="}),
                json!({"audio_base64":"AA==","provider":"cloud","model":"fixture","mime_type":"audio/wav","file_name":"fixture.wav","language":"en"}),
            ],
        },
        ControllerParamContract {
            method: "voice.tts_dispatch",
            required: &["text"],
            optional: &["provider", "voice"],
            examples: vec![
                json!({"text":"Hello"}),
                json!({"text":"Hello","provider":"cloud","voice":"fixture"}),
            ],
        },
        ControllerParamContract {
            method: "voice.set_providers",
            required: &[],
            optional: &["stt_provider", "tts_provider", "stt_model", "tts_voice"],
            examples: vec![
                json!({}),
                json!({"stt_provider":"cloud","tts_provider":"cloud","stt_model":"fixture","tts_voice":"fixture"}),
            ],
        },
        ControllerParamContract {
            method: "voice.update_provider_settings",
            required: &[],
            optional: &["voice_providers", "stt_provider", "tts_provider"],
            examples: vec![
                json!({}),
                json!({"voice_providers":[{"slug":"fixture-provider"}],"stt_provider":"cloud","tts_provider":"cloud"}),
            ],
        },
        ControllerParamContract {
            method: "voice.list_models",
            required: &["provider_id"],
            optional: &["capability"],
            examples: vec![
                json!({"provider_id":"fixture"}),
                json!({"provider_id":"fixture","capability":"stt"}),
            ],
        },
        ControllerParamContract {
            method: "voice.test_provider",
            required: &["workload", "provider"],
            optional: &["validate_only", "api_key"],
            examples: vec![
                json!({"workload":"tts","provider":"cloud"}),
                json!({"workload":"stt","provider":"cloud","validate_only":true,"api_key":"fixture-key"}),
            ],
        },
    ];
    contracts.extend(crate::voice::live::schemas::parameter_contracts());
    contracts
}
