//! Config load/save and environment variable overrides.

use super::{
    proxy::{
        normalize_no_proxy_list, normalize_proxy_url_option, normalize_service_list,
        parse_proxy_enabled, parse_proxy_scope, set_runtime_proxy_config, ProxyScope,
    },
    Config,
};
use anyhow::{Context, Result};
use directories::UserDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::AsyncWriteExt;

/// Read-only environment lookup used by [`Config::apply_env_overrides`]. The
/// seam lets unit tests exercise the overlay without mutating the process
/// environment (which is racy under parallel tests and requires a shared
/// `TEST_ENV_LOCK`).
///
/// Production code uses [`ProcessEnv`], which delegates to `std::env`.
pub(crate) trait EnvLookup {
    /// Equivalent to `std::env::var(key).ok()`.
    fn get(&self, key: &str) -> Option<String>;

    /// Equivalent to `std::env::var_os(key).is_some()`. Used to distinguish
    /// "variable not present" from "variable set to empty" where it matters
    /// (see `OPENHUMAN_CONTEXT_TOOL_RESULT_BUDGET_BYTES` below).
    fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Looks up the first non-`None` value across `keys`, preserving the
    /// precedence used by the manual `or_else` chains throughout this
    /// module (e.g. `OPENHUMAN_FOO` wins over the bare `FOO` alias).
    fn get_any(&self, keys: &[&str]) -> Option<String> {
        keys.iter().find_map(|k| self.get(k))
    }
}

/// Default [`EnvLookup`] implementation backed by `std::env`.
pub(crate) struct ProcessEnv;

impl EnvLookup for ProcessEnv {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn contains(&self, key: &str) -> bool {
        std::env::var_os(key).is_some()
    }
}

fn default_config_and_workspace_dirs() -> Result<(PathBuf, PathBuf)> {
    let config_dir = default_config_dir()?;
    Ok((config_dir.clone(), config_dir.join("workspace")))
}

/// Parse a boolean env-var value. Accepts the usual truthy/falsy tokens
/// (`1/true/yes/on` and `0/false/no/off`, case-insensitive). Returns `None`
/// on unrecognised values and logs a warning so silent mis-spellings don't
/// invisibly leave the config unchanged.
fn parse_env_bool(name: &str, raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => {
            tracing::warn!(
                env = %name,
                value = %raw,
                "invalid boolean env override ignored; expected 1/true/yes/on or 0/false/no/off"
            );
            None
        }
    }
}

const ACTIVE_WORKSPACE_STATE_FILE: &str = "active_workspace.toml";
static WARNED_WORLD_READABLE_CONFIGS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

#[derive(Debug, Serialize, Deserialize)]
struct ActiveWorkspaceState {
    config_dir: String,
}

fn default_config_dir() -> Result<PathBuf> {
    default_root_openhuman_dir()
}

fn default_root_dir_name() -> &'static str {
    if crate::api::config::is_staging_app_env(crate::api::config::app_env_from_env().as_deref()) {
        ".openhuman-staging"
    } else {
        ".openhuman"
    }
}

/// Returns the root openhuman directory (`~/.openhuman`), independent of any
/// per-user scoping.  Used to locate `active_user.toml` and the shared
/// `users/` tree.
pub fn default_root_openhuman_dir() -> Result<PathBuf> {
    let home = UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .context("Could not find home directory")?;
    Ok(home.join(default_root_dir_name()))
}

fn active_workspace_state_path(default_dir: &Path) -> PathBuf {
    default_dir.join(ACTIVE_WORKSPACE_STATE_FILE)
}

async fn load_persisted_workspace_dirs(
    default_config_dir: &Path,
) -> Result<Option<(PathBuf, PathBuf)>> {
    let state_path = active_workspace_state_path(default_config_dir);
    if !state_path.exists() {
        return Ok(None);
    }

    let contents = match fs::read_to_string(&state_path).await {
        Ok(contents) => contents,
        Err(error) => {
            tracing::warn!(
                "Failed to read active workspace marker {}: {error}",
                state_path.display()
            );
            return Ok(None);
        }
    };

    let state: ActiveWorkspaceState = match toml::from_str(&contents) {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(
                "Failed to parse active workspace marker {}: {error}",
                state_path.display()
            );
            return Ok(None);
        }
    };

    let raw_config_dir = state.config_dir.trim();
    if raw_config_dir.is_empty() {
        tracing::warn!(
            "Ignoring active workspace marker {} because config_dir is empty",
            state_path.display()
        );
        return Ok(None);
    }

    let parsed_dir = PathBuf::from(raw_config_dir);
    let config_dir = if parsed_dir.is_absolute() {
        parsed_dir
    } else {
        default_config_dir.join(parsed_dir)
    };
    Ok(Some((config_dir.clone(), config_dir.join("workspace"))))
}

