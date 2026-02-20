//! Tauri commands for the alphahuman subsystem.

use crate::alphahuman::config::Config;
use crate::alphahuman::health;
use crate::alphahuman::security::{SecretStore, SecurityPolicy};
use crate::alphahuman::{doctor, hardware, integrations, migration, onboard, service};
use serde::{Deserialize, Serialize};
use tauri::Manager;

#[derive(Debug, Clone, Serialize)]
pub struct CommandResponse<T> {
    pub result: T,
    pub logs: Vec<String>,
}

fn command_response<T>(result: T, logs: Vec<String>) -> CommandResponse<T> {
    CommandResponse { result, logs }
}

/// Return the current health snapshot as JSON.
#[tauri::command]
pub fn alphahuman_health_snapshot() -> CommandResponse<serde_json::Value> {
    log::info!("[alphahuman:cmd] health_snapshot called");
    let logs = vec!["health_snapshot requested".to_string()];
    command_response(health::snapshot_json(), logs)
}

/// Return the default security policy info (autonomy config summary).
#[tauri::command]
pub fn alphahuman_security_policy_info() -> CommandResponse<serde_json::Value> {
    log::info!("[alphahuman:cmd] security_policy_info called");
    let policy = SecurityPolicy::default();
    let payload = serde_json::json!({
        "autonomy": policy.autonomy,
        "workspace_only": policy.workspace_only,
        "allowed_commands": policy.allowed_commands,
        "max_actions_per_hour": policy.max_actions_per_hour,
        "require_approval_for_medium_risk": policy.require_approval_for_medium_risk,
        "block_high_risk_commands": policy.block_high_risk_commands,
    });
    let logs = vec!["security_policy_info computed".to_string()];
    command_response(payload, logs)
}

/// Encrypt a secret using the alphahuman SecretStore.
#[tauri::command]
pub fn alphahuman_encrypt_secret(
    app: tauri::AppHandle,
    plaintext: String,
) -> Result<CommandResponse<String>, String> {
    log::info!("[alphahuman:cmd] encrypt_secret called");
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("alphahuman");
    let store = SecretStore::new(&data_dir, true);
    store.encrypt(&plaintext).map(|ciphertext| {
        command_response(ciphertext, vec!["secret encrypted".to_string()])
    }).map_err(|e| {
        log::error!("[alphahuman:cmd] encrypt_secret failed: {}", e);
        e.to_string()
    })
}

/// Decrypt a secret using the alphahuman SecretStore.
#[tauri::command]
pub fn alphahuman_decrypt_secret(
    app: tauri::AppHandle,
    ciphertext: String,
) -> Result<CommandResponse<String>, String> {
    log::info!("[alphahuman:cmd] decrypt_secret called");
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("alphahuman");
    let store = SecretStore::new(&data_dir, true);
    store.decrypt(&ciphertext).map(|plaintext| {
        command_response(plaintext, vec!["secret decrypted".to_string()])
    }).map_err(|e| {
        log::error!("[alphahuman:cmd] decrypt_secret failed: {}", e);
        e.to_string()
    })
}

