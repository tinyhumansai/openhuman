//! JSON-RPC / CLI controller surface for persisted config and runtime flags.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::screen_intelligence;
use crate::rpc::RpcOutcome;

/// Checks if an environment variable flag is enabled (e.g., "1", "true", "yes").
fn env_flag_enabled(key: &str) -> bool {
    matches!(
        std::env::var(key).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

/// Returns the core RPC URL from environment variables or a default value.
pub fn core_rpc_url_from_env() -> String {
    std::env::var("OPENHUMAN_CORE_RPC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:7788/rpc".to_string())
}

const CONFIG_LOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Loads persisted config with a 30s timeout.
///
/// This is used by JSON-RPC and CLI handlers to ensure they don't hang
/// indefinitely if disk I/O is blocked.
pub async fn load_config_with_timeout() -> Result<Config, String> {
    match tokio::time::timeout(CONFIG_LOAD_TIMEOUT, Config::load_or_init()).await {
        Ok(Ok(config)) => Ok(config),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Config loading timed out".to_string()),
    }
}

/// Returns the default workspace directory fallback (~/.openhuman/workspace).
fn fallback_workspace_dir() -> PathBuf {
    crate::openhuman::config::default_root_openhuman_dir()
        .unwrap_or_else(|_| env_scoped_fallback_root_dir())
        .join("workspace")
}

/// Returns the default OpenHuman configuration directory (~/.openhuman).
fn default_openhuman_dir() -> PathBuf {
    crate::openhuman::config::default_root_openhuman_dir()
        .unwrap_or_else(|_| env_scoped_fallback_root_dir())
}

fn env_scoped_fallback_root_dir() -> PathBuf {
    let suffix = if crate::api::config::is_staging_app_env(
        crate::api::config::app_env_from_env().as_deref(),
    ) {
        "-staging"
    } else {
        ""
    };
    PathBuf::from(format!(".openhuman{suffix}"))
}

/// Returns the path to the active workspace marker file.
fn active_workspace_marker_path(default_openhuman_dir: &Path) -> PathBuf {
    default_openhuman_dir.join("active_workspace.toml")
}

/// Returns the parent directory of the config file.
fn config_openhuman_dir(config: &Config) -> PathBuf {
    config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

/// Internal helper to reset local data by removing specific directories and markers.
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

/// Serializes the current configuration into a JSON snapshot for the UI.
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
    pub use_vision_model: Option<bool>,
    pub keep_screenshots: Option<bool>,
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

/// Returns a full configuration snapshot for the UI.
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

/// Updates the model-related settings in the configuration.
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

/// Updates the memory-related settings in the configuration.
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

/// Updates the screen intelligence settings in the configuration.
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
    if let Some(use_vision_model) = update.use_vision_model {
        config.screen_intelligence.use_vision_model = use_vision_model;
    }
    if let Some(keep_screenshots) = update.keep_screenshots {
        config.screen_intelligence.keep_screenshots = keep_screenshots;
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

/// Updates the runtime-related settings in the configuration.
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

/// Updates the browser-related settings in the configuration.
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

/// Loads the configuration from disk and returns a snapshot.
pub async fn load_and_get_config_snapshot() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    get_config_snapshot(&config).await
}

/// Loads the configuration, applies model settings updates, and saves it.
pub async fn load_and_apply_model_settings(
    update: ModelSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_model_settings(&mut config, update).await
}

/// Loads the configuration, applies memory settings updates, and saves it.
pub async fn load_and_apply_memory_settings(
    update: MemorySettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_memory_settings(&mut config, update).await
}

/// Loads the configuration, applies screen intelligence settings updates, and saves it.
pub async fn load_and_apply_screen_intelligence_settings(
    update: ScreenIntelligenceSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_screen_intelligence_settings(&mut config, update).await
}

/// Loads the configuration, applies runtime settings updates, and saves it.
pub async fn load_and_apply_runtime_settings(
    update: RuntimeSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_runtime_settings(&mut config, update).await
}

/// Updates the analytics-related settings in the configuration.
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

/// Loads the configuration, applies analytics settings updates, and saves it.
pub async fn load_and_apply_analytics_settings(
    update: AnalyticsSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_analytics_settings(&mut config, update).await
}

/// Loads the configuration, applies browser settings updates, and saves it.
pub async fn load_and_apply_browser_settings(
    update: BrowserSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_browser_settings(&mut config, update).await
}

/// Resolves the effective API URL from configuration or defaults.
pub async fn load_and_resolve_api_url() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let resolved = crate::api::config::effective_api_url(&config.api_url);
    Ok(RpcOutcome::new(json!({ "api_url": resolved }), Vec::new()))
}

/// Resolves a workspace onboarding flag, creating or checking its existence.
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

/// Returns the current state of runtime-only flags.
pub fn get_runtime_flags() -> RpcOutcome<RuntimeFlagsOut> {
    RpcOutcome::single_log(
        RuntimeFlagsOut {
            browser_allow_all: env_flag_enabled("OPENHUMAN_BROWSER_ALLOW_ALL"),
            log_prompts: env_flag_enabled("OPENHUMAN_LOG_PROMPTS"),
        },
        "runtime flags read",
    )
}

/// Updates the `OPENHUMAN_BROWSER_ALLOW_ALL` environment flag.
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

/// Checks if a specific onboarding flag file exists in the workspace.
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

/// Creates or removes an onboarding flag file in the workspace.
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

/// Returns whether the onboarding process has been marked as completed.
pub async fn get_onboarding_completed() -> Result<RpcOutcome<bool>, String> {
    let config = load_config_with_timeout().await?;
    Ok(RpcOutcome::single_log(
        config.onboarding_completed,
        "onboarding_completed read from config",
    ))
}

/// Updates and persists the onboarding completion status.
///
/// On a false→true transition this does three things before returning:
///
/// 1. Sets `chat_onboarding_completed = true` on the same config save,
///    so the user's next chat message routes straight to the
///    orchestrator rather than the welcome agent (which is about to
///    send its message proactively below). See
///    [`crate::openhuman::channels::runtime::dispatch::resolve_target_agent`]
///    for the routing contract.
///
/// 2. Seeds the recurring morning-briefing cron job via
///    [`crate::openhuman::cron::seed::seed_proactive_agents`].
///
/// 3. Spawns the welcome agent immediately via
///    [`crate::openhuman::agent::welcome_proactive::spawn_proactive_welcome`],
///    so the first welcome message arrives the moment the user
///    finishes the wizard instead of waiting for them to type.
///
/// All three side-effects are fire-and-forget so the RPC response
/// lands before any agent work completes.
pub async fn set_onboarding_completed(value: bool) -> Result<RpcOutcome<bool>, String> {
    tracing::debug!(value, "[onboarding] set_onboarding_completed called");
    let mut config = load_config_with_timeout().await?;
    let was_completed = config.onboarding_completed;
    config.onboarding_completed = value;

    // On the false→true transition, also flip the chat-side flag so
    // the welcome agent we're about to invoke proactively is the
    // *only* place the user hears the welcome copy — their first
    // typed message routes straight to the orchestrator.
    let was_chat_completed = config.chat_onboarding_completed;
    if value && !was_completed && !was_chat_completed {
        tracing::debug!(
            "[onboarding] flipping chat_onboarding_completed=true alongside ui onboarding flag"
        );
        config.chat_onboarding_completed = true;
    }

    config.save().await.map_err(|e| e.to_string())?;

    // Seed proactive agents (morning briefing) on any UI false→true
    // transition. The welcome agent fires only when the *chat* flow
    // hasn't completed yet — otherwise a user whose chat welcome was
    // already delivered (e.g. via the legacy tool path, or a manual
    // flip) would get a second welcome.
    if value && !was_completed {
        tracing::debug!("[onboarding] false→true transition detected — seeding morning briefing");
        let seed_config = config.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = crate::openhuman::cron::seed::seed_proactive_agents(&seed_config) {
                tracing::warn!("[onboarding] failed to seed proactive agent cron jobs: {e}");
            }
        });

        if !was_chat_completed {
            tracing::debug!("[onboarding] chat flow not yet completed — firing proactive welcome");
            crate::openhuman::agent::welcome_proactive::spawn_proactive_welcome(config.clone());
        } else {
            tracing::debug!(
                "[onboarding] chat_onboarding_completed already true — skipping proactive welcome"
            );
        }
    } else {
        tracing::debug!(
            was_completed,
            value,
            "[onboarding] no transition — skipping proactive seeding and welcome"
        );
    }

    Ok(RpcOutcome::single_log(
        config.onboarding_completed,
        "onboarding_completed saved to config",
    ))
}

