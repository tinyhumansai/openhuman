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
/// An embedder-supplied config short-circuits the disk read entirely — see
/// [`CoreContext::embedder_config`](crate::core::runtime::context::CoreContext::embedder_config).
/// Without that branch, `CoreBuilder::config(..)` would configure boot and
/// nothing else: every handler calls this function per dispatch, so the turn
/// itself would still run against whatever the process-global workspace
/// resolution found. Normalization still runs, because that is a shaping step
/// handlers depend on, not a re-read.
pub async fn load_config_with_timeout() -> Result<Config, String> {
    if let Some(mut config) = crate::core::runtime::context::CoreContext::current_embedder_config()
    {
        normalize_loaded_config(&mut config).await;
        return Ok(config);
    }
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

/// Loads the config that belongs to `workspace_dir`, rather than whichever one
/// the process-global active-user / `OPENHUMAN_WORKSPACE` resolution currently
/// selects.
///
/// Use this from anything scoped to a workspace it was *handed* — the memory
/// subsystem driver is the first such caller. [`load_config_with_timeout`]
/// re-resolves the process-global workspace on every call, so a component bound
/// to workspace B that loads through it and then merely overwrites
/// `workspace_dir` keeps A's embedding routes, model dimensions and provider
/// credentials, and runs them against B's files.
///
/// The config file is looked for beside the workspace, in the two layouts the
/// resolver itself can produce: `<workspace>/config.toml` (a workspace root
/// that carries its own config) and `<workspace>/../config.toml` (the
/// `~/.openhuman/users/<id>/{config.toml,workspace}` layout). When neither
/// exists there is nothing workspace-specific to read, so this falls back to
/// the process-global load with `workspace_dir` re-anchored — the previous
/// behaviour, and still correct for a single-workspace host.
pub async fn load_config_for_workspace_with_timeout(
    workspace_dir: &Path,
) -> Result<Config, String> {
    let candidate = [
        workspace_dir.join("config.toml"),
        workspace_dir
            .parent()
            .map(|parent| parent.join("config.toml"))
            .unwrap_or_default(),
    ]
    .into_iter()
    .find(|path| path.is_file());

    if let Some(config_path) = candidate {
        tracing::debug!(
            config_path = %config_path.display(),
            workspace = %workspace_dir.display(),
            "[config] loading workspace-anchored config"
        );
        return match tokio::time::timeout(
            CONFIG_LOAD_TIMEOUT,
            Config::load_from_config_path(&config_path, workspace_dir),
        )
        .await
        {
            Ok(Ok(mut config)) => {
                normalize_loaded_config(&mut config).await;
                Ok(config)
            }
            Ok(Err(e)) => Err(format!("{e:#}")),
            Err(_) => Err("Config loading timed out".to_string()),
        };
    }

    tracing::debug!(
        workspace = %workspace_dir.display(),
        "[config] no config.toml beside workspace; falling back to the process-global load"
    );
    let mut config = load_config_with_timeout().await?;
    config.workspace_dir = workspace_dir.to_path_buf();
    Ok(config)
}

/// Reloads the config file represented by an existing runtime snapshot.
///
/// Use this for long-lived objects that need fresh config values while
/// staying anchored to their original user/workspace. Unlike
/// [`load_config_with_timeout`], this does not re-resolve the process-global
/// `OPENHUMAN_WORKSPACE` env var on every call.
pub async fn reload_config_snapshot_with_timeout(snapshot: &Config) -> Result<Config, String> {
    reload_config_from_paths(&snapshot.config_path, &snapshot.workspace_dir).await
}

/// The anchored reload, addressed by path rather than by a whole `Config`.
///
/// Callers that hold the extracted memory subsystem's `dyn MemoryHostConfig`
/// cannot produce a concrete `Config` to pass to
/// [`reload_config_snapshot_with_timeout`] — but they can read the two paths
/// off the seam. Same behaviour, narrower argument.
pub async fn reload_config_from_paths(
    config_path: &std::path::Path,
    workspace_dir: &std::path::Path,
) -> Result<Config, String> {
    match tokio::time::timeout(
        CONFIG_LOAD_TIMEOUT,
        Config::load_from_config_path(config_path, workspace_dir),
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
    use crate::openhuman::platform::cost::catalog;

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
    let running = crate::openhuman::platform::service::mock::mock_agent_running().unwrap_or(true);
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
