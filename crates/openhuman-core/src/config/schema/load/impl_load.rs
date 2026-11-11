use super::super::Config;
use super::branches::pre_login_config_boxed;
use super::context_refresh::is_document_conflict;
use super::dirs::{
    default_action_dir, default_config_and_workspace_dirs, resolve_action_dir,
    resolve_config_dirs_ignoring_env, resolve_runtime_config_dirs_with, ConfigResolutionSource,
};
use super::env::{EnvLookup, NoEnv, ProcessEnv, ProcessEnvWithoutWorkspace};
use super::migrate::{
    apply_loaded_config_migrations, migrate_components, migrate_legacy_memory_backend,
};
use super::secrets::{
    decrypt_config_secrets, decrypt_config_secrets_strict, encrypt_config_secrets,
};
use anyhow::{Context, Result};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tokio::fs;

pub(crate) use super::config_file_read::parse_config_with_recovery;
pub(crate) use super::config_file_read::CONFIG_OWNER_MISMATCH_MARKER;
pub(super) use super::config_file_read::{
    describe_config_ownership, migration_source, parse_config_boxed, parse_toml_off_worker,
};
pub(super) use super::parse_source::parse_source_contents;

/// Guards the "corrupted config read, resetting to defaults" warning so it
/// fires at most once per process lifetime. Without this, a permanently
/// non-UTF-8 config file floods telemetry with hundreds of identical
/// `stream did not contain valid UTF-8` events (#5167).
static WARNED_CONFIG_READ_FAILURE: OnceLock<Mutex<bool>> = OnceLock::new();

/// Try to read `config_path`. On content corruption (non-UTF-8 bytes), rename
/// the corrupted file to `<config_file>.corrupted.<timestamp>`, try the `.bak`
/// backup, and if that also fails return an empty string so the caller's
/// `parse_config_with_recovery` falls through to defaults.
///
/// Only triggers auto-recovery for **content** corruption (`InvalidData`), not
/// for transient or permission errors (`PermissionDenied`, `NotFound`, etc.),
/// which are propagated as errors so the caller can surface them to the user.
///
/// Rate-limits the warning to at most one per process lifetime so a
/// permanently corrupted file does not flood telemetry (#5167).
pub(super) async fn read_config_with_recovery_or_default(
    config_path: &Path,
) -> Result<(String, bool)> {
    let reads = || async {
        match fs::read_to_string(config_path).await {
            Ok(contents) => Ok(contents),
            Err(error) => {
                let ownership = describe_config_ownership(config_path).await;
                Err(anyhow::Error::new(error).context(format!(
                    "Failed to read config file: {}{ownership}",
                    config_path.display()
                )))
            }
        }
    };

    let result = crate::util::retry_with_backoff_async("read config file", 5, 20, reads).await;
    match result {
        Ok(contents) => Ok((contents, false)),
        Err(e) => {
            let is_content_corruption = e.chain().any(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|ioe| ioe.kind() == std::io::ErrorKind::InvalidData)
            });

            if !is_content_corruption {
                tracing::warn!(
                    path = %config_path.display(),
                    error = %format!("{e:#}"),
                    "[config] failed to read config file"
                );
                return Err(e);
            }

            {
                let warned = WARNED_CONFIG_READ_FAILURE.get_or_init(|| Mutex::new(false));
                let mut guard = warned.lock().unwrap_or_else(|e| e.into_inner());
                if !*guard {
                    tracing::warn!(
                        path = %config_path.display(),
                        error = %e,
                        "[config] Config file contains non-UTF-8 content; \
                         renaming to .corrupted and attempting recovery from backup"
                    );
                    *guard = true;
                }
            }

            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let stem = config_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("config");
            let corrupted_name = format!("{stem}.corrupted.{ts}");
            let corrupted_path = config_path.with_file_name(&corrupted_name);
            if let Err(rename_err) = std::fs::rename(config_path, &corrupted_path) {
                tracing::warn!(
                    src = %config_path.display(),
                    dst = %corrupted_path.display(),
                    error = %rename_err,
                    "[config] Failed to rename corrupted config file; \
                     subsequent loads will fail again"
                );
            }

            let backup_path = config_path.with_extension("toml.bak");
            match fs::read_to_string(&backup_path).await {
                Ok(bak_contents) => {
                    tracing::warn!(
                        path = %config_path.display(),
                        backup = %backup_path.display(),
                        "[config] Read of config file failed; recovered from backup"
                    );
                    Ok((bak_contents, true))
                }
                Err(bak_err) => {
                    tracing::warn!(
                        path = %config_path.display(),
                        backup = %backup_path.display(),
                        error = %bak_err,
                        "[config] Backup also unreadable after failed config read; \
                         resetting to defaults"
                    );
                    Ok((String::new(), true))
                }
            }
        }
    }
}

