use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

const DEFAULT_ONBOARDING_FLAG_NAME: &str = ".skip_onboarding";

#[derive(Debug, Deserialize)]
struct ModelRouteUpdate {
    hint: String,
    model: String,
}

#[derive(Debug, Deserialize)]
struct ModelSettingsUpdate {
    api_url: Option<String>,
    /// Optional API key for OpenAI-compatible backends. Stored verbatim in
    /// `config.toml` on the user's machine — see #1342 (local-first / pluggable
    /// backends). The key is never echoed back over RPC; `get_client_config`
    /// only reports `api_key_set: bool`.
    api_key: Option<String>,
    default_model: Option<String>,
    default_temperature: Option<f64>,
    /// When present, REPLACES `config.model_routes` wholesale with these
    /// `(hint, model)` pairs. Send `Some([])` to clear all routes (used when
    /// the user switches back to the OpenHuman backend whose built-in router
    /// picks per-task models on its own). Omit to leave existing routes
    /// untouched.
    model_routes: Option<Vec<ModelRouteUpdate>>,
}

#[derive(Debug, Deserialize)]
struct MemorySettingsUpdate {
    backend: Option<String>,
    auto_save: Option<bool>,
    embedding_provider: Option<String>,
    embedding_model: Option<String>,
    embedding_dimensions: Option<usize>,
    /// One of `"minimal" | "balanced" | "extended" | "maximum"`.
    memory_window: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RuntimeSettingsUpdate {
    kind: Option<String>,
    reasoning_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct BrowserSettingsUpdate {
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ScreenIntelligenceSettingsUpdate {
    enabled: Option<bool>,
    capture_policy: Option<String>,
    policy_mode: Option<String>,
    baseline_fps: Option<f32>,
    vision_enabled: Option<bool>,
    autocomplete_enabled: Option<bool>,
    use_vision_model: Option<bool>,
    keep_screenshots: Option<bool>,
    allowlist: Option<Vec<String>>,
    denylist: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct AnalyticsSettingsUpdate {
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct MeetSettingsUpdate {
    auto_orchestrator_handoff: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct LocalAiSettingsUpdate {
    runtime_enabled: Option<bool>,
    usage_embeddings: Option<bool>,
    usage_heartbeat: Option<bool>,
    usage_learning_reflection: Option<bool>,
    usage_subconscious: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct SetBrowserAllowAllParams {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct WorkspaceOnboardingFlagParams {
    flag_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceOnboardingFlagSetParams {
    flag_name: Option<String>,
    value: bool,
}

#[derive(Debug, Deserialize)]
struct OnboardingCompletedSetParams {
    value: bool,
}

#[derive(Debug, Deserialize)]
struct DictationSettingsUpdate {
    enabled: Option<bool>,
    hotkey: Option<String>,
    activation_mode: Option<String>,
    llm_refinement: Option<bool>,
    streaming: Option<bool>,
    streaming_interval_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct VoiceServerSettingsUpdate {
    auto_start: Option<bool>,
    hotkey: Option<String>,
    activation_mode: Option<String>,
    skip_cleanup: Option<bool>,
    min_duration_secs: Option<f32>,
    silence_threshold: Option<f32>,
    custom_dictionary: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct ComposioTriggerSettingsUpdate {
    triage_disabled: Option<bool>,
    triage_disabled_toolkits: Option<Vec<String>>,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("get_config"),
        schemas("get_client_config"),
        schemas("update_model_settings"),
        schemas("update_memory_settings"),
        schemas("update_screen_intelligence_settings"),
        schemas("update_runtime_settings"),
        schemas("update_browser_settings"),
        schemas("update_local_ai_settings"),
        schemas("resolve_api_url"),
        schemas("get_runtime_flags"),
        schemas("set_browser_allow_all"),
        schemas("workspace_onboarding_flag_exists"),
        schemas("workspace_onboarding_flag_set"),
        schemas("update_analytics_settings"),
        schemas("get_analytics_settings"),
        schemas("update_meet_settings"),
        schemas("get_meet_settings"),
        schemas("agent_server_status"),
        schemas("reset_local_data"),
        schemas("get_onboarding_completed"),
        schemas("set_onboarding_completed"),
        schemas("get_dictation_settings"),
        schemas("update_dictation_settings"),
        schemas("get_voice_server_settings"),
        schemas("update_voice_server_settings"),
        schemas("update_composio_trigger_settings"),
        schemas("get_composio_trigger_settings"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("get_config"),
            handler: handle_get_config,
        },
        RegisteredController {
            schema: schemas("get_client_config"),
            handler: handle_get_client_config,
        },
        RegisteredController {
            schema: schemas("update_model_settings"),
            handler: handle_update_model_settings,
        },
        RegisteredController {
            schema: schemas("update_memory_settings"),
            handler: handle_update_memory_settings,
        },
        RegisteredController {
            schema: schemas("update_screen_intelligence_settings"),
            handler: handle_update_screen_intelligence_settings,
        },
        RegisteredController {
            schema: schemas("update_runtime_settings"),
            handler: handle_update_runtime_settings,
        },
        RegisteredController {
            schema: schemas("update_browser_settings"),
            handler: handle_update_browser_settings,
        },
        RegisteredController {
            schema: schemas("update_local_ai_settings"),
            handler: handle_update_local_ai_settings,
        },
        RegisteredController {
            schema: schemas("resolve_api_url"),
            handler: handle_resolve_api_url,
        },
        RegisteredController {
            schema: schemas("get_runtime_flags"),
            handler: handle_get_runtime_flags,
        },
        RegisteredController {
            schema: schemas("set_browser_allow_all"),
            handler: handle_set_browser_allow_all,
        },
        RegisteredController {
            schema: schemas("workspace_onboarding_flag_exists"),
            handler: handle_workspace_onboarding_flag_exists,
        },
        RegisteredController {
            schema: schemas("workspace_onboarding_flag_set"),
            handler: handle_workspace_onboarding_flag_set,
        },
        RegisteredController {
            schema: schemas("update_analytics_settings"),
            handler: handle_update_analytics_settings,
        },
        RegisteredController {
            schema: schemas("get_analytics_settings"),
            handler: handle_get_analytics_settings,
        },
        RegisteredController {
            schema: schemas("update_meet_settings"),
            handler: handle_update_meet_settings,
        },
        RegisteredController {
            schema: schemas("get_meet_settings"),
            handler: handle_get_meet_settings,
        },
        RegisteredController {
            schema: schemas("agent_server_status"),
            handler: handle_agent_server_status,
        },
        RegisteredController {
            schema: schemas("reset_local_data"),
            handler: handle_reset_local_data,
        },
        RegisteredController {
            schema: schemas("get_onboarding_completed"),
            handler: handle_get_onboarding_completed,
        },
        RegisteredController {
            schema: schemas("set_onboarding_completed"),
            handler: handle_set_onboarding_completed,
        },
        RegisteredController {
            schema: schemas("get_dictation_settings"),
            handler: handle_get_dictation_settings,
        },
        RegisteredController {
            schema: schemas("update_dictation_settings"),
            handler: handle_update_dictation_settings,
        },
        RegisteredController {
            schema: schemas("get_voice_server_settings"),
            handler: handle_get_voice_server_settings,
        },
        RegisteredController {
            schema: schemas("update_voice_server_settings"),
            handler: handle_update_voice_server_settings,
        },
        RegisteredController {
            schema: schemas("update_composio_trigger_settings"),
            handler: handle_update_composio_trigger_settings,
        },
        RegisteredController {
            schema: schemas("get_composio_trigger_settings"),
            handler: handle_get_composio_trigger_settings,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "get_config" => ControllerSchema {
            namespace: "config",
            function: "get",
            description: "Read persisted config snapshot and resolved paths.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "snapshot",
                ty: TypeSchema::Json,
                comment: "Config snapshot with workspace and config paths.",
                required: true,
            }],
        },
        "get_client_config" => ControllerSchema {
            namespace: "config",
            function: "get_client_config",
            description: "Read safe client-facing config fields (api_url, feature flags). No secrets.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "api_url",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Configured backend API URL, if any.",
                    required: false,
                },
                FieldSchema {
                    name: "default_model",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Default model identifier.",
                    required: false,
                },
                FieldSchema {
                    name: "app_version",
                    ty: TypeSchema::String,
                    comment: "OpenHuman core version.",
                    required: true,
                },
                FieldSchema {
                    name: "api_key_set",
                    ty: TypeSchema::Bool,
                    comment: "True when a custom backend api_key is stored locally. The key itself is never returned over RPC.",
                    required: true,
                },
            ],
        },
        "update_model_settings" => ControllerSchema {
            namespace: "config",
            function: "update_model_settings",
            description: "Update model and backend connection settings, including a custom OpenAI-compatible backend (api_url + api_key).",
            inputs: vec![
                optional_string("api_url", "Backend API URL. Set to a non-default URL together with `api_key` to point inference at any OpenAI-compatible provider."),
                optional_string("api_key", "Optional API key for the configured backend. Pass an empty string to clear a previously stored key."),
                optional_string("default_model", "Default model id."),
                FieldSchema {
                    name: "default_temperature",
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "Default model temperature.",
                    required: false,
                },
                FieldSchema {
                    name: "model_routes",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Optional list of {hint, model} pairs mapping task hints (reasoning, agentic, coding, summarization) to provider-specific model ids. Replaces config.model_routes wholesale; send [] to clear (e.g. when switching back to the OpenHuman built-in router).",
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "update_memory_settings" => ControllerSchema {
            namespace: "config",
            function: "update_memory_settings",
            description: "Update memory backend and embedding settings.",
            inputs: vec![
                optional_string("backend", "Memory backend identifier."),
                FieldSchema {
                    name: "auto_save",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Enable auto-save.",
                    required: false,
                },
                optional_string("embedding_provider", "Embedding provider identifier."),
                optional_string("embedding_model", "Embedding model identifier."),
                FieldSchema {
                    name: "embedding_dimensions",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Embedding dimensions.",
                    required: false,
                },
                optional_string(
                    "memory_window",
                    "Stepped long-term memory window preset: minimal | balanced | extended | maximum.",
                ),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "update_screen_intelligence_settings" => ControllerSchema {
            namespace: "config",
            function: "update_screen_intelligence_settings",
            description: "Update screen intelligence runtime settings.",
            inputs: vec![
                optional_bool("enabled", "Enable screen intelligence."),
                optional_string("capture_policy", "Capture policy mode."),
                optional_string("policy_mode", "Policy mode override."),
                FieldSchema {
                    name: "baseline_fps",
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "Baseline capture FPS.",
                    required: false,
                },
                optional_bool("vision_enabled", "Enable vision analysis."),
                optional_bool("autocomplete_enabled", "Enable autocomplete integration."),
                optional_bool(
                    "use_vision_model",
                    "Use a vision LLM for screenshot analysis (false = OCR + text LLM).",
                ),
                optional_bool("keep_screenshots", "Keep screenshots on disk after vision processing."),
                FieldSchema {
                    name: "allowlist",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Allowed app list.",
                    required: false,
                },
                FieldSchema {
                    name: "denylist",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Denied app list.",
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "update_runtime_settings" => ControllerSchema {
            namespace: "config",
            function: "update_runtime_settings",
            description: "Update runtime execution strategy settings.",
            inputs: vec![
                optional_string("kind", "Runtime kind."),
                optional_bool("reasoning_enabled", "Enable reasoning mode."),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "update_browser_settings" => ControllerSchema {
            namespace: "config",
            function: "update_browser_settings",
            description: "Update browser automation settings.",
            inputs: vec![optional_bool("enabled", "Enable browser integration.")],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "update_local_ai_settings" => ControllerSchema {
            namespace: "config",
            function: "update_local_ai_settings",
            description:
                "Update the local AI runtime master switch and per-feature usage flags.",
            inputs: vec![
                optional_bool(
                    "runtime_enabled",
                    "Master switch — when false, no subsystem uses the local Ollama runtime.",
                ),
                optional_bool(
                    "usage_embeddings",
                    "Use the local model for embedding generation (when runtime_enabled).",
                ),
                optional_bool(
                    "usage_heartbeat",
                    "Use the local model inside the heartbeat loop (when runtime_enabled).",
                ),
                optional_bool(
                    "usage_learning_reflection",
                    "Use the local model for learning/reflection passes (when runtime_enabled).",
                ),
                optional_bool(
                    "usage_subconscious",
                    "Use the local model for subconscious evaluation (when runtime_enabled).",
                ),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "resolve_api_url" => ControllerSchema {
            namespace: "config",
            function: "resolve_api_url",
            description: "Resolve effective API base URL using config/env/default from core.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "api_url",
                ty: TypeSchema::String,
                comment: "Resolved backend API URL.",
                required: true,
            }],
        },
        "get_runtime_flags" => ControllerSchema {
            namespace: "config",
            function: "get_runtime_flags",
            description: "Read environment-driven runtime flags.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "flags",
                ty: TypeSchema::Ref("RuntimeFlagsOut"),
                comment: "Runtime flag state.",
                required: true,
            }],
        },
        "set_browser_allow_all" => ControllerSchema {
            namespace: "config",
            function: "set_browser_allow_all",
            description: "Set OPENHUMAN_BROWSER_ALLOW_ALL runtime flag.",
            inputs: vec![FieldSchema {
                name: "enabled",
                ty: TypeSchema::Bool,
                comment: "Whether to enable browser allow-all mode.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "flags",
                ty: TypeSchema::Ref("RuntimeFlagsOut"),
                comment: "Updated runtime flag state.",
                required: true,
            }],
        },
        "workspace_onboarding_flag_exists" => ControllerSchema {
            namespace: "config",
            function: "workspace_onboarding_flag_exists",
            description: "Check if onboarding flag file exists in workspace.",
            inputs: vec![FieldSchema {
                name: "flag_name",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional onboarding flag name override.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "exists",
                ty: TypeSchema::Bool,
                comment: "True when the flag file is present.",
                required: true,
            }],
        },
        "workspace_onboarding_flag_set" => ControllerSchema {
            namespace: "config",
            function: "workspace_onboarding_flag_set",
            description: "Create or remove the onboarding flag file in workspace.",
            inputs: vec![
                FieldSchema {
                    name: "flag_name",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional onboarding flag name override.",
                    required: false,
                },
                FieldSchema {
                    name: "value",
                    ty: TypeSchema::Bool,
                    comment: "True to create, false to remove.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "exists",
                ty: TypeSchema::Bool,
                comment: "True when the flag file is present after the operation.",
                required: true,
            }],
        },
        "update_analytics_settings" => ControllerSchema {
            namespace: "config",
            function: "update_analytics_settings",
            description: "Enable or disable anonymized analytics and error reporting.",
            inputs: vec![optional_bool(
                "enabled",
                "Enable anonymized analytics and crash reports.",
            )],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "get_analytics_settings" => ControllerSchema {
            namespace: "config",
            function: "get_analytics_settings",
            description: "Read current analytics settings.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "enabled",
                ty: TypeSchema::Bool,
                comment: "Whether anonymized analytics is enabled.",
                required: true,
            }],
        },
        "update_meet_settings" => ControllerSchema {
            namespace: "config",
            function: "update_meet_settings",
            description:
                "Update Google Meet integration settings (currently the auto-orchestrator-handoff privacy gate).",
            inputs: vec![optional_bool(
                "auto_orchestrator_handoff",
                "When true, ending a Meet call hands the transcript to the orchestrator for proactive follow-up actions.",
            )],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "get_meet_settings" => ControllerSchema {
            namespace: "config",
            function: "get_meet_settings",
            description: "Read current Google Meet integration settings.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "auto_orchestrator_handoff",
                ty: TypeSchema::Bool,
                comment: "Whether the orchestrator handoff fires on Meet call end.",
                required: true,
            }],
        },
        "agent_server_status" => ControllerSchema {
            namespace: "config",
            function: "agent_server_status",
            description: "Return agent server runtime URL and status.",
            inputs: vec![],
            outputs: vec![json_output("status", "Agent server status payload.")],
        },
        "reset_local_data" => ControllerSchema {
            namespace: "config",
            function: "reset_local_data",
            description:
                "Delete local OpenHuman data for the active config/workspace so the next restart boots clean.",
            inputs: vec![],
            outputs: vec![json_output("result", "Reset result with removed paths.")],
        },
        "get_onboarding_completed" => ControllerSchema {
            namespace: "config",
            function: "get_onboarding_completed",
            description: "Read whether the user has completed the onboarding flow.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "completed",
                ty: TypeSchema::Bool,
                comment: "True when onboarding has been completed.",
                required: true,
            }],
        },
        "get_dictation_settings" => ControllerSchema {
            namespace: "config",
            function: "get_dictation_settings",
            description: "Read current voice dictation settings.",
            inputs: vec![],
            outputs: vec![json_output("settings", "Dictation settings payload.")],
        },
        "update_dictation_settings" => ControllerSchema {
            namespace: "config",
            function: "update_dictation_settings",
            description: "Update voice dictation settings.",
            inputs: vec![
                optional_bool("enabled", "Enable voice dictation."),
                optional_string("hotkey", "Global hotkey string (e.g. Fn)."),
                optional_string("activation_mode", "Activation mode: toggle or push."),
                optional_bool("llm_refinement", "Enable LLM post-processing of transcription."),
                optional_bool("streaming", "Enable WebSocket streaming transcription."),
                FieldSchema {
                    name: "streaming_interval_ms",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Interval between streaming inference passes (ms).",
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "get_voice_server_settings" => ControllerSchema {
            namespace: "config",
            function: "get_voice_server_settings",
            description: "Read current voice server settings.",
            inputs: vec![],
            outputs: vec![json_output("settings", "Voice server settings payload.")],
        },
        "update_voice_server_settings" => ControllerSchema {
            namespace: "config",
            function: "update_voice_server_settings",
            description: "Update voice server settings.",
            inputs: vec![
                optional_bool("auto_start", "Start the voice server automatically with the core."),
                optional_string("hotkey", "Voice server hotkey string (e.g. Fn)."),
                optional_string("activation_mode", "Activation mode: tap or push."),
                optional_bool("skip_cleanup", "Skip LLM cleanup and keep dictation verbatim."),
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
                    comment: "Custom vocabulary words to bias whisper toward.",
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "set_onboarding_completed" => ControllerSchema {
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
        },
        "update_composio_trigger_settings" => ControllerSchema {
            namespace: "config",
            function: "update_composio_trigger_settings",
            description:
                "Update Composio trigger-triage settings. When triage is disabled the \
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
        },
        "get_composio_trigger_settings" => ControllerSchema {
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
        },
        _ => ControllerSchema {
            namespace: "config",
            function: "unknown",
            description: "Unknown config controller function.",
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

fn handle_get_config(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::load_and_get_config_snapshot().await?) })
}

fn handle_get_client_config(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let app_version =
            std::env::var("OPENHUMAN_APP_VERSION").unwrap_or_else(|_| "unknown".to_string());
        let api_key_set = config
            .api_key
            .as_deref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false);
        to_json(RpcOutcome::new(
            serde_json::json!({
                "api_url": config.api_url,
                "default_model": config.default_model,
                "app_version": app_version,
                "api_key_set": api_key_set,
            }),
            vec!["client config read".to_string()],
        ))
    })
}

fn handle_update_model_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<ModelSettingsUpdate>(params)?;
        let patch = config_rpc::ModelSettingsPatch {
            api_url: update.api_url,
            api_key: update.api_key,
            default_model: update.default_model,
            default_temperature: update.default_temperature,
            model_routes: update.model_routes.map(|routes| {
                routes
                    .into_iter()
                    .map(|r| crate::openhuman::config::ModelRouteConfig {
                        hint: r.hint,
                        model: r.model,
                    })
                    .collect()
            }),
        };
        to_json(config_rpc::load_and_apply_model_settings(patch).await?)
    })
}

fn handle_update_memory_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<MemorySettingsUpdate>(params)?;
        let patch = config_rpc::MemorySettingsPatch {
            backend: update.backend,
            auto_save: update.auto_save,
            embedding_provider: update.embedding_provider,
            embedding_model: update.embedding_model,
            embedding_dimensions: update.embedding_dimensions,
            memory_window: update.memory_window,
        };
        to_json(config_rpc::load_and_apply_memory_settings(patch).await?)
    })
}

fn handle_update_screen_intelligence_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<ScreenIntelligenceSettingsUpdate>(params)?;
        let patch = config_rpc::ScreenIntelligenceSettingsPatch {
            enabled: update.enabled,
            capture_policy: update.capture_policy,
            policy_mode: update.policy_mode,
            baseline_fps: update.baseline_fps,
            vision_enabled: update.vision_enabled,
            autocomplete_enabled: update.autocomplete_enabled,
            use_vision_model: update.use_vision_model,
            keep_screenshots: update.keep_screenshots,
            allowlist: update.allowlist,
            denylist: update.denylist,
        };
        to_json(config_rpc::load_and_apply_screen_intelligence_settings(patch).await?)
    })
}

fn handle_update_runtime_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<RuntimeSettingsUpdate>(params)?;
        let patch = config_rpc::RuntimeSettingsPatch {
            kind: update.kind,
            reasoning_enabled: update.reasoning_enabled,
        };
        to_json(config_rpc::load_and_apply_runtime_settings(patch).await?)
    })
}

fn handle_update_browser_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<BrowserSettingsUpdate>(params)?;
        let patch = config_rpc::BrowserSettingsPatch {
            enabled: update.enabled,
        };
        to_json(config_rpc::load_and_apply_browser_settings(patch).await?)
    })
}

