use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

const DEFAULT_ONBOARDING_FLAG_NAME: &str = ".skip_onboarding";

#[derive(Debug, Deserialize)]
struct ModelSettingsUpdate {
    api_url: Option<String>,
    default_model: Option<String>,
    default_temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct MemorySettingsUpdate {
    backend: Option<String>,
    auto_save: Option<bool>,
    embedding_provider: Option<String>,
    embedding_model: Option<String>,
    embedding_dimensions: Option<usize>,
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

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("get_config"),
        schemas("get_client_config"),
        schemas("update_model_settings"),
        schemas("update_memory_settings"),
        schemas("update_screen_intelligence_settings"),
        schemas("update_runtime_settings"),
        schemas("update_browser_settings"),
        schemas("resolve_api_url"),
        schemas("get_runtime_flags"),
        schemas("set_browser_allow_all"),
        schemas("workspace_onboarding_flag_exists"),
        schemas("workspace_onboarding_flag_set"),
        schemas("update_analytics_settings"),
        schemas("get_analytics_settings"),
        schemas("agent_server_status"),
        schemas("reset_local_data"),
        schemas("get_onboarding_completed"),
        schemas("set_onboarding_completed"),
        schemas("get_dictation_settings"),
        schemas("update_dictation_settings"),
        schemas("get_voice_server_settings"),
        schemas("update_voice_server_settings"),
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
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "get_config" => ControllerSchema {
            namespace: "config".to_string(),
            function: "get".to_string(),
            description: "Read persisted config snapshot and resolved paths.".to_string(),
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "snapshot".to_string(),
                ty: TypeSchema::Json,
                comment: "Config snapshot with workspace and config paths.".to_string(),
                required: true,
            }],
        },
        "get_client_config" => ControllerSchema {
            namespace: "config".to_string(),
            function: "get_client_config".to_string(),
            description:
                "Read safe client-facing config fields (api_url, feature flags). No secrets.".to_string(),
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "api_url".to_string(),
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Configured backend API URL, if any.".to_string(),
                    required: false,
                },
                FieldSchema {
                    name: "default_model".to_string(),
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Default model identifier.".to_string(),
                    required: false,
                },
                FieldSchema {
                    name: "app_version".to_string(),
                    ty: TypeSchema::String,
                    comment: "OpenHuman core version.".to_string(),
                    required: true,
                },
            ],
        },
        "update_model_settings" => ControllerSchema {
            namespace: "config".to_string(),
            function: "update_model_settings".to_string(),
            description: "Update model and backend connection settings.".to_string(),
            inputs: vec![
                optional_string("api_url", "Backend API URL."),
                optional_string("default_model", "Default model id."),
                FieldSchema {
                    name: "default_temperature".to_string(),
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "Default model temperature.".to_string(),
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "update_memory_settings" => ControllerSchema {
            namespace: "config".to_string(),
            function: "update_memory_settings".to_string(),
            description: "Update memory backend and embedding settings.".to_string(),
            inputs: vec![
                optional_string("backend", "Memory backend identifier."),
                FieldSchema {
                    name: "auto_save".to_string(),
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Enable auto-save.".to_string(),
                    required: false,
                },
                optional_string("embedding_provider", "Embedding provider identifier."),
                optional_string("embedding_model", "Embedding model identifier."),
                FieldSchema {
                    name: "embedding_dimensions".to_string(),
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Embedding dimensions.".to_string(),
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "update_screen_intelligence_settings" => ControllerSchema {
            namespace: "config".to_string(),
            function: "update_screen_intelligence_settings".to_string(),
            description: "Update screen intelligence runtime settings.".to_string(),
            inputs: vec![
                optional_bool("enabled", "Enable screen intelligence."),
                optional_string("capture_policy", "Capture policy mode."),
                optional_string("policy_mode", "Policy mode override."),
                FieldSchema {
                    name: "baseline_fps".to_string(),
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "Baseline capture FPS.".to_string(),
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
                    name: "allowlist".to_string(),
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Allowed app list.".to_string(),
                    required: false,
                },
                FieldSchema {
                    name: "denylist".to_string(),
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Denied app list.".to_string(),
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "update_runtime_settings" => ControllerSchema {
            namespace: "config".to_string(),
            function: "update_runtime_settings".to_string(),
            description: "Update runtime execution strategy settings.".to_string(),
            inputs: vec![
                optional_string("kind", "Runtime kind."),
                optional_bool("reasoning_enabled", "Enable reasoning mode."),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "update_browser_settings" => ControllerSchema {
            namespace: "config".to_string(),
            function: "update_browser_settings".to_string(),
            description: "Update browser automation settings.".to_string(),
            inputs: vec![optional_bool("enabled", "Enable browser integration.")],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "resolve_api_url" => ControllerSchema {
            namespace: "config".to_string(),
            function: "resolve_api_url".to_string(),
            description: "Resolve effective API base URL using config/env/default from core.".to_string(),
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "api_url".to_string(),
                ty: TypeSchema::String,
                comment: "Resolved backend API URL.".to_string(),
                required: true,
            }],
        },
        "get_runtime_flags" => ControllerSchema {
            namespace: "config".to_string(),
            function: "get_runtime_flags".to_string(),
            description: "Read environment-driven runtime flags.".to_string(),
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "flags".to_string(),
                ty: TypeSchema::Ref("RuntimeFlagsOut".to_string()),
                comment: "Runtime flag state.".to_string(),
                required: true,
            }],
        },
        "set_browser_allow_all" => ControllerSchema {
            namespace: "config".to_string(),
            function: "set_browser_allow_all".to_string(),
            description: "Set OPENHUMAN_BROWSER_ALLOW_ALL runtime flag.".to_string(),
            inputs: vec![FieldSchema {
                name: "enabled".to_string(),
                ty: TypeSchema::Bool,
                comment: "Whether to enable browser allow-all mode.".to_string(),
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "flags".to_string(),
                ty: TypeSchema::Ref("RuntimeFlagsOut".to_string()),
                comment: "Updated runtime flag state.".to_string(),
                required: true,
            }],
        },
        "workspace_onboarding_flag_exists" => ControllerSchema {
            namespace: "config".to_string(),
            function: "workspace_onboarding_flag_exists".to_string(),
            description: "Check if onboarding flag file exists in workspace.".to_string(),
            inputs: vec![FieldSchema {
                name: "flag_name".to_string(),
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional onboarding flag name override.".to_string(),
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "exists".to_string(),
                ty: TypeSchema::Bool,
                comment: "True when the flag file is present.".to_string(),
                required: true,
            }],
        },
        "workspace_onboarding_flag_set" => ControllerSchema {
            namespace: "config".to_string(),
            function: "workspace_onboarding_flag_set".to_string(),
            description: "Create or remove the onboarding flag file in workspace.".to_string(),
            inputs: vec![
                FieldSchema {
                    name: "flag_name".to_string(),
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional onboarding flag name override.".to_string(),
                    required: false,
                },
                FieldSchema {
                    name: "value".to_string(),
                    ty: TypeSchema::Bool,
                    comment: "True to create, false to remove.".to_string(),
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "exists".to_string(),
                ty: TypeSchema::Bool,
                comment: "True when the flag file is present after the operation.".to_string(),
                required: true,
            }],
        },
        "update_analytics_settings" => ControllerSchema {
            namespace: "config".to_string(),
            function: "update_analytics_settings".to_string(),
            description: "Enable or disable anonymized analytics and error reporting.".to_string(),
            inputs: vec![optional_bool(
                "enabled",
                "Enable anonymized analytics and crash reports.",
            )],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "get_analytics_settings" => ControllerSchema {
            namespace: "config".to_string(),
            function: "get_analytics_settings".to_string(),
            description: "Read current analytics settings.".to_string(),
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "enabled".to_string(),
                ty: TypeSchema::Bool,
                comment: "Whether anonymized analytics is enabled.".to_string(),
                required: true,
            }],
        },
        "agent_server_status" => ControllerSchema {
            namespace: "config".to_string(),
            function: "agent_server_status".to_string(),
            description: "Return agent server runtime URL and status.".to_string(),
            inputs: vec![],
            outputs: vec![json_output("status", "Agent server status payload.")],
        },
        "reset_local_data" => ControllerSchema {
            namespace: "config".to_string(),
            function: "reset_local_data".to_string(),
            description:
                "Delete local OpenHuman data for the active config/workspace so the next restart boots clean.".to_string(),
            inputs: vec![],
            outputs: vec![json_output("result", "Reset result with removed paths.")],
        },
        "get_onboarding_completed" => ControllerSchema {
            namespace: "config".to_string(),
            function: "get_onboarding_completed".to_string(),
            description: "Read whether the user has completed the onboarding flow.".to_string(),
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "completed".to_string(),
                ty: TypeSchema::Bool,
                comment: "True when onboarding has been completed.".to_string(),
                required: true,
            }],
        },
        "get_dictation_settings" => ControllerSchema {
            namespace: "config".to_string(),
            function: "get_dictation_settings".to_string(),
            description: "Read current voice dictation settings.".to_string(),
            inputs: vec![],
            outputs: vec![json_output("settings", "Dictation settings payload.")],
        },
        "update_dictation_settings" => ControllerSchema {
            namespace: "config".to_string(),
            function: "update_dictation_settings".to_string(),
            description: "Update voice dictation settings.".to_string(),
            inputs: vec![
                optional_bool("enabled", "Enable voice dictation."),
                optional_string("hotkey", "Global hotkey string (e.g. Fn)."),
                optional_string("activation_mode", "Activation mode: toggle or push."),
                optional_bool("llm_refinement", "Enable LLM post-processing of transcription."),
                optional_bool("streaming", "Enable WebSocket streaming transcription."),
                FieldSchema {
                    name: "streaming_interval_ms".to_string(),
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Interval between streaming inference passes (ms).".to_string(),
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "get_voice_server_settings" => ControllerSchema {
            namespace: "config".to_string(),
            function: "get_voice_server_settings".to_string(),
            description: "Read current voice server settings.".to_string(),
            inputs: vec![],
            outputs: vec![json_output("settings", "Voice server settings payload.")],
        },
        "update_voice_server_settings" => ControllerSchema {
            namespace: "config".to_string(),
            function: "update_voice_server_settings".to_string(),
            description: "Update voice server settings.".to_string(),
            inputs: vec![
                optional_bool("auto_start", "Start the voice server automatically with the core."),
                optional_string("hotkey", "Voice server hotkey string (e.g. Fn)."),
                optional_string("activation_mode", "Activation mode: tap or push."),
                optional_bool("skip_cleanup", "Skip LLM cleanup and keep dictation verbatim."),
                FieldSchema {
                    name: "min_duration_secs".to_string(),
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "Minimum recording duration in seconds.".to_string(),
                    required: false,
                },
                FieldSchema {
                    name: "silence_threshold".to_string(),
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "RMS energy threshold for silence detection.".to_string(),
                    required: false,
                },
                FieldSchema {
                    name: "custom_dictionary".to_string(),
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Custom vocabulary words to bias whisper toward.".to_string(),
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "set_onboarding_completed" => ControllerSchema {
            namespace: "config".to_string(),
            function: "set_onboarding_completed".to_string(),
            description: "Mark the onboarding flow as completed or reset it.".to_string(),
            inputs: vec![FieldSchema {
                name: "value".to_string(),
                ty: TypeSchema::Bool,
                comment: "True to mark completed, false to reset.".to_string(),
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "completed".to_string(),
                ty: TypeSchema::Bool,
                comment: "Updated onboarding completed state.".to_string(),
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "config".to_string(),
            function: "unknown".to_string(),
            description: "Unknown config controller function.".to_string(),
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error".to_string(),
                ty: TypeSchema::String,
                comment: "Lookup error details.".to_string(),
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
        let app_version = std::env::var("OPENHUMAN_APP_VERSION")
            .unwrap_or_else(|_| "unknown".to_string());
        to_json(serde_json::json!({
            "api_url": config.api_url,
            "default_model": config.default_model,
            "app_version": app_version,
        }))
    })
}

fn handle_update_model_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<ModelSettingsUpdate>(params)?;
        let patch = config_rpc::ModelSettingsPatch {
            api_url: update.api_url,
            default_model: update.default_model,
            default_temperature: update.default_temperature,
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
            hotkey: update.hotoken,
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

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name: name.to_string(),
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment: comment.to_string(),
        required: false,
    }
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name: name.to_string(),
        ty: TypeSchema::String,
        comment: comment.to_string(),
        required: true,
    }
}

fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name: name.to_string(),
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment: comment.to_string(),
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name: name.to_string(),
        ty: TypeSchema::Json,
        comment: comment.to_string(),
        required: true,
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
