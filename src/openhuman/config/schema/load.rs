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
use std::path::{Path, PathBuf};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::AsyncWriteExt;

fn default_config_and_workspace_dirs() -> Result<(PathBuf, PathBuf)> {
    let config_dir = default_config_dir()?;
    Ok((config_dir.clone(), config_dir.join("workspace")))
}

const ACTIVE_WORKSPACE_STATE_FILE: &str = "active_workspace.toml";

#[derive(Debug, Serialize, Deserialize)]
struct ActiveWorkspaceState {
    config_dir: String,
}

fn default_config_dir() -> Result<PathBuf> {
    let home = UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .context("Could not find home directory")?;
    Ok(home.join(".openhuman"))
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
    DefaultConfigDir,
}

impl ConfigResolutionSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::EnvWorkspace => "OPENHUMAN_WORKSPACE",
            Self::ActiveWorkspaceMarker => "active_workspace.toml",
            Self::DefaultConfigDir => "default",
        }
    }
}

async fn resolve_runtime_config_dirs(
    default_openhuman_dir: &Path,
    default_workspace_dir: &Path,
) -> Result<(PathBuf, PathBuf, ConfigResolutionSource)> {
    if let Ok(custom_workspace) = std::env::var("OPENHUMAN_WORKSPACE") {
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

    if let Some((openhuman_dir, workspace_dir)) =
        load_persisted_workspace_dirs(default_openhuman_dir).await?
    {
        return Ok((
            openhuman_dir,
            workspace_dir,
            ConfigResolutionSource::ActiveWorkspaceMarker,
        ));
    }

    Ok((
        default_openhuman_dir.to_path_buf(),
        default_workspace_dir.to_path_buf(),
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

            let contents = fs::read_to_string(&config_path)
                .await
                .context("Failed to read config file")?;
            let mut config: Config = toml::from_str(&contents).with_context(|| {
                format!("Failed to parse config file {}", config_path.display())
            })?;
            config.config_path = config_path.clone();
            config.workspace_dir = workspace_dir;
            let store = crate::openhuman::security::SecretStore::new(
                &openhuman_dir,
                config.secrets.encrypt,
            );
            decrypt_optional_secret(&store, &mut config.api_key, "config.api_key")?;
            decrypt_optional_secret(
                &store,
                &mut config.composio.api_key,
                "config.composio.api_key",
            )?;
            decrypt_optional_secret(
                &store,
                &mut config.browser.computer_use.api_key,
                "config.browser.computer_use.api_key",
            )?;
            decrypt_optional_secret(
                &store,
                &mut config.web_search.brave_api_key,
                "config.web_search.brave_api_key",
            )?;
            decrypt_optional_secret(
                &store,
                &mut config.web_search.parallel_api_key,
                "config.web_search.parallel_api_key",
            )?;
            decrypt_optional_secret(
                &store,
                &mut config.storage.provider.config.db_url,
                "config.storage.provider.config.db_url",
            )?;
            for agent in config.agents.values_mut() {
                decrypt_optional_secret(&store, &mut agent.api_key, "config.agents.*.api_key")?;
            }
            migrate_legacy_autocomplete_disabled_apps(&mut config);
            config.apply_env_overrides();
            tracing::info!(
                path = %config.config_path.display(),
                workspace = %config.workspace_dir.display(),
                source = resolution_source.as_str(),
                initialized = false,
                "Config loaded"
            );
            Ok(config)
        } else {
            let mut config = Config::default();
            config.config_path = config_path.clone();
            config.workspace_dir = workspace_dir;
            config.save().await?;

            #[cfg(unix)]
            {
                use std::{fs::Permissions, os::unix::fs::PermissionsExt};
                let _ = fs::set_permissions(&config_path, Permissions::from_mode(0o600)).await;
            }

            config.apply_env_overrides();
            tracing::info!(
                path = %config.config_path.display(),
                workspace = %config.workspace_dir.display(),
                source = resolution_source.as_str(),
                initialized = true,
                "Config loaded"
            );
            Ok(config)
        }
    }

    pub fn apply_env_overrides(&mut self) {
        if let Ok(key) = std::env::var("OPENHUMAN_API_KEY").or_else(|_| std::env::var("API_KEY")) {
            if !key.is_empty() {
                self.api_key = Some(key);
            }
        }
        if let Ok(model) = std::env::var("OPENHUMAN_MODEL").or_else(|_| std::env::var("MODEL")) {
            if !model.is_empty() {
                self.default_model = Some(model);
            }
        }

        if let Ok(workspace) = std::env::var("OPENHUMAN_WORKSPACE") {
            if !workspace.is_empty() {
                let (_, workspace_dir) =
                    resolve_config_dir_for_workspace(&PathBuf::from(workspace));
                self.workspace_dir = workspace_dir;
            }
        }

        if let Ok(temp_str) = std::env::var("OPENHUMAN_TEMPERATURE") {
            if let Ok(temp) = temp_str.parse::<f64>() {
                if (0.0..=2.0).contains(&temp) {
                    self.default_temperature = temp;
                }
            }
        }

        if let Ok(flag) = std::env::var("OPENHUMAN_REASONING_ENABLED")
            .or_else(|_| std::env::var("REASONING_ENABLED"))
        {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.runtime.reasoning_enabled = Some(true),
                "0" | "false" | "no" | "off" => self.runtime.reasoning_enabled = Some(false),
                _ => {}
            }
        }

        if let Ok(enabled) = std::env::var("OPENHUMAN_WEB_SEARCH_ENABLED")
            .or_else(|_| std::env::var("WEB_SEARCH_ENABLED"))
        {
            self.web_search.enabled = enabled == "1" || enabled.eq_ignore_ascii_case("true");
        }

        if let Ok(provider) = std::env::var("OPENHUMAN_WEB_SEARCH_PROVIDER")
            .or_else(|_| std::env::var("WEB_SEARCH_PROVIDER"))
        {
            let provider = provider.trim();
            if !provider.is_empty() {
                self.web_search.provider = provider.to_string();
            }
        }

        if let Ok(api_key) =
            std::env::var("OPENHUMAN_BRAVE_API_KEY").or_else(|_| std::env::var("BRAVE_API_KEY"))
        {
            let api_key = api_key.trim();
            if !api_key.is_empty() {
                self.web_search.brave_api_key = Some(api_key.to_string());
            }
        }

        if let Ok(api_key) = std::env::var("OPENHUMAN_PARALLEL_API_KEY")
            .or_else(|_| std::env::var("PARALLEL_API_KEY"))
        {
            let api_key = api_key.trim();
            if !api_key.is_empty() {
                self.web_search.parallel_api_key = Some(api_key.to_string());
            }
        }

        if let Ok(max_results) = std::env::var("OPENHUMAN_WEB_SEARCH_MAX_RESULTS")
            .or_else(|_| std::env::var("WEB_SEARCH_MAX_RESULTS"))
        {
            if let Ok(max_results) = max_results.parse::<usize>() {
                if (1..=10).contains(&max_results) {
                    self.web_search.max_results = max_results;
                }
            }
        }

        if let Ok(timeout_secs) = std::env::var("OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS")
            .or_else(|_| std::env::var("WEB_SEARCH_TIMEOUT_SECS"))
        {
            if let Ok(timeout_secs) = timeout_secs.parse::<u64>() {
                if timeout_secs > 0 {
                    self.web_search.timeout_secs = timeout_secs;
                }
            }
        }

        if let Ok(provider) = std::env::var("OPENHUMAN_STORAGE_PROVIDER") {
            let provider = provider.trim();
            if !provider.is_empty() {
                self.storage.provider.config.provider = provider.to_string();
            }
        }

        if let Ok(db_url) = std::env::var("OPENHUMAN_STORAGE_DB_URL") {
            let db_url = db_url.trim();
            if !db_url.is_empty() {
                self.storage.provider.config.db_url = Some(db_url.to_string());
            }
        }

        if let Ok(timeout_secs) = std::env::var("OPENHUMAN_STORAGE_CONNECT_TIMEOUT_SECS") {
            if let Ok(timeout_secs) = timeout_secs.parse::<u64>() {
                if timeout_secs > 0 {
                    self.storage.provider.config.connect_timeout_secs = Some(timeout_secs);
                }
            }
        }

        let explicit_proxy_enabled = std::env::var("OPENHUMAN_PROXY_ENABLED")
            .ok()
            .as_deref()
            .and_then(parse_proxy_enabled);
        if let Some(enabled) = explicit_proxy_enabled {
            self.proxy.enabled = enabled;
        }

        let mut proxy_url_overridden = false;
        if let Ok(proxy_url) =
            std::env::var("OPENHUMAN_HTTP_PROXY").or_else(|_| std::env::var("HTTP_PROXY"))
        {
            self.proxy.http_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Ok(proxy_url) =
            std::env::var("OPENHUMAN_HTTPS_PROXY").or_else(|_| std::env::var("HTTPS_PROXY"))
        {
            self.proxy.https_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Ok(proxy_url) =
            std::env::var("OPENHUMAN_ALL_PROXY").or_else(|_| std::env::var("ALL_PROXY"))
        {
            self.proxy.all_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Ok(no_proxy) =
            std::env::var("OPENHUMAN_NO_PROXY").or_else(|_| std::env::var("NO_PROXY"))
        {
            self.proxy.no_proxy = normalize_no_proxy_list(vec![no_proxy]);
        }

        if explicit_proxy_enabled.is_none()
            && proxy_url_overridden
            && self.proxy.has_any_proxy_url()
        {
            self.proxy.enabled = true;
        }

        if let Ok(scope_raw) = std::env::var("OPENHUMAN_PROXY_SCOPE") {
            if let Some(scope) = parse_proxy_scope(&scope_raw) {
                self.proxy.scope = scope;
            } else {
                tracing::warn!(
                    scope = %scope_raw,
                    "Ignoring invalid OPENHUMAN_PROXY_SCOPE (valid: environment|openhuman|services)"
                );
            }
        }

        if let Ok(services_raw) = std::env::var("OPENHUMAN_PROXY_SERVICES") {
            self.proxy.services = normalize_service_list(vec![services_raw]);
        }

        if let Err(error) = self.proxy.validate() {
            tracing::warn!("Invalid proxy configuration ignored: {error}");
            self.proxy.enabled = false;
        }

        if let Ok(tier_str) = std::env::var("OPENHUMAN_LOCAL_AI_TIER") {
            let tier_str = tier_str.trim().to_ascii_lowercase();
            if !tier_str.is_empty() {
                if let Some(tier) =
                    crate::openhuman::local_ai::presets::ModelTier::from_str_opt(&tier_str)
                {
                    if tier != crate::openhuman::local_ai::presets::ModelTier::Custom {
                        crate::openhuman::local_ai::presets::apply_preset_to_config(
                            &mut self.local_ai,
                            tier,
                        );
                        tracing::debug!(tier = %tier_str, "applied local AI tier from OPENHUMAN_LOCAL_AI_TIER");
                    }
                } else {
                    tracing::warn!(
                        tier = %tier_str,
                        "ignoring invalid OPENHUMAN_LOCAL_AI_TIER (valid: low, medium, high)"
                    );
                }
            }
        }

        let dsn_value = std::env::var("OPENHUMAN_SENTRY_DSN")
            .ok()
            .or_else(|| option_env!("OPENHUMAN_SENTRY_DSN").map(|s| s.to_string()));
        if let Some(dsn) = dsn_value {
            let dsn = dsn.trim();
            if !dsn.is_empty() {
                self.observability.sentry_dsn = Some(dsn.to_string());
            }
        }

        if let Ok(flag) = std::env::var("OPENHUMAN_ANALYTICS_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.observability.analytics_enabled = true,
                "0" | "false" | "no" | "off" => self.observability.analytics_enabled = false,
                _ => {}
            }
        }

        // Learning subsystem overrides
        if let Ok(flag) = std::env::var("OPENHUMAN_LEARNING_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.enabled = true,
                "0" | "false" | "no" | "off" => self.learning.enabled = false,
                _ => {}
            }
        }
        if let Ok(flag) = std::env::var("OPENHUMAN_LEARNING_REFLECTION_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.reflection_enabled = true,
                "0" | "false" | "no" | "off" => self.learning.reflection_enabled = false,
                _ => {}
            }
        }
        if let Ok(flag) = std::env::var("OPENHUMAN_LEARNING_USER_PROFILE_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.user_profile_enabled = true,
                "0" | "false" | "no" | "off" => self.learning.user_profile_enabled = false,
                _ => {}
            }
        }
        if let Ok(flag) = std::env::var("OPENHUMAN_LEARNING_TOOL_TRACKING_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.tool_tracking_enabled = true,
                "0" | "false" | "no" | "off" => self.learning.tool_tracking_enabled = false,
                _ => {}
            }
        }
        if let Ok(flag) = std::env::var("OPENHUMAN_LEARNING_SKILL_CREATION_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.learning.skill_creation_enabled = true,
                "0" | "false" | "no" | "off" => self.learning.skill_creation_enabled = false,
                _ => {}
            }
        }
        if let Ok(source) = std::env::var("OPENHUMAN_LEARNING_REFLECTION_SOURCE") {
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
        if let Ok(val) = std::env::var("OPENHUMAN_LEARNING_MAX_REFLECTIONS_PER_SESSION") {
            if let Ok(max) = val.trim().parse::<usize>() {
                self.learning.max_reflections_per_session = max;
            }
        }
        if let Ok(val) = std::env::var("OPENHUMAN_LEARNING_MIN_TURN_COMPLEXITY") {
            if let Ok(min) = val.trim().parse::<usize>() {
                self.learning.min_turn_complexity = min;
            }
        }

        if let Ok(flag) = std::env::var("OPENHUMAN_OVERLAY_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.overlay_enabled = true,
                "0" | "false" | "no" | "off" => self.overlay_enabled = false,
                _ => {}
            }
        }

        if self.proxy.enabled && self.proxy.scope == ProxyScope::Environment {
            self.proxy.apply_to_process_env();
        }

        set_runtime_proxy_config(self.proxy.clone());
    }

    pub async fn save(&self) -> Result<()> {
        let mut config_to_save = self.clone();
        let openhuman_dir = self
            .config_path
            .parent()
            .context("Config path must have a parent directory")?;
        let store =
            crate::openhuman::security::SecretStore::new(openhuman_dir, self.secrets.encrypt);

        encrypt_optional_secret(&store, &mut config_to_save.api_key, "config.api_key")?;
        encrypt_optional_secret(
            &store,
            &mut config_to_save.composio.api_key,
            "config.composio.api_key",
        )?;
        encrypt_optional_secret(
            &store,
            &mut config_to_save.browser.computer_use.api_key,
            "config.browser.computer_use.api_key",
        )?;
        encrypt_optional_secret(
            &store,
            &mut config_to_save.web_search.brave_api_key,
            "config.web_search.brave_api_key",
        )?;
        encrypt_optional_secret(
            &store,
            &mut config_to_save.web_search.parallel_api_key,
            "config.web_search.parallel_api_key",
        )?;
        encrypt_optional_secret(
            &store,
            &mut config_to_save.storage.provider.config.db_url,
            "config.storage.provider.config.db_url",
        )?;
        for agent in config_to_save.agents.values_mut() {
            encrypt_optional_secret(&store, &mut agent.api_key, "config.agents.*.api_key")?;
        }

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