pub(crate) async fn persist_active_workspace_config_dir(config_dir: &Path) -> Result<()> {
    let default_config_dir = default_config_dir()?;
    let state_path = active_workspace_state_path(&default_config_dir);

    if config_dir == default_config_dir {
        if state_path.exists() {
            fs::remove_file(&state_path).await.with_context(|| {
                format!(
                    "Failed to clear active workspace marker: {}",
                    state_path.display()
                )
            })?;
        }
        return Ok(());
    }

    fs::create_dir_all(&default_config_dir)
        .await
        .with_context(|| {
            format!(
                "Failed to create default config directory: {}",
                default_config_dir.display()
            )
        })?;

    let state = ActiveWorkspaceState {
        config_dir: config_dir.to_string_lossy().into_owned(),
    };
    let serialized =
        toml::to_string_pretty(&state).context("Failed to serialize active workspace marker")?;

    let temp_path = default_config_dir.join(format!(
        ".{ACTIVE_WORKSPACE_STATE_FILE}.tmp-{}",
        uuid::Uuid::new_v4()
    ));
    fs::write(&temp_path, serialized).await.with_context(|| {
        format!(
            "Failed to write temporary active workspace marker: {}",
            temp_path.display()
        )
    })?;

    if let Err(error) = fs::rename(&temp_path, &state_path).await {
        let _ = fs::remove_file(&temp_path).await;
        anyhow::bail!(
            "Failed to atomically persist active workspace marker {}: {error}",
            state_path.display()
        );
    }

    sync_directory(&default_config_dir).await?;
    Ok(())
}

fn resolve_config_dir_for_workspace(workspace_dir: &Path) -> (PathBuf, PathBuf) {
    let workspace_config_dir = workspace_dir.to_path_buf();
    if workspace_config_dir.join("config.toml").exists() {
        return (
            workspace_config_dir.clone(),
            workspace_config_dir.join("workspace"),
        );
    }

    let legacy_config_dir = workspace_dir
        .parent()
        .map(|parent| parent.join(".openhuman"));
    if let Some(legacy_dir) = legacy_config_dir {
        if legacy_dir.join("config.toml").exists() {
            return (legacy_dir, workspace_config_dir);
        }

        if workspace_dir
            .file_name()
            .is_some_and(|name| name == std::ffi::OsStr::new("workspace"))
        {
            return (legacy_dir, workspace_config_dir);
        }
    }

    (
        workspace_config_dir.clone(),
        workspace_config_dir.join("workspace"),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConfigResolutionSource {
    EnvWorkspace,
    ActiveWorkspaceMarker,
    ActiveUser,
    DefaultConfigDir,
}

impl ConfigResolutionSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::EnvWorkspace => "OPENHUMAN_WORKSPACE",
            Self::ActiveWorkspaceMarker => "active_workspace.toml",
            Self::ActiveUser => "active_user.toml",
            Self::DefaultConfigDir => "default",
        }
    }
}

async fn resolve_runtime_config_dirs(
    default_openhuman_dir: &Path,
    default_workspace_dir: &Path,
) -> Result<(PathBuf, PathBuf, ConfigResolutionSource)> {
    resolve_runtime_config_dirs_with(default_openhuman_dir, default_workspace_dir, &ProcessEnv)
        .await
}

/// Env-injectable variant of [`resolve_runtime_config_dirs`]. Accepts any
/// [`EnvLookup`] so unit tests can exercise the `OPENHUMAN_WORKSPACE`
/// override path without mutating the process environment.
async fn resolve_runtime_config_dirs_with(
    default_openhuman_dir: &Path,
    default_workspace_dir: &Path,
    env: &(dyn EnvLookup + Send + Sync),
) -> Result<(PathBuf, PathBuf, ConfigResolutionSource)> {
    // 1. Explicit env override always wins.
    if let Some(custom_workspace) = env.get("OPENHUMAN_WORKSPACE") {
        if !custom_workspace.is_empty() {
            let (openhuman_dir, workspace_dir) =
                resolve_config_dir_for_workspace(&PathBuf::from(custom_workspace));
            return Ok((
                openhuman_dir,
                workspace_dir,
                ConfigResolutionSource::EnvWorkspace,
            ));
        }
    }

    resolve_config_dirs_ignoring_env(default_openhuman_dir, default_workspace_dir).await
}