impl Config {
    pub(crate) async fn load_or_init_unsealed() -> Result<Self> {
        let (openhuman_dir, workspace_dir) = default_config_and_workspace_dirs()?;
        Self::load_or_init_unsealed_with_env_lookup(&openhuman_dir, &workspace_dir, &ProcessEnv)
            .await
    }

    /// Open sealed values after the desktop's approval preflight is serving.
    /// Returns whether the legacy ciphertext format was upgraded.
    pub(crate) async fn decrypt_secrets_after_preflight(&mut self) -> Result<bool> {
        let openhuman_dir = self
            .config_path
            .parent()
            .context("config path has no parent directory")?
            .to_path_buf();
        decrypt_config_secrets(self, &openhuman_dir).await
    }

    pub async fn load_or_init() -> Result<Self> {
        Self::load_or_init_unenriched()
            .await
            .map(crate::config::enrichers::enriched)
    }

    async fn load_or_init_unenriched() -> Result<Self> {
        if let Some(scoped) = super::saas_scope::saas_scoped_config() {
            return scoped;
        }
        let (default_openhuman_dir, default_workspace_dir) = default_config_and_workspace_dirs()?;
        Self::load_or_init_with_env_lookup(
            &default_openhuman_dir,
            &default_workspace_dir,
            &ProcessEnv,
        )
        .await
        .inspect(super::active_workspace::publish_loaded_workspace)
    }

    pub(crate) async fn load_or_init_with_env_lookup(
        default_openhuman_dir: &Path,
        default_workspace_dir: &Path,
        env: &(dyn EnvLookup + Send + Sync),
    ) -> Result<Self> {
        Self::load_or_init_with_env_lookup_mode(
            default_openhuman_dir,
            default_workspace_dir,
            env,
            true,
        )
        .await
    }

    /// Load config structure while leaving encrypted values sealed. Hosts use
    /// this during the restricted security preflight, before the cipher module
    /// and approval surface are available.
    pub(crate) async fn load_or_init_unsealed_with_env_lookup(
        default_openhuman_dir: &Path,
        default_workspace_dir: &Path,
        env: &(dyn EnvLookup + Send + Sync),
    ) -> Result<Self> {
        Self::load_or_init_with_env_lookup_mode(
            default_openhuman_dir,
            default_workspace_dir,
            env,
            false,
        )
        .await
    }

    async fn load_or_init_with_env_lookup_mode(
        default_openhuman_dir: &Path,
        default_workspace_dir: &Path,
        env: &(dyn EnvLookup + Send + Sync),
        decrypt_secrets: bool,
    ) -> Result<Self> {
        let (openhuman_dir, workspace_dir, resolution_source) = Box::pin(
            resolve_runtime_config_dirs_with(default_openhuman_dir, default_workspace_dir, env),
        )
        .await?;

        let config_path = openhuman_dir.join("config.toml");

        // With a shared backend the scope's document is the config even when
        // this node has no local file: returning pre-login defaults here would
        // let the next settings save overwrite that document.
        let source = super::source::for_config(&config_path)?;
        let document_exists = source.label() == "document" && source.exists().await?;
        if resolution_source == ConfigResolutionSource::DefaultConfigDir
            && !config_path.exists()
            && !document_exists
        {
            let config = pre_login_config_boxed(config_path.clone(), workspace_dir.clone(), env);

            tracing::debug!(
                path = %config.config_path.display(),
                workspace = %config.workspace_dir.display(),
                source = resolution_source.as_str(),
                initialized = false,
                persisted = false,
                "Config loaded (pre-login, in-memory only — no dirs or files written)"
            );
            return Ok(*config);
        }

        fs::create_dir_all(&openhuman_dir)
            .await
            .context("Failed to create config directory")?;
        fs::create_dir_all(&workspace_dir)
            .await
            .context("Failed to create workspace directory")?;

        // Each branch is its own boxed future. An unoptimised build gives every
        // `Config` temporary its own stack slot, so folding all three branches
        // into this one state machine made the poll frame ~440 KB and stacked
        // on top of the whole agent tower (#6379).
        // Which source holds the config is decided per call: the file unless a
        // shared backend and a scope are installed. A process boots before its
        // backend is installed, so its first load normally reads the file (the
        // bootstrap config); nothing here enforces that ordering.
        if document_exists || source.exists().await? {
            Box::pin(Self::load_existing_config(
                openhuman_dir,
                workspace_dir,
                config_path,
                resolution_source,
                env,
                decrypt_secrets,
            ))
            .await
        } else {
            Box::pin(Self::init_new_config(
                workspace_dir,
                config_path,
                resolution_source,
                env,
            ))
            .await
        }
    }

