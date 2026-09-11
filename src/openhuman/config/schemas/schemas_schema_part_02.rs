use super::*;

pub(super) fn lookup(function: &str) -> Option<ControllerSchema> {
    match function {
        "update_dictation_settings" => Some(ControllerSchema {
            namespace: "config",
            function: "update_dictation_settings",
            description: "Update voice dictation settings.",
            inputs: vec![
                optional_bool("enabled", "Enable voice dictation."),
                optional_string("hotkey", "Global hotkey string (e.g. Fn)."),
                optional_string("activation_mode", "Activation mode: toggle or push."),
                optional_bool(
                    "llm_refinement",
                    "Enable LLM post-processing of transcription.",
                ),
                optional_bool("streaming", "Enable WebSocket streaming transcription."),
                FieldSchema {
                    name: "streaming_interval_ms",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Interval between streaming inference passes (ms).",
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        }),
        "get_voice_server_settings" => Some(ControllerSchema {
            namespace: "config",
            function: "get_voice_server_settings",
            description: "Read current voice server settings.",
            inputs: vec![],
            outputs: vec![json_output("settings", "Voice server settings payload.")],
        }),
        "update_voice_server_settings" => Some(ControllerSchema {
            namespace: "config",
            function: "update_voice_server_settings",
            description: "Update voice server settings.",
            inputs: vec![
                optional_bool(
                    "auto_start",
                    "Start the voice server automatically with the core.",
                ),
                optional_string("hotkey", "Voice server hotkey string (e.g. Fn)."),
                optional_string("activation_mode", "Activation mode: tap or push."),
                optional_bool(
                    "skip_cleanup",
                    "Skip LLM cleanup and keep dictation verbatim.",
                ),
                FieldSchema {
                    name: "min_duration_secs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "Minimum recording duration in seconds.",
                    required: false,
                },
                FieldSchema {
                    name: "silence_threshold",
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "RMS energy threshold for silence detection.",
                    required: false,
                },
                FieldSchema {
                    name: "custom_dictionary",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Custom vocabulary words to bias the STT engine toward.",
                    required: false,
                },
                optional_bool(
                    "always_on_enabled",
                    "Continuous always-on listening (no hotkey). Opt-in.",
                ),
                optional_string(
                    "wake_word",
                    "Always-on wake word; utterances must contain it (default 'Hey Tiny').",
                ),
                optional_string(
                    "stt_engine",
                    "Hosted speech-to-text engine: backend, elevenlabs, or openai.",
                ),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        }),
        "set_onboarding_completed" => Some(ControllerSchema {
            namespace: "config",
            function: "set_onboarding_completed",
            description: "Mark the onboarding flow as completed or reset it.",
            inputs: vec![FieldSchema {
                name: "value",
                ty: TypeSchema::Bool,
                comment: "True to mark completed, false to reset.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "completed",
                ty: TypeSchema::Bool,
                comment: "Updated onboarding completed state.",
                required: true,
            }],
        }),
        "update_composio_trigger_settings" => Some(ControllerSchema {
            namespace: "config",
            function: "update_composio_trigger_settings",
            description: "Update Composio trigger-triage settings. When triage is disabled the \
                 local LLM is NOT invoked per trigger — events are still archived to \
                 trigger history.",
            inputs: vec![
                optional_bool(
                    "triage_disabled",
                    "When true, skip the LLM triage turn for all Composio triggers globally.",
                ),
                FieldSchema {
                    name: "triage_disabled_toolkits",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Toolkit slugs that skip LLM triage (e.g. [\"gmail\", \"slack\"]).",
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        }),
        "get_composio_trigger_settings" => Some(ControllerSchema {
            namespace: "config",
            function: "get_composio_trigger_settings",
            description: "Read current Composio trigger-triage settings.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "triage_disabled",
                    ty: TypeSchema::Bool,
                    comment: "Whether the global triage-disabled flag is set.",
                    required: true,
                },
                FieldSchema {
                    name: "triage_disabled_toolkits",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Toolkit slugs that skip LLM triage.",
                    required: true,
                },
            ],
        }),
        _ => None,
    }
}