fn handle_update_local_ai_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<LocalAiSettingsUpdate>(params)?;
        let patch = config_rpc::LocalAiSettingsPatch {
            runtime_enabled: update.runtime_enabled,
            usage_embeddings: update.usage_embeddings,
            usage_heartbeat: update.usage_heartbeat,
            usage_learning_reflection: update.usage_learning_reflection,
            usage_subconscious: update.usage_subconscious,
        };
        to_json(config_rpc::load_and_apply_local_ai_settings(patch).await?)
    })
}

fn handle_get_runtime_flags(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::get_runtime_flags()) })
}

fn handle_resolve_api_url(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::load_and_resolve_api_url().await?) })
}

fn handle_set_browser_allow_all(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<SetBrowserAllowAllParams>(params)?;
        to_json(config_rpc::set_browser_allow_all(payload.enabled))
    })
}

fn handle_workspace_onboarding_flag_exists(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkspaceOnboardingFlagParams>(params)?;
        to_json(
            config_rpc::workspace_onboarding_flag_resolve(
                payload.flag_name,
                DEFAULT_ONBOARDING_FLAG_NAME,
            )
            .await?,
        )
    })
}

fn handle_workspace_onboarding_flag_set(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkspaceOnboardingFlagSetParams>(params)?;
        to_json(
            config_rpc::workspace_onboarding_flag_set(
                payload.flag_name,
                DEFAULT_ONBOARDING_FLAG_NAME,
                payload.value,
            )
            .await?,
        )
    })
}

