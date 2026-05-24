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
///
/// The TOML parse itself runs on the blocking pool via
/// `parse_config_with_recovery` (see `src/openhuman/config/schema/load.rs`)
/// so the recursive-descent parser's serde Visitor frames don't compound
/// with whatever deep async tower called us. That's the stack-overflow
/// fix from `crahs.log` (2026-05-17); a per-call cache here would shave
/// the disk read on hot paths but proved racy across the in-process
/// integration tests (re-used workspace paths, concurrent server tasks
/// loading mid-mutation), so it isn't worth it.
pub async fn load_config_with_timeout() -> Result<Config, String> {
    match tokio::time::timeout(CONFIG_LOAD_TIMEOUT, Config::load_or_init()).await {
        Ok(Ok(mut config)) => {
            normalize_loaded_config(&mut config).await;
            Ok(config)
        }
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Config loading timed out".to_string()),
    }
}

/// Reloads the config file represented by an existing runtime snapshot.
///
/// Use this for long-lived objects that need fresh config values while
/// staying anchored to their original user/workspace. Unlike
/// [`load_config_with_timeout`], this does not re-resolve the process-global
/// `OPENHUMAN_WORKSPACE` env var on every call.
pub async fn reload_config_snapshot_with_timeout(snapshot: &Config) -> Result<Config, String> {
    match tokio::time::timeout(
        CONFIG_LOAD_TIMEOUT,
        Config::load_from_config_path(&snapshot.config_path, &snapshot.workspace_dir),
    )
    .await
    {
        Ok(Ok(mut config)) => {
            normalize_loaded_config(&mut config).await;
            Ok(config)
        }
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Config loading timed out".to_string()),
    }
}

async fn normalize_loaded_config(_config: &mut Config) {
    // No-op: welcome-agent routing normalization removed. The welcome agent
    // has been deleted; all chat turns route directly to the orchestrator.
    // The `chat_onboarding_completed` field in Config is retained for
    // backward-compatible deserialization of existing config.toml files
    // but is no longer read by routing logic.
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

fn is_windows_file_lock_error(error: &std::io::Error) -> bool {
    cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33))
}

fn reset_local_data_remove_error(path: &Path, error: &std::io::Error) -> String {
    if is_windows_file_lock_error(error) {
        tracing::warn!(
            path = %path.display(),
            error = %error,
            "[config] reset_local_data: Windows file lock blocked local data deletion"
        );
        return format!(
            "Failed to remove {} because it is locked by another OpenHuman window or process. Close all OpenHuman windows and try again. ({error})",
            path.display()
        );
    }

    format!("Failed to remove {}: {error}", path.display())
}

fn reset_local_data_marker_remove_error(path: &Path, error: &std::io::Error) -> String {
    if is_windows_file_lock_error(error) {
        tracing::warn!(
            marker = %path.display(),
            error = %error,
            "[config] reset_local_data: Windows file lock blocked active workspace marker deletion"
        );
        return format!(
            "Failed to remove active workspace marker {} because it is locked by another OpenHuman window or process. Close all OpenHuman windows and try again. ({error})",
            path.display()
        );
    }

    format!("Failed to remove active workspace marker: {error}")
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
        if let Err(error) = tokio::fs::remove_file(&active_workspace_marker).await {
            return Err(reset_local_data_marker_remove_error(
                &active_workspace_marker,
                &error,
            ));
        }
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

        if let Err(error) = tokio::fs::remove_dir_all(target_dir).await {
            return Err(reset_local_data_remove_error(target_dir, &error));
        }
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

/// Serializes the client-facing AI config slice consumed by the settings UI.
pub fn client_config_json(config: &Config) -> serde_json::Value {
    let app_version =
        std::env::var("OPENHUMAN_APP_VERSION").unwrap_or_else(|_| "unknown".to_string());
    let api_key_set = config
        .api_key
        .as_deref()
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    let model_routes: Vec<serde_json::Value> = config
        .model_routes
        .iter()
        .map(|r| serde_json::json!({ "hint": r.hint, "model": r.model }))
        .collect();
    let cloud_providers: Vec<serde_json::Value> = config
        .cloud_providers
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "slug": c.slug,
                "label": c.label,
                "endpoint": c.endpoint,
                "auth_style": c.auth_style.as_str(),
            })
        })
        .collect();

    serde_json::json!({
        "api_url": config.api_url,
        "inference_url": config.inference_url,
        "default_model": config.default_model,
        "app_version": app_version,
        "api_key_set": api_key_set,
        "model_routes": model_routes,
        "cloud_providers": cloud_providers,
        "primary_cloud": config.primary_cloud,
        "chat_provider": config.chat_provider,
        "reasoning_provider": config.reasoning_provider,
        "agentic_provider": config.agentic_provider,
        "coding_provider": config.coding_provider,
        "memory_provider": config.memory_provider,
        "embeddings_provider": config.embeddings_provider,
        "heartbeat_provider": config.heartbeat_provider,
        "learning_provider": config.learning_provider,
        "subconscious_provider": config.subconscious_provider,
    })
}