/// Same as [`resolve_runtime_config_dirs`] but skips the
/// `OPENHUMAN_WORKSPACE` env var override. Used by
/// [`Config::load_from_default_paths`] so callers can reliably load
/// the real user config without mutating the process environment.
async fn resolve_config_dirs_ignoring_env(
    default_openhuman_dir: &Path,
    default_workspace_dir: &Path,
) -> Result<(PathBuf, PathBuf, ConfigResolutionSource)> {
    // 2. Active user — scopes the entire openhuman dir to a per-user directory
    //    so that config, auth, encryption, and workspace are all user-isolated.
    if let Some(user_id) = read_active_user_id(default_openhuman_dir) {
        let user_dir = user_openhuman_dir(default_openhuman_dir, &user_id);
        let user_workspace = user_dir.join("workspace");
        tracing::debug!(
            user_id = %user_id,
            user_dir = %user_dir.display(),
            "Config dirs resolved via active_user.toml"
        );
        return Ok((user_dir, user_workspace, ConfigResolutionSource::ActiveUser));
    }

    // 3. Active workspace marker (legacy / multi-workspace).
    if let Some((openhuman_dir, workspace_dir)) =
        load_persisted_workspace_dirs(default_openhuman_dir).await?
    {
        return Ok((
            openhuman_dir,
            workspace_dir,
            ConfigResolutionSource::ActiveWorkspaceMarker,
        ));
    }

    // 4. Default: no login yet. Encapsulate config/memory/state under the
    //    pre-login user directory so everything is user-scoped from the very
    //    first init. On first real login, this directory is migrated to the
    //    authenticated user id (see `credentials::ops::store_session`).
    let user_dir = pre_login_user_dir(default_openhuman_dir);
    let user_workspace = user_dir.join("workspace");
    tracing::debug!(
        user_id = %PRE_LOGIN_USER_ID,
        user_dir = %user_dir.display(),
        default_workspace_dir = %default_workspace_dir.display(),
        "Config dirs resolved to pre-login user directory (no active user, no workspace marker)"
    );
    Ok((
        user_dir,
        user_workspace,
        ConfigResolutionSource::DefaultConfigDir,
    ))
}

fn decrypt_optional_secret(
    store: &crate::openhuman::security::SecretStore,
    value: &mut Option<String>,
    field_name: &str,
) -> Result<()> {
    if let Some(raw) = value.clone() {
        if crate::openhuman::security::SecretStore::is_encrypted(&raw) {
            *value = Some(
                store
                    .decrypt(&raw)
                    .with_context(|| format!("Failed to decrypt {field_name}"))?,
            );
        }
    }
    Ok(())
}

fn encrypt_optional_secret(
    store: &crate::openhuman::security::SecretStore,
    value: &mut Option<String>,
    field_name: &str,
) -> Result<()> {
    if let Some(raw) = value.clone() {
        if !crate::openhuman::security::SecretStore::is_encrypted(&raw) {
            *value = Some(
                store
                    .encrypt(&raw)
                    .with_context(|| format!("Failed to encrypt {field_name}"))?,
            );
        }
    }
    Ok(())
}

const ACTIVE_USER_STATE_FILE: &str = "active_user.toml";

#[derive(Debug, Serialize, Deserialize)]
struct ActiveUserState {
    user_id: String,
}

/// Reads the active user id from `{default_openhuman_dir}/active_user.toml`.
/// Returns `None` when the file does not exist, is empty, or cannot be parsed.
pub fn read_active_user_id(default_openhuman_dir: &Path) -> Option<String> {
    let path = default_openhuman_dir.join(ACTIVE_USER_STATE_FILE);
    let contents = std::fs::read_to_string(&path).ok()?;
    let state: ActiveUserState = toml::from_str(&contents).ok()?;
    let id = state.user_id.trim().to_string();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

/// Writes the active user id to `{default_openhuman_dir}/active_user.toml`.
pub fn write_active_user_id(default_openhuman_dir: &Path, user_id: &str) -> Result<()> {
    let path = default_openhuman_dir.join(ACTIVE_USER_STATE_FILE);
    let state = ActiveUserState {
        user_id: user_id.to_string(),
    };
    let toml_str = toml::to_string_pretty(&state).context("serialize active_user.toml")?;
    std::fs::write(&path, toml_str)
        .with_context(|| format!("Failed to write active user state: {}", path.display()))?;
    tracing::debug!(user_id = %user_id, path = %path.display(), "active user written");
    Ok(())
}

/// Removes the active user marker.  After this, the next config load will
/// use the default (unauthenticated) openhuman directory.
pub fn clear_active_user(default_openhuman_dir: &Path) -> Result<()> {
    let path = default_openhuman_dir.join(ACTIVE_USER_STATE_FILE);
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("Failed to remove active user state: {}", path.display()))?;
        tracing::debug!(path = %path.display(), "active user cleared");
    }
    Ok(())
}

/// Returns the user-scoped openhuman directory for the given user id:
/// `{default_openhuman_dir}/users/{user_id}`.
pub fn user_openhuman_dir(default_openhuman_dir: &Path, user_id: &str) -> PathBuf {
    default_openhuman_dir.join("users").join(user_id)
}

/// Stable id used to scope the openhuman directory before any user has
/// logged in.  All memory, state, config, sessions and workspace files
/// created on first init land under `{root}/users/{PRE_LOGIN_USER_ID}`
/// so nothing is ever written directly at the root `.openhuman` path.
///
/// On first successful login, this directory is migrated into the real
/// user-scoped directory (see `credentials::ops::store_session`).
pub const PRE_LOGIN_USER_ID: &str = "local";

/// Returns the pre-login (unauthenticated) user directory:
/// `{default_openhuman_dir}/users/local`.
pub fn pre_login_user_dir(default_openhuman_dir: &Path) -> PathBuf {
    user_openhuman_dir(default_openhuman_dir, PRE_LOGIN_USER_ID)
}