fn handle_update_analytics_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<AnalyticsSettingsUpdate>(params)?;
        let patch = config_rpc::AnalyticsSettingsPatch {
            enabled: update.enabled,
        };
        to_json(config_rpc::load_and_apply_analytics_settings(patch).await?)
    })
}

fn handle_get_analytics_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let config = config_rpc::load_config_with_timeout().await?;
        let result = serde_json::json!({
            "enabled": config.observability.analytics_enabled,
        });
        to_json(RpcOutcome::new(
            result,
            vec!["analytics settings read".to_string()],
        ))
    })
}

fn handle_update_meet_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] update_meet_settings enter");
        let update = match deserialize_params::<MeetSettingsUpdate>(params) {
            Ok(u) => u,
            Err(err) => {
                log::warn!("[config][rpc] update_meet_settings invalid params: {err}");
                return Err(err);
            }
        };
        log::debug!(
            "[config][rpc] update_meet_settings patch auto_orchestrator_handoff={:?}",
            update.auto_orchestrator_handoff
        );
        let patch = config_rpc::MeetSettingsPatch {
            auto_orchestrator_handoff: update.auto_orchestrator_handoff,
        };
        match config_rpc::load_and_apply_meet_settings(patch).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] update_meet_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] update_meet_settings failed: {err}");
                Err(err)
            }
        }
    })
}

