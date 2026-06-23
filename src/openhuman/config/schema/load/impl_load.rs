use super::super::Config;
use super::dirs::{
    default_action_dir, default_config_and_workspace_dirs, resolve_action_dir,
    resolve_config_dirs_ignoring_env, resolve_runtime_config_dirs_with, ConfigResolutionSource,
};
use super::env::{EnvLookup, ProcessEnv, ProcessEnvWithoutWorkspace};
use super::migrate::{
    migrate_cloud_provider_slugs, migrate_legacy_autocomplete_disabled_apps,
    migrate_legacy_inference_url,
};
use super::secrets::{decrypt_config_secrets, encrypt_config_secrets};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::AsyncWriteExt;

static WARNED_WORLD_READABLE_CONFIGS: OnceLock<Mutex<HashSet<std::path::PathBuf>>> =
    OnceLock::new();

pub(crate) async fn parse_config_with_recovery(
    config_path: &Path,
    contents: &str,
) -> (Config, bool) {
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

async fn parse_toml_off_worker(contents: String) -> Result<Config, String> {
    match tokio::task::spawn_blocking(move || toml::from_str::<Config>(&contents)).await {
        Ok(Ok(config)) => Ok(config),
        Ok(Err(parse_err)) => Err(parse_err.to_string()),
        Err(join_err) => Err(format!("blocking-pool parse join failed: {join_err}")),
    }
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

    pub(crate) async fn load_or_init_with_env_lookup(
        default_openhuman_dir: &Path,
        default_workspace_dir: &Path,
        env: &(dyn EnvLookup + Send + Sync),
    ) -> Result<Self> {
        let (openhuman_dir, workspace_dir, resolution_source) =
            resolve_runtime_config_dirs_with(default_openhuman_dir, default_workspace_dir, env)
                .await?;

        let config_path = openhuman_dir.join("config.toml");

        if resolution_source == ConfigResolutionSource::DefaultConfigDir && !config_path.exists() {
            let mut config = Config {
                config_path: config_path.clone(),
                workspace_dir: workspace_dir.clone(),
                action_dir: default_action_dir(),
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
                use std::{fs::Permissions, os::unix::fs::PermissionsExt};
                if let Ok(meta) = fs::metadata(&config_path).await {
                    if meta.permissions().mode() & 0o004 != 0 {
                        let warned = WARNED_WORLD_READABLE_CONFIGS
                            .get_or_init(|| Mutex::new(HashSet::new()));
                        let already_fixed = warned
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .contains(&config_path);
                        if !already_fixed {
                            tracing::warn!(
                                "[config] Config file {:?} is world-readable (mode {:o}); \
                                 auto-fixing to 600",
                                config_path,
                                meta.permissions().mode() & 0o777,
                            );
                            match fs::set_permissions(&config_path, Permissions::from_mode(0o600))
                                .await
                            {
                                Ok(()) => {
                                    warned
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .insert(config_path.clone());
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        path = %config_path.display(),
                                        error = %e,
                                        "[config] failed to auto-fix config file permissions to 600",
                                    );
                                }
                            }
                        }
                    }
                }
            }

            // A directory (or other non-regular file) at the config path is a
            // bad-install / corruption signal, not a transient read failure. On
            // Windows `read_to_string` of a directory returns the same
            // `Access is denied. (os error 5)` shape as a real ACL denial, which
            // the observability classifier would otherwise demote. Fail fast with
            // distinct wording so it keeps paging instead of being suppressed as
            // an expected user-state config-read failure (#3962, Codex P2).
            if config_path.is_dir() {
                anyhow::bail!(
                    "Config path is a directory, not a file: {}",
                    config_path.display()
                );
            }

            let contents = crate::openhuman::util::retry_with_backoff_async(
                "read config file",
                5,
                20,
                || async {
                    fs::read_to_string(&config_path).await.with_context(|| {
                        format!("Failed to read config file: {}", config_path.display())
                    })
                },
            )
            .await?;
            let (mut config, config_was_corrupted) =
                parse_config_with_recovery(&config_path, &contents).await;
            config.config_path = config_path.clone();
            config.workspace_dir = workspace_dir;
            config.action_dir = resolve_action_dir(&config.action_dir_override);
            migrate_legacy_autocomplete_disabled_apps(&mut config);
            migrate_legacy_inference_url(&mut config);
            migrate_cloud_provider_slugs(&mut config);
            config.apply_env_overrides_from(env);

            if config_was_corrupted {
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
            let migrated_legacy_secrets = decrypt_config_secrets(&mut config, &openhuman_dir)?;
            if migrated_legacy_secrets {
                // One-time forced migration: a legacy `enc:` (XOR) secret was
                // upgraded to `enc2:` on read. Persist immediately so the
                // insecure ciphertext stops living on disk (audit C8). A save
                // failure is non-fatal — the config is still usable in memory
                // and migration will be retried on the next startup.
                if let Err(e) = config.save().await {
                    log::warn!(
                        "[security][config] failed to persist enc: -> enc2: secret migration; \
                         will retry on next startup: {e}"
                    );
                }
            }
            Ok(config)
        } else {
            let mut config = Config {
                config_path: config_path.clone(),
                workspace_dir,
                action_dir: default_action_dir(),
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
                action_dir: default_action_dir(),
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
        config.action_dir = resolve_action_dir(&config.action_dir_override);
        config.apply_env_overrides();
        // Debug-dump path is read-only; ignore the migration signal (the
        // authoritative `load_or_init` path persists upgraded secrets).
        let _ = decrypt_config_secrets(&mut config, &openhuman_dir)?;
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
                action_dir: default_action_dir(),
                ..Default::default()
            };
            config.apply_env_overrides_from(&ProcessEnvWithoutWorkspace);
            return Ok(config);
        }

        // See the `load_or_init` read branch: a directory at the config path is
        // corruption, not a transient read failure — fail fast with distinct
        // wording so it pages instead of being demoted (#3962, Codex P2).
        if config_path.is_dir() {
            anyhow::bail!(
                "Config path is a directory, not a file: {}",
                config_path.display()
            );
        }

        let raw = fs::read_to_string(&config_path)
            .await
            .with_context(|| format!("reading config.toml from {}", config_path.display()))?;
        let (mut config, config_was_corrupted) =
            parse_config_with_recovery(&config_path, &raw).await;
        config.config_path = config_path;
        config.workspace_dir = workspace_dir;
        config.action_dir = resolve_action_dir(&config.action_dir_override);
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

        super::sync_directory(parent_dir).await?;

        Ok(())
    }
}