async fn load_alphahuman_config() -> Result<Config, String> {
    log::info!("[alphahuman:cmd] load_config called");
    Config::load_or_init().await.map_err(|e| {
        log::error!("[alphahuman:cmd] load config failed: {}", e);
        e.to_string()
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigSnapshot {
    pub config: serde_json::Value,
    pub workspace_dir: String,
    pub config_path: String,
}

fn snapshot_config(config: &Config) -> Result<ConfigSnapshot, String> {
    let value = serde_json::to_value(config).map_err(|e| e.to_string())?;
    Ok(ConfigSnapshot {
        config: value,
        workspace_dir: config.workspace_dir.display().to_string(),
        config_path: config.config_path.display().to_string(),
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelSettingsUpdate {
    pub api_key: Option<String>,
    pub api_url: Option<String>,
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub default_temperature: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemorySettingsUpdate {
    pub backend: Option<String>,
    pub auto_save: Option<bool>,
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_dimensions: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewaySettingsUpdate {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub require_pairing: Option<bool>,
    pub allow_public_bind: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeSettingsUpdate {
    pub kind: Option<String>,
    pub reasoning_enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BrowserSettingsUpdate {
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeFlags {
    pub browser_allow_all: bool,
    pub log_prompts: bool,
}

fn env_flag_enabled(key: &str) -> bool {
    matches!(
        std::env::var(key).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

/// Return the full Alphahuman config snapshot for UI editing.
#[tauri::command]
pub async fn alphahuman_get_config() -> Result<CommandResponse<ConfigSnapshot>, String> {
    log::info!("[alphahuman:cmd] get_config called");
    let config = load_alphahuman_config().await?;
    let snapshot = snapshot_config(&config)?;
    Ok(command_response(
        snapshot,
        vec![format!(
            "config loaded from {}",
            config.config_path.display()
        )],
    ))
}

/// Update model/provider settings.
#[tauri::command]
pub async fn alphahuman_update_model_settings(
    update: ModelSettingsUpdate,
) -> Result<CommandResponse<ConfigSnapshot>, String> {
    log::info!("[alphahuman:cmd] update_model_settings called");
    let mut config = load_alphahuman_config().await?;
    log::info!(
        "[alphahuman:cmd] update_model_settings target config: {}",
        config.config_path.display()
    );
    let ModelSettingsUpdate {
        api_key,
        api_url,
        default_provider,
        default_model,
        default_temperature,
    } = update;
    log::info!(
        "[alphahuman:cmd] update_model_settings apply: api_key={}, api_url={}, default_provider={}, default_model={}, default_temperature={}",
        api_key.as_ref().map(|v| !v.trim().is_empty()).unwrap_or(false),
        api_url.as_ref().map(|v| !v.trim().is_empty()).unwrap_or(false),
        default_provider.as_ref().map(|v| !v.trim().is_empty()).unwrap_or(false),
        default_model.as_ref().map(|v| !v.trim().is_empty()).unwrap_or(false),
        default_temperature.is_some()
    );
    if let Some(api_key) = api_key {
        config.api_key = if api_key.trim().is_empty() {
            None
        } else {
            Some(api_key)
        };
    }
    if let Some(api_url) = api_url {
        config.api_url = if api_url.trim().is_empty() {
            None
        } else {
            Some(api_url)
        };
    }
    if let Some(provider) = default_provider {
        config.default_provider = if provider.trim().is_empty() {
            None
        } else {
            Some(provider)
        };
    }
    if let Some(model) = default_model {
        config.default_model = if model.trim().is_empty() {
            None
        } else {
            Some(model)
        };
    }
    if let Some(temp) = default_temperature {
        config.default_temperature = temp;
    }
    config.save().await.map_err(|e| {
        log::error!("[alphahuman:cmd] update_model_settings save failed: {}", e);
        e.to_string()
    })?;
    log::info!("[alphahuman:cmd] update_model_settings saved");
    let snapshot = snapshot_config(&config)?;
    Ok(command_response(
        snapshot,
        vec![format!(
            "model settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Update memory settings.
#[tauri::command]
pub async fn alphahuman_update_memory_settings(
    update: MemorySettingsUpdate,
) -> Result<CommandResponse<ConfigSnapshot>, String> {
    log::info!("[alphahuman:cmd] update_memory_settings called");
    let mut config = load_alphahuman_config().await?;
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
    config.save().await.map_err(|e| {
        log::error!("[alphahuman:cmd] update_memory_settings save failed: {}", e);
        e.to_string()
    })?;
    log::info!("[alphahuman:cmd] update_memory_settings saved");
    let snapshot = snapshot_config(&config)?;
    Ok(command_response(
        snapshot,
        vec![format!(
            "memory settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Update gateway settings.
#[tauri::command]
pub async fn alphahuman_update_gateway_settings(
    update: GatewaySettingsUpdate,
) -> Result<CommandResponse<ConfigSnapshot>, String> {
    log::info!("[alphahuman:cmd] update_gateway_settings called");
    let mut config = load_alphahuman_config().await?;
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
    config.save().await.map_err(|e| {
        log::error!("[alphahuman:cmd] update_gateway_settings save failed: {}", e);
        e.to_string()
    })?;
    log::info!("[alphahuman:cmd] update_gateway_settings saved");
    let snapshot = snapshot_config(&config)?;
    Ok(command_response(
        snapshot,
        vec![format!(
            "gateway settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Update tunnel settings (full tunnel config).
#[tauri::command]
pub async fn alphahuman_update_tunnel_settings(
    tunnel: crate::alphahuman::config::TunnelConfig,
) -> Result<CommandResponse<ConfigSnapshot>, String> {
    log::info!("[alphahuman:cmd] update_tunnel_settings called");
    let mut config = load_alphahuman_config().await?;
    config.tunnel = tunnel;
    config.save().await.map_err(|e| {
        log::error!("[alphahuman:cmd] update_tunnel_settings save failed: {}", e);
        e.to_string()
    })?;
    log::info!("[alphahuman:cmd] update_tunnel_settings saved");
    let snapshot = snapshot_config(&config)?;
    Ok(command_response(
        snapshot,
        vec![format!(
            "tunnel settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Update runtime settings (skill execution backend).
#[tauri::command]
pub async fn alphahuman_update_runtime_settings(
    update: RuntimeSettingsUpdate,
) -> Result<CommandResponse<ConfigSnapshot>, String> {
    log::info!("[alphahuman:cmd] update_runtime_settings called");
    let mut config = load_alphahuman_config().await?;
    if let Some(kind) = update.kind {
        config.runtime.kind = kind;
    }
    if let Some(reasoning_enabled) = update.reasoning_enabled {
        config.runtime.reasoning_enabled = Some(reasoning_enabled);
    }
    config.save().await.map_err(|e| {
        log::error!("[alphahuman:cmd] update_runtime_settings save failed: {}", e);
        e.to_string()
    })?;
    log::info!("[alphahuman:cmd] update_runtime_settings saved");
    let snapshot = snapshot_config(&config)?;
    Ok(command_response(
        snapshot,
        vec![format!(
            "runtime settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Update browser settings (Chrome/Chromium tool).
#[tauri::command]
pub async fn alphahuman_update_browser_settings(
    update: BrowserSettingsUpdate,
) -> Result<CommandResponse<ConfigSnapshot>, String> {
    log::info!("[alphahuman:cmd] update_browser_settings called");
    log::debug!("[alphahuman:cmd] update_browser_settings requested");
    let mut config = load_alphahuman_config().await?;
    if let Some(enabled) = update.enabled {
        config.browser.enabled = enabled;
    }
    config.save().await.map_err(|e| {
        log::error!("[alphahuman:cmd] update_browser_settings save failed: {}", e);
        e.to_string()
    })?;
    log::info!("[alphahuman:cmd] update_browser_settings saved");
    let snapshot = snapshot_config(&config)?;
    Ok(command_response(
        snapshot,
        vec![format!(
            "browser settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Read runtime flags that are controlled via environment variables.
#[tauri::command]
pub fn alphahuman_get_runtime_flags() -> CommandResponse<RuntimeFlags> {
    log::info!("[alphahuman:cmd] get_runtime_flags called");
    let flags = RuntimeFlags {
        browser_allow_all: env_flag_enabled("ALPHAHUMAN_BROWSER_ALLOW_ALL"),
        log_prompts: env_flag_enabled("ALPHAHUMAN_LOG_PROMPTS"),
    };
    command_response(flags, vec!["runtime flags read".to_string()])
}

/// Set browser allow-all flag for the current process.
#[tauri::command]
pub fn alphahuman_set_browser_allow_all(
    enabled: bool,
) -> CommandResponse<RuntimeFlags> {
    log::info!("[alphahuman:cmd] set_browser_allow_all called");
    if enabled {
        std::env::set_var("ALPHAHUMAN_BROWSER_ALLOW_ALL", "1");
    } else {
        std::env::remove_var("ALPHAHUMAN_BROWSER_ALLOW_ALL");
    }
    let flags = RuntimeFlags {
        browser_allow_all: env_flag_enabled("ALPHAHUMAN_BROWSER_ALLOW_ALL"),
        log_prompts: env_flag_enabled("ALPHAHUMAN_LOG_PROMPTS"),
    };
    command_response(flags, vec!["browser allow-all flag updated".to_string()])
}

/// Send a single message to the Alphahuman agent and return the response text.
#[tauri::command]
pub async fn alphahuman_agent_chat(
    message: String,
    provider_override: Option<String>,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<CommandResponse<String>, String> {
    log::info!("[alphahuman:cmd] agent_chat called");
    let mut config = load_alphahuman_config().await?;
    if let Some(provider) = provider_override {
        config.default_provider = Some(provider);
    }
    if let Some(model) = model_override {
        config.default_model = Some(model);
    }
    if let Some(temp) = temperature {
        config.default_temperature = temp;
    }
    let mut agent = crate::alphahuman::agent::Agent::from_config(&config)
        .map_err(|e| {
            log::error!("[alphahuman:cmd] agent_chat build failed: {}", e);
            e.to_string()
        })?;
    let response = agent.run_single(&message).await.map_err(|e| {
        log::error!("[alphahuman:cmd] agent_chat run failed: {}", e);
        e.to_string()
    })?;
    Ok(command_response(
        response,
        vec!["agent chat completed".to_string()],
    ))
}

/// Run Alphahuman doctor checks and return a structured report.
#[tauri::command]
pub async fn alphahuman_doctor_report() -> Result<CommandResponse<doctor::DoctorReport>, String> {
    log::info!("[alphahuman:cmd] doctor_report called");
    let config = load_alphahuman_config().await?;
    doctor::run(&config).map(|report| {
        command_response(report, vec!["doctor report generated".to_string()])
    }).map_err(|e| e.to_string())
}

/// Run model catalog probes for providers.
#[tauri::command]
pub async fn alphahuman_doctor_models(
    provider_override: Option<String>,
    use_cache: Option<bool>,
) -> Result<CommandResponse<doctor::ModelProbeReport>, String> {
    log::info!("[alphahuman:cmd] doctor_models called");
    let config = load_alphahuman_config().await?;
    let use_cache = use_cache.unwrap_or(true);
    doctor::run_models(&config, provider_override.as_deref(), use_cache)
        .map(|report| command_response(report, vec!["model probes completed".to_string()]))
        .map_err(|e| e.to_string())
}

/// List integrations with status for the current config.
#[tauri::command]
pub async fn alphahuman_list_integrations(
) -> Result<CommandResponse<Vec<integrations::IntegrationInfo>>, String> {
    log::info!("[alphahuman:cmd] list_integrations called");
    let config = load_alphahuman_config().await?;
    Ok(command_response(
        integrations::list_integrations(&config),
        vec!["integrations listed".to_string()],
    ))
}

/// Get details for a single integration.
#[tauri::command]
pub async fn alphahuman_get_integration_info(
    name: String,
) -> Result<CommandResponse<integrations::IntegrationInfo>, String> {
    log::info!("[alphahuman:cmd] get_integration_info called");
    let config = load_alphahuman_config().await?;
    integrations::get_integration_info(&config, &name)
        .map(|info| command_response(info, vec![format!("integration loaded: {name}")]))
        .map_err(|e| {
            log::error!("[alphahuman:cmd] get_integration_info failed: {}", e);
            e.to_string()
        })
}

/// Refresh the model catalog for a provider (or default provider).
#[tauri::command]
pub async fn alphahuman_models_refresh(
    provider_override: Option<String>,
    force: Option<bool>,
) -> Result<CommandResponse<onboard::ModelRefreshResult>, String> {
    log::info!("[alphahuman:cmd] models_refresh called");
    let config = load_alphahuman_config().await?;
    let force = force.unwrap_or(false);
    onboard::run_models_refresh(&config, provider_override.as_deref(), force)
        .map(|result| command_response(result, vec!["model refresh completed".to_string()]))
        .map_err(|e| {
            log::error!("[alphahuman:cmd] models_refresh failed: {}", e);
            e.to_string()
        })
}

/// Migrate OpenClaw memory into the current Alphahuman workspace.
#[tauri::command]
pub async fn alphahuman_migrate_openclaw(
    source_workspace: Option<String>,
    dry_run: Option<bool>,
) -> Result<CommandResponse<migration::MigrationReport>, String> {
    log::info!("[alphahuman:cmd] migrate_openclaw called");
    let config = load_alphahuman_config().await?;
    let source = source_workspace.map(std::path::PathBuf::from);
    let dry_run = dry_run.unwrap_or(true);
    migration::migrate_openclaw_memory(&config, source, dry_run)
        .await
        .map(|report| command_response(report, vec!["migration completed".to_string()]))
        .map_err(|e| {
            log::error!("[alphahuman:cmd] migrate_openclaw failed: {}", e);
            e.to_string()
        })
}

/// Discover connected hardware devices (feature-gated).
#[tauri::command]
pub fn alphahuman_hardware_discover() -> CommandResponse<Vec<hardware::DiscoveredDevice>> {
    log::info!("[alphahuman:cmd] hardware_discover called");
    command_response(
        hardware::discover_hardware(),
        vec!["hardware discovery complete".to_string()],
    )
}

/// Introspect a device path (feature-gated).
#[tauri::command]
pub fn alphahuman_hardware_introspect(
    path: String,
) -> Result<CommandResponse<hardware::HardwareIntrospect>, String> {
    log::info!("[alphahuman:cmd] hardware_introspect called");
    hardware::introspect_device(&path)
        .map(|info| command_response(info, vec![format!("introspected {path}")]))
        .map_err(|e| {
        log::error!("[alphahuman:cmd] hardware_introspect failed: {}", e);
        e.to_string()
    })
}

/// Install the Alphahuman daemon service.
#[tauri::command]
pub async fn alphahuman_service_install(
) -> Result<CommandResponse<service::ServiceStatus>, String> {
    log::info!("[alphahuman:cmd] service_install called");
    let config = load_alphahuman_config().await?;
    service::install(&config)
        .map(|status| command_response(status, vec!["service install completed".to_string()]))
        .map_err(|e| {
            log::error!("[alphahuman:cmd] service_install failed: {}", e);
            e.to_string()
        })
}

/// Start the Alphahuman daemon service.
#[tauri::command]
pub async fn alphahuman_service_start() -> Result<CommandResponse<service::ServiceStatus>, String> {
    log::info!("[alphahuman:cmd] service_start called");
    let config = load_alphahuman_config().await?;
    service::start(&config)
        .map(|status| command_response(status, vec!["service start completed".to_string()]))
        .map_err(|e| {
            log::error!("[alphahuman:cmd] service_start failed: {}", e);
            e.to_string()
        })
}

/// Stop the Alphahuman daemon service.
#[tauri::command]
pub async fn alphahuman_service_stop() -> Result<CommandResponse<service::ServiceStatus>, String> {
    log::info!("[alphahuman:cmd] service_stop called");
    let config = load_alphahuman_config().await?;
    service::stop(&config)
        .map(|status| command_response(status, vec!["service stop completed".to_string()]))
        .map_err(|e| {
            log::error!("[alphahuman:cmd] service_stop failed: {}", e);
            e.to_string()
        })
}

/// Get the Alphahuman daemon service status.
#[tauri::command]
pub async fn alphahuman_service_status(
) -> Result<CommandResponse<service::ServiceStatus>, String> {
    log::info!("[alphahuman:cmd] service_status called");
    let config = load_alphahuman_config().await?;
    service::status(&config)
        .map(|status| command_response(status, vec!["service status fetched".to_string()]))
        .map_err(|e| {
            log::error!("[alphahuman:cmd] service_status failed: {}", e);
            e.to_string()
        })
}

/// Uninstall the Alphahuman daemon service.
#[tauri::command]
pub async fn alphahuman_service_uninstall(
) -> Result<CommandResponse<service::ServiceStatus>, String> {
    log::info!("[alphahuman:cmd] service_uninstall called");
    let config = load_alphahuman_config().await?;
    service::uninstall(&config)
        .map(|status| command_response(status, vec!["service uninstall completed".to_string()]))
        .map_err(|e| {
            log::error!("[alphahuman:cmd] service_uninstall failed: {}", e);
            e.to_string()
        })
}