fn handle_get_meet_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        log::debug!("[config][rpc] get_meet_settings enter");
        let config = match config_rpc::load_config_with_timeout().await {
            Ok(c) => c,
            Err(err) => {
                log::warn!("[config][rpc] get_meet_settings load failed: {err}");
                return Err(err);
            }
        };
        let auto_orchestrator_handoff = config.meet.auto_orchestrator_handoff;
        log::debug!(
            "[config][rpc] get_meet_settings ok auto_orchestrator_handoff={auto_orchestrator_handoff}"
        );
        let result = serde_json::json!({
            "auto_orchestrator_handoff": auto_orchestrator_handoff,
        });
        to_json(RpcOutcome::new(
            result,
            vec!["meet settings read".to_string()],
        ))
    })
}

fn handle_agent_server_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::agent_server_status()) })
}

fn handle_reset_local_data(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::reset_local_data().await?) })
}

fn handle_get_onboarding_completed(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::get_onboarding_completed().await?) })
}

fn handle_get_dictation_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::get_dictation_settings().await?) })
}

fn handle_update_dictation_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<DictationSettingsUpdate>(params)?;
        let patch = config_rpc::DictationSettingsPatch {
            enabled: update.enabled,
            hotkey: update.hotkey,
            activation_mode: update.activation_mode,
            llm_refinement: update.llm_refinement,
            streaming: update.streaming,
            streaming_interval_ms: update.streaming_interval_ms,
        };
        to_json(config_rpc::load_and_apply_dictation_settings(patch).await?)
    })
}

