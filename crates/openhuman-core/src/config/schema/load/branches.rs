//! The two on-disk branches of `Config::load_or_init_with_env_lookup`, split
//! out so each is its own boxed future, plus the sync helpers that build a
//! `Config` on the heap. An unoptimised build gives every `Config` temporary
//! its own stack slot, so folding all of this into one async state machine
//! made its poll frame ~440 KB on top of the agent tower (#6379).

use super::super::Config;
use super::dirs::{default_action_dir, resolve_action_dir, ConfigResolutionSource};
use super::env::EnvLookup;
use super::impl_load::{parse_config_boxed, read_config_with_recovery_or_default};
use super::migrate::{
    migrate_cloud_provider_slugs, migrate_legacy_inference_url, migrate_legacy_memory_sources,
    migrate_search_settings,
};
use super::secrets::decrypt_config_secrets;
use anyhow::Result;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use tokio::fs;

static WARNED_WORLD_READABLE_CONFIGS: OnceLock<Mutex<HashSet<std::path::PathBuf>>> =
    OnceLock::new();

/// A freshly defaulted [`Config`] on the heap. Building it in a sync frame of
/// its own keeps the (large, in an unoptimised build) temporaries off the
/// async callers' poll frames (#6379).
#[inline(never)]
pub(super) fn default_config_boxed() -> Box<Config> {
    Box::new(Config::default())
}

/// Pre-login config: defaults plus the env overlay, never persisted.
#[inline(never)]
pub(super) fn pre_login_config_boxed(
    config_path: std::path::PathBuf,
    workspace_dir: std::path::PathBuf,
    env: &(dyn EnvLookup + Send + Sync),
) -> Box<Config> {
    let mut config = Box::new(Config {
        config_path,
        workspace_dir,
        action_dir: default_action_dir(),
        ..Default::default()
    });
    config.apply_env_overrides_from(env);
    config
}

/// A new workspace's config, stamped at the current schema version.
#[inline(never)]
pub(super) fn new_workspace_config_boxed(
    config_path: std::path::PathBuf,
    workspace_dir: std::path::PathBuf,
) -> Box<Config> {
    Box::new(Config {
        config_path,
        workspace_dir,
        action_dir: default_action_dir(),
        schema_version: crate::config::migrations::CURRENT_SCHEMA_VERSION,
        ..Default::default()
    })
}

