use serde::Deserialize;
use serde_json::json;

use crate::core_server::helpers::{
    default_workspace_dir, env_flag_enabled, load_openhuman_config, parse_params, snapshot_config,
};
use crate::core_server::types::{
    BrowserSettingsUpdate, GatewaySettingsUpdate, InvocationResult, MemorySettingsUpdate,
    ModelSettingsUpdate, RuntimeFlags, RuntimeSettingsUpdate, ScreenIntelligenceSettingsUpdate,
    SetBrowserAllowAllParams,
};
use crate::core_server::DEFAULT_ONBOARDING_FLAG_NAME;
use crate::openhuman::health;
use crate::openhuman::screen_intelligence;
use crate::openhuman::security::SecurityPolicy;

pub async fn try_dispatch(
    method: &str,
    params: serde_json::Value,
) -> Option<Result<InvocationResult, String>> {
    match method {
        "openhuman.health_snapshot" => Some(Ok(InvocationResult::with_logs(
            health::snapshot_json(),
            vec!["health_snapshot requested".to_string()],
        )
        .unwrap())),

        "openhuman.security_policy_info" => {
            let policy = SecurityPolicy::default();
            let payload = json!({
                "autonomy": policy.autonomy,
                "workspace_only": policy.workspace_only,
                "allowed_commands": policy.allowed_commands,
                "max_actions_per_hour": policy.max_actions_per_hour,
                "require_approval_for_medium_risk": policy.require_approval_for_medium_risk,
                "block_high_risk_commands": policy.block_high_risk_commands,
            });
            Some(InvocationResult::with_logs(
                payload,
                vec!["security_policy_info computed".to_string()],
            ))
        }

        "openhuman.get_config" => Some(
            async move {
                let config = load_openhuman_config().await?;
                let snapshot = snapshot_config(&config)?;
                InvocationResult::with_logs(
                    snapshot,
                    vec![format!(
                        "config loaded from {}",
                        config.config_path.display()
                    )],
                )
            }
            .await,
        ),

        "openhuman.update_model_settings" => Some(
            async move {
                let update: ModelSettingsUpdate = parse_params(params)?;
                let mut config = load_openhuman_config().await?;
                if let Some(api_key) = update.api_key {
                    config.api_key = if api_key.trim().is_empty() {
                        None
                    } else {
                        Some(api_key)
                    };
                }
                if let Some(api_url) = update.api_url {
                    config.api_url = if api_url.trim().is_empty() {
                        None
                    } else {
                        Some(api_url)
                    };
                }
                if let Some(provider) = update.default_provider {
                    config.default_provider = if provider.trim().is_empty() {
                        None
                    } else {
                        Some(provider)
                    };
                }
                if let Some(model) = update.default_model {
                    config.default_model = if model.trim().is_empty() {
                        None
                    } else {
                        Some(model)
                    };
                }
                if let Some(temp) = update.default_temperature {
                    config.default_temperature = temp;
                }
                config.save().await.map_err(|e| e.to_string())?;
                let snapshot = snapshot_config(&config)?;
                InvocationResult::with_logs(
                    snapshot,
                    vec![format!(
                        "model settings saved to {}",
                        config.config_path.display()
                    )],
                )
            }
            .await,
        ),

        "openhuman.update_memory_settings" => Some(
            async move {
                let update: MemorySettingsUpdate = parse_params(params)?;
                let mut config = load_openhuman_config().await?;
                if let Some(backend) = update.backend {
                    config.memory.backend = backend;
                }
                if let Some(auto_save) = update.auto_save {
                    config.memory.auto_save = auto_save;
                }
                if let Some(provider) = update.embedding_provider {
                    config.memory.embedding_provider = provider;
                }
                if let Some(model) = update.embedding_model {
                    config.memory.embedding_model = model;
                }
                if let Some(dimensions) = update.embedding_dimensions {
                    config.memory.embedding_dimensions = dimensions;
                }
                config.save().await.map_err(|e| e.to_string())?;
                let snapshot = snapshot_config(&config)?;
                InvocationResult::with_logs(
                    snapshot,
                    vec![format!(
                        "memory settings saved to {}",
                        config.config_path.display()
                    )],
                )
            }
            .await,
        ),

        "openhuman.update_screen_intelligence_settings" => Some(
            async move {
                let update: ScreenIntelligenceSettingsUpdate = parse_params(params)?;
                let mut config = load_openhuman_config().await?;

                if let Some(enabled) = update.enabled {
                    config.screen_intelligence.enabled = enabled;
                }
                if let Some(capture_policy) = update.capture_policy {
                    config.screen_intelligence.capture_policy = capture_policy;
                }
                if let Some(policy_mode) = update.policy_mode {
                    config.screen_intelligence.policy_mode = policy_mode;
                }
                if let Some(baseline_fps) = update.baseline_fps {
                    config.screen_intelligence.baseline_fps = baseline_fps.clamp(0.2, 30.0);
                }
                if let Some(vision_enabled) = update.vision_enabled {
                    config.screen_intelligence.vision_enabled = vision_enabled;
                }
                if let Some(autocomplete_enabled) = update.autocomplete_enabled {
                    config.screen_intelligence.autocomplete_enabled = autocomplete_enabled;
                }
                if let Some(allowlist) = update.allowlist {
                    config.screen_intelligence.allowlist = allowlist;
                }
                if let Some(denylist) = update.denylist {
                    config.screen_intelligence.denylist = denylist;
                }

                config.save().await.map_err(|e| e.to_string())?;
                let _ = screen_intelligence::global_engine()
                    .apply_config(config.screen_intelligence.clone())
                    .await;

                let snapshot = snapshot_config(&config)?;
                InvocationResult::with_logs(
                    snapshot,
                    vec![format!(
                        "screen intelligence settings saved to {}",
                        config.config_path.display()
                    )],
                )
            }
            .await,
        ),

        "openhuman.update_gateway_settings" => Some(
            async move {
                let update: GatewaySettingsUpdate = parse_params(params)?;
                let mut config = load_openhuman_config().await?;
                if let Some(host) = update.host {
                    config.gateway.host = host;
                }
                if let Some(port) = update.port {
                    config.gateway.port = port;
                }
                if let Some(require_pairing) = update.require_pairing {
                    config.gateway.require_pairing = require_pairing;
                }
                if let Some(allow_public_bind) = update.allow_public_bind {
                    config.gateway.allow_public_bind = allow_public_bind;
                }
                config.save().await.map_err(|e| e.to_string())?;
                let snapshot = snapshot_config(&config)?;
                InvocationResult::with_logs(
                    snapshot,
                    vec![format!(
                        "gateway settings saved to {}",
                        config.config_path.display()
                    )],
                )
            }
            .await,
        ),

        "openhuman.update_tunnel_settings" => Some(
            async move {
                let tunnel: crate::openhuman::config::TunnelConfig = parse_params(params)?;
                let mut config = load_openhuman_config().await?;
                config.tunnel = tunnel;
                config.save().await.map_err(|e| e.to_string())?;
                let snapshot = snapshot_config(&config)?;
                InvocationResult::with_logs(
                    snapshot,
                    vec![format!(
                        "tunnel settings saved to {}",
                        config.config_path.display()
                    )],
                )
            }
            .await,
        ),

        "openhuman.update_runtime_settings" => Some(
            async move {
                let update: RuntimeSettingsUpdate = parse_params(params)?;
                let mut config = load_openhuman_config().await?;
                if let Some(kind) = update.kind {
                    config.runtime.kind = kind;
                }
                if let Some(reasoning_enabled) = update.reasoning_enabled {
                    config.runtime.reasoning_enabled = Some(reasoning_enabled);
                }
                config.save().await.map_err(|e| e.to_string())?;
                let snapshot = snapshot_config(&config)?;
                InvocationResult::with_logs(
                    snapshot,
                    vec![format!(
                        "runtime settings saved to {}",
                        config.config_path.display()
                    )],
                )
            }
            .await,
        ),

        "openhuman.update_browser_settings" => Some(
            async move {
                let update: BrowserSettingsUpdate = parse_params(params)?;
                let mut config = load_openhuman_config().await?;
                if let Some(enabled) = update.enabled {
                    config.browser.enabled = enabled;
                }
                config.save().await.map_err(|e| e.to_string())?;
                let snapshot = snapshot_config(&config)?;
                InvocationResult::with_logs(
                    snapshot,
                    vec![format!(
                        "browser settings saved to {}",
                        config.config_path.display()
                    )],
                )
            }
            .await,
        ),

        "openhuman.get_runtime_flags" => Some(Ok(InvocationResult::with_logs(
            RuntimeFlags {
                browser_allow_all: env_flag_enabled("OPENHUMAN_BROWSER_ALLOW_ALL"),
                log_prompts: env_flag_enabled("OPENHUMAN_LOG_PROMPTS"),
            },
            vec!["runtime flags read".to_string()],
        )
        .unwrap())),

        "openhuman.set_browser_allow_all" => Some(
            async move {
                let p: SetBrowserAllowAllParams = parse_params(params)?;
                if p.enabled {
                    std::env::set_var("OPENHUMAN_BROWSER_ALLOW_ALL", "1");
                } else {
                    std::env::remove_var("OPENHUMAN_BROWSER_ALLOW_ALL");
                }
                let flags = RuntimeFlags {
                    browser_allow_all: env_flag_enabled("OPENHUMAN_BROWSER_ALLOW_ALL"),
                    log_prompts: env_flag_enabled("OPENHUMAN_LOG_PROMPTS"),
                };
                InvocationResult::with_logs(
                    flags,
                    vec!["browser allow-all flag updated".to_string()],
                )
            }
            .await,
        ),

        "openhuman.workspace_onboarding_flag_exists" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct WorkspaceOnboardingFlagParams {
                    flag_name: Option<String>,
                }

                let payload: WorkspaceOnboardingFlagParams = parse_params(params)?;
                let name = payload
                    .flag_name
                    .unwrap_or_else(|| DEFAULT_ONBOARDING_FLAG_NAME.to_string());
                let trimmed = name.trim();
                if trimmed.is_empty()
                    || trimmed.contains('/')
                    || trimmed.contains('\\')
                    || trimmed.contains("..")
                {
                    return Err("Invalid onboarding flag name".to_string());
                }

                let workspace_dir = match load_openhuman_config().await {
                    Ok(cfg) => cfg.workspace_dir,
                    Err(_) => default_workspace_dir(),
                };
                InvocationResult::ok(workspace_dir.join(trimmed).is_file())
            }
            .await,
        ),

        "openhuman.agent_server_status" => {
            let payload = json!({
                "running": true,
                "url": crate::core_server::helpers::core_rpc_url(),
            });
            Some(Ok(InvocationResult::with_logs(
                payload,
                vec!["agent server status checked".to_string()],
            )
            .unwrap()))
        }

        _ => None,
    }
}