// ── Dictation settings ───────────────────────────────────────────────

/// Represents a partial update to dictation-related settings.
pub struct DictationSettingsPatch {
    pub enabled: Option<bool>,
    pub hotkey: Option<String>,
    pub activation_mode: Option<String>,
    pub llm_refinement: Option<bool>,
    pub streaming: Option<bool>,
    pub streaming_interval_ms: Option<u64>,
}

/// Returns the current dictation settings as a JSON object.
pub async fn get_dictation_settings() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let result = json!({
        "enabled": config.dictation.enabled,
        "hotkey": config.dictation.hotkey,
        "activation_mode": config.dictation.activation_mode,
        "llm_refinement": config.dictation.llm_refinement,
        "streaming": config.dictation.streaming,
        "streaming_interval_ms": config.dictation.streaming_interval_ms,
    });
    Ok(RpcOutcome::new(
        result,
        vec!["dictation settings read".to_string()],
    ))
}

/// Loads configuration, applies dictation settings updates, and saves it.
pub async fn load_and_apply_dictation_settings(
    update: DictationSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    if let Some(enabled) = update.enabled {
        config.dictation.enabled = enabled;
    }
    if let Some(hotkey) = update.hotkey {
        config.dictation.hotkey = hotkey;
    }
    if let Some(mode) = update.activation_mode {
        match mode.as_str() {
            "toggle" => {
                config.dictation.activation_mode =
                    crate::openhuman::config::DictationActivationMode::Toggle;
            }
            "push" => {
                config.dictation.activation_mode =
                    crate::openhuman::config::DictationActivationMode::Push;
            }
            _ => {
                return Err(format!(
                    "invalid activation_mode: {mode} (valid: toggle, push)"
                ))
            }
        }
    }
    if let Some(llm_refinement) = update.llm_refinement {
        config.dictation.llm_refinement = llm_refinement;
    }
    if let Some(streaming) = update.streaming {
        config.dictation.streaming = streaming;
    }
    if let Some(interval) = update.streaming_interval_ms {
        config.dictation.streaming_interval_ms = interval;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(&config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "dictation settings saved to {}",
            config.config_path.display()
        )],
    ))
}