impl Config {
    /// The branch of [`Self::load_or_init_with_env_lookup`] for a `config.toml`
    /// that exists on disk.
    pub(super) async fn load_existing_config(
        openhuman_dir: std::path::PathBuf,
        workspace_dir: std::path::PathBuf,
        config_path: std::path::PathBuf,
        resolution_source: ConfigResolutionSource,
        env: &(dyn EnvLookup + Send + Sync),
    ) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::{
                fs::Permissions,
                os::unix::fs::{MetadataExt, PermissionsExt},
            };
            if let Ok(meta) = fs::metadata(&config_path).await {
                // SAFETY: `geteuid` takes no arguments, mutates no process
                // state, and is documented as always succeeding.
                let euid = unsafe { libc::geteuid() };
                // Only harden a file we own. Chmod-ing a foreign-owned
                // config either fails with EPERM (log noise) or — with
                // CAP_FOWNER, e.g. a root-run CLI inside a container —
                // succeeds and strips the read bit that was the only thing
                // letting the *other* uid (the one that actually serves
                // requests) open it. Leaving a foreign 0644 config alone is
                // strictly safer: it still loads, and the entrypoint owns
                // repairing ownership.
                if meta.uid() == euid && meta.permissions().mode() & 0o004 != 0 {
                    let warned =
                        WARNED_WORLD_READABLE_CONFIGS.get_or_init(|| Mutex::new(HashSet::new()));
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
                        match fs::set_permissions(&config_path, Permissions::from_mode(0o600)).await
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

        // Use the recovery-aware read path. If the file cannot be read
        // (e.g. non-UTF-8 bytes), the corrupted file is renamed to
        // `.corrupted.<timestamp>` and backup/defaults are attempted,
        // with rate-limited error logging (#5167).
        let (contents, read_was_recovered) =
            read_config_with_recovery_or_default(&config_path).await?;

        // When `read_config_with_recovery_or_default` returned an empty
        // string (both primary and backup were unreadable), skip the TOML
        // parse and use `Config::default()` directly.  An empty TOML would
        // otherwise parse successfully with serde defaults (all fields at
        // their `Option::None` / `vec![]` / `false` values) instead of
        // the richer `Default` impl (issue #5167).
        let (mut config, config_was_corrupted) = if read_was_recovered && contents.is_empty() {
            (default_config_boxed(), true)
        } else {
            Box::pin(parse_config_boxed(&config_path, &contents)).await
        };

        // If the read itself was recovered (non-UTF-8 file renamed, backup
        // used, or file renamed to .corrupted.ts), treat it as corruption so
        // the recovery path below persists the default config.
        let config_was_corrupted = config_was_corrupted || read_was_recovered;
        config.config_path = config_path.clone();
        config.workspace_dir = workspace_dir;
        config.action_dir = resolve_action_dir(&config.action_dir_override);
        // Runtime-only signal consumed once at boot to raise a user-visible
        // "settings were reset" notice (#5167). Set before env overrides so a
        // later override can never mask that recovery happened.
        config.recovered_from_corruption = config_was_corrupted;
        migrate_legacy_inference_url(&mut config);
        migrate_cloud_provider_slugs(&mut config);
        migrate_search_settings(&mut config);
        migrate_legacy_memory_sources(&mut config);
        config.apply_env_overrides_from(env);

        if config_was_corrupted {
            let already_renamed = !tokio::fs::try_exists(&config_path).await.unwrap_or(false);
            if already_renamed {
                // The read helper already renamed the corrupted file to
                // `.corrupted.<ts>` -- just persist the recovered config.
                tracing::debug!(
                    path = %config_path.display(),
                    read_recovered = read_was_recovered,
                    "[config] Config file already renamed by read recovery; \
                     persisting recovered config"
                );
                if let Err(e) = config.save().await {
                    tracing::warn!(
                        path = %config.config_path.display(),
                        error = %e,
                        "[config] Failed to persist recovered config to disk"
                    );
                }
            } else {
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
                             protect the .bak -- will retry recovery on next startup"
                        );
                    }
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
        Box::pin(crate::config::migrations::run_pending(&mut config)).await;
        let migrated_legacy_secrets = decrypt_config_secrets(&mut config, &openhuman_dir)?;
        if migrated_legacy_secrets {
            // One-time forced migration: a legacy `enc:` (XOR) secret was
            // upgraded to `enc2:` on read. Persist immediately so the
            // insecure ciphertext stops living on disk (audit C8). A save
            // failure is non-fatal -- the config is still usable in memory
            // and migration will be retried on the next startup.
            if let Err(e) = config.save().await {
                log::warn!(
                    "[security][config] failed to persist enc: -> enc2: secret migration; \
                         will retry on next startup: {e}"
                );
            }
        }
        Ok(*config)
    }

    /// The branch of [`Self::load_or_init_with_env_lookup`] that creates and
    /// persists a fresh config.
    pub(super) async fn init_new_config(
        workspace_dir: std::path::PathBuf,
        config_path: std::path::PathBuf,
        resolution_source: ConfigResolutionSource,
        env: &(dyn EnvLookup + Send + Sync),
    ) -> Result<Self> {
        let mut config = new_workspace_config_boxed(config_path.clone(), workspace_dir);
        // A workspace created here is stamped at the *current* schema
        // version, so the `run_pending` call below has no gate left to
        // cross — including the `== 1` step that is the only place the
        // managed `openhuman` cloud provider has ever been seeded. Without
        // this, every fresh install starts with `cloud_providers = []` and
        // can never acquire the entry, which is what left real workspaces
        // failing `inference_list_models("openhuman")` before any request
        // was made. Seed before the first `save` so the entry is on disk
        // from the very first write rather than on some later one.
        crate::config::migrations::seed_new_workspace(&mut config);
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
        Box::pin(crate::config::migrations::run_pending(&mut config)).await;
        Ok(*config)
    }
}
