//! Config load/save and environment variable overrides.

use super::{
    proxy::{
        normalize_no_proxy_list, normalize_proxy_url_option, normalize_service_list,
        parse_proxy_enabled, parse_proxy_scope, set_runtime_proxy_config, ProxyScope,
    },
    Config, UpdateRestartStrategy,
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

/// Process env lookup that preserves every override except
/// `OPENHUMAN_WORKSPACE`.
struct ProcessEnvWithoutWorkspace;

impl EnvLookup for ProcessEnvWithoutWorkspace {
    fn get(&self, key: &str) -> Option<String> {
        if key == "OPENHUMAN_WORKSPACE" {
            None
        } else {
            ProcessEnv.get(key)
        }
    }

    fn contains(&self, key: &str) -> bool {
        if key == "OPENHUMAN_WORKSPACE" {
            false
        } else {
            ProcessEnv.contains(key)
        }
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

/// Decrypt all secret fields in the configuration that are marked as encrypted.
///
/// Called during config load when `secrets.encrypt` is true. Only decrypts
/// values that have the `enc:` or `enc2:` prefix; plaintext values are
/// returned as-is. This is a no-op when encryption is disabled.
fn decrypt_config_secrets(config: &mut Config, openhuman_dir: &Path) -> Result<()> {
    if !config.secrets.encrypt {
        return Ok(());
    }
    let store = crate::openhuman::security::SecretStore::new(openhuman_dir, true);

    decrypt_optional_secret(&store, &mut config.api_key, "api_key")?;

    // Channels: decrypt every optional secret field.
    //
    // For required (non-Option<String>) secret fields we wrap the value in a
    // temporary Option, run `decrypt_optional_secret`, then write back via
    // `unwrap_or_default`. This mirrors the encrypt path and — crucially —
    // propagates real decryption errors via `?` instead of silently handing
    // ciphertext back to channel code on a corrupted `enc2:` value.
    // Plaintext values (no `enc:`/`enc2:` prefix) are passed through
    // untouched by `SecretStore::decrypt`, so configs written by pre-#1900
    // builds continue to load correctly.
    let ch = &mut config.channels_config;
    if let Some(ref mut tg) = ch.telegram {
        let mut tok = Some(tg.bot_token.clone());
        decrypt_optional_secret(&store, &mut tok, "telegram.bot_token")?;
        tg.bot_token = tok.unwrap_or_default();
    }
    if let Some(ref mut d) = ch.discord {
        let mut tok = Some(d.bot_token.clone());
        decrypt_optional_secret(&store, &mut tok, "discord.bot_token")?;
        d.bot_token = tok.unwrap_or_default();
    }
    if let Some(ref mut s) = ch.slack {
        let mut tok = Some(s.bot_token.clone());
        decrypt_optional_secret(&store, &mut tok, "slack.bot_token")?;
        s.bot_token = tok.unwrap_or_default();
        decrypt_optional_secret(&store, &mut s.app_token, "slack.app_token")?;
    }
    if let Some(ref mut m) = ch.mattermost {
        let mut tok = Some(m.bot_token.clone());
        decrypt_optional_secret(&store, &mut tok, "mattermost.bot_token")?;
        m.bot_token = tok.unwrap_or_default();
    }
    if let Some(ref mut w) = ch.webhook {
        decrypt_optional_secret(&store, &mut w.secret, "webhook.secret")?;
    }
    if let Some(ref mut mx) = ch.matrix {
        let mut tok = Some(mx.access_token.clone());
        decrypt_optional_secret(&store, &mut tok, "matrix.access_token")?;
        mx.access_token = tok.unwrap_or_default();
    }
    if let Some(ref mut wa) = ch.whatsapp {
        decrypt_optional_secret(&store, &mut wa.access_token, "whatsapp.access_token")?;
        decrypt_optional_secret(&store, &mut wa.verify_token, "whatsapp.verify_token")?;
        decrypt_optional_secret(&store, &mut wa.app_secret, "whatsapp.app_secret")?;
    }
    if let Some(ref mut lq) = ch.linq {
        let mut tok = Some(lq.api_token.clone());
        decrypt_optional_secret(&store, &mut tok, "linq.api_token")?;
        lq.api_token = tok.unwrap_or_default();
    }
    if let Some(ref mut irc) = ch.irc {
        decrypt_optional_secret(&store, &mut irc.server_password, "irc.server_password")?;
        decrypt_optional_secret(&store, &mut irc.nickserv_password, "irc.nickserv_password")?;
        decrypt_optional_secret(&store, &mut irc.sasl_password, "irc.sasl_password")?;
    }
    if let Some(ref mut lk) = ch.lark {
        let mut tok = Some(lk.app_secret.clone());
        decrypt_optional_secret(&store, &mut tok, "lark.app_secret")?;
        lk.app_secret = tok.unwrap_or_default();
        decrypt_optional_secret(&store, &mut lk.encrypt_key, "lark.encrypt_key")?;
        decrypt_optional_secret(
            &store,
            &mut lk.verification_token,
            "lark.verification_token",
        )?;
    }
    if let Some(ref mut dt) = ch.dingtalk {
        let mut tok = Some(dt.client_secret.clone());
        decrypt_optional_secret(&store, &mut tok, "dingtalk.client_secret")?;
        dt.client_secret = tok.unwrap_or_default();
    }
    if let Some(ref mut qq) = ch.qq {
        let mut tok = Some(qq.app_secret.clone());
        decrypt_optional_secret(&store, &mut tok, "qq.app_secret")?;
        qq.app_secret = tok.unwrap_or_default();
    }

    Ok(())
}

/// Encrypt all secret fields in the configuration before writing to disk.
///
/// Called during `Config::save()` when `secrets.encrypt` is true. Only
/// encrypts values that are NOT already encrypted. This is a no-op when
/// encryption is disabled.
fn encrypt_config_secrets(config: &mut Config) -> Result<()> {
    if !config.secrets.encrypt {
        return Ok(());
    }
    let parent_dir = config
        .config_path
        .parent()
        .context("Config path must have a parent directory")?;
    let store = crate::openhuman::security::SecretStore::new(parent_dir, true);

    encrypt_optional_secret(&store, &mut config.api_key, "api_key")?;

    let ch = &mut config.channels_config;
    if let Some(ref mut tg) = ch.telegram {
        let mut tok = Some(tg.bot_token.clone());
        encrypt_optional_secret(&store, &mut tok, "telegram.bot_token")?;
        tg.bot_token = tok.unwrap_or_default();
    }
    if let Some(ref mut d) = ch.discord {
        let mut tok = Some(d.bot_token.clone());
        encrypt_optional_secret(&store, &mut tok, "discord.bot_token")?;
        d.bot_token = tok.unwrap_or_default();
    }
    if let Some(ref mut s) = ch.slack {
        let mut tok = Some(s.bot_token.clone());
        encrypt_optional_secret(&store, &mut tok, "slack.bot_token")?;
        s.bot_token = tok.unwrap_or_default();
        encrypt_optional_secret(&store, &mut s.app_token, "slack.app_token")?;
    }
    if let Some(ref mut m) = ch.mattermost {
        let mut tok = Some(m.bot_token.clone());
        encrypt_optional_secret(&store, &mut tok, "mattermost.bot_token")?;
        m.bot_token = tok.unwrap_or_default();
    }
    if let Some(ref mut w) = ch.webhook {
        encrypt_optional_secret(&store, &mut w.secret, "webhook.secret")?;
    }
    if let Some(ref mut mx) = ch.matrix {
        let mut tok = Some(mx.access_token.clone());
        encrypt_optional_secret(&store, &mut tok, "matrix.access_token")?;
        mx.access_token = tok.unwrap_or_default();
    }
    if let Some(ref mut wa) = ch.whatsapp {
        encrypt_optional_secret(&store, &mut wa.access_token, "whatsapp.access_token")?;
        encrypt_optional_secret(&store, &mut wa.verify_token, "whatsapp.verify_token")?;
        encrypt_optional_secret(&store, &mut wa.app_secret, "whatsapp.app_secret")?;
    }
    if let Some(ref mut lq) = ch.linq {
        let mut tok = Some(lq.api_token.clone());
        encrypt_optional_secret(&store, &mut tok, "linq.api_token")?;
        lq.api_token = tok.unwrap_or_default();
    }
    if let Some(ref mut irc) = ch.irc {
        encrypt_optional_secret(&store, &mut irc.server_password, "irc.server_password")?;
        encrypt_optional_secret(&store, &mut irc.nickserv_password, "irc.nickserv_password")?;
        encrypt_optional_secret(&store, &mut irc.sasl_password, "irc.sasl_password")?;
    }
    if let Some(ref mut lk) = ch.lark {
        let mut tok = Some(lk.app_secret.clone());
        encrypt_optional_secret(&store, &mut tok, "lark.app_secret")?;
        lk.app_secret = tok.unwrap_or_default();
        encrypt_optional_secret(&store, &mut lk.encrypt_key, "lark.encrypt_key")?;
        encrypt_optional_secret(
            &store,
            &mut lk.verification_token,
            "lark.verification_token",
        )?;
    }
    if let Some(ref mut dt) = ch.dingtalk {
        let mut tok = Some(dt.client_secret.clone());
        encrypt_optional_secret(&store, &mut tok, "dingtalk.client_secret")?;
        dt.client_secret = tok.unwrap_or_default();
    }
    if let Some(ref mut qq) = ch.qq {
        let mut tok = Some(qq.app_secret.clone());
        encrypt_optional_secret(&store, &mut tok, "qq.app_secret")?;
        qq.app_secret = tok.unwrap_or_default();
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

/// Try to parse config TOML. On failure, try `.bak`, then fall back to `Config::default()`.
///
/// Returns `(config, was_corrupted)` where `was_corrupted == true` means the
/// primary failed to parse and the caller is responsible for archiving the
/// corrupt primary and persisting the recovered/default config.
///
/// This is a standalone async function (not a method on Config) so it can be
/// called from both `load_or_init` and `load_from_default_paths`.
///
/// **Why the parse runs via `spawn_blocking`:** `toml::from_str::<Config>`
/// is a recursive-descent parser whose serde-monomorphised `Visitor`
/// frames for our deeply-nested `Config` cost several KB each. When this
/// function is called from the bottom of a deep async tower — e.g.
/// `composio_list_tools` reloading the config per call (#1710 Wave 4),
/// reached via `chat → orchestrator → delegate_to_integrations_agent →
/// sub-agent → composio_*` — running the parse inline on the tokio
/// worker thread blows the ~2 MB worker stack and aborts the in-process
/// core with `SIGBUS / KERN_PROTECTION_FAILURE` (see `crahs.log`,
/// 2026-05-17, and `tests/composio_list_tools_stack_overflow_regression.rs`).
/// Moving the parse onto the blocking-pool gives it a *fresh* thread
/// stack with no async tower above it, so the same parser frames easily
/// fit. (An earlier draft of this fix also fronted
/// `config::ops::load_config_with_timeout` with a per-process cache to
/// skip the parse on repeat calls, but it was reverted — the in-process
/// integration tests in `tests/json_rpc_e2e.rs` reuse workspace paths
/// and load config mid-mutation, racing the cache. The spawn_blocking
/// move is sufficient on its own once paired with the Tauri worker
/// stack bump in `app/src-tauri/src/lib.rs`.)
async fn parse_config_with_recovery(config_path: &Path, contents: &str) -> (Config, bool) {
    let parse_err = match parse_toml_off_worker(contents.to_string()).await {
        Ok(config) => {
            tracing::debug!(
                path = %config_path.display(),
                "[config] Config parsed successfully"
            );
            return (config, false);
        }
        Err(parse_err) => parse_err,
    };

    let backup_path = config_path.with_extension("toml.bak");
    if tokio::fs::try_exists(&backup_path).await.unwrap_or(false) {
        tracing::warn!(
            path = %config_path.display(),
            backup = %backup_path.display(),
            error = %parse_err,
            "[config] Config file is corrupted — attempting recovery from backup"
        );
        match fs::read_to_string(&backup_path).await {
            Ok(bak_contents) => match parse_toml_off_worker(bak_contents).await {
                Ok(bak_config) => {
                    tracing::info!(
                        path = %config_path.display(),
                        backup = %backup_path.display(),
                        "[config] Recovered config from backup"
                    );
                    return (bak_config, true);
                }
                Err(bak_err) => {
                    tracing::warn!(
                        path = %config_path.display(),
                        backup = %backup_path.display(),
                        error = %bak_err,
                        "[config] Backup is also corrupted; resetting to defaults"
                    );
                }
            },
            Err(read_err) => {
                tracing::warn!(
                    path = %config_path.display(),
                    backup = %backup_path.display(),
                    error = %read_err,
                    "[config] Failed to read backup; resetting to defaults"
                );
            }
        }
    } else {
        tracing::warn!(
            path = %config_path.display(),
            error = %parse_err,
            "[config] Config file is corrupted (no backup found); resetting to defaults"
        );
    }

    (Config::default(), true)
}

/// Run `toml::from_str::<Config>` on a blocking-pool thread so the
/// parser's stack consumption is independent of how deep the calling
/// async tower is. See [`parse_config_with_recovery`] for the rationale.
///
/// Returns the parse error stringified (rather than `toml::de::Error`)
/// because the rare blocking-pool join failure has no corresponding
/// typed variant and is only ever surfaced as a log line / corruption
/// fallback. Callers only need the message.
async fn parse_toml_off_worker(contents: String) -> Result<Config, String> {
    match tokio::task::spawn_blocking(move || toml::from_str::<Config>(&contents)).await {
        Ok(Ok(config)) => Ok(config),
        Ok(Err(parse_err)) => Err(parse_err.to_string()),
        Err(join_err) => Err(format!("blocking-pool parse join failed: {join_err}")),
    }
}

/// Older builds (#1342) wrote the user's custom OpenAI-compatible URL into
/// `config.api_url`, double-purposing it as both the OpenHuman product
/// backend URL AND the inference URL. That broke auth/billing/voice as
/// soon as someone picked a non-OpenHuman provider. We now keep them in
/// separate fields; on load, detect that legacy shape (any `api_url` whose
/// path looks like a chat-completions endpoint) and move it.
fn migrate_legacy_inference_url(config: &mut Config) {
    if config.inference_url.is_some() {
        return;
    }
    let Some(url) = config.api_url.as_deref() else {
        return;
    };
    let trimmed = url.trim().trim_end_matches('/');
    if !trimmed.ends_with("/chat/completions") {
        return;
    }
    // OpenHuman's hosted backend exposes inference at `/openai/v1/chat/completions`;
    // when api_url points there, the derived inference URL is already correct —
    // just clear api_url so it falls back to the default base. For everything
    // else, move the legacy value into inference_url.
    let is_openhuman_backend = trimmed.starts_with("https://api.tinyhumans.ai/")
        || trimmed.starts_with("https://staging-api.tinyhumans.ai/");
    let moved = if is_openhuman_backend {
        None
    } else {
        Some(trimmed.to_string())
    };
    // Log the URL with userinfo (basic-auth creds) and query string stripped
    // so credentials embedded by callers — `https://user:token@host/v1/...`
    // or `?api_key=...` — don't end up in log files / Sentry breadcrumbs.
    let logged = match moved.as_deref() {
        None => "<derived>".to_string(),
        Some(u) => redact_url_for_log(u),
    };
    tracing::info!(
        "[config][migrate] splitting legacy api_url -> inference_url (api_url cleared, inference_url={})",
        logged
    );
    config.inference_url = moved;
    config.api_url = None;
}

/// Strip userinfo (basic-auth) and query string from a URL string for log
/// emission. Falls back to a coarse `<host>/...` form when parsing fails so
/// we never leak the raw input. Public only so the migration's unit test
/// can assert the behaviour.
pub(super) fn redact_url_for_log(raw: &str) -> String {
    if let Ok(mut url) = url::Url::parse(raw) {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        return url.to_string();
    }
    // Unparseable — keep the scheme+host hint, drop everything after the
    // first `?` or `#`, and replace any `:port@host` userinfo with `***`.
    let truncated = raw
        .split(['?', '#'])
        .next()
        .unwrap_or(raw)
        .trim_end_matches('/');
    if let Some((scheme, rest)) = truncated.split_once("://") {
        if let Some((_, host_path)) = rest.split_once('@') {
            return format!("{scheme}://***@{host_path}");
        }
        return format!("{scheme}://{rest}");
    }
    "<unparseable url>".to_string()
}

/// Migrate `cloud_providers` entries to the new slug-keyed shape and rewrite
/// any per-workload routing strings that still use the old bare-prefix grammar.
///
/// This is idempotent: entries that already have a slug/label are left
/// untouched. Routing fields that already contain a `:` are assumed to be
/// in the new `<slug>:<model>` form.
fn migrate_cloud_provider_slugs(config: &mut Config) {
    use super::cloud_providers::{migrate_legacy_fields, AuthStyle};

    // Step 1: migrate every cloud_providers entry in-place.
    for entry in &mut config.cloud_providers {
        migrate_legacy_fields(entry);
    }

    // Step 2: rewrite per-workload routing strings from legacy bare grammar.
    // Build a lookup: legacy type string → first entry with that slug.
    // After migration, `entry.slug` is populated from `legacy_type` when it
    // was empty, so we can look up by slug now.
    let slug_to_id: std::collections::HashMap<String, String> = config
        .cloud_providers
        .iter()
        .map(|e| (e.slug.clone(), e.id.clone()))
        .collect();

    let legacy_custom_slug = config
        .inference_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty() && !looks_like_openhuman_provider_endpoint(url))
        .and_then(|url| {
            let normalized = normalize_provider_endpoint(url);
            config
                .cloud_providers
                .iter()
                .find(|entry| {
                    !is_openhuman_provider_entry(entry)
                        && normalize_provider_endpoint(&entry.endpoint) == normalized
                })
                .map(|entry| entry.slug.clone())
        });

    // Helper: rewrite a single routing field.
    // Legacy bare strings are: "cloud", "openhuman", "openai", "anthropic",
    // "openrouter", "custom" (no ':').  New strings contain ':'.
    let rewrite = |field: &mut Option<String>| {
        let raw = match field.as_deref() {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return,
        };
        // Already in new grammar (contains ':') or is the openhuman sentinel.
        if raw.contains(':') || raw == "openhuman" {
            return;
        }
        match raw.as_str() {
            "cloud" => {
                // "cloud" sentinel: look for the primary or first non-openhuman entry.
                // If a legacy external inference_url exists and primary still points
                // at OpenHuman, keep routing on that custom provider; that shape was
                // written by older builds that preserved the endpoint but defaulted
                // primary_cloud to OpenHuman.
                let primary_slug = config.primary_cloud.as_deref().and_then(|pid| {
                    config
                        .cloud_providers
                        .iter()
                        .find(|e| e.id == pid)
                        .map(|e| e.slug.clone())
                });
                let slug = match primary_slug.as_deref() {
                    Some("openhuman") => legacy_custom_slug.clone().or(primary_slug),
                    Some(_) => primary_slug,
                    None => legacy_custom_slug.clone().or_else(|| {
                        config
                            .cloud_providers
                            .iter()
                            .find(|entry| !is_openhuman_provider_entry(entry))
                            .map(|entry| entry.slug.clone())
                    }),
                };
                if let Some(s) = slug {
                    if s == "openhuman" {
                        tracing::debug!(
                            "[config][migrate] rewriting routing 'cloud' → 'openhuman'"
                        );
                        *field = Some("openhuman".to_string());
                    } else {
                        tracing::info!(
                            "[config][migrate] rewriting routing 'cloud' → '{s}:' (empty model)"
                        );
                        *field = Some(format!("{s}:"));
                    }
                } else {
                    tracing::debug!(
                        "[config][migrate] routing 'cloud' with no non-openhuman provider → 'openhuman'"
                    );
                    *field = Some("openhuman".to_string());
                }
            }
            other => {
                // Bare type string (e.g. "openai") — find entry by slug.
                if slug_to_id.contains_key(other) {
                    tracing::info!(
                        "[config][migrate] rewriting bare routing '{}' → '{}:'",
                        other,
                        other
                    );
                    *field = Some(format!("{other}:"));
                } else if other != "openhuman" {
                    tracing::warn!(
                        "[config][migrate] bare routing '{}' has no matching provider entry, \
                         falling back to 'openhuman'",
                        other
                    );
                    *field = Some("openhuman".to_string());
                }
            }
        }
    };

    rewrite(&mut config.reasoning_provider);
    rewrite(&mut config.agentic_provider);
    rewrite(&mut config.coding_provider);
    rewrite(&mut config.memory_provider);
    rewrite(&mut config.embeddings_provider);
    rewrite(&mut config.heartbeat_provider);
    rewrite(&mut config.learning_provider);
    rewrite(&mut config.subconscious_provider);

    fn normalize_provider_endpoint(url: &str) -> String {
        url.trim().trim_end_matches('/').to_ascii_lowercase()
    }

    fn looks_like_openhuman_provider_endpoint(url: &str) -> bool {
        let lower = url.trim().to_ascii_lowercase();
        let without_scheme = lower.split("://").nth(1).unwrap_or(&lower);
        let authority = without_scheme.split('/').next().unwrap_or("");
        let host = authority.split('@').next_back().unwrap_or(authority);
        let host_no_port = host.split(':').next().unwrap_or(host);
        matches!(
            host_no_port,
            "api.openhuman.ai" | "api.tinyhumans.ai" | "staging-api.tinyhumans.ai" | "openhuman"
        ) || host_no_port.ends_with(".openhuman.ai")
            || host_no_port.ends_with(".tinyhumans.ai")
    }

    fn is_openhuman_provider_entry(entry: &super::cloud_providers::CloudProviderCreds) -> bool {
        entry.slug == "openhuman"
            || matches!(entry.auth_style, AuthStyle::OpenhumanJwt)
            || looks_like_openhuman_provider_endpoint(&entry.endpoint)
    }
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
        Self::load_or_init_with_env_lookup(
            &default_openhuman_dir,
            &default_workspace_dir,
            &ProcessEnv,
        )
        .await
    }

    async fn load_or_init_with_env_lookup(
        default_openhuman_dir: &Path,
        default_workspace_dir: &Path,
        env: &(dyn EnvLookup + Send + Sync),
    ) -> Result<Self> {
        let (openhuman_dir, workspace_dir, resolution_source) =
            resolve_runtime_config_dirs_with(default_openhuman_dir, default_workspace_dir, env)
                .await?;

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
            config.apply_env_overrides_from(env);

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
            let (mut config, config_was_corrupted) =
                parse_config_with_recovery(&config_path, &contents).await;
            config.config_path = config_path.clone();
            config.workspace_dir = workspace_dir;
            migrate_legacy_autocomplete_disabled_apps(&mut config);
            migrate_legacy_inference_url(&mut config);
            migrate_cloud_provider_slugs(&mut config);
            config.apply_env_overrides_from(env);

            if config_was_corrupted {
                // Rename the corrupted primary away *before* calling save().
                // save() copies config_path → config_path.bak before the
                // atomic replace, so if the corrupted file is still at
                // config_path it would overwrite the good .bak that we just
                // used for recovery. Only call save() when the rename
                // succeeds; on failure log and leave recovery for next boot
                // rather than destroying the good backup.
                let corrupted_path = config_path.with_extension("toml.corrupted");
                match fs::rename(&config_path, &corrupted_path).await {
                    Ok(()) => {
                        tracing::debug!(
                            src = %config_path.display(),
                            dst = %corrupted_path.display(),
                            "[config] Renamed corrupted config; persisting recovered config"
                        );
                        if let Err(e) = config.save().await {
                            tracing::warn!(
                                path = %config.config_path.display(),
                                error = %e,
                                "[config] Failed to persist recovered config to disk"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            src = %config_path.display(),
                            dst = %corrupted_path.display(),
                            error = %e,
                            "[config] Failed to rename corrupted config; skipping save to \
                             protect the .bak — will retry recovery on next startup"
                        );
                    }
                }
            }

            tracing::debug!(
                path = %config.config_path.display(),
                workspace = %config.workspace_dir.display(),
                source = resolution_source.as_str(),
                initialized = false,
                recovered = config_was_corrupted,
                "Config loaded"
            );
            crate::openhuman::migrations::run_pending(&mut config).await;
            decrypt_config_secrets(&mut config, &openhuman_dir)?;
            Ok(config)
        } else {
            // Fresh install: there is no legacy on-disk state, so stamp
            // the workspace at the current schema version up front. This
            // makes `run_pending` a fast no-op on the first launch
            // (nothing to migrate) and keeps the "first launch on this
            // workspace" semantics aligned with "current binary built
            // this workspace".
            let mut config = Config {
                config_path: config_path.clone(),
                workspace_dir,
                schema_version: crate::openhuman::migrations::CURRENT_SCHEMA_VERSION,
                ..Default::default()
            };
            config.save().await?;

            #[cfg(unix)]
            {
                use std::{fs::Permissions, os::unix::fs::PermissionsExt};
                let _ = fs::set_permissions(&config_path, Permissions::from_mode(0o600)).await;
            }

            config.apply_env_overrides_from(env);

            tracing::debug!(
                path = %config.config_path.display(),
                workspace = %config.workspace_dir.display(),
                source = resolution_source.as_str(),
                initialized = true,
                "Config loaded"
            );
            // Defensive: still call run_pending. It will see
            // `schema_version == CURRENT` and return immediately, but
            // the call site stays symmetric with the existing-config
            // branch so a future migration that needs to fire on fresh
            // installs (vanishingly unlikely, but possible) doesn't
            // require touching this path.
            crate::openhuman::migrations::run_pending(&mut config).await;
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

        // NOTE: no backup recovery here by design — this is the debug-dump path only;
        // `load_or_init()` is the authoritative startup path that handles corruption.
        let raw = fs::read_to_string(&config_path)
            .await
            .context("reading config.toml from default paths")?;
        let (mut config, _was_corrupted) = parse_config_with_recovery(&config_path, &raw).await;
        config.config_path = config_path;
        config.workspace_dir = workspace_dir;
        config.apply_env_overrides();
        decrypt_config_secrets(&mut config, &openhuman_dir)?;
        Ok(config)
    }

    /// Reload a config from an already-resolved `config.toml` path.
    ///
    /// This is for long-lived runtime objects that hold a `Config`
    /// snapshot and need to observe updates written back to the same
    /// file. It deliberately bypasses only `OPENHUMAN_WORKSPACE`
    /// resolution: the caller has already been scoped to a user/workspace,
    /// and following the process-global workspace env var again can cross
    /// streams with unrelated tests or runtime tasks that temporarily
    /// repoint it. Other process env overrides still apply.
    pub async fn load_from_config_path(config_path: &Path, workspace_dir: &Path) -> Result<Self> {
        let config_path = config_path.to_path_buf();
        let workspace_dir = workspace_dir.to_path_buf();

        if !config_path.exists() {
            let mut config = Config {
                config_path,
                workspace_dir,
                ..Default::default()
            };
            config.apply_env_overrides_from(&ProcessEnvWithoutWorkspace);
            return Ok(config);
        }

        let raw = fs::read_to_string(&config_path)
            .await
            .with_context(|| format!("reading config.toml from {}", config_path.display()))?;
        let (mut config, config_was_corrupted) =
            parse_config_with_recovery(&config_path, &raw).await;
        config.config_path = config_path;
        config.workspace_dir = workspace_dir;
        migrate_legacy_autocomplete_disabled_apps(&mut config);
        migrate_legacy_inference_url(&mut config);
        migrate_cloud_provider_slugs(&mut config);
        config.apply_env_overrides_from(&ProcessEnvWithoutWorkspace);

        if config_was_corrupted {
            tracing::warn!(
                path = %config.config_path.display(),
                "[config] Snapshot reload recovered a corrupted config; skipping persistence"
            );
        }

        crate::openhuman::migrations::run_pending(&mut config).await;
        Ok(config)
    }

    pub fn apply_env_overrides(&mut self) {
        self.apply_env_overrides_from(&ProcessEnv);
    }

    fn apply_env_overrides_from(&mut self, env: &(dyn EnvLookup + Send + Sync)) {
        self.apply_env_overlay_with(env);

        // The pure overlay above never mutates process-level state. The
        // two side effects below remain here so tests driving
        // `apply_env_overlay_with` directly don't clobber the shared
        // runtime proxy client cache or mutate `HTTP_PROXY` / etc. on
        // the running process.
        if self.proxy.enabled && self.proxy.scope == ProxyScope::Environment {
            self.proxy.apply_to_process_env();
        }

        set_runtime_proxy_config(self.proxy.clone());

        // Push the embedding request budget into its process-global limiter so
        // every cloud embed (via the shared `OpenAiEmbedding` chokepoint) is
        // throttled to the configured rate. Kept here, with the proxy commit,
        // so the pure overlay stays side-effect-free for tests.
        crate::openhuman::embeddings::rate_limit::set_embedding_rate_limit(
            self.memory.embedding_rate_limit_per_min,
        );
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
    pub(crate) fn apply_env_overlay_with<E: EnvLookup + ?Sized>(&mut self, env: &E) {
        // Only the namespaced `OPENHUMAN_MODEL` is honoured. The bare `MODEL`
        // env var used to be accepted as an alias but collides with vendor
        // asset-tag env vars (e.g. Dell OptiPlex sets `MODEL=7080`), which
        // silently clobbered the LLM model and 400'd every backend call
        // (Sentry OPENHUMAN-TAURI-J8).
        if let Some(model) = env.get("OPENHUMAN_MODEL") {
            // Trim before checking so `OPENHUMAN_MODEL="   "` (a common
            // shape from shells that pass through an unset-but-declared
            // variable) doesn't clobber the configured default with a
            // non-usable value.
            let trimmed = model.trim();
            if !trimmed.is_empty() {
                self.default_model = Some(trimmed.to_string());
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

        // Seltz direct-API search.
        if let Some(key) = env.get_any(&["OPENHUMAN_SELTZ_API_KEY", "SELTZ_API_KEY"]) {
            if !key.is_empty() {
                self.seltz.api_key = Some(key);
                // Auto-enable when the key is set via env.
                self.seltz.enabled = true;
            }
        }
        if let Some(url) = env.get_any(&["OPENHUMAN_SELTZ_API_URL", "SELTZ_API_URL"]) {
            if !url.is_empty() {
                self.seltz.api_url = Some(url);
            }
        }
        if let Some(max) = env.get_any(&["OPENHUMAN_SELTZ_MAX_RESULTS", "SELTZ_MAX_RESULTS"]) {
            if let Ok(n) = max.parse::<usize>() {
                if (1..=20).contains(&n) {
                    self.seltz.max_results = n;
                }
            }
        }

        // SearXNG self-hosted search. Unlike Seltz, this needs no API key;
        // keep it opt-in because it reaches a user-controlled HTTP endpoint.
        if let Some(flag) = env.get_any(&["OPENHUMAN_SEARXNG_ENABLED", "SEARXNG_ENABLED"]) {
            if let Some(enabled) = parse_env_bool("OPENHUMAN_SEARXNG_ENABLED", &flag) {
                self.searxng.enabled = enabled;
            }
        }
        if let Some(url) = env.get_any(&["OPENHUMAN_SEARXNG_BASE_URL", "SEARXNG_BASE_URL"]) {
            let url = url.trim();
            if !url.is_empty() {
                self.searxng.base_url = url.to_string();
            }
        }
        if let Some(max) = env.get_any(&["OPENHUMAN_SEARXNG_MAX_RESULTS", "SEARXNG_MAX_RESULTS"]) {
            if let Ok(n) = max.parse::<usize>() {
                if (1..=50).contains(&n) {
                    self.searxng.max_results = n;
                }
            }
        }
        if let Some(language) = env.get_any(&[
            "OPENHUMAN_SEARXNG_DEFAULT_LANGUAGE",
            "SEARXNG_DEFAULT_LANGUAGE",
        ]) {
            let language = language.trim();
            if !language.is_empty() {
                self.searxng.default_language = language.to_string();
            }
        }
        if let Some(timeout_secs) = env.get_any(&[
            "OPENHUMAN_SEARXNG_TIMEOUT_SECS",
            "OPENHUMAN_SEARXNG_TIMEOUT_SECONDS",
            "SEARXNG_TIMEOUT_SECS",
            "SEARXNG_TIMEOUT_SECONDS",
        ]) {
            if let Ok(timeout_secs) = timeout_secs.parse::<u64>() {
                if timeout_secs > 0 {
                    self.searxng.timeout_secs = timeout_secs;
                }
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
                    crate::openhuman::inference::presets::ModelTier::from_str_opt(&tier_str)
                {
                    if tier == crate::openhuman::inference::presets::ModelTier::Custom {
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
                        crate::openhuman::inference::presets::apply_preset_to_config(
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

        // Python runtime overrides
        if let Some(flag) = env.get("OPENHUMAN_RUNTIME_PYTHON_ENABLED") {
            if let Some(enabled) = parse_env_bool("OPENHUMAN_RUNTIME_PYTHON_ENABLED", &flag) {
                self.runtime_python.enabled = enabled;
            }
        }
        if let Some(version) = env.get("OPENHUMAN_RUNTIME_PYTHON_MINIMUM_VERSION") {
            let trimmed = version.trim();
            if !trimmed.is_empty() {
                self.runtime_python.minimum_version = trimmed.to_string();
            }
        }
        if let Some(dir) = env.get("OPENHUMAN_RUNTIME_PYTHON_CACHE_DIR") {
            self.runtime_python.cache_dir = dir.trim().to_string();
        }
        if let Some(tag) = env.get("OPENHUMAN_RUNTIME_PYTHON_MANAGED_RELEASE_TAG") {
            self.runtime_python.managed_release_tag = tag.trim().to_string();
        }
        if let Some(flag) = env.get("OPENHUMAN_RUNTIME_PYTHON_PREFER_SYSTEM") {
            if let Some(prefer_system) =
                parse_env_bool("OPENHUMAN_RUNTIME_PYTHON_PREFER_SYSTEM", &flag)
            {
                self.runtime_python.prefer_system = prefer_system;
            }
        }
        if let Some(command) = env.get("OPENHUMAN_RUNTIME_PYTHON_PREFERRED_COMMAND") {
            self.runtime_python.preferred_command = command.trim().to_string();
        }

        // Prefer the namespaced name. `OPENHUMAN_SENTRY_DSN` is the legacy
        // unprefixed name kept as a fallback so existing CI vars and local
        // `.env` files keep working until the GH org-level variable can be
        // renamed in lock-step.
        let dsn_value = env
            .get("OPENHUMAN_CORE_SENTRY_DSN")
            .or_else(|| env.get("OPENHUMAN_SENTRY_DSN"))
            .or_else(|| option_env!("OPENHUMAN_CORE_SENTRY_DSN").map(|s| s.to_string()))
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
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_TOOL_MEMORY_CAPTURE_ENABLED") {
            if let Some(enabled) = parse_env_bool(
                "OPENHUMAN_LEARNING_TOOL_MEMORY_CAPTURE_ENABLED",
                flag.as_str(),
            ) {
                self.learning.tool_memory_capture_enabled = enabled;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_EXPLICIT_PREFERENCES_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.explicit_preferences_enabled = true,
                "0" | "false" | "no" | "off" => self.learning.explicit_preferences_enabled = false,
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
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_EPISODIC_CAPTURE_ENABLED") {
            if let Some(enabled) =
                parse_env_bool("OPENHUMAN_LEARNING_EPISODIC_CAPTURE_ENABLED", flag.as_str())
            {
                self.learning.episodic_capture_enabled = enabled;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_STM_RECALL_ENABLED") {
            if let Some(enabled) =
                parse_env_bool("OPENHUMAN_LEARNING_STM_RECALL_ENABLED", flag.as_str())
            {
                self.learning.stm_recall_enabled = enabled;
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_LEARNING_UNIFIED_COMPACTION_ENABLED") {
            if let Some(enabled) = parse_env_bool(
                "OPENHUMAN_LEARNING_UNIFIED_COMPACTION_ENABLED",
                flag.as_str(),
            ) {
                self.learning.unified_compaction_enabled = enabled;
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
        // Cloud embedding request budget (requests/min) on `memory.*`. `0`
        // disables throttling. A blank or non-numeric value leaves the
        // configured/default budget untouched. Committed to the process-global
        // limiter in `apply_env_overrides`.
        if let Some(val) = env.get("OPENHUMAN_MEMORY_EMBED_RATE_LIMIT") {
            if let Ok(per_min) = val.trim().parse::<u32>() {
                self.memory.embedding_rate_limit_per_min = per_min;
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
        // Routed through `env.get` so `HashMapEnv`-style test callers see the
        // override too — same seam as every other branch in this function.
        if let Some(dir) = env.get("OPENHUMAN_MEMORY_TREE_CONTENT_DIR") {
            let trimmed = dir.trim();
            self.memory_tree.content_dir = if trimmed.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(trimmed))
            };
        }

        // Memory-tree LLM backend selector: "cloud" (default) routes through
        // the OpenHuman backend's summarizer model; "local" keeps the legacy
        // Ollama-direct path. Empty / unset / unknown leaves the existing
        // value untouched (and we warn on unknown). The embedder is unaffected.
        if let Some(raw) = env.get("OPENHUMAN_MEMORY_TREE_LLM_BACKEND") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                match crate::openhuman::config::LlmBackend::parse(trimmed) {
                    Ok(b) => {
                        log::debug!(
                            "[memory_tree] OPENHUMAN_MEMORY_TREE_LLM_BACKEND override applied: {}",
                            b.as_str()
                        );
                        self.memory_tree.llm_backend = b;
                    }
                    Err(e) => {
                        tracing::warn!(
                            value = trimmed,
                            error = %e,
                            "ignoring invalid OPENHUMAN_MEMORY_TREE_LLM_BACKEND (valid: cloud, local)"
                        );
                    }
                }
            }
        }
        // Cloud LLM model override (only meaningful when llm_backend = cloud).
        // Empty string explicitly clears the default — useful for tests that
        // want to assert the absence of a configured cloud model. Non-empty
        // strings are stored verbatim.
        if let Some(raw) = env.get("OPENHUMAN_MEMORY_TREE_CLOUD_LLM_MODEL") {
            let trimmed = raw.trim();
            self.memory_tree.cloud_llm_model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
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
        if let Some(raw) = env.get("OPENHUMAN_AUTO_UPDATE_RESTART_STRATEGY") {
            match raw.trim().to_ascii_lowercase().as_str() {
                "self_replace" | "self-replace" | "self" => {
                    self.update.restart_strategy = UpdateRestartStrategy::SelfReplace;
                }
                "supervisor" | "stage_only" | "stage-only" => {
                    self.update.restart_strategy = UpdateRestartStrategy::Supervisor;
                }
                other => {
                    tracing::warn!(
                        value = other,
                        "ignoring invalid OPENHUMAN_AUTO_UPDATE_RESTART_STRATEGY \
                         (valid: self_replace, supervisor)"
                    );
                }
            }
        }
        if let Some(flag) = env.get("OPENHUMAN_AUTO_UPDATE_RPC_MUTATIONS_ENABLED") {
            if let Some(enabled) =
                parse_env_bool("OPENHUMAN_AUTO_UPDATE_RPC_MUTATIONS_ENABLED", &flag)
            {
                self.update.rpc_mutations_enabled = enabled;
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
        let mut config_to_save = self.clone();
        encrypt_config_secrets(&mut config_to_save)?;

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

        let had_existing_config = tokio::fs::try_exists(&self.config_path)
            .await
            .unwrap_or(false);
        if had_existing_config {
            // Copy the encrypted temp file as the backup, NOT the old on-disk
            // config. The old config may still contain plaintext secrets from
            // before encryption was wired in (#1900). Using the encrypted
            // bytes ensures the .bak never leaks plaintext credentials.
            fs::copy(&temp_path, &backup_path).await.with_context(|| {
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

        // Note: we intentionally keep the .bak file after a successful save so
        // that `parse_config_with_recovery` can use it if the primary is later
        // corrupted. The .bak is updated on every successful save, so it always
        // holds the last-known-good config.

        Ok(())
    }
}

#[cfg(test)]
#[path = "load_tests.rs"]
mod tests;
