//! Config loading, snapshotting, and core runtime-flag helpers.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

pub(crate) fn env_flag_enabled(key: &str) -> bool {
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

pub(super) const CONFIG_LOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

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
        // Surface the full anyhow chain (`{:#}`), not just the top `with_context`
        // line, so the underlying io error kind (e.g. `(os error 5)` access-denied
        // / `(os error 32)` sharing-lock) reaches Sentry. Without it the config
        // classifier and triage only ever see "Failed to read config file: <path>"
        // and cannot tell a user-environment denial from an app-side race
        // (#3962 / TAURI-RUST-DME).
        Ok(Err(e)) => Err(format!("{e:#}")),
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
        // Surface the full anyhow chain (`{:#}`), not just the top `with_context`
        // line, so the underlying io error kind (e.g. `(os error 5)` access-denied
        // / `(os error 32)` sharing-lock) reaches Sentry. Without it the config
        // classifier and triage only ever see "Failed to read config file: <path>"
        // and cannot tell a user-environment denial from an app-side race
        // (#3962 / TAURI-RUST-DME).
        Ok(Err(e)) => Err(format!("{e:#}")),
        Err(_) => Err("Config loading timed out".to_string()),
    }
}

async fn normalize_loaded_config(config: &mut Config) {
    // Welcome-agent routing normalization removed (the welcome agent has been
    // deleted; all chat turns route directly to the orchestrator). The
    // `chat_onboarding_completed` field is retained only for backward-compatible
    // deserialization.

    seed_and_enrich_model_registry(config);
}

/// Populate per-token pricing on the model registry from the static catalog.
///
/// Runs on every load and is **in-memory only** — it does not rewrite
/// `config.toml`. This keeps the user's persisted config clean (the catalog
/// stays the single source of truth, so price refreshes apply automatically)
/// while ensuring the Model Health dashboard, cost estimates, and the client
/// config snapshot see real numbers out of the box.
///
/// - Empty registry → seed it with one entry per catalogued model
///   ([`catalog::default_registry_entries`]).
/// - Otherwise → backfill any missing (zero) price on each existing entry,
///   preserving user-supplied prices and the `vision` flag
///   ([`catalog::enrich_entry`]).
///
/// Idempotent: re-running over an already-priced registry is a no-op.
fn seed_and_enrich_model_registry(config: &mut Config) {
    use crate::openhuman::cost::catalog;

    if config.model_registry.is_empty() {
        config.model_registry = catalog::default_registry_entries();
        log::debug!(
            "[config] seeded empty model_registry with {} catalogued models (as_of {})",
            config.model_registry.len(),
            catalog::PRICING_AS_OF
        );
        return;
    }

    let mut filled = 0usize;
    for entry in &mut config.model_registry {
        if catalog::enrich_entry(entry) {
            filled += 1;
        }
    }
    if filled > 0 {
        log::debug!("[config] backfilled pricing on {filled} model_registry entries from catalog");
    }
}

/// Returns the default workspace directory fallback (~/.openhuman/workspace).
pub(crate) fn fallback_workspace_dir() -> PathBuf {
    crate::openhuman::config::default_root_openhuman_dir()
        .unwrap_or_else(|_| env_scoped_fallback_root_dir())
        .join("workspace")
}

/// Returns the default OpenHuman configuration directory (~/.openhuman).
pub(crate) fn default_openhuman_dir() -> PathBuf {
    crate::openhuman::config::default_root_openhuman_dir()
        .unwrap_or_else(|_| env_scoped_fallback_root_dir())
}

pub(crate) fn env_scoped_fallback_root_dir() -> PathBuf {
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
pub(crate) fn active_workspace_marker_path(default_openhuman_dir: &Path) -> PathBuf {
    default_openhuman_dir.join("active_workspace.toml")
}

/// Returns the parent directory of the config file.
pub(crate) fn config_openhuman_dir(config: &Config) -> PathBuf {
    config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

pub(crate) fn is_windows_file_lock_error(error: &std::io::Error) -> bool {
    cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33))
}

