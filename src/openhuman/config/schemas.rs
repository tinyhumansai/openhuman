use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::TunnelConfig;
use crate::rpc::RpcOutcome;

const DEFAULT_ONBOARDING_FLAG_NAME: &str = ".skip_onboarding";

#[derive(Debug, Deserialize)]
struct ModelSettingsUpdate {
    api_key: Option<String>,
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
    allowlist: Option<Vec<String>>,
    denylist: Option<Vec<String>>,
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

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("get_config"),
        schemas("update_model_settings"),
        schemas("update_memory_settings"),
        schemas("update_screen_intelligence_settings"),
        schemas("update_tunnel_settings"),
        schemas("update_runtime_settings"),
        schemas("update_browser_settings"),
        schemas("resolve_api_url"),
        schemas("get_runtime_flags"),
        schemas("set_browser_allow_all"),
        schemas("workspace_onboarding_flag_exists"),
        schemas("workspace_onboarding_flag_set"),
        schemas("agent_server_status"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("get_config"),
            handler: handle_get_config,
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
            schema: schemas("update_tunnel_settings"),
            handler: handle_update_tunnel_settings,
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
            schema: schemas("agent_server_status"),
            handler: handle_agent_server_status,
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
        "update_model_settings" => ControllerSchema {
            namespace: "config",
            function: "update_model_settings",
            description: "Update model and API connection settings.",
            inputs: vec![
                optional_string("api_key", "Provider API key."),
                optional_string("api_url", "Backend API URL."),
                optional_string("default_model", "Default model id."),
                FieldSchema {
                    name: "default_temperature",
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "Default model temperature.",
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
        "update_tunnel_settings" => ControllerSchema {
            namespace: "config",
            function: "update_tunnel_settings",
            description: "Replace tunnel settings with provided config payload.",
            inputs: vec![
                required_string("provider", "Tunnel provider id."),
                FieldSchema {
                    name: "cloudflare",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Ref("CloudflareTunnelConfig"))),
                    comment: "Cloudflare tunnel settings.",
                    required: false,
                },
                FieldSchema {
                    name: "tailscale",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Ref("TailscaleTunnelConfig"))),
                    comment: "Tailscale tunnel settings.",
                    required: false,
                },
                FieldSchema {
                    name: "ngrok",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Ref("NgrokTunnelConfig"))),
                    comment: "ngrok tunnel settings.",
                    required: false,
                },
                FieldSchema {
                    name: "custom",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Ref("CustomTunnelConfig"))),
                    comment: "Custom tunnel settings.",
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
        "agent_server_status" => ControllerSchema {
            namespace: "config",
            function: "agent_server_status",
            description: "Return agent server runtime URL and status.",
            inputs: vec![],
            outputs: vec![json_output("status", "Agent server status payload.")],
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

fn handle_update_model_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let update = deserialize_params::<ModelSettingsUpdate>(params)?;
        let patch = config_rpc::ModelSettingsPatch {
            api_key: update.api_key,
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
            allowlist: update.allowlist,
            denylist: update.denylist,
        };
        to_json(config_rpc::load_and_apply_screen_intelligence_settings(patch).await?)
    })
}

fn handle_update_tunnel_settings(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let tunnel = deserialize_params::<TunnelConfig>(params)?;
        to_json(config_rpc::load_and_apply_tunnel_settings(tunnel).await?)
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

fn handle_agent_server_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { to_json(config_rpc::agent_server_status()) })
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
