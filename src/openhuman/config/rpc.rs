//! JSON-RPC / CLI controller surface for persisted config and runtime flags.

use std::path::PathBuf;

use serde::Serialize;
use serde_json::json;

use crate::openhuman::config::{Config, TunnelConfig};
use crate::openhuman::rpc::RpcOutcome;
use crate::openhuman::screen_intelligence;

fn env_flag_enabled(key: &str) -> bool {
    matches!(
        std::env::var(key).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

pub fn core_rpc_url_from_env() -> String {
    std::env::var("OPENHUMAN_CORE_RPC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:7788/rpc".to_string())
}

const CONFIG_LOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Loads persisted config with the same 30s timeout used by JSON-RPC and the core CLI.
pub async fn load_config_with_timeout() -> Result<Config, String> {
    match tokio::time::timeout(CONFIG_LOAD_TIMEOUT, Config::load_or_init()).await {
        Ok(Ok(config)) => Ok(config),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Config loading timed out".to_string()),
    }
}

fn fallback_workspace_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".openhuman")
        .join("workspace")
}

pub fn snapshot_config_json(config: &Config) -> Result<serde_json::Value, String> {
    let value = serde_json::to_value(config).map_err(|e| e.to_string())?;
    Ok(json!({
        "config": value,
        "workspace_dir": config.workspace_dir.display().to_string(),
        "config_path": config.config_path.display().to_string(),
    }))
}

#[derive(Debug, Clone, Default)]
pub struct ModelSettingsPatch {
    pub api_key: Option<String>,
    pub api_url: Option<String>,
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub default_temperature: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct MemorySettingsPatch {
    pub backend: Option<String>,
    pub auto_save: Option<bool>,
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_dimensions: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeSettingsPatch {
    pub kind: Option<String>,
    pub reasoning_enabled: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct BrowserSettingsPatch {
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct ScreenIntelligenceSettingsPatch {
    pub enabled: Option<bool>,
    pub capture_policy: Option<String>,
    pub policy_mode: Option<String>,
    pub baseline_fps: Option<f32>,
    pub vision_enabled: Option<bool>,
    pub autocomplete_enabled: Option<bool>,
    pub allowlist: Option<Vec<String>>,
    pub denylist: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeFlagsOut {
    pub browser_allow_all: bool,
    pub log_prompts: bool,
}

pub async fn get_config_snapshot(config: &Config) -> Result<RpcOutcome<serde_json::Value>, String> {
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "config loaded from {}",
            config.config_path.display()
        )],
    ))
}

pub async fn apply_model_settings(
    config: &mut Config,
    update: ModelSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
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
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "model settings saved to {}",
            config.config_path.display()
        )],
    ))
}

pub async fn apply_memory_settings(
    config: &mut Config,
    update: MemorySettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
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
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "memory settings saved to {}",
            config.config_path.display()
        )],
    ))
}

pub async fn apply_screen_intelligence_settings(
    config: &mut Config,
    update: ScreenIntelligenceSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
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

    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "screen intelligence settings saved to {}",
            config.config_path.display()
        )],
    ))
}

pub async fn apply_tunnel_settings(
    config: &mut Config,
    tunnel: TunnelConfig,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    config.tunnel = tunnel;
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "tunnel settings saved to {}",
            config.config_path.display()
        )],
    ))
}

pub async fn apply_runtime_settings(
    config: &mut Config,
    update: RuntimeSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(kind) = update.kind {
        config.runtime.kind = kind;
    }
    if let Some(reasoning_enabled) = update.reasoning_enabled {
        config.runtime.reasoning_enabled = Some(reasoning_enabled);
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "runtime settings saved to {}",
            config.config_path.display()
        )],
    ))
}

pub async fn apply_browser_settings(
    config: &mut Config,
    update: BrowserSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(enabled) = update.enabled {
        config.browser.enabled = enabled;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "browser settings saved to {}",
            config.config_path.display()
        )],
    ))
}

pub async fn load_and_get_config_snapshot() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    get_config_snapshot(&config).await
}

pub async fn load_and_apply_model_settings(
    update: ModelSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_model_settings(&mut config, update).await
}

pub async fn load_and_apply_memory_settings(
    update: MemorySettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_memory_settings(&mut config, update).await
}

pub async fn load_and_apply_screen_intelligence_settings(
    update: ScreenIntelligenceSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_screen_intelligence_settings(&mut config, update).await
}

pub async fn load_and_apply_tunnel_settings(
    tunnel: TunnelConfig,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_tunnel_settings(&mut config, tunnel).await
}

pub async fn load_and_apply_runtime_settings(
    update: RuntimeSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_runtime_settings(&mut config, update).await
}

pub async fn load_and_apply_browser_settings(
    update: BrowserSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_browser_settings(&mut config, update).await
}

/// Resolves workspace (load config or fallback), validates flag name, returns whether the flag file exists.
pub async fn workspace_onboarding_flag_resolve(
    flag_name: Option<String>,
    default_name: &str,
) -> Result<RpcOutcome<bool>, String> {
    let name = flag_name.unwrap_or_else(|| default_name.to_string());
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        return Err("Invalid onboarding flag name".to_string());
    }
    let workspace_dir = match load_config_with_timeout().await {
        Ok(cfg) => cfg.workspace_dir,
        Err(_) => fallback_workspace_dir(),
    };
    workspace_onboarding_flag_exists(workspace_dir, trimmed)
}

pub fn get_runtime_flags() -> RpcOutcome<RuntimeFlagsOut> {
    RpcOutcome::single_log(
        RuntimeFlagsOut {
            browser_allow_all: env_flag_enabled("OPENHUMAN_BROWSER_ALLOW_ALL"),
            log_prompts: env_flag_enabled("OPENHUMAN_LOG_PROMPTS"),
        },
        "runtime flags read",
    )
}

pub fn set_browser_allow_all(enabled: bool) -> RpcOutcome<RuntimeFlagsOut> {
    if enabled {
        std::env::set_var("OPENHUMAN_BROWSER_ALLOW_ALL", "1");
    } else {
        std::env::remove_var("OPENHUMAN_BROWSER_ALLOW_ALL");
    }
    let flags = RuntimeFlagsOut {
        browser_allow_all: env_flag_enabled("OPENHUMAN_BROWSER_ALLOW_ALL"),
        log_prompts: env_flag_enabled("OPENHUMAN_LOG_PROMPTS"),
    };
    RpcOutcome::single_log(flags, "browser allow-all flag updated")
}

pub fn workspace_onboarding_flag_exists(
    workspace_dir: PathBuf,
    flag_name: &str,
) -> Result<RpcOutcome<bool>, String> {
    let trimmed = flag_name.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        return Err("Invalid onboarding flag name".to_string());
    }
    Ok(RpcOutcome::single_log(
        workspace_dir.join(trimmed).is_file(),
        "onboarding flag checked",
    ))
}

pub fn agent_server_status() -> RpcOutcome<serde_json::Value> {
    let payload = json!({
        "running": true,
        "url": core_rpc_url_from_env(),
    });
    RpcOutcome::single_log(payload, "agent server status checked")
}