    /// Load config from the default user paths, bypassing the
    /// `OPENHUMAN_WORKSPACE` environment variable.
    ///
    /// This is used by the debug dump to load the real user config
    /// for auth token resolution when the dump script overrides
    /// `OPENHUMAN_WORKSPACE` to a throwaway temp directory.
    pub async fn load_from_default_paths() -> Result<Self> {
        Self::load_from_default_paths_unenriched()
            .await
            .map(crate::config::enrichers::enriched)
    }

    async fn load_from_default_paths_unenriched() -> Result<Self> {
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

        // NOTE: no backup recovery here by design -- this is the debug-dump path only;
        // `load_or_init()` is the authoritative startup path that handles corruption.
        // However, we still use `read_config_with_recovery_or_default` to handle the
        // non-UTF-8 case: a corrupted file is renamed to `.corrupted.<ts>` so the next
        // authoritative load can create a fresh config.
        let (raw, _read_was_recovered) =
            Box::pin(read_config_with_recovery_or_default(&config_path)).await?;
        let (mut config, _was_corrupted) = parse_config_with_recovery(&config_path, &raw).await;
        config.config_path = config_path.clone();
        config.workspace_dir = workspace_dir;
        config.action_dir = resolve_action_dir(&config.action_dir_override);
        let migration_raw = migration_source(&config_path, &raw).await;
        migrate_legacy_memory_backend(&mut config, &migration_raw);
        migrate_components(&mut config, &migration_raw);
        config.apply_env_overrides();
        // Debug-dump path is read-only; ignore the migration signal (the
        // authoritative `load_or_init` path persists upgraded secrets).
        let _ = decrypt_config_secrets(&mut config, &openhuman_dir).await?;
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
        Self::load_from_config_path_with_env(
            config_path,
            workspace_dir,
            &ProcessEnvWithoutWorkspace,
        )
        .await
        .map(crate::config::enrichers::enriched)
    }

    /// Reloads the resolved config source and its revision without applying
    /// process environment overrides.
    pub(crate) async fn load_from_config_path_without_env(
        config_path: &Path,
        workspace_dir: &Path,
    ) -> Result<Self> {
        Self::load_from_config_path_with_env(config_path, workspace_dir, &NoEnv)
            .await
            .map(crate::config::enrichers::enriched)
    }

    async fn load_from_config_path_with_env(
        config_path: &Path,
        workspace_dir: &Path,
        env: &(dyn EnvLookup + Send + Sync),
    ) -> Result<Self> {
        let config_path = config_path.to_path_buf();
        let workspace_dir = workspace_dir.to_path_buf();

        // A snapshot reload reads through the config source: the file, or the
        // scope's config document on a shared backend (bootstrap tables still
        // come from the file). The first load of a process does not: see
        // `source`.
        let source = super::source::for_config(&config_path)?;
        if !source.exists().await? {
            let mut config = Config {
                config_path,
                workspace_dir,
                action_dir: default_action_dir(),
                ..Default::default()
            };
            config.apply_env_overrides_from(env);
            return Ok(config);
        }

        // See the `load_or_init` read branch: a directory at the config path is
        // corruption, not a transient read failure -- fail fast with distinct
        // wording so it pages instead of being demoted (#3962, Codex P2).
        if config_path.is_dir() {
            anyhow::bail!(
                "Config path is a directory, not a file: {}",
                config_path.display()
            );
        }

        let read = Box::pin(source.read()).await?;
        let (config, config_was_corrupted) = parse_source_contents(&read, &config_path).await?;
        let super::source::ConfigRead {
            contents: raw,
            recovered: read_was_recovered,
            ..
        } = read;
        let mut config = *config;
        let config_was_corrupted = config_was_corrupted || read_was_recovered;
        config.config_path = config_path.clone();
        config.workspace_dir = workspace_dir;
        config.action_dir = resolve_action_dir(&config.action_dir_override);
        config.recovered_from_corruption = config_was_corrupted;
        let migration_raw = migration_source(&config_path, &raw).await;
        apply_loaded_config_migrations(&mut config, &migration_raw);
        config.apply_env_overrides_from(env);
        // Hand callers the same plaintext secrets the first load does. All or
        // nothing: callers reload, edit and save, so a field that cannot be
        // opened must stay sealed rather than be cleared and saved as `None`.
        if let Some(openhuman_dir) = config.config_path.parent().map(Path::to_path_buf) {
            let mut opened = config.clone();
            match decrypt_config_secrets_strict(&mut opened, &openhuman_dir).await {
                Ok(()) => config = opened,
                Err(error) => tracing::warn!(
                    path = %config.config_path.display(),
                    %error,
                    "[config] snapshot reload keeps secrets sealed"
                ),
            }
        }

        if config_was_corrupted {
            tracing::warn!(
                path = %config.config_path.display(),
                "[config] Snapshot reload recovered a corrupted config; skipping persistence"
            );
        }

        Box::pin(crate::config::migrations::run_pending(&mut config)).await;
        Ok(config)
    }