fn handle_get_voice_server_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::get_voice_server_settings().await?) })
}

fn handle_update_voice_server_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<VoiceServerSettingsUpdate>(params)?;
        let patch = config_rpc::VoiceServerSettingsPatch {
            auto_start: update.auto_start,
            hotkey: update.hotkey,
            activation_mode: update.activation_mode,
            skip_cleanup: update.skip_cleanup,
            min_duration_secs: update.min_duration_secs,
            silence_threshold: update.silence_threshold,
            custom_dictionary: update.custom_dictionary,
        };
        to_json(config_rpc::load_and_apply_voice_server_settings(patch).await?)
    })
}

fn handle_set_onboarding_completed(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<OnboardingCompletedSetParams>(params)?;
        to_json(config_rpc::set_onboarding_completed(payload.value).await?)
    })
}

fn handle_update_composio_trigger_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[config][rpc] update_composio_trigger_settings enter");
        let update = match deserialize_params::<ComposioTriggerSettingsUpdate>(params) {
            Ok(u) => u,
            Err(err) => {
                log::warn!("[config][rpc] update_composio_trigger_settings invalid params: {err}");
                return Err(err);
            }
        };
        let patch = config_rpc::ComposioTriggerSettingsPatch {
            triage_disabled: update.triage_disabled,
            triage_disabled_toolkits: update.triage_disabled_toolkits,
        };
        match config_rpc::load_and_apply_composio_trigger_settings(patch).await {
            Ok(outcome) => {
                log::debug!("[config][rpc] update_composio_trigger_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] update_composio_trigger_settings failed: {err}");
                Err(err)
            }
        }
    })
}

fn handle_get_composio_trigger_settings(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        log::debug!("[config][rpc] get_composio_trigger_settings enter");
        match config_rpc::get_composio_trigger_settings().await {
            Ok(outcome) => {
                log::debug!("[config][rpc] get_composio_trigger_settings ok");
                to_json(outcome)
            }
            Err(err) => {
                log::warn!("[config][rpc] get_composio_trigger_settings failed: {err}");
                Err(err)
            }
        }
    })
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
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

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
