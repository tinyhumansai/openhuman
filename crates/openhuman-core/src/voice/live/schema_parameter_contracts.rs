use crate::core::all::ControllerParamContract;
use serde_json::json;

pub(crate) fn contracts() -> Vec<ControllerParamContract> {
    vec![
        ControllerParamContract {
            method: "voice.live_providers",
            required: &[],
            optional: &[],
            examples: vec![json!({})],
        },
        ControllerParamContract {
            method: "voice.live_settings_get",
            required: &[],
            optional: &[],
            examples: vec![json!({})],
        },
        ControllerParamContract {
            method: "voice.live_settings_set",
            required: &[],
            optional: &["default_provider", "gemini", "sarvam", "elevenlabs"],
            examples: vec![
                json!({}),
                json!({"default_provider":"gemini","gemini":{"model":"fixture","voice":null},"sarvam":{"language":"en"},"elevenlabs":{"voice_id":"fixture"}}),
            ],
        },
        ControllerParamContract {
            method: "voice.live_test_provider",
            required: &["provider"],
            optional: &[],
            examples: vec![json!({"provider":"gemini"})],
        },
    ]
}