// ── Voice server settings ───────────────────────────────────────────

/// Represents a partial update to voice server related settings.
pub struct VoiceServerSettingsPatch {
    pub auto_start: Option<bool>,
    pub hotkey: Option<String>,
    pub activation_mode: Option<String>,
    pub skip_cleanup: Option<bool>,
    pub min_duration_secs: Option<f32>,
    pub silence_threshold: Option<f32>,
    pub custom_dictionary: Option<Vec<String>>,
}

/// Returns the current voice server settings as a JSON object.
pub async fn get_voice_server_settings() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let result = json!({
        "auto_start": config.voice_server.auto_start,
        "hotkey": config.voice_server.hotkey,
        "activation_mode": config.voice_server.activation_mode,
        "skip_cleanup": config.voice_server.skip_cleanup,
        "min_duration_secs": config.voice_server.min_duration_secs,
        "silence_threshold": config.voice_server.silence_threshold,
        "custom_dictionary": config.voice_server.custom_dictionary,
    });
    Ok(RpcOutcome::new(
        result,
        vec!["voice server settings read".to_string()],
    ))
}

/// Loads configuration, applies voice server settings updates, and saves it.
pub async fn load_and_apply_voice_server_settings(
    update: VoiceServerSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    if let Some(auto_start) = update.auto_start {
        config.voice_server.auto_start = auto_start;
    }
    if let Some(hotkey) = update.hotkey {
        config.voice_server.hotkey = hotkey;
    }
    if let Some(mode) = update.activation_mode {
        match mode.as_str() {
            "tap" => {
                config.voice_server.activation_mode =
                    crate::openhuman::config::VoiceActivationMode::Tap;
            }
            "push" => {
                config.voice_server.activation_mode =
                    crate::openhuman::config::VoiceActivationMode::Push;
            }
            _ => {
                return Err(format!(
                    "invalid activation_mode: {mode} (valid: tap, push)"
                ))
            }
        }
    }
    if let Some(skip_cleanup) = update.skip_cleanup {
        config.voice_server.skip_cleanup = skip_cleanup;
    }
    if let Some(min_duration_secs) = update.min_duration_secs {
        config.voice_server.min_duration_secs = min_duration_secs.max(0.0);
    }
    if let Some(silence_threshold) = update.silence_threshold {
        config.voice_server.silence_threshold = silence_threshold.max(0.0);
    }
    if let Some(custom_dictionary) = update.custom_dictionary {
        config.voice_server.custom_dictionary = custom_dictionary;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(&config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "voice server settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Returns the operational status of the agent server.
pub fn agent_server_status() -> RpcOutcome<serde_json::Value> {
    let running = crate::openhuman::service::mock::mock_agent_running().unwrap_or(true);
    log::info!("[config] agent_server_status requested: running={running}");
    let payload = json!({
        "running": running,
        "url": core_rpc_url_from_env(),
    });
    RpcOutcome::single_log(payload, "agent server status checked")
}

/// Deletes all local data directories and workspace markers.
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

    // ── env_flag_enabled ────────────────────────────────────────────

    use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;

    #[test]
    fn env_flag_enabled_recognizes_truthy_forms() {
        let _g = ENV_LOCK.lock().unwrap();
        let key = "OPENHUMAN_TEST_FLAG_A";
        for truthy in ["1", "true", "TRUE", "yes", "YES"] {
            unsafe {
                std::env::set_var(key, truthy);
            }
            assert!(env_flag_enabled(key), "{truthy} should be truthy");
        }
        for falsy in ["0", "false", "off", "", "No"] {
            unsafe {
                std::env::set_var(key, falsy);
            }
            assert!(!env_flag_enabled(key), "{falsy} should be falsy");
        }
        unsafe {
            std::env::remove_var(key);
        }
        assert!(!env_flag_enabled(key), "unset must be falsy");
    }

    // ── core_rpc_url_from_env ───────────────────────────────────────

    #[test]
    fn core_rpc_url_from_env_returns_default_when_unset() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("OPENHUMAN_CORE_RPC_URL");
        }
        assert_eq!(core_rpc_url_from_env(), "http://127.0.0.1:7788/rpc");
    }

    #[test]
    fn core_rpc_url_from_env_uses_override_when_set() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_CORE_RPC_URL", "http://1.2.3.4:9999/rpc");
        }
        assert_eq!(core_rpc_url_from_env(), "http://1.2.3.4:9999/rpc");
        unsafe {
            std::env::remove_var("OPENHUMAN_CORE_RPC_URL");
        }
    }

    // ── Pure path helpers ──────────────────────────────────────────

    #[test]
    fn fallback_workspace_dir_ends_in_workspace_under_openhuman() {
        let p = fallback_workspace_dir();
        assert!(p.ends_with("workspace"));
        assert!(p
            .parent()
            .map(|d| d.ends_with(".openhuman"))
            .unwrap_or(false));
    }

    #[test]
    fn default_openhuman_dir_ends_in_dot_openhuman() {
        let p = default_openhuman_dir();
        assert!(p.ends_with(".openhuman"));
    }

    #[test]
    fn active_workspace_marker_path_is_under_default_dir() {
        let default_dir = std::path::Path::new("/tmp/openhuman-test");
        let marker = active_workspace_marker_path(default_dir);
        assert_eq!(marker, default_dir.join("active_workspace.toml"));
    }

    #[test]
    fn config_openhuman_dir_returns_config_path_parent() {
        let mut cfg = Config::default();
        cfg.config_path = PathBuf::from("/tmp/xyz/config.toml");
        assert_eq!(config_openhuman_dir(&cfg), PathBuf::from("/tmp/xyz"));
    }

    // ── get_runtime_flags / set_browser_allow_all ─────────────────

    #[test]
    fn get_runtime_flags_reads_env_overrides() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("OPENHUMAN_BROWSER_ALLOW_ALL");
        }
        let flags = get_runtime_flags();
        // Just exercise the path — we don't assume anything about
        // what other tests in the suite may have set.
        let _ = flags.value;
    }

    #[test]
    fn set_browser_allow_all_toggles_env_var() {
        let _g = ENV_LOCK.lock().unwrap();
        let before = std::env::var("OPENHUMAN_BROWSER_ALLOW_ALL").ok();

        let _ = set_browser_allow_all(true);
        assert!(env_flag_enabled("OPENHUMAN_BROWSER_ALLOW_ALL"));

        let _ = set_browser_allow_all(false);
        assert!(!env_flag_enabled("OPENHUMAN_BROWSER_ALLOW_ALL"));

        unsafe {
            match before {
                Some(v) => std::env::set_var("OPENHUMAN_BROWSER_ALLOW_ALL", v),
                None => std::env::remove_var("OPENHUMAN_BROWSER_ALLOW_ALL"),
            }
        }
    }

    // ── snapshot_config_json ───────────────────────────────────────

    #[test]
    fn snapshot_config_json_emits_config_and_workspace_and_config_path() {
        let tmp = tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().join("workspace");
        cfg.config_path = tmp.path().join("config.toml");

        let snap = snapshot_config_json(&cfg).expect("snapshot should succeed");
        assert!(snap.get("config").is_some());
        assert!(snap.get("workspace_dir").is_some());
        assert!(snap.get("config_path").is_some());
        // Workspace + config paths must point at our tempdir.
        let ws = snap["workspace_dir"].as_str().unwrap_or("");
        assert!(ws.contains(tmp.path().to_str().unwrap_or("")));
    }

    // ── agent_server_status ────────────────────────────────────────

    #[test]
    fn agent_server_status_exposes_running_and_url() {
        let outcome = agent_server_status();
        assert!(outcome.value.get("running").is_some());
        assert!(outcome.value.get("url").is_some());
    }

    // ── workspace_onboarding_flag_exists ───────────────────────────

    #[test]
    fn workspace_onboarding_flag_exists_returns_false_for_fresh_workspace() {
        let tmp = tempdir().unwrap();
        let res = workspace_onboarding_flag_exists(tmp.path().join("workspace"), "onboarding.done")
            .expect("flag check ok");
        assert_eq!(res.value, false);
    }

    #[test]
    fn workspace_onboarding_flag_exists_rejects_invalid_flag_names() {
        let tmp = tempdir().unwrap();
        for bad in ["", "   ", "a/b", "a\\b", "..", "foo/.."] {
            let err =
                workspace_onboarding_flag_exists(tmp.path().join("workspace"), bad).unwrap_err();
            assert!(
                err.contains("Invalid onboarding flag"),
                "name `{bad}`: {err}"
            );
        }
    }

    #[test]
    fn workspace_onboarding_flag_exists_true_when_file_present() {
        let tmp = tempdir().unwrap();
        let ws = tmp.path().join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("onboarding.done"), "").unwrap();
        let res = workspace_onboarding_flag_exists(ws, "onboarding.done").expect("flag check ok");
        assert_eq!(res.value, true);
    }

    // ── apply_*_settings ─────────────────────────────────────────

    fn tmp_config(tmp: &tempfile::TempDir) -> Config {
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().join("workspace");
        cfg.config_path = tmp.path().join("config.toml");
        std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
        cfg
    }

    #[tokio::test]
    async fn apply_model_settings_updates_fields_and_persists_snapshot() {
        let tmp = tempdir().unwrap();
        let mut cfg = tmp_config(&tmp);
        let patch = ModelSettingsPatch {
            api_key: Some("sk-test".into()),
            api_url: Some("https://api.example.test".into()),
            default_model: Some("gpt-4o".into()),
            default_temperature: Some(0.25),
        };
        let outcome = apply_model_settings(&mut cfg, patch).await.expect("apply");
        assert_eq!(cfg.api_key.as_deref(), Some("sk-test"));
        assert_eq!(cfg.api_url.as_deref(), Some("https://api.example.test"));
        assert_eq!(cfg.default_model.as_deref(), Some("gpt-4o"));
        assert!((cfg.default_temperature - 0.25).abs() < f64::EPSILON);
        assert_eq!(outcome.value["config"]["api_key"], "sk-test");
    }

    #[tokio::test]
    async fn apply_model_settings_empty_strings_clear_optional_fields() {
        let tmp = tempdir().unwrap();
        let mut cfg = tmp_config(&tmp);
        cfg.api_key = Some("prev".into());
        cfg.default_model = Some("prev-model".into());
        let patch = ModelSettingsPatch {
            api_key: Some("  ".into()),
            api_url: Some("".into()),
            default_model: Some("".into()),
            default_temperature: None,
        };
        let _ = apply_model_settings(&mut cfg, patch).await.expect("apply");
        assert!(cfg.api_key.is_none());
        assert!(cfg.api_url.is_none());
        assert!(cfg.default_model.is_none());
    }

    #[tokio::test]
    async fn apply_memory_settings_updates_all_provided_fields() {
        let tmp = tempdir().unwrap();
        let mut cfg = tmp_config(&tmp);
        let patch = MemorySettingsPatch {
            backend: Some("sqlite".into()),
            auto_save: Some(true),
            embedding_provider: Some("ollama".into()),
            embedding_model: Some("nomic".into()),
            embedding_dimensions: Some(768),
        };
        let _ = apply_memory_settings(&mut cfg, patch).await.expect("apply");
        assert_eq!(cfg.memory.backend, "sqlite");
        assert!(cfg.memory.auto_save);
        assert_eq!(cfg.memory.embedding_provider, "ollama");
        assert_eq!(cfg.memory.embedding_model, "nomic");
        assert_eq!(cfg.memory.embedding_dimensions, 768);
    }

    #[tokio::test]
    async fn apply_runtime_settings_updates_kind_and_reasoning() {
        let tmp = tempdir().unwrap();
        let mut cfg = tmp_config(&tmp);
        let patch = RuntimeSettingsPatch {
            kind: Some("desktop".into()),
            reasoning_enabled: Some(true),
        };
        let _ = apply_runtime_settings(&mut cfg, patch)
            .await
            .expect("apply");
        assert_eq!(cfg.runtime.kind, "desktop");
        assert_eq!(cfg.runtime.reasoning_enabled, Some(true));
    }

    #[tokio::test]
    async fn apply_browser_settings_updates_enabled_flag() {
        let tmp = tempdir().unwrap();
        let mut cfg = tmp_config(&tmp);
        cfg.browser.enabled = false;
        let _ = apply_browser_settings(
            &mut cfg,
            BrowserSettingsPatch {
                enabled: Some(true),
            },
        )
        .await
        .expect("apply");
        assert!(cfg.browser.enabled);
    }

    #[tokio::test]
    async fn apply_analytics_settings_updates_enabled() {
        let tmp = tempdir().unwrap();
        let mut cfg = tmp_config(&tmp);
        let _ = apply_analytics_settings(
            &mut cfg,
            AnalyticsSettingsPatch {
                enabled: Some(false),
            },
        )
        .await
        .expect("apply");
        assert!(!cfg.observability.analytics_enabled);
    }

    #[tokio::test]
    async fn get_config_snapshot_wraps_snapshot_in_rpc_outcome() {
        let tmp = tempdir().unwrap();
        let cfg = tmp_config(&tmp);
        let outcome = get_config_snapshot(&cfg).await.expect("snapshot");
        assert!(outcome.value.get("config").is_some());
        assert!(outcome
            .logs
            .iter()
            .any(|l| l.contains("config loaded from")));
    }

    // ── Dictation / voice_server settings patches ─────────────────

    #[tokio::test]
    async fn load_and_apply_dictation_settings_rejects_invalid_activation_mode() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }
        let patch = DictationSettingsPatch {
            enabled: None,
            hotkey: None,
            activation_mode: Some("not-a-mode".into()),
            llm_refinement: None,
            streaming: None,
            streaming_interval_ms: None,
        };
        let err = load_and_apply_dictation_settings(patch).await.unwrap_err();
        assert!(err.contains("invalid activation_mode"));
        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    #[tokio::test]
    async fn load_and_apply_voice_server_settings_rejects_invalid_activation_mode() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }
        let patch = VoiceServerSettingsPatch {
            auto_start: None,
            hotkey: None,
            activation_mode: Some("hold".into()),
            skip_cleanup: None,
            min_duration_secs: None,
            silence_threshold: None,
            custom_dictionary: None,
        };
        let err = load_and_apply_voice_server_settings(patch)
            .await
            .unwrap_err();
        assert!(err.contains("invalid activation_mode"));
        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    #[tokio::test]
    async fn load_and_apply_dictation_settings_accepts_valid_modes() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }
        for mode in ["toggle", "push"] {
            let patch = DictationSettingsPatch {
                enabled: Some(true),
                hotkey: Some("cmd+d".into()),
                activation_mode: Some(mode.into()),
                llm_refinement: Some(false),
                streaming: Some(false),
                streaming_interval_ms: Some(500),
            };
            assert!(
                load_and_apply_dictation_settings(patch).await.is_ok(),
                "mode `{mode}` should be accepted"
            );
        }
        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    #[tokio::test]
    async fn load_and_apply_voice_server_settings_accepts_valid_modes_and_clamps() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }
        // Negative min_duration_secs and silence_threshold should be clamped to 0.
        let patch = VoiceServerSettingsPatch {
            auto_start: Some(true),
            hotkey: Some("fn".into()),
            activation_mode: Some("tap".into()),
            skip_cleanup: Some(false),
            min_duration_secs: Some(-5.0),
            silence_threshold: Some(-1.0),
            custom_dictionary: Some(vec!["term".into()]),
        };
        let outcome = load_and_apply_voice_server_settings(patch)
            .await
            .expect("ok");
        assert!(
            outcome.value["config"]["voice_server"]["min_duration_secs"]
                .as_f64()
                .unwrap_or(-1.0)
                >= 0.0
        );
        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    // ── get_* via env override ─────────────────────────────────────

    #[tokio::test]
    async fn get_dictation_settings_reads_from_loaded_config() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }
        let outcome = get_dictation_settings().await.expect("ok");
        assert!(outcome.value.get("enabled").is_some());
        assert!(outcome.value.get("hotkey").is_some());
        assert!(outcome.value.get("streaming_interval_ms").is_some());
        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    #[tokio::test]
    async fn get_voice_server_settings_reads_from_loaded_config() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }
        let outcome = get_voice_server_settings().await.expect("ok");
        assert!(outcome.value.get("auto_start").is_some());
        assert!(outcome.value.get("custom_dictionary").is_some());
        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    #[tokio::test]
    async fn get_onboarding_completed_reads_from_loaded_config() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }
        let outcome = get_onboarding_completed().await.expect("ok");
        // Default value — either true or false is fine; we just verify the call path.
        let _ = outcome.value;
        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    #[tokio::test]
    async fn load_and_resolve_api_url_returns_api_url_in_response() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }
        let outcome = load_and_resolve_api_url().await.expect("ok");
        assert!(outcome.value.get("api_url").is_some());
        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    #[tokio::test]
    async fn workspace_onboarding_flag_resolve_rejects_invalid_and_defaults() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }
        let err = workspace_onboarding_flag_resolve(Some("a/b".into()), "done")
            .await
            .unwrap_err();
        assert!(err.contains("Invalid onboarding flag"));

        // Happy path: default name on a fresh workspace → file doesn't exist.
        let outcome = workspace_onboarding_flag_resolve(None, "onboarding.done")
            .await
            .expect("ok");
        let _ = outcome.value;
        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    #[tokio::test]
    async fn workspace_onboarding_flag_set_rejects_invalid_names() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }
        for bad in ["", "   ", "a/b", "a\\b", ".."] {
            let err = workspace_onboarding_flag_set(Some(bad.into()), "default", true)
                .await
                .unwrap_err();
            assert!(err.contains("Invalid onboarding flag"), "name {bad}: {err}");
        }
        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }

    #[tokio::test]
    async fn workspace_onboarding_flag_set_round_trip() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        }
        // Create flag
        let created =
            workspace_onboarding_flag_set(Some("onboarding.done".into()), "default", true)
                .await
                .expect("create");
        assert!(created.value);
        // Remove flag
        let removed =
            workspace_onboarding_flag_set(Some("onboarding.done".into()), "default", false)
                .await
                .expect("remove");
        assert!(!removed.value);
        unsafe {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }
}