/// Loads config and returns the client-facing AI config slice.
pub async fn load_and_get_client_config_snapshot() -> Result<RpcOutcome<serde_json::Value>, String>
{
    let config = load_config_with_timeout().await?;
    let snapshot = client_config_json(&config);
    Ok(RpcOutcome::new(
        snapshot,
        vec!["client config read".to_string()],
    ))
}

#[derive(Debug, Clone, Default)]
pub struct ModelSettingsPatch {
    pub api_url: Option<String>,
    /// Custom OpenAI-compatible LLM endpoint. Empty string clears the
    /// override (inference falls back through the OpenHuman backend).
    pub inference_url: Option<String>,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
    pub default_temperature: Option<f64>,
    /// When `Some`, REPLACES the entire `config.model_routes` array with the
    /// supplied (hint, model) pairs. Pass `Some(vec![])` to clear all routes
    /// (e.g. when switching back to the OpenHuman backend whose built-in
    /// router picks per-task models on its own). Leave `None` to keep the
    /// current routes untouched.
    pub model_routes: Option<Vec<crate::openhuman::config::ModelRouteConfig>>,
    /// When `Some`, REPLACES the entire `config.cloud_providers` array with
    /// the supplied entries (each lacking the API key — those live in
    /// `auth-profiles.json` via [`crate::openhuman::credentials::AuthService`]).
    /// Pass `Some(vec![])` to clear all third-party cloud providers.
    pub cloud_providers:
        Option<Vec<crate::openhuman::config::schema::cloud_providers::CloudProviderCreds>>,
    /// Id of the `cloud_providers` entry used when a workload routes to
    /// `"cloud"`. Empty string clears (factory falls back to OpenHuman).
    pub primary_cloud: Option<String>,
    pub chat_provider: Option<String>,
    pub reasoning_provider: Option<String>,
    pub agentic_provider: Option<String>,
    pub coding_provider: Option<String>,
    pub memory_provider: Option<String>,
    pub embeddings_provider: Option<String>,
    pub heartbeat_provider: Option<String>,
    pub learning_provider: Option<String>,
    pub subconscious_provider: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MemorySettingsPatch {
    pub backend: Option<String>,
    pub auto_save: Option<bool>,
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_dimensions: Option<usize>,
    /// Stepped user-facing memory-context window preset (see
    /// [`crate::openhuman::config::schema::agent::MemoryContextWindow`]).
    /// Accepts `"minimal" | "balanced" | "extended" | "maximum"`.
    /// Unknown values are silently ignored so old clients can keep
    /// posting partial patches.
    pub memory_window: Option<String>,
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

#[derive(Debug, Clone, Default)]
pub struct MeetSettingsPatch {
    pub auto_orchestrator_handoff: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct AutonomySettingsPatch {
    pub max_actions_per_hour: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchSettingsPatch {
    /// One of `managed` | `parallel` | `brave`. Empty string / unknown values
    /// fall back to `managed` at registration time.
    pub engine: Option<String>,
    /// 1..=20. Clamped silently at apply time.
    pub max_results: Option<usize>,
    /// Per-request timeout in seconds (default 15).
    pub timeout_secs: Option<u64>,
    /// Parallel API key. An empty string clears the stored key.
    pub parallel_api_key: Option<String>,
    /// Brave Search API key. An empty string clears the stored key.
    pub brave_api_key: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LocalAiSettingsPatch {
    pub runtime_enabled: Option<bool>,
    /// MVP opt-in marker. Bootstrap hard-overrides status to "disabled"
    /// when this is `false`, regardless of `runtime_enabled`. The unified
    /// AI panel ties the two together (both flip on enable, both flip
    /// off on disable) so a single toggle gives the user the obvious
    /// behaviour without needing to apply a preset first.
    pub opt_in_confirmed: Option<bool>,
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub model_id: Option<String>,
    pub chat_model_id: Option<String>,
    pub usage_embeddings: Option<bool>,
    pub usage_heartbeat: Option<bool>,
    pub usage_learning_reflection: Option<bool>,
    pub usage_subconscious: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct ComposioTriggerSettingsPatch {
    /// When `Some(true)`, disables triage for all toolkits.
    pub triage_disabled: Option<bool>,
    /// When `Some(v)`, replaces the per-toolkit opt-out list entirely.
    pub triage_disabled_toolkits: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeFlagsOut {
    pub browser_allow_all: bool,
    pub log_prompts: bool,
}

const BROWSER_ALLOW_ALL_ENV: &str = "OPENHUMAN_BROWSER_ALLOW_ALL";
const BROWSER_ALLOW_ALL_RPC_ENABLE_ENV: &str = "OPENHUMAN_BROWSER_ALLOW_ALL_RPC_ENABLE";

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
    if let Some(api_url) = update.api_url {
        config.api_url = if api_url.trim().is_empty() {
            None
        } else {
            Some(api_url)
        };
    }
    if let Some(inference_url) = update.inference_url {
        config.inference_url = if inference_url.trim().is_empty() {
            None
        } else {
            Some(inference_url.trim().to_string())
        };
    }
    if let Some(api_key) = update.api_key {
        let trimmed_key = api_key.trim();
        config.api_key = if trimmed_key.is_empty() {
            None
        } else {
            Some(trimmed_key.to_string())
        };
    }
    if let Some(model) = update.default_model {
        let trimmed = model.trim();
        config.default_model = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
        if let Some(ref m) = config.default_model {
            if !crate::openhuman::inference::provider::factory::is_known_openhuman_tier(m) {
                log::warn!(
                    "[config][model-settings] default_model '{}' is not a recognized \
                     OpenHuman backend tier — it will be replaced with the platform \
                     default at inference time.",
                    m
                );
            }
        }
    }
    if let Some(temp) = update.default_temperature {
        config.default_temperature = temp;
    }
    if let Some(routes) = update.model_routes {
        // Full replacement — UI sends the canonical set for the active provider
        // (or an empty vec when switching back to the OpenHuman in-built router).
        config.model_routes = routes;
    }
    if let Some(providers) = update.cloud_providers {
        // The schema handlers strip reserved-slug entries (e.g. the built-in
        // "openhuman" provider seeded by `migrations::unify_ai_provider_settings`)
        // from the user's payload. Preserve any reserved-slug entries that
        // already live in the stored config so a routine settings save
        // doesn't accidentally delete them — `primary_cloud` and the
        // per-workload routing fields can reference these built-ins, and
        // losing them would break inference routing.
        use crate::openhuman::config::schema::cloud_providers::is_slug_reserved;
        let preserved: Vec<_> = config
            .cloud_providers
            .iter()
            .filter(|e| is_slug_reserved(e.slug.trim()))
            .cloned()
            .collect();
        log::debug!(
            "[config] apply_model_settings: preserving {} reserved cloud provider(s) before overwrite",
            preserved.len()
        );
        config.cloud_providers = providers;
        let before_reinject = config.cloud_providers.len();
        for entry in preserved {
            // Defensive: don't double-add if the payload (somehow) already
            // contained an entry with this reserved slug — the schema-handler
            // filter is the canonical guard, but apply_model_settings is also
            // reachable from tests and CLI paths that bypass that filter.
            let preserved_slug = entry.slug.trim();
            if !config
                .cloud_providers
                .iter()
                .any(|e| e.slug.trim() == preserved_slug)
            {
                config.cloud_providers.push(entry);
            }
        }
        log::debug!(
            "[config] apply_model_settings: reinjected {} reserved cloud provider(s)",
            config.cloud_providers.len() - before_reinject
        );
    }
    if let Some(primary) = update.primary_cloud {
        let trimmed = primary.trim();
        config.primary_cloud = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }

    // Per-workload provider strings. Empty / blank → None (factory default).
    let normalise_provider = |s: String| -> Option<String> {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    };
    if let Some(s) = update.chat_provider {
        config.chat_provider = normalise_provider(s);
    }
    if let Some(s) = update.reasoning_provider {
        config.reasoning_provider = normalise_provider(s);
    }
    if let Some(s) = update.agentic_provider {
        config.agentic_provider = normalise_provider(s);
    }
    if let Some(s) = update.coding_provider {
        config.coding_provider = normalise_provider(s);
    }
    if let Some(s) = update.memory_provider {
        config.memory_provider = normalise_provider(s);
    }
    if let Some(s) = update.embeddings_provider {
        config.embeddings_provider = normalise_provider(s);
    }
    if let Some(s) = update.heartbeat_provider {
        config.heartbeat_provider = normalise_provider(s);
    }
    if let Some(s) = update.learning_provider {
        config.learning_provider = normalise_provider(s);
    }
    if let Some(s) = update.subconscious_provider {
        config.subconscious_provider = normalise_provider(s);
    }

    config.save().await.map_err(|e| e.to_string())?;
    // #1574 §4: the AIPanel workload matrix changes the embedder via THIS
    // (model-settings) path — `embeddings_provider` above — not the
    // memory-settings path. Trigger the same idempotent re-embed backfill
    // so a UI embedder switch recovers prior memory under the new
    // signature. Coverage-gated + non-fatal: if the active signature did
    // not actually change, this enqueues nothing.
    crate::openhuman::memory::jobs::ensure_reembed_backfill(config);
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
    if let Some(window_label) = update.memory_window.as_deref() {
        if let Some(window) =
            crate::openhuman::config::schema::MemoryContextWindow::from_str_opt(window_label)
        {
            config.agent.memory_window = Some(window);
        } else {
            tracing::warn!(
                requested = window_label,
                "[config] unknown memory_window preset — leaving existing setting unchanged"
            );
        }
    }
    config.save().await.map_err(|e| e.to_string())?;
    // #1574 §4: the embedder may have just changed (provider/model/dims).
    // Ensure a re-embed backfill chain exists for the new active signature
    // so prior memory becomes retrievable again instead of silently going
    // dark. Idempotent + non-fatal (covered space enqueues nothing; errors
    // are logged, never fail the settings save). §7's migration is
    // one-shot so it does not cover a later switch — this does.
    crate::openhuman::memory::jobs::ensure_reembed_backfill(config);
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

/// Updates the Google Meet integration settings in the configuration.
pub async fn apply_meet_settings(
    config: &mut Config,
    update: MeetSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(enabled) = update.auto_orchestrator_handoff {
        config.meet.auto_orchestrator_handoff = enabled;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "meet settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies meet settings updates, and saves it.
pub async fn load_and_apply_meet_settings(
    update: MeetSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_meet_settings(&mut config, update).await
}

/// Updates the autonomy policy settings in the configuration.
/// Validation: 1 <= max_actions_per_hour <= u32::MAX. The upper bound is the
/// sentinel for "unlimited" (matches the schema default); the UI surfaces
/// this preset explicitly.
pub async fn apply_autonomy_settings(
    config: &mut Config,
    update: AutonomySettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(v) = update.max_actions_per_hour {
        if v == 0 {
            return Err(format!("max_actions_per_hour must be at least 1 (got {v})"));
        }
        config.autonomy.max_actions_per_hour = v;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "autonomy settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies autonomy settings updates, and saves it.
pub async fn load_and_apply_autonomy_settings(
    update: AutonomySettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_autonomy_settings(&mut config, update).await
}

/// Updates the search engine configuration. Empty API-key strings clear the
/// stored value rather than treat empty-string as "credential present".
pub async fn apply_search_settings(
    config: &mut Config,
    update: SearchSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(engine) = update.engine {
        let trimmed = engine.trim();
        // Reject blatantly bogus values so the panel can show a friendly
        // error. Unknown values still resolve to managed at registration
        // time via `effective_engine()`, but failing fast in the writer keeps
        // the TOML clean.
        match trimmed {
            "managed" | "parallel" | "brave" => {
                config.search.engine = trimmed.to_string();
            }
            other => {
                return Err(format!(
                    "engine must be one of managed/parallel/brave (got {other:?})"
                ));
            }
        }
    }
    if let Some(n) = update.max_results {
        if !(1..=20).contains(&n) {
            return Err(format!("max_results must be between 1 and 20 (got {n})"));
        }
        config.search.max_results = n;
    }
    if let Some(secs) = update.timeout_secs {
        if !(1..=120).contains(&secs) {
            return Err(format!(
                "timeout_secs must be between 1 and 120 (got {secs})"
            ));
        }
        config.search.timeout_secs = secs;
    }
    if let Some(raw) = update.parallel_api_key {
        let trimmed = raw.trim();
        config.search.parallel.api_key = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    if let Some(raw) = update.brave_api_key {
        let trimmed = raw.trim();
        config.search.brave.api_key = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "search settings saved to {}",
            config.config_path.display()
        )],
    ))
}

pub async fn load_and_apply_search_settings(
    update: SearchSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_search_settings(&mut config, update).await
}

/// Read the current search engine settings (with API keys redacted to a
/// presence boolean so the UI can show "configured" without ever rendering
/// the raw secret).
pub async fn get_search_settings() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let result = serde_json::json!({
        "engine": config.search.requested_engine_str(),
        "effective_engine": match config.search.effective_engine() {
            crate::openhuman::config::SearchEngine::Managed => "managed",
            crate::openhuman::config::SearchEngine::Parallel => "parallel",
            crate::openhuman::config::SearchEngine::Brave => "brave",
        },
        "max_results": config.search.max_results,
        "timeout_secs": config.search.timeout_secs,
        "parallel_configured": config.search.parallel.has_key(),
        "brave_configured": config.search.brave.has_key(),
    });
    Ok(RpcOutcome::new(
        result,
        vec!["search settings read".to_string()],
    ))
}

/// Loads the configuration, applies browser settings updates, and saves it.
pub async fn load_and_apply_browser_settings(
    update: BrowserSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_browser_settings(&mut config, update).await
}

/// Updates the local-AI runtime + per-feature usage flags in the configuration.
pub async fn apply_local_ai_settings(
    config: &mut Config,
    update: LocalAiSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(v) = update.runtime_enabled {
        config.local_ai.runtime_enabled = v;
    }
    if let Some(v) = update.opt_in_confirmed {
        config.local_ai.opt_in_confirmed = v;
    }
    if let Some(provider) = update.provider {
        config.local_ai.provider =
            crate::openhuman::inference::local::provider::normalize_provider(&provider);
    }
    if let Some(base_url) = update.base_url {
        config.local_ai.base_url = if base_url.trim().is_empty() {
            None
        } else {
            Some(base_url.trim().to_string())
        };
    }
    if let Some(model_id) = update.model_id {
        config.local_ai.model_id = model_id.trim().to_string();
    }
    if let Some(chat_model_id) = update.chat_model_id {
        config.local_ai.chat_model_id = chat_model_id.trim().to_string();
    }
    if let Some(v) = update.usage_embeddings {
        config.local_ai.usage.embeddings = v;
    }
    if let Some(v) = update.usage_heartbeat {
        config.local_ai.usage.heartbeat = v;
    }
    if let Some(v) = update.usage_learning_reflection {
        config.local_ai.usage.learning_reflection = v;
    }
    if let Some(v) = update.usage_subconscious {
        config.local_ai.usage.subconscious = v;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "local AI settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies local-AI settings updates, and saves it.
pub async fn load_and_apply_local_ai_settings(
    update: LocalAiSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_local_ai_settings(&mut config, update).await
}

/// Updates the Composio trigger-triage settings in the configuration.
pub async fn apply_composio_trigger_settings(
    config: &mut Config,
    update: ComposioTriggerSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(v) = update.triage_disabled {
        config.composio.triage_disabled = v;
        tracing::debug!(
            triage_disabled = v,
            "[config][composio] triage_disabled updated"
        );
    }
    if let Some(toolkits) = update.triage_disabled_toolkits {
        tracing::debug!(
            count = toolkits.len(),
            "[config][composio] triage_disabled_toolkits updated"
        );
        config.composio.triage_disabled_toolkits = toolkits;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "composio trigger settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies composio trigger settings, and saves it.
pub async fn load_and_apply_composio_trigger_settings(
    update: ComposioTriggerSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_composio_trigger_settings(&mut config, update).await
}

/// Reads the current composio trigger-triage settings.
pub async fn get_composio_trigger_settings() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let result = serde_json::json!({
        "triage_disabled": config.composio.triage_disabled,
        "triage_disabled_toolkits": config.composio.triage_disabled_toolkits,
    });
    Ok(RpcOutcome::new(
        result,
        vec!["composio trigger settings read".to_string()],
    ))
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
    RpcOutcome::single_log(runtime_flags(), "runtime flags read")
}

fn runtime_flags() -> RuntimeFlagsOut {
    RuntimeFlagsOut {
        browser_allow_all: env_flag_enabled(BROWSER_ALLOW_ALL_ENV),
        log_prompts: env_flag_enabled("OPENHUMAN_LOG_PROMPTS"),
    }
}

/// Updates the `OPENHUMAN_BROWSER_ALLOW_ALL` environment flag.
///
/// **Security note:** when enabled, this disables the browser tool's
/// per-domain allowlist for the entire process. Both transitions are
/// audit-logged at WARN level with a `[SECURITY]` prefix so operators
/// (and `journalctl -g '\[SECURITY\]'` style scrapes) can spot
/// allowlist toggles in the live log stream.
///
/// `is_private_host` checks still apply to the resolved IP, so this
/// flag does not unlock loopback / RFC1918 destinations.
pub fn set_browser_allow_all(enabled: bool) -> Result<RpcOutcome<RuntimeFlagsOut>, String> {
    if enabled && !env_flag_enabled(BROWSER_ALLOW_ALL_RPC_ENABLE_ENV) {
        tracing::warn!(
            "[SECURITY] refused browser allow-all enable via RPC: \
             set {BROWSER_ALLOW_ALL_ENV}=1 at startup or explicitly set \
             {BROWSER_ALLOW_ALL_RPC_ENABLE_ENV}=1 before using the runtime toggle"
        );
        return Err(format!(
            "Refusing to enable {BROWSER_ALLOW_ALL_ENV} via RPC. Start OpenHuman with \
             {BROWSER_ALLOW_ALL_ENV}=1, or set {BROWSER_ALLOW_ALL_RPC_ENABLE_ENV}=1 for an \
             explicit operator-approved runtime override."
        ));
    }

    let was_enabled = env_flag_enabled(BROWSER_ALLOW_ALL_ENV);
    if enabled {
        unsafe {
            std::env::set_var(BROWSER_ALLOW_ALL_ENV, "1");
        }
    } else {
        unsafe {
            std::env::remove_var(BROWSER_ALLOW_ALL_ENV);
        }
    }
    let flags = runtime_flags();
    let now_enabled = flags.browser_allow_all;

    if was_enabled != now_enabled {
        if now_enabled {
            tracing::warn!(
                "[SECURITY] browser allow-all enabled via RPC: \
                 per-domain allowlist is now bypassed for all sessions \
                 (private-host check still applies)"
            );
        } else {
            tracing::info!(
                "[SECURITY] browser allow-all disabled via RPC: \
                 per-domain allowlist re-enforced"
            );
        }
    }

    let log_msg = if now_enabled {
        "[SECURITY] browser allow-all flag set to enabled"
    } else {
        "[SECURITY] browser allow-all flag set to disabled"
    };
    Ok(RpcOutcome::single_log(flags, log_msg))
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
/// On a false→true transition, seeds the recurring morning-briefing
/// cron job via [`crate::openhuman::cron::seed::seed_proactive_agents`].
pub async fn set_onboarding_completed(value: bool) -> Result<RpcOutcome<bool>, String> {
    tracing::debug!(value, "[onboarding] set_onboarding_completed called");
    let mut config = load_config_with_timeout().await?;
    let was_completed = config.onboarding_completed;
    config.onboarding_completed = value;

    config.save().await.map_err(|e| e.to_string())?;

    if value && !was_completed {
        tracing::debug!(
            "[onboarding] false→true transition detected — seeding cron jobs (welcome is renderer-triggered)"
        );
        let seed_config = config.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = crate::openhuman::cron::seed::seed_proactive_agents(&seed_config) {
                tracing::warn!("[onboarding] failed to seed proactive agent cron jobs: {e}");
            }
        });
    } else {
        tracing::debug!(
            was_completed,
            value,
            "[onboarding] no transition — skipping proactive seeding"
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
///
/// Runs **inside the core's tokio task**, which means the running core
/// holds open handles to SQLite databases, log files, the Sentry session
/// store, etc. On Windows, `remove_dir_all` therefore fails with
/// `ERROR_SHARING_VIOLATION` (os error 32) — see OPENHUMAN-TAURI-AF.
///
/// GUI callers must use the Tauri-side `reset_local_data` command instead:
/// it stops the embedded core via `CoreProcessHandle::shutdown` (dropping
/// the file handles), removes the directories from the Tauri host process,
/// and restarts the core. This JSON-RPC method is kept for headless / CLI
/// callers where in-process removal is acceptable (POSIX file semantics
/// tolerate unlinking open files; on Windows the CLI invocation runs
/// without the core attached, so no handle is in the way).
pub async fn reset_local_data() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let current_openhuman_dir = config_openhuman_dir(&config);
    let default_openhuman_dir = default_openhuman_dir();
    reset_local_data_for_paths(&current_openhuman_dir, &default_openhuman_dir).await
}

/// Reports the resolved paths that `reset_local_data` would remove, without
/// performing any filesystem changes.
///
/// Lets the Tauri-side `reset_local_data` command discover the active
/// workspace dir, the default `~/.openhuman` dir (which can differ when
/// `OPENHUMAN_WORKSPACE` is set or a staging build is in use), and the
/// active workspace marker file **before** the core sidecar is shut down —
/// after which the Tauri shell removes them while no process holds open
/// handles. See OPENHUMAN-TAURI-AF for the Windows file-locking failure
/// that motivated the split.
pub async fn get_data_paths() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let current_openhuman_dir = config_openhuman_dir(&config);
    let default_openhuman_dir = default_openhuman_dir();
    let active_workspace_marker = active_workspace_marker_path(&default_openhuman_dir);
    Ok(RpcOutcome::new(
        json!({
            "current_openhuman_dir": current_openhuman_dir.display().to_string(),
            "default_openhuman_dir": default_openhuman_dir.display().to_string(),
            "active_workspace_marker_path": active_workspace_marker.display().to_string(),
        }),
        vec![format!(
            "data paths resolved (current={}, default={})",
            current_openhuman_dir.display(),
            default_openhuman_dir.display()
        )],
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