fn migrate_legacy_autocomplete_disabled_apps(config: &mut Config) {
    // Legacy defaults blocked both terminal and code, which prevented Codex/CLI usage.
    // Migrate only the exact legacy default so custom user preferences remain untouched.
    let mut normalized: Vec<String> = config
        .autocomplete
        .disabled_apps
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect();
    normalized.sort();
    normalized.dedup();

    if normalized == ["code".to_string(), "terminal".to_string()] {
        config.autocomplete.disabled_apps = vec!["code".to_string()];
    }
}

#[cfg(unix)]
async fn sync_directory(path: &Path) -> Result<()> {
    let dir = File::open(path)
        .await
        .with_context(|| format!("Failed to open directory for fsync: {}", path.display()))?;
    dir.sync_all()
        .await
        .with_context(|| format!("Failed to fsync directory metadata: {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
async fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

impl Config {
    pub async fn load_or_init() -> Result<Self> {
        let (default_openhuman_dir, default_workspace_dir) = default_config_and_workspace_dirs()?;

        let (openhuman_dir, workspace_dir, resolution_source) =
            resolve_runtime_config_dirs(&default_openhuman_dir, &default_workspace_dir).await?;

        let config_path = openhuman_dir.join("config.toml");

        // Pre-login path: no active user, no workspace marker, no env override,
        // and no existing config.toml on disk.  Return an in-memory default
        // config without creating any directories or writing any files — disk
        // state is deferred until the first successful login in
        // `credentials::ops::store_session`, which writes `active_user.toml`
        // and triggers a reload that materializes the user-scoped directory.
        if resolution_source == ConfigResolutionSource::DefaultConfigDir && !config_path.exists() {
            let mut config = Config {
                config_path: config_path.clone(),
                workspace_dir: workspace_dir.clone(),
                ..Default::default()
            };
            config.apply_env_overrides();

            tracing::debug!(
                path = %config.config_path.display(),
                workspace = %config.workspace_dir.display(),
                source = resolution_source.as_str(),
                initialized = false,
                persisted = false,
                "Config loaded (pre-login, in-memory only — no dirs or files written)"
            );
            return Ok(config);
        }

        fs::create_dir_all(&openhuman_dir)
            .await
            .context("Failed to create config directory")?;
        fs::create_dir_all(&workspace_dir)
            .await
            .context("Failed to create workspace directory")?;

        if config_path.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = fs::metadata(&config_path).await {
                    if meta.permissions().mode() & 0o004 != 0 {
                        let warned = WARNED_WORLD_READABLE_CONFIGS
                            .get_or_init(|| Mutex::new(HashSet::new()));
                        let mut warned_guard = warned.lock().unwrap_or_else(|e| e.into_inner());
                        if warned_guard.insert(config_path.clone()) {
                            tracing::warn!(
                                "Config file {:?} is world-readable (mode {:o}). \
                                 Consider restricting with: chmod 600 {:?}",
                                config_path,
                                meta.permissions().mode() & 0o777,
                                config_path,
                            );
                        }
                    }
                }
            }

            let contents = fs::read_to_string(&config_path)
                .await
                .context("Failed to read config file")?;
            let mut config: Config = toml::from_str(&contents).with_context(|| {
                format!("Failed to parse config file {}", config_path.display())
            })?;
            config.config_path = config_path.clone();
            config.workspace_dir = workspace_dir;
            migrate_legacy_autocomplete_disabled_apps(&mut config);
            config.apply_env_overrides();

            tracing::debug!(
                path = %config.config_path.display(),
                workspace = %config.workspace_dir.display(),
                source = resolution_source.as_str(),
                initialized = false,
                "Config loaded"
            );
            Ok(config)
        } else {
            let mut config = Config {
                config_path: config_path.clone(),
                workspace_dir,
                ..Default::default()
            };
            config.save().await?;

            #[cfg(unix)]
            {
                use std::{fs::Permissions, os::unix::fs::PermissionsExt};
                let _ = fs::set_permissions(&config_path, Permissions::from_mode(0o600)).await;
            }

            config.apply_env_overrides();

            tracing::debug!(
                path = %config.config_path.display(),
                workspace = %config.workspace_dir.display(),
                source = resolution_source.as_str(),
                initialized = true,
                "Config loaded"
            );
            Ok(config)
        }
    }

    /// Load config from the default user paths, bypassing the
    /// `OPENHUMAN_WORKSPACE` environment variable.
    ///
    /// This is used by the debug dump to load the real user config
    /// for auth token resolution when the dump script overrides
    /// `OPENHUMAN_WORKSPACE` to a throwaway temp directory.
    pub async fn load_from_default_paths() -> Result<Self> {
        let (default_openhuman_dir, default_workspace_dir) = default_config_and_workspace_dirs()?;
        let (openhuman_dir, workspace_dir, _source) =
            resolve_config_dirs_ignoring_env(&default_openhuman_dir, &default_workspace_dir)
                .await?;
        let config_path = openhuman_dir.join("config.toml");

        if !config_path.exists() {
            let mut config = Config {
                config_path,
                workspace_dir,
                ..Default::default()
            };
            config.apply_env_overrides();
            return Ok(config);
        }

        let raw = fs::read_to_string(&config_path)
            .await
            .context("reading config.toml from default paths")?;
        let mut config: Config =
            toml::from_str(&raw).context("parsing config.toml from default paths")?;
        config.config_path = config_path;
        config.workspace_dir = workspace_dir;
        config.apply_env_overrides();
        Ok(config)
    }

    pub fn apply_env_overrides(&mut self) {
        self.apply_env_overlay_with(&ProcessEnv);

        // The pure overlay above never mutates process-level state. The
        // two side effects below remain here so tests driving
        // `apply_env_overlay_with` directly don't clobber the shared
        // runtime proxy client cache or mutate `HTTP_PROXY` / etc. on
        // the running process.
        if self.proxy.enabled && self.proxy.scope == ProxyScope::Environment {
            self.proxy.apply_to_process_env();
        }

        set_runtime_proxy_config(self.proxy.clone());
    }

    /// Pure-ish env overlay: applies overrides read from `env` to `self`.
    ///
    /// "Pure-ish" because it still emits `tracing` logs and calls
    /// `self.proxy.validate()` (which only reads). Crucially, it does
    /// **not** write to the process environment nor the
    /// `set_runtime_proxy_config` global — those stay in the public
    /// [`Self::apply_env_overrides`] wrapper so unit tests can call this
    /// with a [`HashMapEnv`] (see tests) without requiring the
    /// `TEST_ENV_LOCK` or tainting sibling tests.
    pub(crate) fn apply_env_overlay_with<E: EnvLookup>(&mut self, env: &E) {
        if let Some(model) = env.get_any(&["OPENHUMAN_MODEL", "MODEL"]) {
            if !model.is_empty() {
                self.default_model = Some(model);
            }
        }

        if let Some(workspace) = env.get("OPENHUMAN_WORKSPACE") {
            if !workspace.is_empty() {
                let (_, workspace_dir) =
                    resolve_config_dir_for_workspace(&PathBuf::from(workspace));
                self.workspace_dir = workspace_dir;
            }
        }

        if let Some(temp_str) = env.get("OPENHUMAN_TEMPERATURE") {
            if let Ok(temp) = temp_str.parse::<f64>() {
                if (0.0..=2.0).contains(&temp) {
                    self.default_temperature = temp;
                }
            }
        }

        if let Some(flag) = env.get_any(&["OPENHUMAN_REASONING_ENABLED", "REASONING_ENABLED"]) {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.runtime.reasoning_enabled = Some(true),
                "0" | "false" | "no" | "off" => self.runtime.reasoning_enabled = Some(false),
                _ => {}
            }
        }

        // `OPENHUMAN_WEB_SEARCH_ENABLED` is intentionally ignored —
        // web search is unconditionally registered in the tool set.
        // Only the result/timeout budget knobs remain environment-configurable.
        if env.contains("OPENHUMAN_WEB_SEARCH_ENABLED") {
            log::warn!(
                "[config] OPENHUMAN_WEB_SEARCH_ENABLED is deprecated and ignored — \
                 web search is always registered; provider/API-key overrides were removed."
            );
        }

        if let Some(max_results) =
            env.get_any(&["OPENHUMAN_WEB_SEARCH_MAX_RESULTS", "WEB_SEARCH_MAX_RESULTS"])
        {
            if let Ok(max_results) = max_results.parse::<usize>() {
                if (1..=10).contains(&max_results) {
                    self.web_search.max_results = max_results;
                }
            }
        }

        if let Some(timeout_secs) = env.get_any(&[
            "OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS",
            "WEB_SEARCH_TIMEOUT_SECS",
        ]) {
            if let Ok(timeout_secs) = timeout_secs.parse::<u64>() {
                if timeout_secs > 0 {
                    self.web_search.timeout_secs = timeout_secs;
                }
            }
        }

        let explicit_proxy_enabled = env
            .get("OPENHUMAN_PROXY_ENABLED")
            .as_deref()
            .and_then(parse_proxy_enabled);
        if let Some(enabled) = explicit_proxy_enabled {
            self.proxy.enabled = enabled;
        }

        let mut proxy_url_overridden = false;
        if let Some(proxy_url) = env.get_any(&["OPENHUMAN_HTTP_PROXY", "HTTP_PROXY"]) {
            self.proxy.http_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Some(proxy_url) = env.get_any(&["OPENHUMAN_HTTPS_PROXY", "HTTPS_PROXY"]) {
            self.proxy.https_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Some(proxy_url) = env.get_any(&["OPENHUMAN_ALL_PROXY", "ALL_PROXY"]) {
            self.proxy.all_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Some(no_proxy) = env.get_any(&["OPENHUMAN_NO_PROXY", "NO_PROXY"]) {
            self.proxy.no_proxy = normalize_no_proxy_list(vec![no_proxy]);
        }

        if explicit_proxy_enabled.is_none()
            && proxy_url_overridden
            && self.proxy.has_any_proxy_url()
        {
            self.proxy.enabled = true;
        }

        if let Some(scope_raw) = env.get("OPENHUMAN_PROXY_SCOPE") {
            let trimmed = scope_raw.trim();
            if !trimmed.is_empty() {
                match parse_proxy_scope(trimmed) {
                    Some(scope) => self.proxy.scope = scope,
                    None => {
                        tracing::warn!("Invalid OPENHUMAN_PROXY_SCOPE value {:?} ignored", trimmed);
                    }
                }
            }
        }

        if let Some(services_raw) = env.get("OPENHUMAN_PROXY_SERVICES") {
            self.proxy.services = normalize_service_list(vec![services_raw]);
        }

        if let Err(error) = self.proxy.validate() {
            tracing::warn!("Invalid proxy configuration ignored: {error}");
            self.proxy.enabled = false;
        }

        if let Some(tier_str) = env.get("OPENHUMAN_LOCAL_AI_TIER") {
            let tier_str = tier_str.trim().to_ascii_lowercase();
            if !tier_str.is_empty() {
                if let Some(tier) =
                    crate::openhuman::local_ai::presets::ModelTier::from_str_opt(&tier_str)
                {
                    if tier == crate::openhuman::local_ai::presets::ModelTier::Custom {
                        tracing::warn!(
                            tier = %tier_str,
                            "ignoring custom OPENHUMAN_LOCAL_AI_TIER; only built-in presets are supported"
                        );
                    } else if !tier.is_mvp_allowed() {
                        tracing::warn!(
                            tier = %tier_str,
                            "ignoring OPENHUMAN_LOCAL_AI_TIER outside the 1B local-model allowlist"
                        );
                    } else {
                        crate::openhuman::local_ai::presets::apply_preset_to_config(
                            &mut self.local_ai,
                            tier,
                        );
                        tracing::debug!(tier = %tier_str, "applied local AI tier from OPENHUMAN_LOCAL_AI_TIER");
                    }
                } else {
                    tracing::warn!(
                        tier = %tier_str,
                        "ignoring invalid OPENHUMAN_LOCAL_AI_TIER (valid: ram_2_4gb)"
                    );
                }
            }
        }

        // Node runtime overrides
        if let Some(flag) = env.get("OPENHUMAN_NODE_ENABLED") {
            if let Some(enabled) = parse_env_bool("OPENHUMAN_NODE_ENABLED", &flag) {
                self.node.enabled = enabled;
            }
        }
        if let Some(version) = env.get("OPENHUMAN_NODE_VERSION") {
            let trimmed = version.trim();
            if !trimmed.is_empty() {
                self.node.version = trimmed.to_string();
            }
        }
        if let Some(dir) = env.get("OPENHUMAN_NODE_CACHE_DIR") {
            let trimmed = dir.trim();
            if !trimmed.is_empty() {
                self.node.cache_dir = trimmed.to_string();
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_NODE_PREFER_SYSTEM") {
            if let Some(prefer_system) = parse_env_bool("OPENHUMAN_NODE_PREFER_SYSTEM", &flag) {
                self.node.prefer_system = prefer_system;
            }
        }

        let dsn_value = env
            .get("OPENHUMAN_SENTRY_DSN")
            .or_else(|| option_env!("OPENHUMAN_SENTRY_DSN").map(|s| s.to_string()));
        if let Some(dsn) = dsn_value {
            let dsn = dsn.trim();
            if !dsn.is_empty() {
                self.observability.sentry_dsn = Some(dsn.to_string());
            }
        }

        if let Some(flag) = env.get("OPENHUMAN_ANALYTICS_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.observability.analytics_enabled = true,
                "0" | "false" | "no" | "off" => self.observability.analytics_enabled = false,
                _ => {}
            }
        }

        // Learning subsystem overrides
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.enabled = true,
                "0" | "false" | "no" | "off" => self.learning.enabled = false,
                _ => {}
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_REFLECTION_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.reflection_enabled = true,
                "0" | "false" | "no" | "off" => self.learning.reflection_enabled = false,
                _ => {}
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_USER_PROFILE_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.user_profile_enabled = true,
                "0" | "false" | "no" | "off" => self.learning.user_profile_enabled = false,
                _ => {}
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_TOOL_TRACKING_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.tool_tracking_enabled = true,
                "0" | "false" | "no" | "off" => self.learning.tool_tracking_enabled = false,
                _ => {}
            }
        }
        if let Some(source) = env.get("OPENHUMAN_LEARNING_REFLECTION_SOURCE") {
            let normalized = source.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "local" => {
                    self.learning.reflection_source =
                        crate::openhuman::config::ReflectionSource::Local
                }
                "cloud" => {
                    self.learning.reflection_source =
                        crate::openhuman::config::ReflectionSource::Cloud
                }
                _ => {
                    tracing::warn!(
                        source = %source,
                        "ignoring invalid OPENHUMAN_LEARNING_REFLECTION_SOURCE (valid: local, cloud)"
                    );
                }
            }
        }
        if let Some(val) = env.get("OPENHUMAN_LEARNING_MAX_REFLECTIONS_PER_SESSION") {
            if let Ok(max) = val.trim().parse::<usize>() {
                self.learning.max_reflections_per_session = max;
            }
        }
        if let Some(val) = env.get("OPENHUMAN_LEARNING_MIN_TURN_COMPLEXITY") {
            if let Ok(min) = val.trim().parse::<usize>() {
                self.learning.min_turn_complexity = min;
            }
        }

        // Phase 4 memory-tree embedding overrides (#710). Setting the env
        // var to an empty string explicitly clears the default — useful
        // for CI and other environments that want to opt into the
        // InertEmbedder fallback without editing config.toml.
        if let Ok(endpoint) = std::env::var("OPENHUMAN_MEMORY_EMBED_ENDPOINT") {
            let trimmed = endpoint.trim();
            self.memory_tree.embedding_endpoint = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(model) = std::env::var("OPENHUMAN_MEMORY_EMBED_MODEL") {
            let trimmed = model.trim();
            self.memory_tree.embedding_model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(val) = std::env::var("OPENHUMAN_MEMORY_EMBED_TIMEOUT_MS") {
            if let Ok(timeout_ms) = val.trim().parse::<u64>() {
                if timeout_ms > 0 {
                    self.memory_tree.embedding_timeout_ms = Some(timeout_ms);
                }
            }
        }
        if let Ok(flag) = std::env::var("OPENHUMAN_MEMORY_EMBED_STRICT") {
            if let Some(strict) = parse_env_bool("OPENHUMAN_MEMORY_EMBED_STRICT", &flag) {
                self.memory_tree.embedding_strict = strict;
            }
        }

        // LLM entity extractor overrides — set endpoint + model to route
        // ingest scoring through Ollama NER (Phase 2 follow-up). Empty
        // string explicitly clears (opts out).
        if let Ok(endpoint) = std::env::var("OPENHUMAN_MEMORY_EXTRACT_ENDPOINT") {
            let trimmed = endpoint.trim();
            self.memory_tree.llm_extractor_endpoint = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(model) = std::env::var("OPENHUMAN_MEMORY_EXTRACT_MODEL") {
            let trimmed = model.trim();
            self.memory_tree.llm_extractor_model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(val) = std::env::var("OPENHUMAN_MEMORY_EXTRACT_TIMEOUT_MS") {
            if let Ok(ms) = val.trim().parse::<u64>() {
                if ms > 0 {
                    self.memory_tree.llm_extractor_timeout_ms = Some(ms);
                }
            }
        }

        // LLM summariser overrides — set endpoint + model to route
        // bucket-seal summaries through Ollama instead of InertSummariser
        // (Phase 3a real-summariser hook).
        if let Ok(endpoint) = std::env::var("OPENHUMAN_MEMORY_SUMMARISE_ENDPOINT") {
            let trimmed = endpoint.trim();
            self.memory_tree.llm_summariser_endpoint = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(model) = std::env::var("OPENHUMAN_MEMORY_SUMMARISE_MODEL") {
            let trimmed = model.trim();
            self.memory_tree.llm_summariser_model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(val) = std::env::var("OPENHUMAN_MEMORY_SUMMARISE_TIMEOUT_MS") {
            if let Ok(ms) = val.trim().parse::<u64>() {
                if ms > 0 {
                    self.memory_tree.llm_summariser_timeout_ms = Some(ms);
                }
            }
        }

        // Phase MD-content: chunk body directory override. Empty string means
        // "fall back to default", consistent with other memory_tree env vars.
        if let Ok(dir) = std::env::var("OPENHUMAN_MEMORY_TREE_CONTENT_DIR") {
            let trimmed = dir.trim();
            self.memory_tree.content_dir = if trimmed.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(trimmed))
            };
        }

        // Auto-update overrides
        if let Some(flag) = env.get("OPENHUMAN_AUTO_UPDATE_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.update.enabled = true,
                "0" | "false" | "no" | "off" => self.update.enabled = false,
                _ => {}
            }
        }
        if let Some(val) = env.get("OPENHUMAN_AUTO_UPDATE_INTERVAL_MINUTES") {
            if let Ok(minutes) = val.trim().parse::<u32>() {
                self.update.interval_minutes = minutes;
            }
        }

        // Dictation overrides
        if let Some(flag) = env.get("OPENHUMAN_DICTATION_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.dictation.enabled = true,
                "0" | "false" | "no" | "off" => self.dictation.enabled = false,
                _ => {}
            }
        }
        if let Some(hotkey) = env.get("OPENHUMAN_DICTATION_HOTKEY") {
            let hotkey = hotkey.trim();
            if !hotkey.is_empty() {
                self.dictation.hotkey = hotkey.to_string();
            }
        }
        if let Some(mode) = env.get("OPENHUMAN_DICTATION_ACTIVATION_MODE") {
            let normalized = mode.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "toggle" => {
                    self.dictation.activation_mode =
                        crate::openhuman::config::DictationActivationMode::Toggle
                }
                "push" => {
                    self.dictation.activation_mode =
                        crate::openhuman::config::DictationActivationMode::Push
                }
                _ => {
                    tracing::warn!(
                        mode = %mode,
                        "ignoring invalid OPENHUMAN_DICTATION_ACTIVATION_MODE (valid: toggle, push)"
                    );
                }
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_DICTATION_LLM_REFINEMENT") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.dictation.llm_refinement = true,
                "0" | "false" | "no" | "off" => self.dictation.llm_refinement = false,
                _ => {}
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_DICTATION_STREAMING") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.dictation.streaming = true,
                "0" | "false" | "no" | "off" => self.dictation.streaming = false,
                _ => {}
            }
        }
        if let Some(val) = env.get("OPENHUMAN_DICTATION_STREAMING_INTERVAL_MS") {
            if let Ok(ms) = val.trim().parse::<u64>() {
                self.dictation.streaming_interval_ms = ms;
            }
        }

        // ── Context management overrides ───────────────────────────────
        if let Some(flag) = env.get("OPENHUMAN_CONTEXT_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.context.enabled = true,
                "0" | "false" | "no" | "off" => self.context.enabled = false,
                _ => {}
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_CONTEXT_MICROCOMPACT_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.context.microcompact_enabled = true,
                "0" | "false" | "no" | "off" => self.context.microcompact_enabled = false,
                _ => {}
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_CONTEXT_AUTOCOMPACT_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.context.autocompact_enabled = true,
                "0" | "false" | "no" | "off" => self.context.autocompact_enabled = false,
                _ => {}
            }
        }
        if let Some(val) = env.get("OPENHUMAN_CONTEXT_TOOL_RESULT_BUDGET_BYTES") {
            if let Ok(n) = val.trim().parse::<usize>() {
                self.context.tool_result_budget_bytes = n;
            }
        }
        if let Some(model) = env.get("OPENHUMAN_CONTEXT_SUMMARIZER_MODEL") {
            let model = model.trim();
            if !model.is_empty() {
                self.context.summarizer_model = Some(model.to_string());
            }
        }

        // Migration: `agent.tool_result_budget_bytes` used to own this
        // knob before it moved to `context.tool_result_budget_bytes`. If
        // an existing config.toml sets the old field to a non-default
        // value and the new field is still at its default AND the env
        // var is not present, copy the old value forward and emit a
        // deprecation warning so the user knows to move it. The env var
        // check is important: without it a user who explicitly sets
        // `OPENHUMAN_CONTEXT_TOOL_RESULT_BUDGET_BYTES` to the default
        // value would have their env override silently clobbered by the
        // agent-field migration.
        let context_default = crate::openhuman::context::DEFAULT_TOOL_RESULT_BUDGET_BYTES;
        let context_env_set = env.contains("OPENHUMAN_CONTEXT_TOOL_RESULT_BUDGET_BYTES");
        if !context_env_set
            && self.context.tool_result_budget_bytes == context_default
            && self.agent.tool_result_budget_bytes != context_default
        {
            tracing::warn!(
                old = self.agent.tool_result_budget_bytes,
                "[context:config] `agent.tool_result_budget_bytes` is \
                 deprecated — please move it to \
                 `context.tool_result_budget_bytes` in your config.toml"
            );
            self.context.tool_result_budget_bytes = self.agent.tool_result_budget_bytes;
        }
    }

    pub async fn save(&self) -> Result<()> {
        let config_to_save = self.clone();

        let toml_str =
            toml::to_string_pretty(&config_to_save).context("Failed to serialize config")?;

        let parent_dir = self
            .config_path
            .parent()
            .context("Config path must have a parent directory")?;

        fs::create_dir_all(parent_dir).await.with_context(|| {
            format!(
                "Failed to create config directory: {}",
                parent_dir.display()
            )
        })?;

        let file_name = self
            .config_path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("config.toml");
        let temp_path = parent_dir.join(format!(".{file_name}.tmp-{}", uuid::Uuid::new_v4()));
        let backup_path = parent_dir.join(format!("{file_name}.bak"));

        let mut temp_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .await
            .with_context(|| {
                format!(
                    "Failed to create temporary config file: {}",
                    temp_path.display()
                )
            })?;
        temp_file
            .write_all(toml_str.as_bytes())
            .await
            .context("Failed to write temporary config contents")?;
        temp_file
            .sync_all()
            .await
            .context("Failed to fsync temporary config file")?;
        drop(temp_file);

        let had_existing_config = self.config_path.exists();
        if had_existing_config {
            fs::copy(&self.config_path, &backup_path)
                .await
                .with_context(|| {
                    format!(
                        "Failed to create config backup before atomic replace: {}",
                        backup_path.display()
                    )
                })?;
        }

        if let Err(e) = fs::rename(&temp_path, &self.config_path).await {
            let _ = fs::remove_file(&temp_path).await;
            if had_existing_config && backup_path.exists() {
                fs::copy(&backup_path, &self.config_path)
                    .await
                    .context("Failed to restore config backup")?;
            }
            anyhow::bail!("Failed to atomically replace config file: {e}");
        }

        sync_directory(parent_dir).await?;

        if had_existing_config {
            let _ = fs::remove_file(&backup_path).await;
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "load_tests.rs"]
mod tests;