    pub async fn save(&self) -> Result<()> {
        // A thin shim: the real body's future is ~25 KB, and `save` is awaited
        // from ~15 places in the load and migration paths. An unoptimised build
        // gives each of those await sites its own copy of the callee's future in
        // the caller's poll frame (`migrations::run_pending` alone was ~350 KB),
        // so the shim keeps every such copy at pointer size (#6379).
        Box::pin(self.save_inner()).await
    }

    async fn save_inner(&self) -> Result<()> {
        // Where the text lives is the source's business: the file (atomic
        // replace with a `.bak`) or, on a shared backend, the config document.
        let source = super::source::for_config(&self.config_path)?;
        self.save_to(source.as_ref()).await
    }

    /// Save into `source`. The loader also uses it to rewrite the local
    /// bootstrap file when a document-backed load recovered it.
    pub(super) async fn save_to(&self, source: &dyn super::source::ConfigSource) -> Result<()> {
        self.save_to_inner(source, true).await
    }

    /// Persist a recovered local bootstrap file without changing the
    /// document-backed snapshot that was just recovered and saved.
    pub(super) async fn save_file_export(
        &self,
        source: &dyn super::source::ConfigSource,
    ) -> Result<()> {
        self.save_to_inner(source, false).await
    }

    async fn save_to_inner(
        &self,
        source: &dyn super::source::ConfigSource,
        update_snapshot: bool,
    ) -> Result<()> {
        let source_anchor = source.anchor();
        let expected_snapshot = self.runtime_state.document_snapshot.lock().unwrap().clone();
        let initial_file_import = matches!(
            (&expected_snapshot.anchor, &source_anchor),
            (
                Some(super::source::ConfigSourceAnchor::File(path)),
                super::source::ConfigSourceAnchor::Document { .. }
            ) if path == &super::source::normalized_path(&self.config_path)
                && expected_snapshot.version.is_none()
        );
        if update_snapshot
            && expected_snapshot
                .anchor
                .as_ref()
                .is_some_and(|anchor| anchor != &source_anchor && !initial_file_import)
        {
            return Err(tinystoragedrivers::StorageError::conflict(
                "config snapshot belongs to a different persistence source",
            )
            .into());
        }
        let mut config_to_save = self.clone();
        super::super::cli_overrides::restore_persisted_inference_fields(&mut config_to_save);
        // A document source seals the whole body under the scope's data key;
        // the process-local field key would make it unreadable on another node.
        // A config seeded from the local file (or saved by a migration before
        // the load decrypted it) can still hold process-key `enc2:` fields, so
        // they are opened first: the document must never nest node-local
        // ciphertext.
        if source.encrypts_body() {
            let openhuman_dir = config_to_save
                .config_path
                .parent()
                .context("Config path must have a parent directory")?
                .to_path_buf();
            decrypt_config_secrets_strict(&mut config_to_save, &openhuman_dir).await?;
        } else {
            encrypt_config_secrets(&mut config_to_save).await?;
        }

        let toml_str =
            toml::to_string_pretty(&config_to_save).context("Failed to serialize config")?;

        tracing::debug!(source = source.label(), "[config] saving config");
        let expected = if update_snapshot {
            expected_snapshot.version
        } else {
            None
        };
        let version = match source.write_versioned(&toml_str, expected).await {
            Ok(version) => version,
            Err(error) => {
                if is_document_conflict(&error) {
                    self.refresh_context_after_conflict(&expected_snapshot)
                        .await;
                }
                return Err(error);
            }
        };
        if !update_snapshot {
            return Ok(());
        }
        *self.runtime_state.document_snapshot.lock().unwrap() =
            super::super::ConfigDocumentSnapshot {
                version,
                anchor: Some(source_anchor.clone()),
            };

        if let Some(snapshot) = self
            .runtime_state
            .context_snapshot
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
        {
            let mut current = snapshot.lock().unwrap();
            let current_state = current
                .runtime_state
                .document_snapshot
                .lock()
                .unwrap()
                .clone();
            if current_state == expected_snapshot {
                let published = self.clone();
                *published.runtime_state.document_snapshot.lock().unwrap() =
                    super::super::ConfigDocumentSnapshot {
                        version,
                        anchor: Some(source_anchor),
                    };
                *current = published;
            }
        }

        Ok(())
    }
}