pub(crate) fn reset_local_data_remove_error(path: &Path, error: &std::io::Error) -> String {
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

pub(crate) fn reset_local_data_marker_remove_error(path: &Path, error: &std::io::Error) -> String {
    // This is called for every root-level marker (active_workspace.toml,
    // active_user.toml, …), so the wording is derived from the actual file
    // name rather than hardcoded to one marker.
    let marker_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("marker");

    if is_windows_file_lock_error(error) {
        tracing::warn!(
            marker = %path.display(),
            error = %error,
            "[config] reset_local_data: Windows file lock blocked marker deletion"
        );
        return format!(
            "Failed to remove marker {} ({marker_name}) because it is locked by another OpenHuman window or process. Close all OpenHuman windows and try again. ({error})",
            path.display()
        );
    }

    format!(
        "Failed to remove marker {} ({marker_name}): {error}",
        path.display()
    )
}

/// Internal helper to reset local data for the **active user only**.
///
/// Removes the current user's data directory (`~/.openhuman/users/<id>`) plus
/// the two shared marker files at the root — `active_workspace.toml` and
/// `active_user.toml` — so the next launch boots signed-out into the
/// pre-login (`users/local`) scope.
///
/// It deliberately does **not** delete the shared root `~/.openhuman`
/// directory: that root holds every user's `users/<other>` subtree, and
/// wiping it during a single user's "Clear App Data" destroyed sibling
/// accounts' data (the scoping bug this replaces). The root is left in place;
/// only the current user's slice and the active markers are removed.
pub(crate) async fn reset_local_data_for_paths(
    current_openhuman_dir: &Path,
    default_openhuman_dir: &Path,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let active_workspace_marker = active_workspace_marker_path(default_openhuman_dir);
    let active_user_marker =
        crate::openhuman::config::active_user_marker_path(default_openhuman_dir);
    tracing::debug!(
        current_dir = %current_openhuman_dir.display(),
        default_dir = %default_openhuman_dir.display(),
        workspace_marker = %active_workspace_marker.display(),
        user_marker = %active_user_marker.display(),
        "[config] reset_local_data: starting (user-scoped)"
    );

    let mut removed_paths = Vec::new();

    // Remove the two shared root-level markers so the current user is signed
    // out and any non-default workspace pointer is dropped. Each is a single
    // file under the root; the root itself is preserved for sibling users.
    for marker in [&active_workspace_marker, &active_user_marker] {
        if marker.exists() {
            if let Err(error) = tokio::fs::remove_file(marker).await {
                return Err(reset_local_data_marker_remove_error(marker, &error));
            }
            tracing::debug!(
                marker = %marker.display(),
                "[config] reset_local_data: removed marker"
            );
            removed_paths.push(marker.display().to_string());
        }
    }

    // Remove only the active user's directory — NOT the shared root, which
    // contains other users' `users/<id>` subtrees.
    if current_openhuman_dir.exists() {
        if let Err(error) = tokio::fs::remove_dir_all(current_openhuman_dir).await {
            return Err(reset_local_data_remove_error(current_openhuman_dir, &error));
        }
        tracing::debug!(
            dir = %current_openhuman_dir.display(),
            "[config] reset_local_data: removed current user directory"
        );
        removed_paths.push(current_openhuman_dir.display().to_string());
    } else {
        tracing::debug!(
            dir = %current_openhuman_dir.display(),
            "[config] reset_local_data: current user directory already absent"
        );
    }

    Ok(RpcOutcome::new(
        json!({
            "removed_paths": removed_paths,
            "current_openhuman_dir": current_openhuman_dir.display().to_string(),
            "default_openhuman_dir": default_openhuman_dir.display().to_string(),
        }),
        vec![format!(
            "reset local data for active user dir {} (shared root {} preserved)",
            current_openhuman_dir.display(),
            default_openhuman_dir.display()
        )],
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
    let model_registry: Vec<serde_json::Value> = config
        .model_registry
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "provider": m.provider,
                "cost_per_1m_input": m.cost_per_1m_input,
                "cost_per_1m_cached_input": m.cost_per_1m_cached_input,
                "cost_per_1m_output": m.cost_per_1m_output,
                "context_window": m.context_window,
                "vision": m.vision,
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
        "model_registry": model_registry,
        "primary_cloud": config.primary_cloud,
        // #3767: authoritative, core-side decision telling the UI whether the
        // managed-credits gate should be bypassed, per chat-mode tier. The chat
        // header's "Quick" mode runs on the `chat` tier and "Reasoning" mode on
        // the `reasoning` tier, so each is reported separately and the UI checks
        // the tier the user actually selected. True for a tier when it runs on a
        // non-managed provider the user funds themselves (BYO key / local /
        // claude-code) with usable creds. Managed tiers that run anyway surface
        // credit errors per-call.
        "credits_bypass": {
            "chat": crate::openhuman::inference::provider::factory::role_bypasses_managed_credits(
                "chat", config,
            ),
            "reasoning":
                crate::openhuman::inference::provider::factory::role_bypasses_managed_credits(
                    "reasoning", config,
                ),
        },
        "chat_provider": config.chat_provider,
        "reasoning_provider": config.reasoning_provider,
        "agentic_provider": config.agentic_provider,
        "coding_provider": config.coding_provider,
        "vision_provider": config.vision_provider,
        "memory_provider": config.memory_provider,
        "embeddings_provider": config.embeddings_provider,
        "heartbeat_provider": config.heartbeat_provider,
        "learning_provider": config.learning_provider,
        "subconscious_provider": config.subconscious_provider,
        "voice_providers": config.voice_providers.iter().map(|v| {
            serde_json::json!({
                "id": v.id,
                "slug": v.slug,
                "label": v.label,
                "endpoint": v.endpoint,
                "auth_style": v.auth_style.as_str(),
                "capability": v.capability.as_str(),
                "stt_api_style": v.stt_api_style,
                "tts_api_style": v.tts_api_style,
                "default_stt_model": v.default_stt_model,
                "default_tts_voice": v.default_tts_voice,
            })
        }).collect::<Vec<_>>(),
        "stt_provider": config.stt_provider,
        "tts_provider": config.tts_provider,
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

/// Loads the configuration from disk and returns a snapshot.
pub async fn load_and_get_config_snapshot() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    get_config_snapshot(&config).await
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeFlagsOut {
    pub browser_allow_all: bool,
    pub log_prompts: bool,
}

pub(crate) const BROWSER_ALLOW_ALL_ENV: &str = "OPENHUMAN_BROWSER_ALLOW_ALL";
pub(crate) const BROWSER_ALLOW_ALL_RPC_ENABLE_ENV: &str = "OPENHUMAN_BROWSER_ALLOW_ALL_RPC_ENABLE";

/// Returns the current state of runtime-only flags.
pub fn get_runtime_flags() -> RpcOutcome<RuntimeFlagsOut> {
    RpcOutcome::single_log(runtime_flags(), "runtime flags read")
}

pub(crate) fn runtime_flags() -> RuntimeFlagsOut {
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

/// Reads dashboard settings exposed to the desktop UI.
pub async fn get_dashboard_settings() -> Result<RpcOutcome<serde_json::Value>, String> {
    let request_id = uuid::Uuid::new_v4().to_string();
    tracing::debug!(
        target: "openhuman_core::config",
        request_id = %request_id,
        method = "openhuman.config_get_dashboard_settings",
        "OPENHUMAN: get_dashboard_settings entry"
    );
    tracing::debug!(
        target: "openhuman_core::config",
        request_id = %request_id,
        method = "openhuman.config_get_dashboard_settings",
        "OPENHUMAN: get_dashboard_settings loading config"
    );

    let config = load_config_with_timeout().await.map_err(|error| {
        tracing::warn!(
            target: "openhuman_core::config",
            request_id = %request_id,
            method = "openhuman.config_get_dashboard_settings",
            error = %error,
            "OPENHUMAN: get_dashboard_settings config load failed"
        );
        error
    })?;

    tracing::debug!(
        target: "openhuman_core::config",
        request_id = %request_id,
        method = "openhuman.config_get_dashboard_settings",
        "OPENHUMAN: get_dashboard_settings serializing dashboard settings"
    );
    let result = serde_json::to_value(&config.dashboard).map_err(|error| {
        let message = error.to_string();
        tracing::warn!(
            target: "openhuman_core::config",
            request_id = %request_id,
            method = "openhuman.config_get_dashboard_settings",
            error = %message,
            "OPENHUMAN: get_dashboard_settings serialization failed"
        );
        message
    })?;

    tracing::debug!(
        target: "openhuman_core::config",
        request_id = %request_id,
        method = "openhuman.config_get_dashboard_settings",
        "OPENHUMAN: get_dashboard_settings exit"
    );
    Ok(RpcOutcome::new(
        result,
        vec!["dashboard settings read".to_string()],
    ))
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
    // The active-user marker lives at the *shared* root `~/.openhuman`, not
    // inside the per-user dir. A clear removes it (to sign the current user
    // out) but must leave the sibling `users/<other>` dirs and the root
    // itself intact — see `reset_local_data_for_paths`.
    let active_user_marker =
        crate::openhuman::config::active_user_marker_path(&default_openhuman_dir);
    Ok(RpcOutcome::new(
        json!({
            "current_openhuman_dir": current_openhuman_dir.display().to_string(),
            "default_openhuman_dir": default_openhuman_dir.display().to_string(),
            "active_workspace_marker_path": active_workspace_marker.display().to_string(),
            "active_user_marker_path": active_user_marker.display().to_string(),
        }),
        vec![format!(
            "data paths resolved (current={}, default={})",
            current_openhuman_dir.display(),
            default_openhuman_dir.display()
        )],
    ))
}

#[cfg(test)]
mod model_registry_seed_tests {
    use super::*;

    #[test]
    fn seeds_empty_registry_from_catalog() {
        let mut config = Config {
            model_registry: Vec::new(),
            ..Default::default()
        };
        seed_and_enrich_model_registry(&mut config);
        assert!(
            !config.model_registry.is_empty(),
            "empty registry should be seeded from the catalog"
        );
        // Every seeded entry carries pricing + a context window.
        for entry in &config.model_registry {
            assert!(entry.cost_per_1m_input > 0.0, "{}", entry.id);
            assert!(entry.cost_per_1m_output > 0.0, "{}", entry.id);
            assert!(entry.context_window > 0, "{}", entry.id);
        }
    }

    #[test]
    fn backfills_existing_entries_but_preserves_user_values_and_count() {
        let mut config = Config {
            model_registry: vec![
                // Known model, missing prices → backfilled.
                crate::openhuman::config::schema::ModelRegistryEntry {
                    id: "claude-opus-4-8".to_string(),
                    provider: "anthropic".to_string(),
                    cost_per_1m_output: 99.0, // user override — must survive
                    vision: true,
                    ..Default::default()
                },
                // Unknown model → left untouched.
                crate::openhuman::config::schema::ModelRegistryEntry {
                    id: "my-byok-model".to_string(),
                    provider: "custom".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        seed_and_enrich_model_registry(&mut config);

        assert_eq!(
            config.model_registry.len(),
            2,
            "must not seed when non-empty"
        );
        let opus = &config.model_registry[0];
        assert_eq!(opus.cost_per_1m_input, 5.00, "backfilled");
        assert_eq!(opus.context_window, 1_000_000, "backfilled");
        assert_eq!(opus.cost_per_1m_output, 99.0, "user value preserved");
        let byok = &config.model_registry[1];
        assert_eq!(byok.cost_per_1m_input, 0.0, "unknown model untouched");
        assert_eq!(byok.context_window, 0);
    }
}

#[cfg(test)]
mod loader_io_chain_tests {
    use super::*;

    // A directory at the config path is corruption, not a transient/denied read:
    // the read site fails it fast with distinct wording, and the observability
    // classifier MUST keep paging it (never demote to ConfigReadIoFailure). This
    // guards the Codex P2 hole where a Windows directory-at-config surfaces the
    // same `os error 5` shape as a real ACL denial (#3962).
    #[tokio::test]
    async fn config_directory_pages_and_is_not_demoted() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        std::fs::create_dir(&config_path).unwrap();

        let snapshot = Config {
            config_path: config_path.clone(),
            workspace_dir: tmp.path().join("workspace"),
            ..Default::default()
        };

        let err = reload_config_snapshot_with_timeout(&snapshot)
            .await
            .expect_err("a directory at the config path must fail");

        assert!(
            err.contains("is a directory") || err.contains("not a file"),
            "directory-at-config must report a distinct, non-read error: {err}"
        );
        assert_ne!(
            crate::core::observability::expected_error_kind(&err),
            Some(crate::core::observability::ExpectedErrorKind::ConfigReadIoFailure),
            "a directory at the config path is corruption — it must keep paging, not demote: {err}"
        );
    }

    // load_config_with_timeout resolves the process-global OPENHUMAN_WORKSPACE,
    // so serialize against the other env-mutating config tests. Exercises the
    // load_or_init directory guard + the `Ok(Err) => format!("{e:#}")` arm.
    #[tokio::test]
    async fn load_config_with_timeout_rejects_directory_config() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        std::fs::create_dir(&config_path).unwrap();

        let _g = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("OPENHUMAN_WORKSPACE").ok();
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path().to_str().unwrap());

        let result = load_config_with_timeout().await;

        match prev {
            Some(v) => std::env::set_var("OPENHUMAN_WORKSPACE", v),
            None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
        }

        let err = result.expect_err("a directory at the config path must fail");
        assert!(
            err.contains("is a directory") || err.contains("not a file"),
            "directory-at-config must report a distinct, non-read error: {err}"
        );
    }

    // The #3962 keystone: a genuine read failure on a regular file must surface
    // the full anyhow chain (`{:#}`) — the read context PLUS the underlying io
    // cause (`os error N`) — through the RPC String boundary, not just the top
    // `with_context` line. Triggered portably with a 0o000 (unreadable) file;
    // skipped under root, which ignores file permissions.
    #[cfg(unix)]
    #[tokio::test]
    async fn load_surfaces_full_io_chain_on_unreadable_file() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "default_temperature = 0.5\n").unwrap();
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o000)).unwrap();

        // Root bypasses file-permission checks — the read would succeed and the
        // assertion would be meaningless, so skip in that environment.
        if std::fs::read_to_string(&config_path).is_ok() {
            return;
        }

        let snapshot = Config {
            config_path: config_path.clone(),
            workspace_dir: tmp.path().join("workspace"),
            ..Default::default()
        };

        let err = reload_config_snapshot_with_timeout(&snapshot)
            .await
            .expect_err("an unreadable config file must fail");

        assert!(
            err.contains("reading config.toml from"),
            "error must carry the read context: {err}"
        );
        assert!(
            err.contains("os error"),
            "error must carry the underlying io cause via {{:#}} (#3962): {err}"
        );
    }
}
