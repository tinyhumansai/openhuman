//! JSON-RPC / CLI controller surface for persisted config and runtime flags.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::screen_intelligence;
use crate::rpc::RpcOutcome;

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

fn default_openhuman_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".openhuman")
}

fn active_workspace_marker_path(default_openhuman_dir: &Path) -> PathBuf {
    default_openhuman_dir.join("active_workspace.toml")
}

fn config_openhuman_dir(config: &Config) -> PathBuf {
    config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

async fn reset_local_data_for_paths(
    current_openhuman_dir: &Path,
    default_openhuman_dir: &Path,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let active_workspace_marker = active_workspace_marker_path(default_openhuman_dir);
    tracing::debug!(
        current_dir = %current_openhuman_dir.display(),
        default_dir = %default_openhuman_dir.display(),
        marker = %active_workspace_marker.display(),
        "[config] reset_local_data: starting"
    );

    let mut removed_paths = Vec::new();

    if active_workspace_marker.exists() {
        tokio::fs::remove_file(&active_workspace_marker)
            .await
            .map_err(|e| format!("Failed to remove active workspace marker: {e}"))?;
        tracing::debug!(
            marker = %active_workspace_marker.display(),
            "[config] reset_local_data: removed active workspace marker"
        );
        removed_paths.push(active_workspace_marker.display().to_string());
    }

    for target_dir in [current_openhuman_dir, default_openhuman_dir] {
        if !target_dir.exists() {
            tracing::debug!(
                dir = %target_dir.display(),
                "[config] reset_local_data: directory already absent"
            );
            continue;
        }

        tokio::fs::remove_dir_all(target_dir)
            .await
            .map_err(|e| format!("Failed to remove {}: {e}", target_dir.display()))?;
        tracing::debug!(
            dir = %target_dir.display(),
            "[config] reset_local_data: removed directory"
        );
        removed_paths.push(target_dir.display().to_string());
    }

    Ok(RpcOutcome::new(
        json!({
            "removed_paths": removed_paths,
            "current_openhuman_dir": current_openhuman_dir.display().to_string(),
            "default_openhuman_dir": default_openhuman_dir.display().to_string(),
        }),
        vec![
            format!(
                "reset local data for active config dir {}",
                current_openhuman_dir.display()
            ),
            format!(
                "removed default data dir {} if present",
                default_openhuman_dir.display()
            ),
        ],
    ))
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

#[derive(Debug, Clone, Default)]
pub struct AnalyticsSettingsPatch {
    pub enabled: Option<bool>,
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

pub async fn load_and_apply_runtime_settings(
    update: RuntimeSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_runtime_settings(&mut config, update).await
}

pub async fn apply_analytics_settings(
    config: &mut Config,
    update: AnalyticsSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(enabled) = update.enabled {
        config.observability.analytics_enabled = enabled;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "analytics settings saved to {}",
            config.config_path.display()
        )],
    ))
}

pub async fn load_and_apply_analytics_settings(
    update: AnalyticsSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_analytics_settings(&mut config, update).await
}

pub async fn load_and_apply_browser_settings(
    update: BrowserSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_browser_settings(&mut config, update).await
}

pub async fn load_and_resolve_api_url() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let resolved = crate::api::config::effective_api_url(&config.api_url);
    Ok(RpcOutcome::new(json!({ "api_url": resolved }), Vec::new()))
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

/// Creates or removes the workspace onboarding flag file.
pub async fn workspace_onboarding_flag_set(
    flag_name: Option<String>,
    default_name: &str,
    value: bool,
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
    let flag_path = workspace_dir.join(trimmed);
    if value {
        if let Some(parent) = flag_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create workspace dir: {e}"))?;
        }
        std::fs::write(&flag_path, "")
            .map_err(|e| format!("Failed to create onboarding flag: {e}"))?;
    } else if flag_path.is_file() {
        std::fs::remove_file(&flag_path)
            .map_err(|e| format!("Failed to remove onboarding flag: {e}"))?;
    }
    Ok(RpcOutcome::single_log(
        flag_path.is_file(),
        "onboarding flag updated",
    ))
}

pub async fn get_onboarding_completed() -> Result<RpcOutcome<bool>, String> {
    let config = load_config_with_timeout().await?;
    Ok(RpcOutcome::single_log(
        config.onboarding_completed,
        "onboarding_completed read from config",
    ))
}

pub async fn set_onboarding_completed(value: bool) -> Result<RpcOutcome<bool>, String> {
    let mut config = load_config_with_timeout().await?;
    config.onboarding_completed = value;
    config.save().await.map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        config.onboarding_completed,
        "onboarding_completed saved to config",
    ))
}

pub fn agent_server_status() -> RpcOutcome<serde_json::Value> {
    let running = crate::openhuman::service::mock::mock_agent_running().unwrap_or(true);
    log::info!("[config] agent_server_status requested: running={running}");
    let payload = json!({
        "running": running,
        "url": core_rpc_url_from_env(),
    });
    RpcOutcome::single_log(payload, "agent server status checked")
}

pub async fn reset_local_data() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let current_openhuman_dir = config_openhuman_dir(&config);
    let default_openhuman_dir = default_openhuman_dir();
    reset_local_data_for_paths(&current_openhuman_dir, &default_openhuman_dir).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn reset_local_data_removes_current_dir_default_dir_and_marker() {
        let temp = tempdir().unwrap();
        let default_openhuman_dir = temp.path().join("default-openhuman");
        let current_openhuman_dir = temp.path().join("custom-openhuman");
        let marker = active_workspace_marker_path(&default_openhuman_dir);

        tokio::fs::create_dir_all(default_openhuman_dir.join("workspace"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(current_openhuman_dir.join("workspace"))
            .await
            .unwrap();
        tokio::fs::write(&marker, "config_dir = '/tmp/custom-openhuman'\n")
            .await
            .unwrap();

        let outcome = reset_local_data_for_paths(&current_openhuman_dir, &default_openhuman_dir)
            .await
            .unwrap();

        assert!(!current_openhuman_dir.exists());
        assert!(!default_openhuman_dir.exists());
        assert!(outcome
            .value
            .get("removed_paths")
            .and_then(|value| value.as_array())
            .is_some_and(|paths| !paths.is_empty()));
    }
}
